//! A Realm's resource pack, its join address, its pack id and its extend link.

use super::client::RealmServer;
use std::sync::Mutex;
use sunrise_core::md5::name_uuid;

/// The world's resource pack the player agreed to download before joining
/// (`GetServerDetailsTask.scheduleResourcePackDownload`): the next connection to `address`
/// takes it as an allowed server pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealmsPack {
    pub address: String,
    pub id: u128,
    pub url: String,
    pub hash: String,
}

/// A completed download handed to the matching connection for activation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedRealmsPack {
    pub address: String,
    pub id: u128,
    pub staged_path: std::path::PathBuf,
}

pub(super) static PENDING_PACK: Mutex<Option<StagedRealmsPack>> = Mutex::new(None);

/// The pack for the connection to `address`, if the Realms screen staged one.
pub fn take_pending_pack(address: &str) -> Option<StagedRealmsPack> {
    let pack = PENDING_PACK.lock().ok()?.take()?;
    (pack.address == address).then_some(pack)
}

pub(super) fn download_directory(root: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let cache = root.join("downloads");
    std::fs::create_dir_all(&cache)?;
    loop {
        let directory = cache.join(format!(
            "realms-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn download(
    pack: RealmsPack,
    directory: &std::path::Path,
    headers: &[(&'static str, String)],
) -> Result<StagedRealmsPack, String> {
    use crate::server_packs::{Request, download, parse_sha1, valid_url};
    let result = if valid_url(&pack.url) {
        let request = Request {
            id: pack.id,
            url: pack.url,
            sha1: parse_sha1(&pack.hash),
        };
        let (progress, receiver) = std::sync::mpsc::channel();
        drop(receiver);
        download::download_file(directory, &request, headers, download::MAX_SIZE, &progress)
    } else {
        Err("Invalid Realms resource-pack URL".into())
    };
    if result.is_err() {
        let _ = std::fs::remove_dir_all(directory);
    }
    result.map(|staged_path| StagedRealmsPack {
        address: pack.address,
        id: pack.id,
        staged_path,
    })
}

/// `GetServerDetailsTask.generatePackId` uses `UUID.nameUUIDFromBytes`.
pub fn pack_id(server: &RealmServer) -> u128 {
    match &server.minigame_name {
        Some(minigame) => name_uuid(format!("minigame:{minigame}").as_bytes()),
        None => name_uuid(format!("realms:{}:{}", server.name, server.active_slot).as_bytes()),
    }
}

/// The address `/join/pc` gave, checked before it becomes `Action::Connect`: a host name,
/// IPv4 or `[IPv6]` host, then `:` and a port number. A scheme, a path, credentials or a
/// missing port are refused.
pub fn connect_address(address: &str) -> Option<String> {
    let address = address.trim();
    let (host, port, host_ok): (&str, &str, fn(u8) -> bool) =
        if let Some(rest) = address.strip_prefix('[') {
            let (host, port) = rest.split_once("]:")?;
            (host, port, |b| {
                b.is_ascii_hexdigit() || b == b':' || b == b'.'
            })
        } else {
            let (host, port) = address.rsplit_once(':')?;
            (host, port, |b| {
                b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_'
            })
        };
    if host.is_empty() || !host.bytes().all(host_ok) {
        return None;
    }
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    (port != 0).then(|| address.to_owned())
}
