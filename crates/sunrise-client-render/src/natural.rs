//! Natural textures at mesh time (`optifine/natural.properties`, or OptiFine's own list where a
//! pack has none: `sunrise_optifine::natural`). The entries are read once at load and become one
//! slot for each sprite layer they name, so meshing a face asks a slot ([`NaturalTextures::variant`])
//! and turns or flips the face's texture coordinates when it has an entry.
//!
//! Which faces are drawn this way is the `Natural Textures` option's (`Mesher::natural_textures`).

use sunrise_optifine::natural::{Entry, Natural, Variant};

use crate::resource_source::ResourceSource;

/// The entries `root` gives: the pack's list, else OptiFine's own.
pub fn read_rules(root: &ResourceSource) -> Vec<Entry> {
    sunrise_optifine::natural::load(root)
}

/// The sprites `rules` name, for [`crate::sprites::Sprites::keep_apart`]: a face finds its entry by
/// its sprite's layer, so a texture identical to one of these must not share it.
pub fn sprites(rules: &[Entry]) -> Vec<String> {
    rules.iter().map(|rule| rule.sprite.clone()).collect()
}

/// What each sprite layer does when a face draws it.
#[derive(Default)]
pub struct NaturalTextures {
    /// By sprite layer; empty when no entry names a sprite the models use.
    by_layer: Vec<Option<Natural>>,
}

impl NaturalTextures {
    /// The entries of `rules` whose sprite has a layer (`layer_of`): an entry for a sprite no model
    /// draws does nothing.
    pub fn load(rules: &[Entry], layer_of: impl Fn(&str) -> Option<u32>) -> Self {
        let mut by_layer = Vec::new();
        for rule in rules {
            let Some(layer) = layer_of(&rule.sprite).map(|layer| layer as usize) else {
                continue;
            };
            if by_layer.len() <= layer {
                by_layer.resize(layer + 1, None);
            }
            by_layer[layer] = Some(rule.natural);
        }
        Self { by_layer }
    }

    /// What the face of the block at `pos` does if it draws the sprite `layer` with the texture
    /// coordinates `uv` and looks toward `face` (a `Direction`'s index); `None` leaves the face as
    /// it is. A face turns only if it shows its whole sprite (turning a part of one would show
    /// another part of it); a flip is made anyway.
    pub fn variant(
        &self,
        layer: u32,
        uv: &[[f32; 2]; 4],
        pos: [i32; 3],
        face: u8,
    ) -> Option<Variant> {
        let natural = (*self.by_layer.get(layer as usize)?)?;
        let mut variant = natural.variant(pos, face);
        if !shows_whole_sprite(uv) {
            variant.turns = 0;
        }
        (!variant.is_identity()).then_some(variant)
    }
}

/// Whether `uv`, in sprite units, lies on the edges of the sprite: every coordinate is at 0 or 1,
/// to a 256th of the sprite.
fn shows_whole_sprite(uv: &[[f32; 2]; 4]) -> bool {
    const TOLERANCE: f32 = 1.0 / 256.0;
    uv.iter()
        .flatten()
        .all(|&c| c.abs() < TOLERANCE || (c - 1.0).abs() < TOLERANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

    fn entry(sprite: &str, value: &str) -> Entry {
        Entry {
            sprite: sprite.into(),
            natural: Natural::parse(value).unwrap(),
        }
    }

    fn table(rules: &[Entry]) -> NaturalTextures {
        // `minecraft:block/a` is layer 0, `b` layer 1; no other sprite exists.
        NaturalTextures::load(rules, |name| match name {
            "minecraft:block/a" => Some(0),
            "minecraft:block/b" => Some(1),
            _ => None,
        })
    }

    #[test]
    fn a_face_of_a_listed_sprite_takes_its_variant() {
        let t = table(&[entry("minecraft:block/b", "4F")]);
        // The numbers OptiFine's choice gives the up face of the blocks at the origin and at x = 1.
        assert_eq!(
            t.variant(1, &WHOLE, [0, 0, 0], 1),
            Some(Variant {
                turns: 2,
                flip: true
            })
        );
        assert_eq!(
            t.variant(1, &WHOLE, [1, 0, 0], 1),
            Some(Variant {
                turns: 3,
                flip: false
            })
        );
        // A sprite with no entry, and a layer beyond the table, stay as they are.
        assert_eq!(t.variant(0, &WHOLE, [0, 0, 0], 1), None);
        assert_eq!(t.variant(7, &WHOLE, [0, 0, 0], 1), None);
        assert_eq!(
            NaturalTextures::default().variant(0, &WHOLE, [0, 0, 0], 1),
            None
        );
    }

    #[test]
    fn a_variant_that_changes_nothing_is_none() {
        // The face toward north of the block at (12, 70, -5) gets neither a turn nor a flip
        // from a turn-only entry.
        let t = table(&[entry("minecraft:block/a", "4")]);
        assert_eq!(t.variant(0, &WHOLE, [12, 70, -5], 2), None);
    }

    #[test]
    fn only_a_whole_sprite_turns_but_any_face_flips() {
        let t = table(&[entry("minecraft:block/a", "4F")]);
        let half = [[0.0, 0.0], [0.0, 0.5], [0.5, 0.5], [0.5, 0.0]];
        // The up face at the origin turns by two quarters and flips on a whole sprite...
        assert_eq!(
            t.variant(0, &WHOLE, [0, 0, 0], 1),
            Some(Variant {
                turns: 2,
                flip: true
            })
        );
        // ... and only flips when it shows part of one.
        assert_eq!(
            t.variant(0, &half, [0, 0, 0], 1),
            Some(Variant {
                turns: 0,
                flip: true
            })
        );
        // The up face at x = 1 only turns: a part of a sprite stays as it is.
        assert_eq!(t.variant(0, &half, [1, 0, 0], 1), None);
    }

    #[test]
    fn a_whole_sprite_is_on_the_edges() {
        assert!(shows_whole_sprite(&WHOLE));
        assert!(shows_whole_sprite(&[
            [0.001, 0.0],
            [0.0, 1.0],
            [0.999, 1.0],
            [1.0, 0.0]
        ]));
        assert!(!shows_whole_sprite(&[
            [0.0, 0.0],
            [0.0, 1.0],
            [1.0, 0.9],
            [1.0, 0.0]
        ]));
        assert!(!shows_whole_sprite(&[
            [0.0625, 0.0],
            [0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0]
        ]));
    }

    #[test]
    fn entries_for_sprites_no_model_uses_are_dropped() {
        let t = table(&[
            entry("minecraft:block/missing", "4F"),
            entry("minecraft:block/a", "2"),
        ]);
        assert_eq!(t.by_layer.len(), 1);
        assert!(
            table(&[entry("minecraft:block/missing", "4F")])
                .by_layer
                .is_empty()
        );
    }

    #[test]
    fn sprites_are_listed_for_their_own_layers() {
        let rules = [entry("minecraft:block/a", "2"), entry("mymod:block/c", "F")];
        assert_eq!(sprites(&rules), ["minecraft:block/a", "mymod:block/c"]);
    }
}
