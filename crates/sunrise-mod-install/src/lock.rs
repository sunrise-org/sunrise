//! The on-disk managed mod selection and its validation boundary.
use crate::input::{ModId, Mods};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::Read,
    path::{Component, Path, PathBuf},
};
use sunrise_mod_package::manifest::version_from_str;

pub const LOCK: &str = "mods.lock";
pub const MAX_LOCK: u64 = 1 << 20;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModLock {
    format: Format,
    pub sunrise: BuildId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseTag>,
    pub input: Mods,
    #[serde(default, deserialize_with = "lock_rows")]
    pub mods: Vec<LockedMod>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runtime: BTreeMap<PackagePath, Sha256>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(try_from = "u32", into = "u32")]
struct Format;

impl TryFrom<u32> for Format {
    type Error = String;

    fn try_from(format: u32) -> Result<Self, String> {
        if format == 1 {
            Ok(Self)
        } else {
            Err(format!("{LOCK} format {format} is not supported"))
        }
    }
}

impl From<Format> for u32 {
    fn from(_: Format) -> Self {
        1
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LockedMod {
    pub id: ModId,
    #[serde(deserialize_with = "version_from_str")]
    pub version: semver::Version,
    pub sha256: Sha256,
    pub(crate) url: HttpsUrl,
    pub file: PackagePath,
    pub reason: Reason,
    #[serde(default)]
    pub required_by: Vec<ModId>,
    pub on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Native>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Requested,
    Pack,
    Required,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Native {
    Prebuilt(Sha256),
    Built(BuildId),
    Inactive,
}

pub type BuildId = Hex<40>;
pub type Sha256 = Hex<64>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hex<const LEN: usize>(String);

impl<const LEN: usize> TryFrom<String> for Hex<LEN> {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        if text.len() == LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(text))
        } else {
            Err(format!("expected {LEN} lowercase hex characters"))
        }
    }
}

impl<const LEN: usize> From<Hex<LEN>> for String {
    fn from(hex: Hex<LEN>) -> Self {
        hex.0
    }
}

impl<const LEN: usize> Hex<LEN> {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const LEN: usize> fmt::Display for Hex<LEN> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReleaseTag(String);

impl TryFrom<String> for ReleaseTag {
    type Error = String;

    fn try_from(tag: String) -> Result<Self, String> {
        if tag
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            Ok(Self(tag))
        } else {
            Err("expected a Sunrise release tag".into())
        }
    }
}

impl From<ReleaseTag> for String {
    fn from(tag: ReleaseTag) -> Self {
        tag.0
    }
}

impl fmt::Display for ReleaseTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct HttpsUrl(url::Url);

impl TryFrom<String> for HttpsUrl {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let url = url::Url::parse(&text).map_err(|error| format!("invalid source URL: {error}"))?;
        if url.scheme() == "https" {
            Ok(Self(url))
        } else {
            Err("expected an HTTPS source URL".into())
        }
    }
}

impl From<HttpsUrl> for String {
    fn from(url: HttpsUrl) -> Self {
        url.0.into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct PackagePath(PathBuf);

impl TryFrom<String> for PackagePath {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let path = Path::new(&text);
        // Keep the lock portable, including on hosts where backslashes and colons aren't separators.
        if text.is_empty()
            || text.contains(['\\', ':', '\0'])
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            Err(format!(
                "{text:?}: expected a relative file path without '..'"
            ))
        } else {
            Ok(Self(text.into()))
        }
    }
}

impl From<PackagePath> for String {
    fn from(path: PackagePath) -> Self {
        path.0.to_string_lossy().into_owned()
    }
}

impl LockedMod {
    pub fn source_url(&self) -> &str {
        self.url.0.as_str()
    }
}

impl PackagePath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

// These invariants relate already validated fields, rather than parsing them again.
fn lock_rows<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<LockedMod>, D::Error> {
    let rows = Vec::<LockedMod>::deserialize(de)?;
    let (mut ids, mut paths) = (BTreeSet::new(), BTreeSet::new());
    for row in &rows {
        if !ids.insert(&row.id) {
            return Err(D::Error::custom(format!("duplicate mod ID {}", row.id)));
        }
        if !paths.insert(row.file.as_path()) {
            return Err(D::Error::custom(format!(
                "mod {} has a duplicate file path",
                row.id
            )));
        }
        if matches!(row.reason, Reason::Required) == row.required_by.is_empty() {
            return Err(D::Error::custom(format!(
                "mod {} has an invalid required_by list for its reason",
                row.id
            )));
        }
    }
    Ok(rows)
}

impl ModLock {
    pub(crate) fn new(
        sunrise: BuildId,
        release: Option<ReleaseTag>,
        input: Mods,
        mods: Vec<LockedMod>,
    ) -> Self {
        Self {
            format: Format,
            sunrise,
            release,
            input,
            mods,
            runtime: BTreeMap::new(),
        }
    }

    /// Read and validate the lock in the canonical server folder `root`.
    pub fn read(root: &Path) -> Result<Self, String> {
        let path = local_file(root, Path::new(LOCK))?;
        let text = read_text(&path, MAX_LOCK)?;
        toml::from_str(&text).map_err(|e| format!("{LOCK}: {e}"))
    }
}

/// Call only with a validated relative path. Resolve links before checking containment, including
/// links in intermediate directories, and don't let a lock direct reads to a device or pipe.
pub fn local_file(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let actual =
        fs::canonicalize(root.join(path)).map_err(|e| format!("{}: {e}", path.display()))?;
    if !actual.starts_with(root) {
        return Err(format!(
            "{} points outside the server folder",
            path.display()
        ));
    }
    if !actual.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    Ok(actual)
}

/// Read a bounded UTF-8 record, refusing a valid prefix of an oversized file.
pub fn read_text(path: &Path, limit: u64) -> Result<String, String> {
    let error = |e| format!("{}: {e}", path.display());
    let mut text = String::new();
    let bytes = fs::File::open(path)
        .map_err(error)?
        .take(limit.saturating_add(1))
        .read_to_string(&mut text)
        .map_err(error)?;
    if bytes as u64 > limit {
        return Err(format!("{} exceeds the {limit}-byte limit", path.display()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::ModLock;

    const HEADER: &str = "\
format = 1
sunrise = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
release = '26.3-0.1.0'
input = { off = ['rails'] }
";
    const ROW: &str = "
[[mods]]
id = 'rails'
version = '1.2.3'
sha256 = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
url = 'https://example.net/rails.smod'
file = 'mods/rails.smod'
reason = 'required'
required_by = ['ladders']
on = true
native = { built = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }
";

    #[test]
    fn toml_rejects_invalid_lock_fields_without_reading_files() {
        let valid = format!("{HEADER}{ROW}");
        let lock: ModLock = toml::from_str(&valid).unwrap();
        let original: toml::Table = valid.parse().unwrap();
        let saved: toml::Table = toml::to_string(&lock).unwrap().parse().unwrap();
        assert_eq!(saved["mods"], original["mods"]);
        assert_eq!(saved["sunrise"], original["sunrise"]);
        assert_eq!(saved["release"], original["release"]);
        for (from, to, message) in [
            ("format = 1", "format = 2", "format 2 is not supported"),
            (
                "sunrise = 'aaaa",
                "sunrise = 'AAAA",
                "expected 40 lowercase hex characters",
            ),
            (
                "release = '26.3-0.1.0'",
                "release = '../bad'",
                "expected a Sunrise release tag",
            ),
            (
                "off = ['rails']",
                "off = ['rails@2']",
                "mods.off \"rails@2\": not a mod ID",
            ),
            ("id = 'rails'", "id = '../rails'", "not a mod ID"),
            (
                "sha256 = 'ba78",
                "sha256 = 'zz78",
                "expected 64 lowercase hex characters",
            ),
            ("https://", "http://", "expected an HTTPS source URL"),
            (
                "file = 'mods/rails.smod'",
                "file = '../rails.smod'",
                "expected a relative file path",
            ),
            (
                "required_by = ['ladders']",
                "required_by = ['BAD']",
                "not a mod ID",
            ),
            (
                "built = 'aaaa",
                "built = 'zzzz",
                "expected 40 lowercase hex characters",
            ),
            (
                "built = 'aaaa",
                "prebuilt = 'aaaa",
                "expected 64 lowercase hex characters",
            ),
        ] {
            let error = toml::from_str::<ModLock>(&valid.replacen(from, to, 1))
                .unwrap_err()
                .to_string();
            assert!(error.contains(message), "{from} -> {to}: {error}");
        }
    }

    #[test]
    fn toml_rejects_conflicts_between_valid_lock_fields() {
        let valid = format!("{HEADER}{ROW}");
        let cases = [
            (format!("{valid}{ROW}"), "duplicate mod ID rails"),
            (
                format!("{valid}{}", ROW.replace("id = 'rails'", "id = 'beams'")),
                "mod beams has a duplicate file path",
            ),
            (
                valid.replace("required_by = ['ladders']", "required_by = []"),
                "invalid required_by list",
            ),
            (
                valid.replace("reason = 'required'", "reason = 'pack'"),
                "invalid required_by list",
            ),
        ];
        for (source, message) in cases {
            let error = toml::from_str::<ModLock>(&source).unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        }
    }
}
