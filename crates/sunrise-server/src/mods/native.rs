//! Stage source bundles, exact author-signed binaries and the signed server SDK runtime. Nothing
//! here is called until the operator has consented to native code.
use super::toolchain;
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};
use sunrise_mod_install::{
    Side,
    index::{native::Bundle, registry::builds},
    mod_list, sdk,
};
use sunrise_mod_install::{
    index::{self, FitVerdict, Index, Project, Version},
    lock::{Native, PackagePath},
    registry::{Fetch, Https},
    selection::{Plan, Target},
    transaction::{NATIVE_DIR, Stage},
};
use sunrise_native_build::{
    self as builder, contract, host, library_record::Library, prebuilt, sdk_fetch,
};

pub fn refusal(
    target: &Target<'_>,
    index: &Index,
    project: &Project,
    version: &Version,
) -> Option<String> {
    if version.api.split('/').nth(1) == Some(target.build.as_str()) {
        return None;
    }
    let verdict = target
        .release
        .as_ref()
        .and_then(|tag| version.verdicts.get(&tag.to_string()))
        .or_else(|| {
            index
                .nightly_build
                .as_ref()
                .filter(|nightly| nightly.commit == target.build.as_str())?;
            version.verdicts.get("nightly")
        });
    let why = match verdict.map(|v| &v.fit) {
        Some(FitVerdict::MadeFor | FitVerdict::Fits) => return None,
        Some(FitVerdict::Rebuild { review }) => {
            format!("changed function bodies: {}", review.join(", "))
        }
        Some(FitVerdict::Breaks { changed }) => {
            format!("changed API items: {}", changed.join(", "))
        }
        Some(FitVerdict::Exact { reason }) => reason.clone(),
        None => "the registry has no fit verdict for that build".into(),
    };
    let release = target
        .release
        .as_ref()
        .map_or_else(|| target.build.to_string(), ToString::to_string);
    Some(format!(
        "{} {} doesn't fit Sunrise {release}: {why}",
        project.id, version.version
    ))
}

thread_local! {
    static BUILD_DIR: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}
static NETWORK: LazyLock<Https> = LazyLock::new(Https::default);

/// The package, verified independently of its index description, decides which side it runs on.
pub fn stage(
    root: &Path,
    stage: &Stage,
    index: &Index,
    fetch: &dyn Fetch,
    plan: &mut Plan<'_>,
) -> Result<(), String> {
    let active = plan
        .lock
        .mods
        .iter()
        .any(|row| row.on && row.native.is_some());
    let runtime = stage.path().join(NATIVE_DIR);
    let mut packages = Vec::new();
    for (n, row) in plan
        .lock
        .mods
        .iter()
        .enumerate()
        .filter(|(_, row)| row.native.is_some())
    {
        let bundle = Bundle::read(&stage.path().join(row.file.as_path()))
            .map_err(|e| format!("{}: {e}", row.id))?;
        if bundle.manifest.side == Side::Client {
            return Err(format!(
                "{} is client-only: players add it to their own profile",
                row.id
            ));
        }
        if row.on {
            packages.push((n, bundle));
        }
    }
    if !active {
        return Ok(());
    }
    let triple = toolchain::host()?;
    let sdk_host = sdk::Asset::host_name(triple, sdk::Side::Server);
    let build = plan.lock.sunrise.as_str();
    let base = sunrise_addresses::SDK_RELEASES;
    let keys = sunrise_release_sign::sdk_keys_for(sunrise_release_sign::REPOSITORY);
    let (sdk, _) = sdk_fetch::index(build, &keys, |tag, file, max| {
        toolchain::read(fetch, &format!("{base}/{tag}/{file}"), max)
    })?;
    let work = tempfile::tempdir_in(stage.path()).map_err(|e| e.to_string())?;
    let slices = prebuilt::Package::at(work.path());
    let mut libraries = Vec::new();
    let mut source = Vec::new();
    for (n, bundle) in &packages {
        let row = &mut plan.lock.mods[*n];
        let artifact = &plan.artifacts[*n];
        // A dependent needs this library at compile time as well as at load time. Build such sets
        // together: published standalone libraries do not carry their Cargo metadata.
        let linked = packages
            .iter()
            .any(|(_, other)| other.manifest.dependencies.contains_key(row.id.as_str()));
        let binary = artifact.version.native_binary(build, &sdk_host).ok();
        if let Some(binary) = binary.filter(|_| !linked && prebuilt::release_source(bundle).is_ok())
        {
            let file = &binary.file;
            let key = artifact
                .project
                .key
                .as_deref()
                .ok_or("no author key for the native archive")?;
            let signature = file
                .signature
                .as_deref()
                .ok_or("no author signature for the native archive")?;
            if let Some(revoked) = index.revoked(row.id.as_str(), &file.sha256) {
                return Err(format!(
                    "{}'s native archive was taken down: {}",
                    row.id, revoked.reason
                ));
            }
            let verify = || -> Result<_, String> {
                let bytes = toolchain::read(fetch, &file.url, file.size)?;
                if bytes.len() as u64 != file.size || index::sha256(&bytes) != file.sha256 {
                    return Err("native archive size or SHA-256 differs from the index".into());
                }
                index::verify_signature_bytes(key, signature, &bytes)?;
                let expected = builds::library_identity(
                    build,
                    row.id.as_str(),
                    &row.version.to_string(),
                    &sdk.toolchain.rust,
                );
                builds::read_archive(bytes, &sdk_host, &expected)
            };
            let (stamp, bytes) = verify().map_err(|e| format!("{}: {e}", row.id))?;
            create(&runtime.join(&stamp.file), &bytes)?;
            libraries.push(Library {
                file: stamp.file,
                sha256: stamp.sha256,
                kind: "mod".into(),
                mod_id: row.id.to_string(),
                identity: stamp.identity,
            });
            row.native = Some(Native::Prebuilt(file.sha256.clone().try_into()?));
            eprintln!(
                "{} {}: prebuilt for {sdk_host}, signature checked",
                row.id, row.version
            );
        } else {
            source.push(*n);
            row.native = Some(Native::Built(plan.lock.sunrise.clone()));
        }
    }
    // Stage canonical lookup paths for the game; only the compiler needs the source files.
    for (_, bundle) in &packages {
        let to = runtime.join(mod_list::NATIVE_DIR).join(&bundle.manifest.id);
        for (name, bytes) in &bundle.files {
            create(&to.join(name), bytes)?;
        }
    }
    let kinds = if source.is_empty() {
        &[sdk::Kind::Runtime][..]
    } else {
        &[sdk::Kind::Runtime, sdk::Kind::Api, sdk::Kind::Engine]
    };
    for &kind in kinds {
        let name = sdk::Asset::name(kind, triple, sdk::Side::Server);
        let url = format!("{base}/sdk-{build}/{name}");
        let asset = sdk.asset(&name).map_err(|e| e.to_string())?;
        eprintln!(
            "Downloading {name} ({} MB)…",
            asset.size.div_ceil(1_000_000)
        );
        sdk_fetch::unpack(
            &sdk,
            &name,
            &work.path().join(kind.name()),
            8 << 30,
            fetch.open(&url)?,
        )?;
    }
    if !source.is_empty() && sdk.toolchain.rust != builder::rust::VERSION {
        return Err(format!(
            "the SDK needs Rust {} but this server's build tool supports {}\n  Install the server tools that ship with this release: sunrise-server update --version {}",
            sdk.toolchain.rust,
            builder::rust::VERSION,
            plan.lock
                .release
                .as_ref()
                .map_or_else(|| build.to_owned(), ToString::to_string)
        ));
    }
    let cache = root.join(".mods-toolchains");
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let toolchain = toolchain::provision(
        &cache,
        fetch,
        &sdk.toolchain.rust,
        triple,
        !source.is_empty(),
    )?;
    let (exe, files) = if source.is_empty() {
        let identity = prebuilt::identity(&slices.runtime)?;
        (
            slices.runtime.join("bin").join(builder::CLIENT),
            prebuilt::runtime_libraries(&toolchain, &identity, &slices.runtime)?,
        )
    } else {
        if !cfg!(windows)
            && !crate::process::command("cc")
                .arg("--version")
                .output()
                .is_ok_and(|result| result.status.success())
        {
            let ids: Vec<_> = source
                .iter()
                .map(|i| plan.lock.mods[*i].id.as_str())
                .collect();
            return Err(format!(
                "{} needs a C linker (cc) to build\n  Install one: sudo apt install build-essential (Fedora: sudo dnf install gcc; macOS: xcode-select --install)\n  Then: sunrise-server mods add {} --trust-native-code",
                ids.join(", "),
                ids.join(" ")
            ));
        }
        let source_dir = work.path().join("sources");
        for (n, bundle) in packages.iter().filter(|(n, _)| source.contains(n)) {
            let to = source_dir.join(plan.lock.mods[*n].id.as_str());
            for (name, bytes) in &bundle.files {
                create(&to.join(name), bytes)?;
            }
        }
        let build_dir = cache.join("build");
        BUILD_DIR.with(|dir| dir.replace(build_dir.clone()));
        host::install(host::Host {
            tr: |text| text,
            root: || BUILD_DIR.with(|dir| dir.borrow().clone()),
            get: |url| NETWORK.open_optional(url),
            mark_used: contract::note_used,
        });
        builder::clear_log()?;
        let build = contract::Plan {
            contract: contract::CONTRACT,
            work: build_dir,
            toolchain,
            package: slices.clone(),
            mods: source_dir,
            target: contract::TargetName::Server,
            registry: Some(true),
            mixins_from_source: false,
        };
        eprintln!(
            "Building {} native mod(s) with Rust {}…",
            source.len(),
            sdk.toolchain.rust
        );
        let mut events = Vec::new();
        contract::run(&build, &mut std::io::empty(), &mut events)
            .map_err(|e| format!("{e}\n  Build log: {}", builder::log_path().display()))?;
        let event = events
            .split(|&b| b == b'\n')
            .rfind(|line| !line.is_empty())
            .ok_or("native build produced no result")?;
        let contract::Event::Built {
            binary,
            libraries: files,
            records,
            ..
        } = serde_json::from_slice(event).map_err(|e| e.to_string())?
        else {
            return Err("native build produced no executable".into());
        };
        libraries.extend(records);
        (binary, files)
    };
    for file in std::iter::once(&exe).chain(&files) {
        let name = file.file_name().ok_or("build output has no name")?;
        let out = runtime.join(name);
        if out.try_exists().map_err(|e| e.to_string())? {
            return Err(format!(
                "{}: native output name is already in use",
                name.to_string_lossy()
            ));
        }
        fs::copy(file, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    let identity = slices.runtime.join("sunrise-prebuilt.json");
    fs::copy(identity, runtime.join("identity.json")).map_err(|e| e.to_string())?;
    // The builder's own records are already dependency-ordered with the patch last. Standalone
    // prebuilts have no links in either direction and precede them.
    let record = serde_json::to_vec(&serde_json::json!({ "sunrise": build, "rust": sdk.toolchain.rust, "libraries": libraries })).map_err(|e| e.to_string())?;
    create(&runtime.join("native-mods.json"), &record)?;
    for entry in walkdir(&runtime)? {
        let path = entry.strip_prefix(stage.path()).expect("staged runtime");
        if path.starts_with(Path::new(NATIVE_DIR).join("sources")) {
            continue;
        }
        let hash = index::sha256_file(&entry)?.try_into()?;
        let name = path
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        plan.lock.runtime.insert(PackagePath::try_from(name)?, hash);
    }
    Ok(())
}

fn create(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(path.parent().expect("file has a folder"))?;
        let mut file = fs::File::create_new(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    write().map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            files.extend(walkdir(&entry.path())?);
        } else if kind.is_file() {
            files.push(entry.path());
        } else {
            return Err(format!("{} is not a regular file", entry.path().display()));
        }
    }
    Ok(files)
}
