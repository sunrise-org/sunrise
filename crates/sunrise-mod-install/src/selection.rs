//! Adapt the registry's resolver to a managed set, retaining the caller's requested inputs.
use crate::{
    Side,
    index::{self, Index, Kind, Project, Version, resolve},
    input::{ModId, Mods, Pack},
    lock::{self, BuildId, LockedMod, ModLock, Native, PackagePath, Reason, ReleaseTag},
    modpack::{self, Modpack},
    registry::{self, Fetch},
    transaction::{self, PACK_FILE, Stage},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use sunrise_mod_package::{
    manifest::{STORAGE_DIR, parse_version},
    mod_list::{self, ModList},
};

/// Target game/build supplied by the application, not inferred from an artifact's own metadata.
pub struct Target<'a> {
    pub game: &'a str,
    pub build: BuildId,
    pub release: Option<ReleaseTag>,
    pub side: Side,
}

/// What the operator supplied, before resolving a registry pack to its exact version.
pub enum PackSource {
    Newest(ModId),
    Exact(Pack),
    Url(url::Url),
}

impl TryFrom<String> for PackSource {
    type Error = String;
    fn try_from(text: String) -> Result<Self, String> {
        if text.starts_with("https://") {
            return url::Url::parse(&text)
                .map(Self::Url)
                .map_err(|e| format!("{text:?}: {e}"));
        }
        match ModId::try_from(text.clone()) {
            Ok(id) => Ok(Self::Newest(id)),
            Err(_) => Pack::try_from(text).map(Self::Exact),
        }
    }
}

/// Read one pack into the transaction. Reuse the staged, verified archive for both resolution and
/// overrides. Files/URLs supplied by the operator still pin every package via the current index.
pub fn stage_pack(
    root: &Path,
    stage: &Stage,
    fetch: &dyn Fetch,
    index: &Index,
    game: &str,
    source: PackSource,
    keep_cached: bool,
) -> Result<(Pack, Modpack), String> {
    let out = stage.path().join(PACK_FILE);
    let registry_pack = |id: &ModId, wanted: Option<&semver::Version>| {
        let project = index
            .project(id.as_str())
            .filter(|p| p.kind == Kind::Modpack)
            .ok_or_else(|| format!("{id} isn't a Sunrise registry pack"))?;
        let version = project
            .versions
            .iter()
            .find(|version| {
                version.game == game
                    && version.capabilities.unknown().is_empty()
                    && index.revoked(id.as_str(), &version.file.sha256).is_none()
                    && wanted.is_none_or(|wanted| {
                        parse_version(&version.version).is_ok_and(|v| v == *wanted)
                    })
            })
            .ok_or_else(|| format!("{id} has no matching pack for Minecraft {game}"))?;
        let cached = root.join(PACK_FILE);
        registry::checked_file(
            fetch,
            index,
            project,
            version,
            keep_cached.then_some(cached.as_path()),
            &out,
        )?;
        Pack::try_from(format!("{id}@{}", version.version))
    };
    let pin = match source {
        PackSource::Newest(id) => registry_pack(&id, None)?,
        PackSource::Exact(Pack::Registry { id, version }) => {
            registry_pack(&id, Some(version.value()))?
        }
        PackSource::Exact(Pack::File(path)) => {
            let cached = root.join(PACK_FILE);
            let file = if keep_cached && cached.is_file() {
                cached
            } else {
                root.join(&path)
            };
            let copy = || -> std::io::Result<()> {
                let mut source = fs::File::open(&file)?;
                if !source.metadata()?.is_file() {
                    return Err(std::io::Error::other("not a regular file"));
                }
                let mut output = fs::File::create_new(&out)?;
                let copied = std::io::copy(
                    &mut std::io::Read::take(&mut source, index::MAX_FILE + 1),
                    &mut output,
                )?;
                if copied > index::MAX_FILE {
                    return Err(std::io::Error::other("pack exceeds its size limit"));
                }
                output.sync_all()
            };
            copy().map_err(|e| format!("{}: {e}", file.display()))?;
            Pack::File(path)
        }
        PackSource::Url(url) => {
            registry::download(fetch, url.as_str(), index::MAX_FILE, &out)?;
            Pack::File(PACK_FILE.into())
        }
    };
    let pack = Modpack::read(&out)?;
    if pack.manifest.game != game {
        return Err(format!(
            "{} needs Minecraft {}, not {game}",
            pack.manifest.id, pack.manifest.game
        ));
    }
    Ok((pin, pack))
}

pub struct Artifact<'a> {
    pub project: &'a Project,
    pub version: &'a Version,
}

pub struct Plan<'a> {
    pub lock: ModLock,
    /// In lock row order, including off mods, so installation verifies all selected bytes.
    pub artifacts: Vec<Artifact<'a>>,
    /// A pack can contain client-only members; none of their files or overrides go on a server.
    pub skipped_pack_members: Vec<String>,
}

/// Which installed versions not to prefer. A named update may still move a dependency if its
/// current version cannot satisfy the new set. Everything else keeps its installed version.
pub enum Refresh<'a> {
    None,
    All,
    Only(&'a [ModId]),
}

/// Resolve the whole set or refuse it, even if the underlying resolver has a partial plan.
pub fn plan<'a>(
    index: &'a Index,
    target: &Target<'_>,
    mut input: Mods,
    pack: Option<&modpack::Manifest>,
    old: Option<&ModLock>,
    refresh: Refresh<'_>,
    native_refusal: impl Fn(&Project, &Version) -> Option<String>,
) -> Result<Plan<'a>, String> {
    fn fits_side(install: Side, declared: Option<Side>) -> bool {
        !matches!((install, declared), (Side::Server, Some(Side::Client)))
    }
    let mut requests = Vec::new();
    let mut ranges = BTreeMap::<_, Vec<_>>::new();
    for request in &input.list {
        let id = request.id.as_str();
        if let Some(range) = request.range() {
            ranges.entry(id).or_default().push(range);
        }
        requests.push(resolve::Request {
            id: id.into(),
            version: None,
        });
    }
    let mut from_pack = BTreeSet::new();
    let mut skipped_pack_members = Vec::new();
    let mut defaults_off = Vec::new();
    for member in pack.into_iter().flat_map(|pack| &pack.mods) {
        let project = index
            .project(&member.id)
            .filter(|p| matches!(p.kind, Kind::Mod | Kind::NativeMod))
            .ok_or_else(|| format!("{}: pack member is not in the registry", member.id))?;
        let pinned = parse_version(&member.version).map_err(|e| format!("{}: {e}", member.id))?;
        let exact = project
            .versions
            .iter()
            .find(|v| {
                v.file.sha256 == member.sha256
                    && parse_version(&v.version).is_ok_and(|version| version == pinned)
            })
            .ok_or_else(|| {
                format!(
                    "{} {}: pack member's exact version and hash are not in the registry",
                    member.id, member.version
                )
            })?;
        if !fits_side(target.side, exact.side) {
            skipped_pack_members.push(member.id.clone());
            continue;
        }
        if !member.on
            && old.is_none_or(|old| !old.mods.iter().any(|row| row.id.as_str() == member.id))
        {
            defaults_off.push(ModId::try_from(member.id.clone())?);
        }
        from_pack.insert(member.id.as_str());
        requests.push(resolve::Request {
            id: member.id.clone(),
            version: Some(exact.version.clone()),
        });
    }
    let prefer: Vec<_> = old
        .into_iter()
        .flat_map(|lock| &lock.mods)
        .filter(|row| match refresh {
            Refresh::None => true,
            Refresh::All => false,
            Refresh::Only(ids) => !ids.contains(&row.id),
        })
        .filter_map(|row| {
            // A lock serializes canonical SemVer; the registry allows abbreviated labels.
            let project = index.project(row.id.as_str())?;
            let version = project.versions.iter().find(|v| {
                v.file.sha256 == row.sha256.as_str()
                    && parse_version(&v.version).ok().as_ref() == Some(&row.version)
            })?;
            Some(resolve::InstalledMod {
                id: project.id.clone(),
                version: version.version.clone(),
                sha256: Some(version.file.sha256.clone()),
            })
        })
        .collect();
    let resolved = resolve::install_where(
        index,
        &requests,
        &prefer,
        target.game,
        target.side,
        |_, _| true,
        |p, v| {
            if !fits_side(target.side, v.side) {
                return Some(format!(
                    "{} is {}-only",
                    p.id,
                    if target.side == Side::Server {
                        "client"
                    } else {
                        "server"
                    }
                ));
            }
            if let Some(ranges) = ranges.get(p.id.as_str())
                && !parse_version(&v.version)
                    .is_ok_and(|version| ranges.iter().all(|range| range.matches(&version)))
            {
                return Some(format!(
                    "{} {} does not satisfy {}",
                    p.id,
                    v.version,
                    ranges
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" and ")
                ));
            }
            if p.kind == Kind::NativeMod {
                native_refusal(p, v)
            } else {
                None
            }
        },
    );
    let errors: BTreeSet<_> = resolved
        .requested
        .values()
        .filter_map(|result| match result {
            resolve::RequestResult::Refused { reason } => Some(reason.as_str()),
            _ => None,
        })
        .collect();
    if !errors.is_empty() {
        return Err(errors.into_iter().collect::<Vec<_>>().join("\n"));
    }
    let selection: BTreeMap<_, _> = resolved
        .install
        .iter()
        .chain(&resolved.kept)
        .map(|v| (v.id.as_str(), v.version.as_str()))
        .chain(
            resolved
                .update
                .iter()
                .map(|v| (v.id.as_str(), v.to.as_str())),
        )
        .collect();
    drop(ranges);
    for id in defaults_off {
        if !input.off.contains(&id) {
            input.off.push(id);
        }
    }
    input.off.retain(|id| selection.contains_key(id.as_str()));
    let off: BTreeSet<_> = input.off.iter().map(ModId::as_str).collect();
    let asked: BTreeSet<_> = input.list.iter().map(|r| r.id.as_str()).collect();
    let mut rows = Vec::new();
    let mut artifacts = Vec::new();
    for (id, version) in selection {
        let project = index
            .project(id)
            .ok_or_else(|| format!("{id}: no registry entry"))?;
        let version = project
            .versions
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| format!("{id} {version}: no registry file"))?;
        let parents = resolved
            .required_by
            .get(id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let on = !off.contains(id);
        if !on && let Some(parent) = parents.iter().find(|p| !off.contains(p.as_str())) {
            return Err(format!(
                "{parent} needs {id}, so it cannot be off\n  Enable it: sunrise-server mods enable {id}"
            ));
        }
        let reason = if from_pack.contains(id) {
            Reason::Pack
        } else if asked.contains(id) {
            Reason::Requested
        } else {
            Reason::Required
        };
        let native = project.kind == Kind::NativeMod;
        let folder = if native {
            "native/sources"
        } else {
            mod_list::DIR
        };
        rows.push(LockedMod {
            id: ModId::try_from(id.to_owned())?,
            version: parse_version(&version.version)?,
            sha256: version.file.sha256.clone().try_into()?,
            url: version.file.url.clone().try_into()?,
            file: PackagePath::try_from(format!("{folder}/{id}.smod"))?,
            reason,
            required_by: if reason == Reason::Required {
                parents
                    .iter()
                    .cloned()
                    .map(ModId::try_from)
                    .collect::<Result<_, _>>()?
            } else {
                Vec::new()
            },
            on,
            native: native.then_some(Native::Inactive),
        });
        artifacts.push(Artifact { project, version });
    }
    Ok(Plan {
        lock: ModLock::new(target.build.clone(), target.release.clone(), input, rows),
        artifacts,
        skipped_pack_members,
    })
}

#[derive(Serialize)]
pub struct Change<'a> {
    pub id: &'a ModId,
    pub from: Option<&'a semver::Version>,
    pub to: Option<&'a semver::Version>,
    pub on: Option<bool>,
}

pub fn changes<'a>(old: Option<&'a ModLock>, next: Option<&'a ModLock>) -> Vec<Change<'a>> {
    let before: BTreeMap<_, _> = old
        .into_iter()
        .flat_map(|l| &l.mods)
        .map(|r| (&r.id, r))
        .collect();
    let after: BTreeMap<_, _> = next
        .into_iter()
        .flat_map(|l| &l.mods)
        .map(|r| (&r.id, r))
        .collect();
    let ids: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
    ids.into_iter()
        .filter_map(|id| {
            let from = before.get(id);
            let to = after.get(id);
            if let (Some(from), Some(to)) = (from, to)
                && from.version == to.version
                && from.sha256 == to.sha256
                && from.on == to.on
            {
                return None;
            }
            Some(Change {
                id,
                from: from.map(|r| &r.version),
                to: to.map(|r| &r.version),
                on: to.map(|r| r.on),
            })
        })
        .collect()
}

/// Preserve manual files and the server's mod data, then put verified package files into a fresh
/// generation. All files in the lock, including disabled files, are checked again before commit.
pub fn stage_content(
    root: &Path,
    stage: &Stage,
    fetch: &dyn Fetch,
    index: &Index,
    old: Option<&ModLock>,
    plan: &Plan<'_>,
    apply_overrides: bool,
) -> Result<(), String> {
    let dir = stage.path().join(mod_list::DIR);
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let old_files: BTreeSet<_> = old
        .into_iter()
        .flat_map(|l| &l.mods)
        .map(|r| r.file.as_path())
        .collect();
    match fs::read_dir(root.join(mod_list::DIR)) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                if !old_files.contains(Path::new(mod_list::DIR).join(entry.file_name()).as_path()) {
                    transaction::copy_tree(&entry.path(), &dir.join(entry.file_name()))
                        .map_err(|e| format!("{}: {e}", entry.path().display()))?;
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", mod_list::DIR)),
    }
    for (row, artifact) in plan.lock.mods.iter().zip(&plan.artifacts) {
        let path = row.file.as_path();
        let out = stage.path().join(path);
        if out.try_exists().map_err(|e| e.to_string())? || dir.join(row.id.as_str()).is_dir() {
            return Err(format!(
                "{}: a manually installed mod uses that name",
                path.display()
            ));
        }
        let current = old_files
            .contains(path)
            .then(|| lock::local_file(root, path).ok())
            .flatten();
        fs::create_dir_all(out.parent().expect("package folder")).map_err(|e| e.to_string())?;
        registry::checked_file(
            fetch,
            index,
            artifact.project,
            artifact.version,
            current.as_deref(),
            &out,
        )?;
    }
    if apply_overrides && stage.path().join(PACK_FILE).is_file() {
        let prefix = format!("{}/{STORAGE_DIR}/", mod_list::DIR);
        Modpack::extract_overrides_where(&stage.path().join(PACK_FILE), stage.path(), |name| {
            name.strip_prefix(&prefix)
                .and_then(|tail| tail.split_once('/'))
                .is_some_and(|(id, _)| plan.lock.mods.iter().any(|row| row.id.as_str() == id))
        })?;
    }
    Ok(())
}

/// Refuse an on/off selector that doesn't match the lock. Only the installed set's rows
/// belong to the installer; manual entries keep their existing settings.
pub fn check_enabled(root: &Path, lock: &ModLock) -> Result<(), String> {
    let list = ModList::load(root);
    for row in &lock.mods {
        let id = row.id.as_str();
        if list.enabled.contains(id) != row.on || list.disabled.contains(id) == row.on {
            return Err(format!(
                "{} says {} is {}, but {} disagrees\n  Put it back: sunrise-server mods update {}",
                lock::LOCK,
                row.id,
                if row.on { "on" } else { "off" },
                mod_list::FILE,
                row.id
            ));
        }
    }
    Ok(())
}

pub fn stage_records(
    root: &Path,
    stage: &Stage,
    old: Option<&ModLock>,
    lock: &ModLock,
    config: &str,
) -> Result<(), String> {
    let settings = transaction::config_with_mods(config, &lock.input)?;
    let wire = toml::to_string(lock).map_err(|e| e.to_string())?;
    if wire.len() as u64 > lock::MAX_LOCK {
        return Err("the mod lock exceeds its size limit".into());
    }
    let mut list = ModList::load(root);
    for row in old.into_iter().flat_map(|l| &l.mods).chain(&lock.mods) {
        list.enabled.remove(row.id.as_str());
        list.disabled.remove(row.id.as_str());
    }
    for row in &lock.mods {
        let set = if row.on {
            &mut list.enabled
        } else {
            &mut list.disabled
        };
        set.insert(row.id.to_string());
    }
    for (name, text) in [
        ("server.toml", settings),
        (lock::LOCK, wire),
        (mod_list::FILE, list.serialize()),
    ] {
        transaction::write_record(root, stage.path(), name, &text)?;
    }
    Ok(())
}
