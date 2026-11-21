//! Signed rolling updates of the server manager, independent of the game release in `current`.

use crate::update;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};
use sunrise_release_sign::{self as sign, PublicKey};
use sunrise_server_launch::{self as launch, ContractRange, config::Config};

#[cfg(test)]
mod tests;

pub(crate) const TAG: &str = "server-tool";
const MANIFEST: &str = "server-tool.json";

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Build {
    commit: String,
    date: DateTime<Utc>,
    supports: ContractRange,
}

impl Build {
    pub(crate) fn running() -> Result<Self, String> {
        let unknown = || {
            "this sunrise-server has no build commit and date: update it the way you installed it"
                .to_owned()
        };
        let commit = option_env!("SUNRISE_SERVER_COMMIT").ok_or_else(unknown)?;
        let date = option_env!("SUNRISE_SERVER_COMMIT_TIME")
            .and_then(|s| s.parse().ok())
            .and_then(|secs| DateTime::from_timestamp(secs, 0))
            .ok_or_else(unknown)?;
        Ok(Self {
            commit: commit.into(),
            date,
            supports: launch::SUPPORTED_CONTRACTS,
        })
    }

    fn newer_than(&self, running: &Self) -> Result<bool, String> {
        if self.commit == running.commit {
            return Ok(false);
        }
        // The rolling tag can be replayed with an old but still validly signed release set.
        if self.date <= running.date {
            return Err(format!(
                "manager {} ({}) is not newer than the running manager {} ({}): not installed. Use `sunrise-server self-update --rollback` to go back.",
                self.commit, self.date, running.commit, running.date
            ));
        }
        Ok(true)
    }

    fn check_game(&self, game: &str, contract: u32) -> Result<(), String> {
        let ContractRange { min, max } = self.supports;
        if self.supports.contains(contract) {
            Ok(())
        } else {
            Err(format!(
                "the installed game {game} uses dedicated contract {contract}, but manager {} supports contracts {min}–{max}: not installed",
                self.commit
            ))
        }
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

struct ToolRelease {
    build: Build,
    archive: Asset,
    sha256: String,
}

/// Authenticate the metadata as well as the executable: an unsigned commit, date or range could
/// bypass the version or compatibility check even if the executable itself has a signed checksum.
fn signed_release(
    release: GithubRelease,
    archive_name: &str,
    fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
    keys: &[PublicKey],
) -> Result<ToolRelease, String> {
    if release.tag_name != TAG {
        return Err(format!(
            "GitHub returned {} instead of {TAG}",
            release.tag_name
        ));
    }
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.as_str())
            .ok_or_else(|| format!("{TAG} has no {name}"))
    };
    let sums = fetch(url(sign::SUMS)?)?;
    let signature = fetch(url(sign::SIGNATURE)?)?;
    let signature =
        std::str::from_utf8(&signature).map_err(|e| format!("{}: {e}", sign::SIGNATURE))?;
    let checksums = sign::verify(TAG, &sums, signature, keys)?;
    let digest = |name: &str| {
        checksums
            .iter()
            .find(|(file, _)| file == name)
            .map(|(_, hex)| hex.as_str())
            .ok_or_else(|| format!("{name} is missing from {TAG}'s signed checksums"))
    };
    let expected = digest(MANIFEST)?;
    let bytes = fetch(url(MANIFEST)?)?;
    sign::check(MANIFEST, expected, &sign::sha256_hex(&bytes))?;
    let build: Build = serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if build.commit.len() != 40
        || !build
            .commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || build.supports.min == 0
        || build.supports.min > build.supports.max
    {
        return Err(format!(
            "{MANIFEST} has an invalid commit or contract range"
        ));
    }
    let sha256 = digest(archive_name)?.to_owned();
    let archive = release
        .assets
        .into_iter()
        .find(|a| a.name == archive_name)
        .ok_or_else(|| format!("{TAG} has no {archive_name}"))?;
    Ok(ToolRelease {
        build,
        archive,
        sha256,
    })
}

fn platform_archive(os: &str, arch: &str) -> Result<&'static str, String> {
    include_str!("../server-tool-platforms.txt")
        .lines()
        .find_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            if fields.next()? == os && fields.next()? == arch {
                fields.next()
            } else {
                None
            }
        })
        .ok_or_else(|| format!("{TAG} is not published for {os} {arch}"))
}

pub(crate) fn update(dir: &Path, check: bool) -> Result<(), String> {
    // The Linux archive targets glibc. A manually installed musl build must not switch to it.
    if cfg!(target_env = "musl") {
        return Err(format!("{TAG} is not published for musl builds"));
    }
    let archive_name = platform_archive(std::env::consts::OS, std::env::consts::ARCH)?;
    let running = Build::running()?;
    let response = update::fetch_tag(TAG).map_err(|e| {
        if e == "404" {
            format!("no {TAG} release on GitHub")
        } else {
            e
        }
    })?;
    let release = signed_release(
        serde_json::from_slice(&response).map_err(|e| format!("GitHub's answer: {e}"))?,
        archive_name,
        update::fetch,
        &sign::trusted_keys_for(sign::REPOSITORY),
    )?;
    if !release.build.newer_than(&running)? {
        println!(
            "The server manager is already current ({}).",
            running.commit
        );
        return Ok(());
    }
    if check {
        println!(
            "A newer server manager is available ({}): `sunrise-server self-update` installs it.",
            release.build.commit
        );
        return Ok(());
    }

    let exe = running_executable()?;
    let install = launch::Install::around(&exe);
    let config_path = dir.join(launch::config::FILE);
    let configured = config_path
        .try_exists()
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let config = if configured {
        Config::load(dir)?
    } else {
        Config::default()
    };
    match launch::program_in(&config, dir, install.as_ref()) {
        Ok(program) => {
            let game = launch::probe_contract(&program)?;
            release.build.check_game(&game.sunrise, game.contract())?;
        }
        Err(e) if configured || install.is_some() => return Err(e),
        Err(_) => {} // A standalone manager can be updated before installing any game.
    }

    let _lock = lock(&exe)?;
    let folder = exe
        .parent()
        .ok_or("the manager has no installation folder")?;
    let staged = tempfile::tempdir_in(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let archive = staged.path().join(&release.archive.name);
    println!(
        "Downloading the server manager ({})...",
        release.archive.name
    );
    update::download_to(
        &release.archive.name,
        &release.archive.browser_download_url,
        &release.sha256,
        File::create(&archive).map_err(|e| format!("{}: {e}", archive.display()))?,
    )?;
    let incoming = staged.path().join(update::exe("sunrise-server"));
    unpack(&archive, &incoming).map_err(|e| format!("{}: {e}", archive.display()))?;
    replace(&exe, &incoming, staged.path())?;
    println!(
        "Updated the server manager to {}. `sunrise-server self-update --rollback` goes back; the game is unchanged.",
        release.build.commit
    );
    Ok(())
}

pub(crate) fn rollback() -> Result<(), String> {
    let exe = running_executable()?;
    let _lock = lock(&exe)?;
    let folder = exe
        .parent()
        .ok_or("the manager has no installation folder")?;
    let staged = tempfile::tempdir_in(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    rollback_to_previous(&exe, staged.path())?;
    println!("Restored the previous server manager; the game is unchanged.");
    Ok(())
}

fn running_executable() -> Result<PathBuf, String> {
    std::env::current_exe()
        .and_then(dunce::canonicalize)
        .map_err(|e| format!("finding the running sunrise-server: {e}"))
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}

fn lock(exe: &Path) -> Result<File, String> {
    let path = sibling(exe, ".update.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.try_lock()
        .map_err(|e| format!("locking {}: {e}", path.display()))?;
    Ok(file)
}

#[cfg(unix)]
fn unpack(archive: &Path, incoming: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    let mut entries = archive.entries()?;
    let mut entry = entries.next().transpose()?.ok_or_else(invalid_archive)?;
    if !entry.header().entry_type().is_file() || entry.path_bytes().as_ref() != b"sunrise-server" {
        return Err(invalid_archive());
    }
    let mut binary = File::create(incoming)?;
    io::copy(&mut entry, &mut binary)?;
    drop(entry);
    if entries.next().is_some() {
        return Err(invalid_archive());
    }
    binary.set_permissions(fs::Permissions::from_mode(0o755))?;
    binary.sync_all()
}

#[cfg(windows)]
fn unpack(archive: &Path, incoming: &Path) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
    if archive.len() != 1 {
        return Err(invalid_archive());
    }
    let mut entry = archive.by_index(0)?;
    if entry.name()?.as_ref() != "sunrise-server.exe" || !entry.is_file() {
        return Err(invalid_archive());
    }
    let mut binary = File::create(incoming)?;
    io::copy(&mut entry, &mut binary)?;
    binary.sync_all()
}

fn invalid_archive() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "expected an archive holding only the sunrise-server executable",
    )
}

/// Use the same replacement for rollback. Stage a copy so the previous binary stays where it was
/// until replacement commits, and becomes the running one even on Windows.
fn rollback_to_previous(exe: &Path, staged: &Path) -> Result<(), String> {
    let previous = sibling(exe, ".previous");
    if !has_previous(&previous)? {
        return Err(format!(
            "no previous server manager at {}",
            previous.display()
        ));
    }
    let incoming = staged.join("rollback");
    let stage_copy = || -> io::Result<()> {
        let mut source = File::open(&previous)?;
        let mut copy = File::create(&incoming)?;
        io::copy(&mut source, &mut copy)?;
        copy.set_permissions(source.metadata()?.permissions())?;
        copy.sync_all()
    };
    stage_copy().map_err(|e| format!("staging {}: {e}", previous.display()))?;
    replace(exe, &incoming, staged)
}

#[cfg(unix)]
fn replace(exe: &Path, incoming: &Path, staged: &Path) -> Result<(), String> {
    // Save the current inode before replacing it. The executable path is never missing.
    let previous = sibling(exe, ".previous");
    let older = staged.join("older");
    let had_previous = has_previous(&previous)?;
    if had_previous {
        fs::hard_link(&previous, &older)
            .map_err(|e| format!("saving {}: {e}", previous.display()))?;
    }
    let old = staged.join("old");
    fs::hard_link(exe, &old)
        .and_then(|()| fs::rename(&old, &previous))
        .map_err(|e| format!("saving the previous manager: {e}"))?;
    if let Err(e) = fs::rename(incoming, exe) {
        let error = format!("replacing {}: {e}", exe.display());
        return Err(if had_previous {
            restoring(&older, &previous, error)
        } else {
            match fs::remove_file(&previous) {
                Ok(()) => error,
                Err(e) => format!("{error}; couldn't remove {}: {e}", previous.display()),
            }
        });
    }
    Ok(())
}

#[cfg(windows)]
fn replace(exe: &Path, incoming: &Path, staged: &Path) -> Result<(), String> {
    let previous = sibling(exe, ".previous");
    let older = staged.join("older");
    let had_previous = has_previous(&previous)?;
    if had_previous {
        fs::rename(&previous, &older).map_err(|e| format!("saving {}: {e}", previous.display()))?;
    }
    if let Err(e) = fs::rename(exe, &previous) {
        let error = format!("moving {} aside: {e}", exe.display());
        return Err(if had_previous {
            restoring(&older, &previous, error)
        } else {
            error
        });
    }
    if let Err(e) = fs::rename(incoming, exe) {
        let error = format!("replacing {}: {e}", exe.display());
        fs::rename(&previous, exe).map_err(|restore| {
            format!(
                "{error}; couldn't restore it: {restore}. The original is at {}",
                previous.display()
            )
        })?;
        return Err(if had_previous {
            restoring(&older, &previous, error)
        } else {
            error
        });
    }
    Ok(())
}

fn has_previous(path: &Path) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(m) if m.is_file() => Ok(true),
        Ok(_) => Err(format!("{} is not a regular file", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn restoring(saved: &Path, to: &Path, error: String) -> String {
    match fs::rename(saved, to) {
        Ok(()) => error,
        Err(e) => format!("{error}; couldn't restore {}: {e}", to.display()),
    }
}
