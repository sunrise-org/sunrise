//! The managed mod input copied in `mods.lock`.
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};
use sunrise_mod_package::manifest::{parse_version, valid_mod_id};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mods {
    pub pack: Option<Pack>,
    pub list: Vec<ModRequest>,
    #[serde(deserialize_with = "off_ids")]
    pub off: Vec<ModId>,
    pub trust_native_code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModId(String);

impl TryFrom<String> for ModId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, String> {
        if valid_mod_id(&id) {
            Ok(Self(id))
        } else {
            Err(format!("{id:?}: not a mod ID"))
        }
    }
}

impl From<ModId> for String {
    fn from(id: ModId) -> Self {
        id.0
    }
}

impl ModId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn off_ids<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Vec<ModId>, D::Error> {
    Vec::<String>::deserialize(de)?
        .into_iter()
        .map(|id| {
            ModId::try_from(id).map_err(|e| serde::de::Error::custom(format!("mods.off {e}")))
        })
        .collect()
}

/// Keep the author's spelling for round trips; only the parsed value contributes to lock equality.
#[derive(Clone, Debug)]
pub struct Spelled<T> {
    value: T,
    text: String,
}

impl<T> Spelled<T> {
    pub fn value(&self) -> &T {
        &self.value
    }
}

impl<T: PartialEq> PartialEq for Spelled<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Eq> Eq for Spelled<T> {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Pack {
    Registry {
        id: ModId,
        version: Spelled<Version>,
    },
    File(PathBuf),
}

impl TryFrom<String> for Pack {
    type Error = String;

    fn try_from(pack: String) -> Result<Self, String> {
        if pack.ends_with(".spack") && !pack.contains(['\0', '\n', '\r']) {
            return Ok(Self::File(pack.into()));
        }
        if let Some((id, text)) = pack.split_once('@')
            && let Ok(id) = ModId::try_from(id.to_owned())
            && let Ok(value) = parse_version(text)
        {
            return Ok(Self::Registry {
                id,
                version: Spelled {
                    value,
                    text: text.to_owned(),
                },
            });
        }
        Err(format!(
            "mods.pack {pack:?}: use id@version or a .spack path"
        ))
    }
}

impl From<Pack> for String {
    fn from(pack: Pack) -> Self {
        match pack {
            Pack::Registry { id, version } => format!("{id}@{}", version.text),
            Pack::File(path) => path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModRequest {
    pub id: ModId,
    range: Option<Spelled<VersionReq>>,
}

impl ModRequest {
    pub fn range(&self) -> Option<&VersionReq> {
        self.range.as_ref().map(Spelled::value)
    }
}

impl TryFrom<String> for ModRequest {
    type Error = String;

    fn try_from(request: String) -> Result<Self, String> {
        let error = || format!("mods.list {request:?}: use id or id@semver-range");
        let (id, range) = request
            .split_once('@')
            .map_or((request.as_str(), None), |(id, text)| (id, Some(text)));
        let id = ModId::try_from(id.to_owned()).map_err(|_| error())?;
        let range = range
            .map(|text| {
                VersionReq::parse(text)
                    .map(|value| Spelled {
                        value,
                        text: text.to_owned(),
                    })
                    .map_err(|_| error())
            })
            .transpose()?;
        Ok(Self { id, range })
    }
}

impl From<ModRequest> for String {
    fn from(request: ModRequest) -> Self {
        match request.range {
            Some(range) => format!("{}@{}", request.id, range.text),
            None => request.id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_can_have_extras_and_disabled_members() {
        let source = r#"
[mods]
pack = "cozy@1.0"
list = ["rails@^2, <3", "windows"]
off = ["pack_member"]
trust_native_code = true
"#;
        let original: toml::Table = toml::from_str(source).unwrap();
        let mods: Mods = original["mods"].clone().try_into().unwrap();
        let Some(Pack::Registry { id, version }) = &mods.pack else {
            panic!("expected a registry pack");
        };
        assert_eq!(id.as_str(), "cozy");
        assert_eq!(version.value(), &Version::new(1, 0, 0));
        assert_eq!(mods.list[0].id.as_str(), "rails");
        assert_eq!(
            mods.list[0].range(),
            Some(&VersionReq::parse("^2,<3").unwrap())
        );
        assert_eq!(mods.list[1].id.as_str(), "windows");
        assert_eq!(mods.list[1].range(), None);
        assert_eq!(mods.off[0].as_str(), "pack_member");
        assert!(mods.trust_native_code);

        let saved = toml::to_string(&mods)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert_eq!(&saved, original["mods"].as_table().unwrap());
        let restyled = source
            .replace("cozy@1.0", "cozy@1.0.0")
            .replace("^2, <3", "^2,<3");
        let restyled: toml::Table = toml::from_str(&restyled).unwrap();
        let reparsed: Mods = restyled["mods"].clone().try_into().unwrap();
        assert_eq!(reparsed, mods);

        let local: Mods = toml::from_str("pack = './local.sp aces.spack'").unwrap();
        assert!(
            matches!(local.pack, Some(Pack::File(path)) if path == std::path::Path::new("./local.sp aces.spack"))
        );
    }

    #[test]
    fn rejects_invalid_mod_settings() {
        for (field, message) in [
            ("unknown = true", "unknown field"),
            (
                "pack = 'cozy@bad'",
                "mods.pack \"cozy@bad\": use id@version or a .spack path",
            ),
            (
                "list = ['bad/id']",
                "mods.list \"bad/id\": use id or id@semver-range",
            ),
            (
                "list = ['rails@']",
                "mods.list \"rails@\": use id or id@semver-range",
            ),
            (
                "list = ['rails@not-a-range']",
                "mods.list \"rails@not-a-range\": use id or id@semver-range",
            ),
            ("off = ['rails@2']", "mods.off \"rails@2\": not a mod ID"),
            ("trust_native_code = 'true'", "expected a boolean"),
        ] {
            let error = toml::from_str::<Mods>(field).unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        }
    }
}
