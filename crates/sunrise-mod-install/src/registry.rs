//! Bounded registry reads and verified downloads. Only mutations fetch a fresh index;
//! a read-only consumer can inspect the last checked copy without contacting the registry.
use crate::index::{self, Index, Project, Trust, Version};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
    time::Duration,
};
use sunrise_mod_package::durable;

const CACHE: &str = ".mods-index.json";

/// The transport used by the shared installer, injectable for an offline test.
pub trait Fetch {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String>;
}

pub struct Https(ureq::Agent);

impl Default for Https {
    fn default() -> Self {
        Self(
            ureq::AgentBuilder::new()
                .user_agent(concat!("sunrise-mod-install/", env!("CARGO_PKG_VERSION")))
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(Duration::from_secs(30))
                .redirects(0)
                .build(),
        )
    }
}

impl Fetch for Https {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
        self.open_optional(url)?
            .map(|body| body as Box<dyn Read>)
            .ok_or_else(|| format!("{url} returned HTTP 404"))
    }
}

impl Https {
    /// `None` is returned only for a missing resource, not for a transport failure.
    pub fn open_optional(&self, url: &str) -> Result<Option<Box<dyn Read + Send>>, String> {
        let mut url = url::Url::parse(url).map_err(|e| e.to_string())?;
        for _ in 0..6 {
            if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
                return Err("a mod download needs an HTTPS URL without credentials".into());
            }
            let response = match self.0.get(url.as_str()).call() {
                Ok(response) => response,
                Err(ureq::Error::Status(404, _)) => return Ok(None),
                Err(e) => return Err(format!("{}: {e}", url.origin().ascii_serialization())),
            };
            match response.status() {
                200 => return Ok(Some(response.into_reader())),
                301 | 302 | 303 | 307 | 308 => {
                    let new = response
                        .header("Location")
                        .ok_or("download redirect has no destination")?;
                    url = url.join(new).map_err(|e| e.to_string())?;
                }
                status => return Err(format!("download returned HTTP {status}")),
            }
        }
        Err("too many download redirects".into())
    }
}

/// Refresh the cache only after the complete index parses. A mutation never installs from a
/// stale cache: it would miss a takedown published since the last read.
pub fn refresh(home: &Path, at: &str, fetch: &dyn Fetch) -> Result<Index, String> {
    let text = read_index(at, fetch)?;
    let index = Index::parse(&text)?;
    durable::write(&home.join(CACHE), text.as_bytes()).map_err(|e| format!("{CACHE}: {e}"))?;
    Ok(index)
}

/// Inspect the latest registry without writing to the installation.
pub fn load(at: &str, fetch: &dyn Fetch) -> Result<Index, String> {
    Index::parse(&read_index(at, fetch)?)
}

fn read_index(at: &str, fetch: &dyn Fetch) -> Result<String, String> {
    let reader: Box<dyn Read> = if at.starts_with("https://") {
        fetch.open(at)?
    } else {
        // A developer can point at a local index, but it still describes HTTPS downloads.
        Box::new(File::open(at).map_err(|e| format!("{at}: {e}"))?)
    };
    bounded_index(reader)
}

pub fn cached(home: &Path) -> Result<Option<Index>, String> {
    match File::open(home.join(CACHE)) {
        Ok(file) => Index::parse(&bounded_index(file)?).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{CACHE}: {e}")),
    }
}

fn bounded_index(reader: impl Read) -> Result<String, String> {
    let mut text = String::new();
    let len = reader
        .take(index::MAX_INDEX + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if len as u64 > index::MAX_INDEX {
        return Err("the mod index exceeds its size limit".into());
    }
    Ok(text)
}

/// Stream a bounded response to a new staging file. The caller owns the staging directory.
pub fn download(fetch: &dyn Fetch, url: &str, max: u64, path: &Path) -> Result<(), String> {
    let mut input = fetch.open(url)?.take(max.saturating_add(1));
    let mut out = File::create_new(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let written = io::copy(&mut input, &mut out).map_err(|e| format!("{}: {e}", path.display()))?;
    if written > max {
        return Err(format!("the download exceeds its {max}-byte limit"));
    }
    out.sync_all()
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Keep a verified local copy when possible; a drifted or missing file is repaired by download.
pub fn checked_file(
    fetch: &dyn Fetch,
    index: &Index,
    project: &Project,
    version: &Version,
    current: Option<&Path>,
    out: &Path,
) -> Result<Trust, String> {
    let result = (|| {
        if let Some(current) = current
            && let Ok(meta) = fs::metadata(current)
            && meta.is_file()
            && meta.len() == version.file.size
        {
            fs::copy(current, out).map_err(|e| format!("{}: {e}", out.display()))?;
            if let Ok(trust) = index::check_file(index, project, version, out) {
                return Ok(trust);
            }
            fs::remove_file(out).map_err(|e| format!("{}: {e}", out.display()))?;
        }
        download(fetch, &version.file.url, version.file.size, out)?;
        index::check_file(index, project, version, out)
    })();
    result.map_err(|e| format!("{} {}: {e}", project.id, version.version))
}
