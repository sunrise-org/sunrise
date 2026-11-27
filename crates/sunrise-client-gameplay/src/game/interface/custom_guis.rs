//! OptiFine Custom GUIs on the container screens: what the open screen belongs to, as a pack's
//! rules ask it (`sunrise_optifine::gui`), and the sprite drawn in place of one of its textures
//! (`HudAtlas::container_sprite`).

use crate::game::ClientGame;
use sunrise_level::containers::is_chest_block;
use sunrise_optifine::gui::{Container, Detail, Subject, Variant};
use sunrise_sim::mobs::{Mob, Species, llama};

/// The block a mob stands in: where the screen of its inventory or trades is.
pub(in crate::game) fn mob_block(mob: &Mob) -> [i32; 3] {
    [
        mob.position.x.floor() as i32,
        mob.position.y.floor() as i32,
        mob.position.z.floor() as i32,
    ]
}

/// What a mount is, for a horse screen: its kind (none for a zombie or skeleton horse) and the
/// carpet a llama wears.
pub(in crate::game) fn mount_detail(mob: &Mob) -> Detail<'_> {
    let variant = match mob.species() {
        Species::Horse => Some(Variant::Horse),
        Species::Donkey => Some(Variant::Donkey),
        Species::Mule => Some(Variant::Mule),
        Species::Llama | Species::TraderLlama => Some(Variant::Llama),
        _ => None,
    };
    Detail::Horse {
        variant,
        carpet: llama::carpet(mob),
    }
}

impl ClientGame<'_> {
    /// The sprite to draw for `default`, a texture of the open `container` screen
    /// (`gui/container/furnace`): the picture a pack's Custom GUIs rules put there for the screen of
    /// the block at `at`, with the title or custom name it shows and what its block or entity is
    /// (`detail`); `default` itself when no rule holds.
    pub(in crate::game) fn gui_sprite<'a>(
        &'a self,
        default: &'a str,
        container: Container,
        at: [i32; 3],
        name: Option<&str>,
        detail: Detail<'_>,
    ) -> &'a str {
        self.client.gui.atlas.container_sprite(default, || Subject {
            container,
            biome: self.biome_at(at.map(f64::from)),
            y: at[1],
            name,
            detail,
        })
    }

    /// The block the open screen is of: the one the server opened its menu from, else the open
    /// station's or container's, else the one the player stands in (the inventories' screens, an
    /// entity's or a minecart's menu).
    pub(in crate::game) fn gui_block(&self) -> [i32; 3] {
        let opened_from = self.client.remote.as_ref().and_then(|r| r.menu.target);
        let station = self.common.station.as_ref().map(|s| s.pos);
        // A minecart's and a double chest's menu hold their slots under keys no block has.
        let container = self
            .common
            .container
            .filter(|k| {
                k.0 != i32::MIN && k.1 != sunrise_gameplay_common::game::minecarts::CART_KEY_Y
            })
            .map(|k| [k.0, k.1, k.2]);
        opened_from.or(station).or(container).unwrap_or_else(|| {
            let at = self.player_position();
            at.map(|c| c.floor() as i32)
        })
    }

    /// What the block at `at` is, for a chest screen: a chest of any kind, an ender chest. A
    /// barrel, a minecart and anything else has no chest to ask about.
    pub(in crate::game) fn gui_chest(&self, at: [i32; 3]) -> Detail<'static> {
        let registry = self.common.pack.assets.registry;
        let Some(state) = registry.state(self.common.world.block(at[0], at[1], at[2])) else {
            return Detail::None;
        };
        let name: &str = &state.name;
        let ender = name == "minecraft:ender_chest";
        if !ender && !is_chest_block(name) {
            return Detail::None;
        }
        Detail::Chest {
            large: sunrise_gameplay_common::chests::chest_half(state)
                .is_some_and(|half| half.kind != "single"),
            trapped: name == "minecraft:trapped_chest",
            christmas: sunrise_client::block_entity_render::extended_christmas(),
            ender,
        }
    }
}
