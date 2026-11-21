//! Read-only checks for a managed server, before the engine or its mods are started.
use crate::engine;
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use sunrise_server_launch::{
    self as launch,
    config::{Config, ModId, Mods},
    mod_lock::{
        LOCK, LockedMod, MAX_LOCK, ModLock as Lock, Native, Reason, ReleaseTag, local_file,
        read_text,
    },
};

use sunrise_mod_install::{
    lock::{BuildId, PackagePath, Sha256},
    transaction::NATIVE_DIR,
};
use sunrise_native_build::{CLIENT, library_record::Library};

#[derive(Deserialize)]
struct NativeRuntime {
    sunrise: BuildId,
    rust: String,
    libraries: Vec<Library>,
}

#[derive(Deserialize)]
struct SdkIdentity {
    commit: BuildId,
    rustc: String,
    side: sunrise_mod_install::sdk::Side,
}

enum Repair<'a> {
    Mods,
    Mod(&'a ModId),
    Engine,
    Release(&'a ReleaseTag),
}

fn refuse_mod(row: &LockedMod, cause: &str) -> Refusal {
    Refusal::new(
        format!("{LOCK} says {} {}, but {cause}", row.id, row.version),
        Repair::Mod(&row.id),
    )
}

fn check_files(lock: &Lock, root: &Path, input: &Mods) -> Result<(), Refusal> {
    if lock.input != *input {
        return Err(
            "mods.lock was resolved from different [mods] settings than server.toml."
                .to_owned()
                .into(),
        );
    }
    for row in &lock.mods {
        let path = row.file.as_path();
        let file = local_file(root, path).map_err(|e| refuse_mod(row, &e))?;
        let hash = engine::sha256_file(&file)
            .map_err(|e| refuse_mod(row, &format!("{}: {e}", path.display())))?;
        if hash != row.sha256.as_str() {
            return Err(refuse_mod(
                row,
                &format!("{} has a different checksum.", path.display()),
            ));
        }
        if row.on && row.native.is_some() && !input.trust_native_code {
            return Err(refuse_mod(
                row,
                "native code is not trusted in server.toml.",
            ));
        }
    }
    Ok(())
}

fn check_native(lock: &Lock, root: &Path) -> Result<Option<PathBuf>, Refusal> {
    let active = lock
        .mods
        .iter()
        .filter(|row| row.on && row.native.is_some());
    let Some(first) = active.clone().next() else {
        if !lock.runtime.is_empty() {
            return Err(
                "mods.lock lists a native runtime with no enabled native mods."
                    .to_owned()
                    .into(),
            );
        }
        return Ok(None);
    };
    let mut expected: std::collections::BTreeMap<_, _> =
        active.map(|row| (row.id.as_str(), row)).collect();
    let error = |cause: String| refuse_mod(first, &cause);
    let build = lock.sunrise.as_str();
    let runtime = Path::new(NATIVE_DIR);
    let program = runtime.join(CLIENT);
    let identity_path = runtime.join("identity.json");
    let record = runtime.join("native-mods.json");
    let source = runtime.join("sources");
    let mut checked = std::collections::BTreeMap::new();
    for file in native::walkdir(&root.join(runtime)).map_err(error)? {
        let path = file
            .strip_prefix(root)
            .expect("native runtime is inside server folder");
        if path.starts_with(&source) {
            continue;
        }
        let name = path
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let name = PackagePath::try_from(name).map_err(error)?;
        let hash = lock.runtime.get(&name).ok_or_else(|| {
            error(format!(
                "{} isn't listed in the native runtime",
                path.display()
            ))
        })?;
        let file = local_file(root, path).map_err(error)?;
        if engine::sha256_file(&file).map_err(|e| error(e.to_string()))? != hash.as_str() {
            return Err(error(format!(
                "{} has a different checksum",
                path.display()
            )));
        }
        checked.insert(path.to_owned(), hash);
    }
    if checked.len() != lock.runtime.len()
        || ![&program, &identity_path, &record]
            .iter()
            .all(|path| checked.contains_key(path.as_path()))
    {
        return Err(error("native runtime files are missing".into()));
    }
    let parse = |path: &Path| read_text(&root.join(path), MAX_LOCK).map_err(error);
    let sdk: SdkIdentity =
        serde_json::from_str(&parse(&identity_path)?).map_err(|e| error(e.to_string()))?;
    let runtime: NativeRuntime =
        serde_json::from_str(&parse(&record)?).map_err(|e| error(e.to_string()))?;
    if sdk.commit != lock.sunrise
        || runtime.sunrise != lock.sunrise
        || sdk.side != sunrise_mod_install::sdk::Side::Server
        || !sdk
            .rustc
            .lines()
            .any(|line| line.strip_prefix("release: ") == Some(runtime.rust.as_str()))
    {
        return Err(error(format!(
            "native runtime is not the server SDK for {build}"
        )));
    }
    let mut patch = false;
    for lib in &runtime.libraries {
        let (id, version) = match lib.kind.as_str() {
            "mod" if !patch => {
                let row = expected.remove(lib.mod_id.as_str()).ok_or_else(|| {
                    error(format!("native library for unexpected mod {}", lib.mod_id))
                })?;
                match &row.native {
                    Some(Native::Built(id)) if id == &lock.sunrise => (),
                    Some(Native::Prebuilt(_)) => (),
                    _ => return Err(refuse_mod(row, "the runtime doesn't match the mod's build")),
                }
                (row.id.as_str(), row.version.to_string())
            }
            "patch" if !patch && lib.mod_id == "sunrise_patch" => {
                patch = true;
                ("sunrise_patch", "0".to_owned())
            }
            _ => {
                return Err(error(format!(
                    "unexpected native library {} ({})",
                    lib.file, lib.kind
                )));
            }
        };
        let name = PackagePath::try_from(lib.file.clone()).map_err(error)?;
        let linked = name.as_path();
        let hash: Sha256 = lib.sha256.clone().try_into().map_err(error)?;
        if linked.components().count() != 1
            || checked.get(&Path::new(NATIVE_DIR).join(linked)) != Some(&&hash)
            || lib.identity
                != sunrise_mod_install::index::registry::builds::library_identity(
                    build,
                    id,
                    &version,
                    &runtime.rust,
                )
        {
            return Err(error(format!(
                "native library {} doesn't match the locked runtime",
                lib.file
            )));
        }
    }
    if let Some(row) = expected.values().next() {
        return Err(refuse_mod(row, "its native library is missing."));
    }
    Ok(Some(root.join(program)))
}

fn write_list(lock: &Lock, mut out: impl Write, as_json: bool) -> std::io::Result<()> {
    if as_json {
        serde_json::to_writer(&mut out, lock)?;
        return writeln!(out);
    }
    if let Some(release) = &lock.release {
        writeln!(out, "Sunrise {release}")?;
    } else {
        writeln!(out, "Sunrise build {}", lock.sunrise)?;
    }
    for row in &lock.mods {
        let on = if row.on { "on" } else { "off" };
        let kind = match &row.native {
            Some(Native::Prebuilt(_)) => "native, prebuilt",
            Some(Native::Built(_)) => "native, built here",
            Some(Native::Inactive) => "native",
            None => "content",
        };
        let why = match row.reason {
            Reason::Requested => "requested".into(),
            Reason::Pack => "from the pack".into(),
            Reason::Required => {
                let parents: Vec<_> = row.required_by.iter().map(|id| id.as_str()).collect();
                format!("needed by {}", parents.join(", "))
            }
        };
        writeln!(out, "  {} {}  {on}  {kind}  ({why})", row.id, row.version)?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct Refusal(String);

impl Refusal {
    fn new(cause: String, repair: Repair<'_>) -> Self {
        let advice = match repair {
            Repair::Mods => "Put the mods back: sunrise-server mods update".to_owned(),
            Repair::Mod(id) => format!("Put it back: sunrise-server mods update {id}"),
            Repair::Engine => "Install a supported engine: sunrise-server update".to_owned(),
            Repair::Release(tag) => {
                format!("Bring them along: sunrise-server update --version {tag}")
            }
        };
        Self(format!("{cause}\n  {advice}"))
    }
}

impl From<String> for Refusal {
    fn from(cause: String) -> Self {
        Self::new(cause, Repair::Mods)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Returns the selected program when a lock is in use, so startup uses exactly what was checked.
pub fn check_start(
    dir: &Path,
    config: &Config,
    install: Option<&launch::Install>,
) -> Result<Option<PathBuf>, Refusal> {
    let Some(input) = &config.mods else {
        return Ok(None);
    };
    let root = fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock = Lock::read(&root)?;
    check_files(&lock, &root, input)?;
    sunrise_mod_install::selection::check_enabled(&root, &lock).map_err(Refusal)?;
    let program = launch::program_in(config, dir, install)
        .map_err(|cause| Refusal::new(cause, Repair::Engine))?;
    if let Some(expected) = &lock.release {
        let selected = install
            .filter(|install| {
                config.engine.path.is_empty()
                    && program.parent() == Some(install.programs().as_path())
            })
            .and_then(launch::Install::current)
            .and_then(|tag| ReleaseTag::try_from(tag).ok());
        match selected.as_ref() {
            Some(tag) if tag != expected => {
                return Err(Refusal::new(
                    format!(
                        "the mods were set up for Sunrise {expected}, but this server now runs {tag}."
                    ),
                    Repair::Release(tag),
                ));
            }
            None if !lock.mods.iter().any(|row| row.on && row.native.is_some()) => {
                eprintln!(
                    "sunrise-server: engine release unknown; cannot compare it with mods.lock ({expected})."
                );
            }
            _ => {}
        }
    }
    let native = check_native(&lock, &root)?;
    if native.is_some()
        && (!config.engine.path.is_empty() || install.and_then(launch::Install::current).is_none())
    {
        return Err(Refusal::new(
            "managed native mods need an installed Sunrise release, not a custom engine.".into(),
            Repair::Engine,
        ));
    }
    warn_takedowns(&root, &lock);
    Ok(Some(native.unwrap_or(program)))
}

mod client_pack;
mod manage;
mod native;
mod toolchain;
pub use manage::{ReleaseChange, carry_release, command, init};

pub fn read_lease(dir: &Path) -> Result<Option<fs::File>, Refusal> {
    sunrise_mod_install::transaction::read_lease(dir).map_err(|error| {
        Refusal(format!(
            "{error}\n  If interrupted: sunrise-server mods rollback"
        ))
    })
}

fn warn_takedowns(root: &Path, lock: &Lock) {
    if let Ok(Some(index)) = sunrise_mod_install::registry::cached(root) {
        for warning in takedowns(&index, lock) {
            eprintln!("{warning}");
        }
    }
}

fn takedowns(index: &sunrise_mod_install::index::Index, lock: &Lock) -> Vec<String> {
    lock.mods
        .iter()
        .filter_map(|row| {
            let entry = index
                .revoked(row.id.as_str(), row.sha256.as_str())
                .or_else(|| match &row.native {
                    Some(Native::Prebuilt(hash)) => index.revoked(row.id.as_str(), hash.as_str()),
                    _ => None,
                })?;
            Some(format!(
                "{} {} was taken down ({}). Remove it: sunrise-server mods remove {}",
                row.id, row.version, entry.reason, row.id
            ))
        })
        .collect()
}

/// Report newly taken down packages without changing the running selection.
pub fn refresh_takedowns(dir: &Path) -> Result<Vec<String>, String> {
    if !dir.join(LOCK).is_file() {
        return Ok(Vec::new());
    }
    let _lease = sunrise_mod_install::transaction::read_lease(dir)?;
    let root = dir.canonicalize().map_err(|e| e.to_string())?;
    let lock = Lock::read(&root)?;
    let index = sunrise_mod_install::registry::refresh(
        &root,
        &manage::index_location(),
        &sunrise_mod_install::registry::Https::default(),
    )?;
    Ok(takedowns(&index, &lock))
}

#[cfg(test)]
mod tests;
