//! Write the same pinned `.spack` format read by the registry and mod tools.
use crate::modpack::{MANIFEST, Manifest};
use std::{io::Write, path::Path};

pub fn write(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let text = toml::to_string(manifest).map_err(|e| e.to_string())?;
    if text.len() > 1 << 20 {
        return Err(format!("{MANIFEST} exceeds its size limit"));
    }
    Manifest::parse(&text)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut stage = tempfile::NamedTempFile::new_in(parent)?;
        let mut zip = zip::ZipWriter::new(stage.as_file_mut());
        zip.start_file(MANIFEST, zip::write::SimpleFileOptions::default())?;
        zip.write_all(text.as_bytes())?;
        zip.finish()?.sync_all()?;
        stage.persist_noclobber(path)?;
        Ok(())
    })();
    result.map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modpack::{FORMAT, Modpack, PackMod};

    #[test]
    fn written_pack_reads_back_and_does_not_clobber_an_existing_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smp.spack");
        let mut manifest = Manifest {
            format: FORMAT,
            id: "smp".into(),
            name: "SMP".into(),
            version: "1.0.0".into(),
            description: String::new(),
            authors: Vec::new(),
            game: "26.3".into(),
            mods: vec![PackMod {
                id: "rails".into(),
                version: "1.2.3".into(),
                sha256: "a".repeat(64),
                on: true,
            }],
        };
        write(&path, &manifest).unwrap();
        let original = manifest.clone();
        manifest.mods.clear();
        assert!(write(&path, &manifest).is_err());
        assert_eq!(Modpack::read(&path).unwrap().manifest, original);
    }
}
