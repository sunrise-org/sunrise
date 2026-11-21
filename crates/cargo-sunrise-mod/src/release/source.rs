//! Check, test and pack a tagged source release with its reviewed library mods.
use super::{plan, prepare::write_files};
use crate::{PackageOutput, bounded, options::Options};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    process::Command,
};
use sunrise_mod_package::{
    Package, dependencies,
    index::{self, FitVerdict, Index, Kind, SunriseRelease, Trust, native, resolve},
    manifest::Side,
    mod_surface::child::ChildEntry,
};
use sunrise_native_build::{Bundle, api, registry_generate};

pub(super) const USAGE: &str = "release source --tag <vVERSION> --releases <file> [--index <file> [--files <dir>]] [--update-lock]";

pub(super) fn run(entry: ChildEntry, project: &Path, args: &[String]) -> Result<(), String> {
    let mut flags = Options::new(
        "release source",
        args,
        &["--tag", "--releases", "--index", "--files"],
    )
    .usage(USAGE);
    let tag = flags.value("--tag")?.ok_or("release source needs --tag")?;
    let releases = flags
        .value("--releases")?
        .ok_or("release source needs --releases")?;
    let published = flags.value("--index")?;
    let files = flags.value("--files")?.map(Path::new);
    let update = flags.switch("--update-lock");
    flags.finish()?;
    if files.is_some() && published.is_none() {
        return Err("--files needs --index".into());
    }
    let releases = super::releases(Path::new(releases))?;
    let package = Package::read(project)?;
    let manifest = &package.manifest;
    let (_, release) = plan::tagged_source(manifest, tag, &releases)?;
    if manifest.dependencies.is_empty() {
        return Err("release source needs mod dependencies in sunrise-mod.toml".into());
    }
    let output = PackageOutput::in_project(project)?;
    let lock = project.join("Cargo.lock");
    let lock_hint = "Prepare and review Cargo.lock with `cargo sunrise-mod release source --update-lock` (with the same tag and releases).";
    if !update && !lock.is_file() {
        return Err(format!("{} has no Cargo.lock. {lock_hint}", manifest.id));
    }
    let index = match published {
        Some(path) => bounded::read(Path::new(path), index::MAX_INDEX)?,
        None => {
            let url = format!("{}index.json", crate::registry::SITE);
            let mut text = String::new();
            sunrise_proxy::agent("cargo-sunrise-mod", &url, std::convert::identity)
                .get(&url)
                .call()
                .map_err(|e| format!("{url}: {e}"))?
                .into_reader()
                .take(index::MAX_INDEX + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("{url}: {e}"))?;
            if text.len() as u64 > index::MAX_INDEX {
                return Err(format!("{url}: index exceeds its size limit"));
            }
            text
        }
    };
    let index = Index::parse(&index)?;
    let game = index::stable_tag(&release.tag)
        .expect("validated stable tag")
        .0;
    // A source release has one lock and one build job, so include requirements for both sides.
    let selected = resolve::for_manifest(&index, manifest, game, Side::Both, |_, version| {
        fits(&index, version, release)
    })?;
    fs::create_dir_all(&output.0).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(&output.0).map_err(|e| e.to_string())?;
    let root = stage.path().join(&manifest.id);
    write_files(&root, package.directory_files(project)?)?;
    let mut bundles = vec![Bundle {
        manifest: manifest.clone(),
        dir: root.clone(),
    }];
    for (dependency, row) in selected {
        if dependency.kind != Kind::NativeMod {
            continue;
        }
        let download = tempfile::NamedTempFile::new_in(stage.path()).map_err(|e| e.to_string())?;
        let failed = |why| format!("{} {}: {why}", dependency.id, row.version);
        let input: Box<dyn Read> = match files {
            Some(files) => Box::new(
                fs::File::open(files.join(&row.file.sha256)).map_err(|e| failed(e.to_string()))?,
            ),
            None => {
                sunrise_proxy::agent("cargo-sunrise-mod", &row.file.url, std::convert::identity)
                    .get(&row.file.url)
                    .call()
                    .map_err(|e| failed(e.to_string()))?
                    .into_reader()
            }
        };
        io::copy(&mut input.take(row.file.size + 1), &mut download.as_file())
            .map_err(|e| failed(e.to_string()))?;
        if index::check_file(&index, dependency, row, download.path()).map_err(failed)?
            != Trust::Verified
        {
            return Err(failed("library source has no author signature".into()));
        }
        let source = native::Bundle::read(download.path()).map_err(failed)?;
        let dir = stage.path().join(&dependency.id);
        write_files(&dir, source.files)?;
        bundles.push(Bundle {
            manifest: source.manifest,
            dir,
        });
        println!("Using library {} {}", dependency.id, row.version);
    }
    let projects = bundles
        .iter()
        .map(api::Project::read)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let siblings = format!("../{}/", manifest.id);
    for mod_project in &projects {
        let bundle = mod_project.bundle;
        let declared = dependencies::build_dependencies(&bundle.manifest, Side::Both, |id| {
            projects.iter().any(|p| p.bundle.manifest.id == id)
        });
        let targets = declared
            .iter()
            .map(|dependency| dependencies::Package {
                package: projects
                    .iter()
                    .find(|p| p.bundle.manifest.id == dependency.id)
                    .map(|p| p.package.as_str()),
                dependency,
            })
            .collect::<Vec<_>>();
        let is_root = bundle.manifest.id == manifest.id;
        registry_generate::stage_for_release(
            bundle,
            &targets,
            if is_root { "" } else { &siblings },
            is_root,
        )?;
    }
    let reviewed = root.join("Cargo.lock");
    let original = if update {
        None
    } else {
        Some(fs::read(&reviewed).map_err(|e| format!("Cargo.lock: {e}"))?)
    };
    crate::sdk_cmds::check_mod(entry, &root, false, !update)
        .map_err(|e| format!("{e}\n{lock_hint}"))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .args(["test", "--locked"])
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cargo test --locked: {e}"))?;
    if !status.success() {
        return Err(format!("cargo test --locked failed: {status}"));
    }
    let checked = fs::read(&reviewed).map_err(|e| format!("Cargo.lock: {e}"))?;
    if let Some(original) = original {
        if original != checked
            || fs::read(&lock).map_err(|e| format!("Cargo.lock: {e}"))? != checked
        {
            return Err("the release build changed the reviewed Cargo.lock".into());
        }
    } else {
        crate::internals::atomic_write(&lock, checked)?;
    }
    // Use the author's manifest and lock, never a staged manifest containing generated paths.
    let path = crate::build(entry, project, &package, &output, None)?;
    println!("Built {}", path.display());
    Ok(())
}

fn fits(index: &Index, version: &index::Version, target: &SunriseRelease) -> bool {
    index.releases.as_deref().is_some_and(|releases| {
        releases.iter().any(|r| r == target)
            && version.verdicts.get(&target.tag).map_or_else(
                || version.sunrise.as_ref() == Some(&target.tag),
                |row| matches!(row.fit, FitVerdict::MadeFor | FitVerdict::Fits),
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_mod_package::Manifest;

    #[test]
    fn tagged_source_resolves_required_transitive_libraries_only_when_they_fit() {
        let index = Index::parse(include_str!(
            "../../../sunrise-mod-package/tests/index/stable-native/schema-1.json"
        ))
        .unwrap();
        let releases = index.releases.as_deref().unwrap();
        let manifest = Manifest::parse(
            "id = 'unpublished'\nname = 'Unpublished'\nversion = '1.0.0'\n\
             [dependencies]\ndependent = '^1'\nindependent = { version = '^1', optional = true }",
        )
        .unwrap();
        let select = |release| {
            resolve::for_manifest(&index, &manifest, "26.3", Side::Both, |_, version| {
                fits(&index, version, release)
            })
        };
        let names: Vec<_> = select(&releases[0])
            .unwrap()
            .into_iter()
            .map(|(project, version)| (project.id.as_str(), version.version.as_str()))
            .collect();
        assert_eq!(names, [("dependent", "1.0.0"), ("release_probe", "1.0.0")]);
        assert!(select(&releases[1]).is_err());
    }
}
