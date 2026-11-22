//! Authentication and bounded extraction shared by native SDK consumers.
use std::{io::Read, path::Path};
use sunrise_mod_package::sdk::{self, Index};
use sunrise_release_sign::PublicKey;

/// Download the signed index of exactly this build, leaving mirror and network policy to the caller.
pub fn index(
    build: &str,
    keys: &[PublicKey],
    read: impl Fn(&str, &str, u64) -> Result<Vec<u8>, String>,
) -> Result<(Index, Vec<u8>), String> {
    const SMALL: u64 = 16 << 20;
    let tag = format!("sdk-{build}");
    let sums = read(&tag, sunrise_release_sign::SUMS, SMALL)
        .map_err(|e| format!("game build {build} has no signed SDK (release {tag}): {e}"))?;
    let signature = read(&tag, sunrise_release_sign::SIGNATURE, SMALL)?;
    let expected = sunrise_release_sign::sdk_digest(
        build,
        sdk::INDEX,
        Some(&sums),
        Some(std::str::from_utf8(&signature).map_err(|e| format!("{tag} signature: {e}"))?),
        keys,
    )?;
    let bytes = read(&tag, sdk::INDEX, SMALL)?;
    if sunrise_mod_package::index::sha256(&bytes) != expected {
        return Err(format!(
            "{tag}: its {} is not the one its signed {} list",
            sdk::INDEX,
            sunrise_release_sign::SUMS
        ));
    }
    Index::read(&bytes, build)
        .map(|index| (index, bytes))
        .map_err(|e| format!("{tag}: {e}"))
}

/// Stream one archive, then verify the same open bytes before decoding it.
pub fn unpack(
    index: &Index,
    name: &str,
    into: &Path,
    limit: u64,
    body: impl Read,
) -> Result<(), String> {
    let size = index.asset(name).map_err(|e| e.to_string())?.size;
    let mut archive =
        tempfile::NamedTempFile::new_in(into.parent().ok_or("SDK root has no parent")?)
            .map_err(|e| e.to_string())?;
    std::io::copy(&mut body.take(size.saturating_add(1)), &mut archive)
        .map_err(|e| format!("{name}: {e}"))?;
    // On Windows the downloader must close its writer before the decoder opens the file.
    let archive = archive.into_temp_path();
    sdk::unpack(index, name, &archive, into, limit).map_err(|e| e.to_string())?;
    archive.close().map_err(|e| format!("{name}: {e}"))
}
