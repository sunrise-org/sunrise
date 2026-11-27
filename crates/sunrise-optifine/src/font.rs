//! HD fonts: a higher resolution sheet for a bitmap font provider, kept apart from the one vanilla
//! reads so that a pack works with and without OptiFine.
//!
//! A bitmap provider names its sheet by `file` (`minecraft:font/ascii.png`), which vanilla reads
//! from `assets/<namespace>/textures/<path>`. OptiFine looks first for the same path under
//! `optifine/` instead of `textures/` (`assets/minecraft/optifine/font/ascii.png`), in any pack,
//! and reads that sheet when it exists. OptiDocs names the default font's `ascii.png` and the
//! enchanting table's `ascii_sga.png`; the rule here is the provider's `file`, so it applies to
//! every bitmap provider, whatever its font (`accented.png`, a pack's own sheets), in the
//! provider's own namespace.
//!
//! The sheet may have any size. A provider cuts its sheet into the grid its `chars` rows give, so
//! an HD sheet has bigger cells and the same glyph grid; the glyphs keep their size and advance
//! in GUI pixels, because the provider scales a cell by its `height`.
//!
//! The `.properties` file the documentation puts beside a sheet (`width.<n>`, `blend`,
//! `offsetBold`) is not read: OptiFine 26.x no longer applies it to a bitmap provider's sheet.
//!
//! Written from OptiFine's public documentation of HD fonts (`OptiFineDoc/doc/hd_fonts.txt` and
//! the HD Fonts page of OptiDocs).

use sunrise_assets::resource_source::ResourceSource;

use crate::paths::valid;

/// The folder of a namespace that holds HD sheets, where vanilla's sheets are under `textures/`.
const FOLDER: &str = "optifine";

/// The HD sheet a pack may have for a bitmap provider's sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HdSheet {
    /// Its resource path: `assets/<namespace>/optifine/<path>`.
    pub resource: String,
    /// The name the HUD atlas packs it under, `<namespace>:optifine/<path>` without the `.png`,
    /// the default namespace's included (its textures are named by their path alone, so none
    /// meets it). It is never the name of the sheet it replaces.
    pub atlas_name: String,
}

/// Where the HD sheet of the provider sheet `file` would be: a provider's `file` is
/// `namespace:path` or just `path` (`minecraft`), and the sheet vanilla reads is
/// `assets/<namespace>/textures/<path>`.
///
/// `None` when `file` is not a resource name: an empty or not lower-case namespace or path, a
/// character vanilla's names refuse, or a `.`, `..` or empty segment that would leave the
/// resource root (see [`valid`]). Such a provider keeps the sheet it names.
pub fn hd_sheet(file: &str) -> Option<HdSheet> {
    let (namespace, path) = file.split_once(':').unwrap_or(("minecraft", file));
    if !namespace_ok(namespace) || !path_ok(path) {
        return None;
    }
    let stem = path.strip_suffix(".png").unwrap_or(path);
    Some(HdSheet {
        resource: format!("assets/{namespace}/{FOLDER}/{path}"),
        atlas_name: format!("{namespace}:{FOLDER}/{stem}"),
    })
}

/// The HD sheet of the provider sheet `file` when a pack in `root` has it ([`hd_sheet`]); the
/// provider then reads it instead of its own sheet. A pack's sheet counts even when another pack
/// replaces the plain one.
pub fn find(root: &ResourceSource, file: &str) -> Option<HdSheet> {
    hd_sheet(file).filter(|sheet| root.contains(&sheet.resource))
}

/// A namespace as vanilla's resource names spell it (`a-z`, `0-9`, `_`, `-` and `.`), but not
/// `.` or `..`, which would leave the resource root.
fn namespace_ok(namespace: &str) -> bool {
    valid(namespace)
        && namespace
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
}

/// A path as vanilla's resource names spell it (the namespace's characters and `/`), staying in
/// the resource root.
fn path_ok(path: &str) -> bool {
    valid(path)
        && path
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use sunrise_assets::resource_source::Resource;

    fn sheet(file: &str) -> (String, String) {
        let sheet = hd_sheet(file).unwrap_or_else(|| panic!("{file}"));
        (sheet.resource, sheet.atlas_name)
    }

    fn root(files: &[&str]) -> ResourceSource {
        let files: BTreeMap<String, Resource> = files
            .iter()
            .map(|file| ((*file).to_owned(), Resource::bytes(Vec::new())))
            .collect();
        ResourceSource::indexed(files)
    }

    #[test]
    fn textures_become_optifine() {
        assert_eq!(
            sheet("minecraft:font/ascii.png"),
            (
                "assets/minecraft/optifine/font/ascii.png".to_owned(),
                "minecraft:optifine/font/ascii".to_owned()
            )
        );
        // The default namespace may be left out, as in a provider's `file`.
        assert_eq!(sheet("font/ascii.png"), sheet("minecraft:font/ascii.png"));
        // Any namespace, any folder under `textures/`, any depth.
        assert_eq!(
            sheet("mymod:font/glyphs/icons.png"),
            (
                "assets/mymod/optifine/font/glyphs/icons.png".to_owned(),
                "mymod:optifine/font/glyphs/icons".to_owned()
            )
        );
        assert_eq!(
            sheet("minecraft:hud/sheet.png").0,
            "assets/minecraft/optifine/hud/sheet.png"
        );
    }

    #[test]
    fn a_sheet_without_an_extension_keeps_its_name() {
        assert_eq!(
            sheet("font/sheet"),
            (
                "assets/minecraft/optifine/font/sheet".to_owned(),
                "minecraft:optifine/font/sheet".to_owned()
            )
        );
    }

    #[test]
    fn names_that_are_not_resources_are_refused() {
        for bad in [
            "",
            "minecraft:",
            ":font/ascii.png",
            "minecraft:../ascii.png",
            "minecraft:font/../../x.png",
            "minecraft:font//ascii.png",
            "minecraft:/font/ascii.png",
            "minecraft:font/./ascii.png",
            "minecraft:font\\ascii.png",
            "..:font/ascii.png",
            "my/mod:font/ascii.png",
            "minecraft:Font/ascii.png",
            "MyMod:font/ascii.png",
            "minecraft:font/ascii sheet.png",
            "minecraft:font/a:b.png",
        ] {
            assert_eq!(hd_sheet(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_sheet_counts_when_a_pack_has_it() {
        let root = root(&[
            "assets/minecraft/textures/font/ascii.png",
            "assets/minecraft/optifine/font/ascii.png",
            "assets/minecraft/textures/font/accented.png",
            "assets/mymod/optifine/font/icons.png",
            "assets/mymod/textures/font/other.png",
        ]);
        assert_eq!(
            find(&root, "minecraft:font/ascii.png").map(|s| s.resource),
            Some("assets/minecraft/optifine/font/ascii.png".to_owned())
        );
        assert_eq!(
            find(&root, "mymod:font/icons.png").map(|s| s.atlas_name),
            Some("mymod:optifine/font/icons".to_owned())
        );
        // Only the plain sheet is there, or no sheet at all.
        assert_eq!(find(&root, "minecraft:font/accented.png"), None);
        assert_eq!(find(&root, "mymod:font/other.png"), None);
        assert_eq!(find(&root, "minecraft:font/missing.png"), None);
        // A provider that already names a file under `optifine/` looks under `optifine/optifine/`.
        assert_eq!(find(&root, "minecraft:optifine/font/ascii.png"), None);
        assert_eq!(find(&root, "minecraft:../ascii.png"), None);
    }
}
