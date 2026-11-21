//! The HUD's layer order (NeoForge's `GuiLayerManager`): vanilla's layers in the order Sunrise
//! draws them, with native layers inserted and bodies installed once at client startup.

use mod_api::{
    GuiLayerDraw, GuiLayerPlacement, GuiLayerRegistrar, GuiLayerWrapper, Identifier,
    content::crash::current_native_mod,
};
use std::sync::OnceLock;

/// Vanilla HUD anchors used by the engine's HUD pass (no camera overlays or debug screen).
#[derive(Clone, Copy)]
pub enum VanillaGuiLayer {
    Crosshair,
    Hotbar,
    Effects,
    BossOverlay,
    VehicleHealth,
    PlayerHealth,
    ContextualInfoBar,
    SelectedItemName,
    SleepOverlay,
    OverlayMessage,
    Title,
    ScoreboardSidebar,
    SubtitleOverlay,
    Chat,
    TabList,
}

impl VanillaGuiLayer {
    /// Vanilla anchors in the layer registrar's bottom-to-top order.
    pub const ALL: [Self; 15] = [
        Self::Crosshair,
        Self::Hotbar,
        Self::Effects,
        Self::BossOverlay,
        Self::VehicleHealth,
        Self::PlayerHealth,
        Self::ContextualInfoBar,
        Self::SelectedItemName,
        Self::SleepOverlay,
        Self::OverlayMessage,
        Self::Title,
        Self::ScoreboardSidebar,
        Self::SubtitleOverlay,
        Self::Chat,
        Self::TabList,
    ];

    /// The vanilla layer ID reported to HUD observers.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Crosshair => "minecraft:crosshair",
            Self::Hotbar => "minecraft:hotbar",
            Self::Effects => "minecraft:effects",
            Self::BossOverlay => "minecraft:boss_overlay",
            Self::VehicleHealth => "minecraft:vehicle_health",
            Self::PlayerHealth => "minecraft:player_health",
            Self::ContextualInfoBar => "minecraft:contextual_info_bar",
            Self::SelectedItemName => "minecraft:selected_item_name",
            Self::SleepOverlay => "minecraft:sleep_overlay",
            Self::OverlayMessage => "minecraft:overlay_message",
            Self::Title => "minecraft:title",
            Self::ScoreboardSidebar => "minecraft:scoreboard_sidebar",
            Self::SubtitleOverlay => "minecraft:subtitle_overlay",
            Self::Chat => "minecraft:chat",
            Self::TabList => "minecraft:tab_list",
        }
    }
}

// no wire: slots own process-local layer bodies.
/// An installed HUD layer, with an ID and anchor independent of its body.
pub struct GuiLayerSlot {
    id: GuiLayerId,
    pub body: GuiLayerBody,
}

enum GuiLayerId {
    Vanilla(VanillaGuiLayer),
    Native(String),
}

// no wire: native bodies own process-local draw callbacks.
/// The current implementation of one HUD slot.
pub enum GuiLayerBody {
    Vanilla,
    Native {
        owner: Option<&'static str>,
        draw: Box<GuiLayerDraw>,
    },
    Wrapped {
        owner: Option<&'static str>,
        wrap: Box<GuiLayerWrapper>,
        previous: Box<GuiLayerBody>,
    },
}

impl GuiLayerSlot {
    /// The stable slot ID reported by Pre and Post.
    pub fn id(&self) -> &str {
        match &self.id {
            GuiLayerId::Vanilla(layer) => layer.id(),
            GuiLayerId::Native(id) => id,
        }
    }

    /// The engine paint site for a vanilla slot, even when its body is replaced or wrapped.
    pub fn vanilla_anchor(&self) -> Option<VanillaGuiLayer> {
        match &self.id {
            GuiLayerId::Vanilla(layer) => Some(*layer),
            GuiLayerId::Native(_) => None,
        }
    }
}

/// The client startup registrar, seeded with vanilla's layers.
pub struct GuiLayers {
    slots: Vec<GuiLayerSlot>,
}

impl Default for GuiLayers {
    fn default() -> Self {
        Self {
            slots: VanillaGuiLayer::ALL
                .into_iter()
                .map(|layer| GuiLayerSlot {
                    id: GuiLayerId::Vanilla(layer),
                    body: GuiLayerBody::Vanilla,
                })
                .collect(),
        }
    }
}

impl GuiLayerRegistrar for GuiLayers {
    fn register(
        &mut self,
        id: String,
        placement: GuiLayerPlacement,
        draw: Box<GuiLayerDraw>,
    ) -> bool {
        if Identifier::parse_namespaced(&id).is_none_or(|id| id.namespace() == "minecraft") {
            log::warn!(
                "HUD layer {id:?} must be <namespace>:<path> outside the minecraft namespace"
            );
            return false;
        }
        let position = |key: &str| self.slots.iter().position(|slot| slot.id() == key);
        if position(&id).is_some() {
            log::warn!("HUD layer {id} is registered twice; the first is kept");
            return false;
        }
        let at = match &placement {
            GuiLayerPlacement::BelowAll => Some(0),
            GuiLayerPlacement::AboveAll => Some(self.slots.len()),
            GuiLayerPlacement::Below(other) => position(other),
            GuiLayerPlacement::Above(other) => position(other).map(|i| i + 1),
        };
        let owner = current_native_mod();
        let Some(at) = at else {
            log::warn!(
                "{} cannot register HUD layer {id}: no registered anchor for {placement:?}",
                owner.unwrap_or("unknown mod")
            );
            return false;
        };
        self.slots.insert(
            at,
            GuiLayerSlot {
                id: GuiLayerId::Native(id),
                body: GuiLayerBody::Native { owner, draw },
            },
        );
        true
    }

    fn replace_layer(&mut self, id: &str, draw: Box<GuiLayerDraw>) -> bool {
        let owner = current_native_mod();
        let Some(slot) = self.slot_for_edit(id, owner, "replace") else {
            return false;
        };
        slot.body = GuiLayerBody::Native { owner, draw };
        true
    }

    fn wrap_layer(&mut self, id: &str, wrap: Box<GuiLayerWrapper>) -> bool {
        let owner = current_native_mod();
        let Some(slot) = self.slot_for_edit(id, owner, "wrap") else {
            return false;
        };
        let previous = Box::new(std::mem::replace(&mut slot.body, GuiLayerBody::Vanilla));
        slot.body = GuiLayerBody::Wrapped {
            owner,
            wrap,
            previous,
        };
        true
    }
}

/// The frozen draw order, with a direct index for each vanilla anchor.
pub struct GuiLayerOrder {
    slots: Vec<GuiLayerSlot>,
    positions: [usize; VanillaGuiLayer::ALL.len()],
    has_native_bodies: bool,
}

impl GuiLayers {
    fn slot_for_edit(
        &mut self,
        id: &str,
        owner: Option<&str>,
        action: &str,
    ) -> Option<&mut GuiLayerSlot> {
        let slot = self.slots.iter_mut().find(|slot| slot.id() == id);
        if slot.is_none() {
            log::warn!(
                "{} cannot {action} HUD layer {id}: it is not registered",
                owner.unwrap_or("unknown mod")
            );
        }
        slot
    }

    fn into_order(self) -> GuiLayerOrder {
        let mut positions = [0; VanillaGuiLayer::ALL.len()];
        let mut has_native_bodies = false;
        for (at, slot) in self.slots.iter().enumerate() {
            if let Some(layer) = slot.vanilla_anchor() {
                positions[layer as usize] = at;
            }
            has_native_bodies |= !matches!(slot.body, GuiLayerBody::Vanilla);
        }
        GuiLayerOrder {
            slots: self.slots,
            positions,
            has_native_bodies,
        }
    }
}

impl GuiLayerOrder {
    /// The position of a vanilla anchor, including anchors skipped this frame.
    pub fn position(&self, layer: VanillaGuiLayer) -> usize {
        self.positions[layer as usize]
    }

    /// Every layer, bottom first.
    pub fn slots(&self) -> &[GuiLayerSlot] {
        &self.slots
    }
}

static ORDER: OnceLock<GuiLayerOrder> = OnceLock::new();

/// Publish the client startup registration once, before any HUD draws.
pub fn freeze(layers: GuiLayers) {
    if ORDER.set(layers.into_order()).is_err() {
        log::warn!("HUD layers were registered already; these are left out");
    }
}

/// The installed order, if it has a mod's own layer, replacement or wrapper.
pub fn order() -> Option<&'static GuiLayerOrder> {
    ORDER.get().filter(|order| order.has_native_bodies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_go_where_register_gui_layers_event_inserts_them() {
        use GuiLayerPlacement::*;
        let mut layers = GuiLayers::default();
        for (id, placement) in [
            ("compass:bar", Above("minecraft:hotbar".into())),
            // Below the compass, so between the hotbar and it.
            ("compass:shadow", Below("compass:bar".into())),
            ("compass:first", BelowAll),
            ("compass:last", AboveAll),
            // A second `BelowAll` goes under the first.
            ("compass:very_first", BelowAll),
            ("compass:over_chat", Above("minecraft:chat".into())),
        ] {
            assert!(layers.register(id.into(), placement, Box::new(|_, _| {})));
        }
        let order = layers.into_order();
        let ids: Vec<&str> = order.slots().iter().map(GuiLayerSlot::id).collect();
        assert_eq!(
            ids,
            [
                "compass:very_first",
                "compass:first",
                "minecraft:crosshair",
                "minecraft:hotbar",
                "compass:shadow",
                "compass:bar",
                "minecraft:effects",
                "minecraft:boss_overlay",
                "minecraft:vehicle_health",
                "minecraft:player_health",
                "minecraft:contextual_info_bar",
                "minecraft:selected_item_name",
                "minecraft:sleep_overlay",
                "minecraft:overlay_message",
                "minecraft:title",
                "minecraft:scoreboard_sidebar",
                "minecraft:subtitle_overlay",
                "minecraft:chat",
                "compass:over_chat",
                "minecraft:tab_list",
                "compass:last",
            ]
        );
        for layer in VanillaGuiLayer::ALL {
            assert_eq!(ids[order.position(layer)], layer.id());
        }
    }

    #[test]
    fn a_layer_that_cannot_be_placed_is_left_out() {
        use GuiLayerPlacement::*;
        let mut layers = GuiLayers::default();
        for (id, placement, accepted) in [
            ("minecraft:hotbar", AboveAll, false),
            ("minecraft:mine", AboveAll, false),
            ("no_namespace", AboveAll, false),
            ("compass:bad space", AboveAll, false),
            ("compass:bar", AboveAll, true),
            ("compass:bar", BelowAll, false),
            // An unknown anchor does not reserve the ID: it can be registered again later.
            ("compass:early", Below("compass:late".into()), false),
            ("compass:late", AboveAll, true),
            ("compass:early", Below("compass:late".into()), true),
            (
                "compass:lost",
                Above("minecraft:no_such_layer".into()),
                false,
            ),
        ] {
            assert_eq!(
                layers.register(id.into(), placement, Box::new(|_, _| {})),
                accepted,
                "registration of {id}"
            );
        }
        let order = layers.into_order();
        let mods: Vec<&str> = order
            .slots()
            .iter()
            .filter(|slot| slot.vanilla_anchor().is_none())
            .map(GuiLayerSlot::id)
            .collect();
        assert_eq!(mods, ["compass:bar", "compass:early", "compass:late"]);
        assert_eq!(
            order.slots()[VanillaGuiLayer::ALL.len()].id(),
            "compass:bar"
        );
    }

    #[test]
    fn no_registration_leaves_vanilla_alone() {
        let order = GuiLayers::default().into_order();
        assert!(
            order
                .slots()
                .iter()
                .all(|slot| matches!(slot.body, GuiLayerBody::Vanilla))
        );
        for (at, layer) in VanillaGuiLayer::ALL.into_iter().enumerate() {
            assert_eq!(order.position(layer), at);
            assert_eq!(order.slots()[at].id(), layer.id());
        }
    }

    #[test]
    fn only_existing_vanilla_layers_are_registration_anchors() {
        let staged_ids = [
            "camera_overlays",
            "after_camera_decorations",
            "contextual_info_bar_background",
            "experience_level",
            "armor_level",
            "food_level",
            "air_level",
            "spectator_tooltip",
            "demo_overlay",
        ];
        let mut layers = GuiLayers::default();
        for id in staged_ids {
            assert!(!layers.register(
                "compass:bar".into(),
                GuiLayerPlacement::Above(format!("minecraft:{id}")),
                Box::new(|_, _| {}),
            ));
        }
        let order = layers.into_order();
        assert_eq!(order.slots().len(), VanillaGuiLayer::ALL.len());
        assert!(
            order
                .slots()
                .iter()
                .all(|slot| slot.vanilla_anchor().is_some())
        );
    }
}
