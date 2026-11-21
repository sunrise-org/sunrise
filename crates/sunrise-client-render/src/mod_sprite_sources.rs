//! The atlas sprite source types native mods register (NeoForge's `RegisterSpriteSourcesEvent`),
//! kept once at start, and the hook point that asks a mod's source for its sprites.
use crate::atlas_sources::ModSources;
use mod_api::{HookId, Identifier, RegistrationContext, ResourceReader, SpriteSourceOutput};
use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
};
use sunrise_core::nbt::Tag;

static TYPES: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());

/// Keeps the sprite source types native mods registered, before resources load. An id that is not
/// `<namespace>:<path>` in a namespace other than vanilla's, or one registered already, is logged
/// and left out. Returns the ids kept, in registration order.
pub fn register(ids: Vec<String>) -> Vec<String> {
    sunrise_assets::item_model::keep_mod_ids(&TYPES, "sprite source type", ids)
}

/// Hook point for native mods (docs/modding/src/how-to/native-mods.md, declared in `hooks/` with
/// the game's): an atlas definition's entry of a type a mod registered (`SpriteSource.run`).
/// `config` is the entry's object and `resources` the load's resources; the mod that owns
/// `source` pushes its sprites to `output`.
#[inline(never)]
pub fn native_sprite_source(
    context: &RegistrationContext,
    source: &str,
    atlas: &Identifier,
    config: &Tag,
    resources: &Arc<dyn ResourceReader>,
    output: &mut Vec<SpriteSourceOutput>,
) {
    if !mod_api::__private::listened(HookId::SpriteSource) {
        return;
    }
    mod_api::dispatch_sprite_source(context, source, atlas, config, resources, output);
}

fn is_registered(kind: &str) -> bool {
    TYPES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(kind)
}

/// The native mods' source types, declared but never answered: for a load that draws nothing.
pub struct Declared;
impl ModSources for Declared {
    fn is_registered(&self, kind: &str) -> bool {
        is_registered(kind)
    }
    fn ask(
        &self,
        _: &Identifier,
        _: &str,
        _: &Tag,
        _: &Arc<dyn ResourceReader>,
    ) -> Vec<SpriteSourceOutput> {
        Vec::new()
    }
}

/// The native mods' source types, answered on the resource loader as registrations.
pub struct Registered;
impl ModSources for Registered {
    fn is_registered(&self, kind: &str) -> bool {
        is_registered(kind)
    }
    fn ask(
        &self,
        atlas: &Identifier,
        kind: &str,
        config: &Tag,
        resources: &Arc<dyn ResourceReader>,
    ) -> Vec<SpriteSourceOutput> {
        use mod_api::__private::game::RegistrationContextNew;
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let mut output = Vec::new();
        native_sprite_source(
            &RegistrationContext::new(),
            kind,
            atlas,
            config,
            resources,
            &mut output,
        );
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas_sources::{SpriteOrigin, SpriteSource, decode, list, test_mods::Mods};

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    fn textures() -> Vec<String> {
        [
            "srctest:item/apple",
            "srctest:item/gem",
            "srctest:item/stick",
        ]
        .map(String::from)
        .to_vec()
    }
    fn pixels(sprite: &str, width: u32, height: u32, rgba: Vec<u8>) -> SpriteSourceOutput {
        SpriteSourceOutput::Pixels {
            sprite: id(sprite),
            width,
            height,
            rgba,
        }
    }
    /// A definition of a directory source, the registered source, then a single file.
    fn definition() -> Vec<SpriteSource> {
        decode(
            r#"{"sources":[
                {"type":"directory","source":"item","prefix":"item/"},
                {"type":"srctest:own","shade":7},
                {"type":"single","resource":"srctest:item/stick","sprite":"srctest:late"}
            ]}"#,
            &Mods::new(&["srctest:own"], |_, _, _, _| Vec::new()),
        )
        .unwrap()
    }

    #[test]
    fn registration_keeps_only_new_ids_in_a_mods_namespace() {
        assert_eq!(
            register(vec![
                "srcreg:one".into(),
                "srcreg:one".into(),
                "minecraft:mine".into(),
                "no_namespace".into(),
                "srcreg:two".into(),
            ]),
            ["srcreg:one", "srcreg:two"]
        );
        assert!(is_registered("srcreg:one") && is_registered("srcreg:two"));
        assert!(!is_registered("minecraft:mine") && !is_registered("no_namespace"));
        assert!(
            register(vec!["srcreg:one".into()]).is_empty(),
            "already registered"
        );
    }

    #[test]
    fn an_unregistered_type_fails_its_definition_by_name() {
        let text = r#"{"sources":[{"type":"directory","source":"item","prefix":""},{"type":"srcfree:nobody"}]}"#;
        assert_eq!(
            decode(text, &Mods::none()).unwrap_err().to_string(),
            "sources[1]: unknown sprite source type srcfree:nobody"
        );
        let registered = Mods::new(&["srcfree:nobody"], |_, _, _, _| Vec::new());
        assert!(matches!(
            decode(text, &registered).unwrap().as_slice(),
            [_, SpriteSource::Registered { kind, .. }] if kind == "srcfree:nobody"
        ));
    }

    #[test]
    fn the_source_is_asked_with_its_entry_and_nothing_the_entries_before_it_listed() {
        let mut asked = Vec::new();
        list(&definition(), &textures(), &mut |kind, config| {
            assert!(
                matches!(config, Tag::Compound(_)),
                "an object converts to a compound"
            );
            asked.push((kind.to_owned(), config.get("shade").cloned()));
            Vec::new()
        });
        assert_eq!(
            asked,
            // A JSON number converts to the narrowest tag that holds it.
            [("srctest:own".to_owned(), Some(Tag::Byte(7)))]
        );
    }

    #[test]
    fn outputs_apply_in_order_and_a_later_sprite_replaces_an_earlier_one() {
        let listed = list(&definition(), &textures(), &mut |_, _| {
            vec![
                // Replaces the directory's apple, then is replaced by the next output.
                pixels("srctest:item/apple", 1, 1, vec![1, 1, 1, 255]),
                SpriteSourceOutput::Texture {
                    sprite: id("srctest:item/apple"),
                    texture: id("srctest:item/gem"),
                },
                pixels("srctest:new", 1, 2, vec![9; 8]),
                // The single source after the mod's replaces this one.
                pixels("srctest:late", 1, 1, vec![0; 4]),
            ]
        });
        assert_eq!(
            listed.sprites["srctest:item/apple"],
            SpriteOrigin::File("srctest:item/gem".into())
        );
        assert_eq!(
            listed.sprites["srctest:new"],
            SpriteOrigin::Pixels((1, 2, vec![9; 8]))
        );
        assert_eq!(
            listed.sprites["srctest:late"],
            SpriteOrigin::File("srctest:item/stick".into())
        );
        assert_eq!(
            listed.by_mods.iter().collect::<Vec<_>>(),
            ["srctest:item/apple", "srctest:new"],
            "the mod's, but not what a later built-in source took back"
        );
        let generated = crate::atlas_sources::generate(&listed.sprites["srctest:new"], &|_| None);
        assert_eq!(
            generated,
            Some((1, 2, vec![9; 8])),
            "a mod's pixels need no file"
        );
    }

    #[test]
    fn remove_drops_a_listed_sprite_and_an_unlisted_id_changes_nothing() {
        let listed = list(&definition(), &textures(), &mut |_, _| {
            vec![
                SpriteSourceOutput::Remove {
                    sprite: id("srctest:item/gem"),
                },
                SpriteSourceOutput::Remove {
                    sprite: id("srctest:never_listed"),
                },
            ]
        });
        let ids: Vec<&str> = listed.sprites.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            ["srctest:item/apple", "srctest:item/stick", "srctest:late"]
        );
    }

    #[test]
    fn what_cant_be_a_sprite_is_skipped_alone() {
        let side = 32768_u32;
        let listed = list(&definition(), &textures(), &mut |_, _| {
            vec![
                pixels("srctest:short", 2, 2, vec![0; 15]),
                pixels("srctest:long", 2, 2, vec![0; 17]),
                pixels("srctest:empty", 0, 4, Vec::new()),
                // The product overflows: refused without a panic.
                pixels("srctest:overflow", u32::MAX, u32::MAX, vec![0; 4]),
                pixels(
                    "srctest:too_wide",
                    side + 1,
                    1,
                    vec![0; (side as usize + 1) * 4],
                ),
                SpriteSourceOutput::Texture {
                    sprite: id("srctest:missing"),
                    texture: id("srctest:item/nothing"),
                },
                pixels("srctest:widest", side, 1, vec![0; side as usize * 4]),
                pixels("srctest:good", 2, 2, vec![0; 16]),
            ]
        });
        let ids: Vec<&str> = listed.sprites.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "srctest:good",
                "srctest:item/apple",
                "srctest:item/gem",
                "srctest:item/stick",
                "srctest:late",
                "srctest:widest"
            ]
        );
    }
}
