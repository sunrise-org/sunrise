//! Export only the registry packages a joining client and server both use. Never export an
//! executable, data override or files from the server's world.
use super::{Lock, check_files};
use std::path::Path;
use sunrise_mod_install::{Package, Side, modpack, selection, spack, transaction};
use sunrise_server_launch::config::Config;

pub(super) fn write(dir: &Path, out: &Path, json: bool) -> Result<(), String> {
    if out.extension().is_none_or(|ext| ext != "spack") {
        return Err("choose a new output ending in .spack\n  Export it: sunrise-server mods client-pack --out players.spack".into());
    }
    let _lease = transaction::read_lease(dir)?;
    let root = dir.canonicalize().map_err(|e| e.to_string())?;
    let input = Config::load(&root)?
        .mods
        .ok_or("no managed mods here\n  Add a mod to export: sunrise-server mods add ID")?;
    let lock = Lock::read(&root)?;
    check_files(&lock, &root, &input).map_err(|e| e.to_string())?;
    selection::check_enabled(&root, &lock)?;
    let manifest = manifest(&root, &lock)?;
    spack::write(out, &manifest).map_err(|e| format!("{e}\n  Choose an unused output path: sunrise-server mods client-pack --out players.spack"))?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "path": out, "mods": manifest.mods })
        );
    } else {
        println!("Wrote {}. Give it to your players.", out.display());
    }
    Ok(())
}

fn manifest(root: &Path, lock: &Lock) -> Result<modpack::Manifest, String> {
    let mut mods = Vec::new();
    for row in lock.mods.iter().filter(|row| row.on) {
        let package = Package::read(&root.join(row.file.as_path())).map_err(|e| {
            format!(
                "{}: {e}\n  Put it back: sunrise-server mods update {}",
                row.id, row.id
            )
        })?;
        if package.manifest.side == Side::Both {
            mods.push(modpack::PackMod {
                id: row.id.to_string(),
                version: row.version.to_string(),
                sha256: row.sha256.to_string(),
                on: true,
            });
        }
    }
    Ok(modpack::Manifest {
        format: modpack::FORMAT,
        id: "server_mods".into(),
        name: "Server mods".into(),
        version: "1.0.0".into(),
        description: String::new(),
        authors: Vec::new(),
        game: sunrise_platform::version::VERSION_ID.into(),
        mods,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};
    use sunrise_mod_install::index;

    #[test]
    fn export_pins_only_enabled_both_side_packages_and_cannot_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let contents = [
            ("waystones", "both", true, true),
            ("spawn", "server", true, false),
            ("colors", "both", false, false),
        ];
        let mut source = "format = 1\nsunrise = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\ninput = { trust_native_code = true }\n".to_owned();
        let mut enabled = String::new();
        let mut wanted = String::new();
        for (id, side, on, native) in contents {
            let package = format!("{id}.smod");
            let package_path = root.join(&package);
            let mut zip = zip::ZipWriter::new(fs::File::create(&package_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("sunrise-mod.toml", options).unwrap();
            write!(
                zip,
                "id='{id}'\nname='{id}'\nversion='1.2.3'\napi='0.2'\nside='{side}'\n"
            )
            .unwrap();
            if native {
                write!(
                    zip,
                    "[native]\ngame='26.3'\nsunrise='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\n"
                )
                .unwrap();
                for (name, text) in [
                    (
                        "Cargo.toml",
                        "[package]\nname='waystones'\nversion='1.2.3'\n",
                    ),
                    ("src/lib.rs", ""),
                ] {
                    zip.start_file(name, options).unwrap();
                    zip.write_all(text.as_bytes()).unwrap();
                }
            }
            zip.finish().unwrap();
            let hash = index::sha256_file(&package_path).unwrap();
            source.push_str(&format!("\n[[mods]]\nid='{id}'\nversion='1.2.3'\nsha256='{hash}'\nurl='https://example.org/{package}'\nfile='{package}'\nreason='requested'\non={on}\n"));
            if native {
                source
                    .push_str("native = { built = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }\n");
                wanted = hash;
            }
            enabled.push_str(&format!("mod.{id}:{on}\n"));
        }
        fs::write(root.join("mods.lock"), &source).unwrap();
        fs::write(
            root.join("server.toml"),
            "[mods]\ntrust_native_code = true\n",
        )
        .unwrap();
        fs::write(root.join("sunrise-mods.txt"), &enabled).unwrap();
        let out = root.join("players.spack");
        write(&root, &out, true).unwrap();
        let pack = modpack::Modpack::read(&out).unwrap();
        assert_eq!(
            pack.manifest.mods,
            [modpack::PackMod {
                id: "waystones".into(),
                version: "1.2.3".into(),
                sha256: wanted,
                on: true
            }]
        );
        assert!(pack.overrides.is_empty());
        let original = fs::read(&out).unwrap();
        assert!(write(&root, &out, true).is_err());
        assert_eq!(fs::read(&out).unwrap(), original);
    }
}
