//! A server-local Rust toolchain, downloaded only after native-code consent. Rust's official
//! HTTPS distribution supplies the component checksums. Install only regular payload files;
//! the archives' installer scripts are never run.
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path},
};
use sunrise_mod_install::{
    lock::Sha256,
    registry::{self, Fetch},
};
use sunrise_native_build::rust::{Unpacked, WINDOWS_X64_TARGET};

pub fn host() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if !cfg!(target_env = "musl") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("windows", "x86_64") => Ok(WINDOWS_X64_TARGET),
        (os, arch) => Err(format!(
            "the native server SDK has no build for {arch} {os}"
        )),
    }
}

pub fn read(fetch: &dyn Fetch, url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fetch
        .open(url)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(format!("{url} exceeds its size limit"));
    }
    Ok(bytes)
}

pub fn provision(
    cache: &Path,
    fetch: &dyn Fetch,
    rust: &str,
    target: &str,
    build: bool,
) -> Result<Unpacked, String> {
    let release =
        semver::Version::parse(rust).map_err(|e| format!("invalid SDK Rust release: {e}"))?;
    if !release.pre.is_empty() || !release.build.is_empty() {
        return Err("the server SDK does not pin a stable Rust release".into());
    }
    let windows = target == WINDOWS_X64_TARGET;
    let scope = if build { "build" } else { "runtime" };
    let dir = cache.join(format!("rust-{release}-{target}-{scope}"));
    let toolchain = Unpacked {
        dir: dir.clone(),
        target: target.to_owned(),
        cargo_home: cache.join("cargo"),
        c_bin: windows.then(|| dir.join("mingw64/bin")),
        cargo_config: Vec::new(),
    };
    if dir.is_dir() {
        return Ok(toolchain);
    }
    let stage = tempfile::tempdir_in(cache).map_err(|e| e.to_string())?;
    let prefix = stage.path().join("rust");
    fs::create_dir(&prefix).map_err(|e| e.to_string())?;
    let all = [
        ("rust-std", 128 << 20),
        ("rustc", 384 << 20),
        ("cargo", 128 << 20),
        ("rust-mingw", 128 << 20),
    ];
    let count = if !build {
        1
    } else if windows {
        4
    } else {
        3
    };
    for &(component, max) in &all[..count] {
        let name = format!("{component}-{release}-{target}");
        let url = format!("{}/{name}.tar.gz", sunrise_addresses::RUST_DIST);
        let sums = read(fetch, &format!("{url}.sha256"), 1024)?;
        let hash: Sha256 = std::str::from_utf8(&sums)
            .ok()
            .and_then(|s| s.split_ascii_whitespace().next())
            .ok_or("Rust component checksum is missing")?
            .to_owned()
            .try_into()?;
        let path = stage.path().join(&name);
        eprintln!("Downloading Rust {release} ({component})…");
        registry::download(fetch, &url, max, &path)?;
        checked(&path, &hash)?;
        extract_component(&path, &prefix, &name, component)?;
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    if windows {
        let path = stage.path().join("mingw.zip");
        let hash = "62af32ea58acdea1318ea5dd8eb7017eaee694dd40b269d52a59c769dcfefb82"
            .to_owned()
            .try_into()?;
        registry::download(fetch, sunrise_addresses::NATIVE_MINGW_X64, 277203392, &path)?;
        checked(&path, &hash)?;
        let mut zip = zip::ZipArchive::new(fs::File::open(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = file.enclosed_name().ok_or("MinGW has an invalid path")?;
            if file.is_dir()
                || !name.starts_with("mingw64")
                || (!build
                    && !matches!(
                        name.to_str(),
                        Some("mingw64/bin/libgcc_s_seh-1.dll" | "mingw64/bin/libwinpthread-1.dll")
                    ))
            {
                continue;
            }
            let to = prefix.join(name);
            fs::create_dir_all(to.parent().expect("inside prefix")).map_err(|e| e.to_string())?;
            io::copy(
                &mut file,
                &mut fs::File::create_new(&to).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    fs::rename(prefix, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(toolchain)
}

fn checked(path: &Path, hash: &Sha256) -> Result<(), String> {
    if sunrise_mod_install::index::sha256_file(path)? != hash.as_str() {
        return Err(format!("{}: toolchain checksum mismatch", path.display()));
    }
    Ok(())
}

fn extract_component(
    archive: &Path,
    prefix: &Path,
    top: &str,
    component: &str,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(
        flate2::read::GzDecoder::new(fs::File::open(archive).map_err(|e| e.to_string())?)
            .take(2 << 30),
    );
    let root = Path::new(top).join(component);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let Ok(path) = path.strip_prefix(&root) else {
            continue;
        };
        if !path.components().all(|p| matches!(p, Component::Normal(_))) {
            return Err("Rust component has an invalid path".into());
        }
        if !entry.header().entry_type().is_file() || path == Path::new("manifest.in") {
            continue;
        }
        let to = prefix.join(path);
        fs::create_dir_all(to.parent().expect("inside prefix")).map_err(|e| e.to_string())?;
        entry
            .unpack(&to)
            .map_err(|e| format!("{}: {e}", to.display()))?;
    }
    if archive.into_inner().limit() == 0 {
        return Err("expanded Rust component exceeds its size limit".into());
    }
    Ok(())
}
