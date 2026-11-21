//! The HUD's layer registration order (NeoForge's `GuiLayerManager`), not its draw order.
//! Layers native mods register (`register_gui_layers`) are placed among the built-in anchors
//! once at start, as `RegisterGuiLayersEvent` inserts them.

use mod_api::{GuiLayer, GuiLayerPlacement, Identifier};
use std::sync::OnceLock;

/// Vanilla HUD paint IDs. Keep the current order of public layers; `false` IDs are
/// internal only, and neither register as anchors nor fire hooks.
const VANILLA_PAINT_IDS: [(&str, bool); 24] = [
    ("minecraft:camera_overlays", false),
    ("minecraft:crosshair", true),
    ("minecraft:after_camera_decorations", false),
    ("minecraft:hotbar", true),
    ("minecraft:effects", true),
    ("minecraft:boss_overlay", true),
    ("minecraft:vehicle_health", true),
    ("minecraft:contextual_info_bar_background", false),
    ("minecraft:contextual_info_bar", true),
    ("minecraft:experience_level", false),
    ("minecraft:player_health", true),
    ("minecraft:armor_level", false),
    ("minecraft:food_level", false),
    ("minecraft:air_level", false),
    ("minecraft:selected_item_name", true),
    ("minecraft:spectator_tooltip", false),
    ("minecraft:sleep_overlay", true),
    ("minecraft:demo_overlay", false),
    ("minecraft:overlay_message", true),
    ("minecraft:title", true),
    ("minecraft:scoreboard_sidebar", true),
    ("minecraft:subtitle_overlay", true),
    ("minecraft:chat", true),
    ("minecraft:tab_list", true),
];

/// The public HUD layer IDs in registration order: the anchors `register_gui_layers` accepts.
pub const VANILLA: [&str; 15] = {
    let mut ids = [""; 15];
    let mut layer = 0;
    let mut public = 0;
    while layer < VANILLA_PAINT_IDS.len() {
        let (id, exposed) = VANILLA_PAINT_IDS[layer];
        if exposed {
            ids[public] = id;
            public += 1;
        }
        layer += 1;
    }
    assert!(public == ids.len());
    ids
};

/// The whole layer order: vanilla's layers and the mods'.
#[derive(Debug, PartialEq, Eq)]
pub struct GuiLayers {
    /// Every layer, bottom first: a vanilla id from [`VANILLA`] or a mod's id.
    layers: Vec<String>,
    /// Whether any mod layer is in `layers`.
    mods: bool,
}

/// No mod layers: the HUD draws vanilla's alone, without looking up an order.
static NONE: GuiLayers = GuiLayers {
    layers: Vec::new(),
    mods: false,
};

/// The order with the native mods' layers, kept once at start (`register`).
static ORDER: OnceLock<GuiLayers> = OnceLock::new();

/// Places the layers native mods registered among vanilla's, once at start.
pub fn register(layers: Vec<GuiLayer>) {
    if ORDER.set(GuiLayers::place(layers)).is_err() {
        log::warn!("HUD layers were registered already; these are left out");
    }
}

/// The layer order now: vanilla's alone until the mods' layers are registered.
pub fn order() -> &'static GuiLayers {
    ORDER.get().unwrap_or(&NONE)
}

impl GuiLayers {
    /// `RegisterGuiLayersEvent.register` for each layer in push order: below or above every
    /// layer, or right below or above one already in the list. An id that is not
    /// `<namespace>:<path>` in a mod's namespace, is in the list already, or is placed against
    /// a layer that is not is logged and left out.
    pub fn place(registered: Vec<GuiLayer>) -> Self {
        let mut layers: Vec<String> = VANILLA.iter().map(|&id| id.to_owned()).collect();
        let mut mods = false;
        for GuiLayer { id, placement } in registered {
            let own =
                Identifier::parse_namespaced(&id).is_some_and(|id| id.namespace() != "minecraft");
            let index = |other: &str| layers.iter().position(|layer| layer == other);
            let at = match &placement {
                _ if !own => {
                    log::warn!("HUD layer {id:?} is not <namespace>:<path> in a mod's namespace");
                    continue;
                }
                _ if index(&id).is_some() => {
                    log::warn!("HUD layer {id} is registered twice; the first is kept");
                    continue;
                }
                GuiLayerPlacement::BelowAll => Some(0),
                GuiLayerPlacement::AboveAll => Some(layers.len()),
                GuiLayerPlacement::Below(other) => index(other),
                GuiLayerPlacement::Above(other) => index(other).map(|i| i + 1),
            };
            match at {
                Some(at) => {
                    layers.insert(at, id);
                    mods = true;
                }
                None => log::warn!(
                    "HUD layer {id} is placed against {placement:?}, which is not registered; \
                     it is left out"
                ),
            }
        }
        Self { layers, mods }
    }

    /// Whether a mod registered any layer.
    pub fn has_mod_layers(&self) -> bool {
        self.mods
    }

    /// How many layers there are, vanilla's and the mods'.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// Whether there are none: the order before the mods' layers are registered.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Where vanilla layer `id` is in the order.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.layers.iter().position(|layer| layer == id)
    }

    /// The mod layer at `index`; `None` for a vanilla layer or past the end.
    pub fn mod_layer(&self, index: usize) -> Option<&str> {
        self.layers
            .get(index)
            .map(String::as_str)
            .filter(|id| !VANILLA.contains(id))
    }

    /// Every layer's id, bottom first.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.layers.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(id: &str, placement: GuiLayerPlacement) -> GuiLayer {
        GuiLayer {
            id: id.into(),
            placement,
        }
    }

    /// The order around each mod layer: the layer under it and the one over it.
    fn around<'a>(order: &'a GuiLayers, id: &str) -> (Option<&'a str>, Option<&'a str>) {
        let ids: Vec<&str> = order.ids().collect();
        let at = ids.iter().position(|layer| *layer == id).unwrap();
        (
            at.checked_sub(1).map(|below| ids[below]),
            ids.get(at + 1).copied(),
        )
    }

    #[test]
    fn layers_go_where_register_gui_layers_event_inserts_them() {
        use GuiLayerPlacement::*;
        let order = GuiLayers::place(vec![
            layer("compass:bar", Above("minecraft:hotbar".into())),
            // Below the compass, so between the hotbar and it.
            layer("compass:shadow", Below("compass:bar".into())),
            layer("compass:first", BelowAll),
            layer("compass:last", AboveAll),
            // A second `BelowAll` goes under the first.
            layer("compass:very_first", BelowAll),
            layer("compass:over_chat", Above("minecraft:chat".into())),
        ]);
        assert!(order.has_mod_layers());
        assert_eq!(order.len(), VANILLA.len() + 6);
        assert_eq!(
            around(&order, "compass:shadow"),
            (Some("minecraft:hotbar"), Some("compass:bar"))
        );
        assert_eq!(
            around(&order, "compass:bar"),
            (Some("compass:shadow"), Some("minecraft:effects"))
        );
        assert_eq!(
            around(&order, "compass:very_first"),
            (None, Some("compass:first"))
        );
        assert_eq!(
            around(&order, "compass:first"),
            (Some("compass:very_first"), Some("minecraft:crosshair"))
        );
        assert_eq!(
            around(&order, "compass:over_chat"),
            (Some("minecraft:chat"), Some("minecraft:tab_list"))
        );
        assert_eq!(
            around(&order, "compass:last"),
            (Some("minecraft:tab_list"), None)
        );
        // Vanilla's own order is unchanged around them.
        let vanilla: Vec<&str> = order.ids().filter(|id| VANILLA.contains(id)).collect();
        assert_eq!(vanilla, VANILLA);
        assert_eq!(order.mod_layer(0), Some("compass:very_first"));
        assert_eq!(order.mod_layer(2), None, "the crosshair is vanilla's");
        assert_eq!(order.position("minecraft:crosshair"), Some(2));
    }

    #[test]
    fn a_layer_that_cannot_be_placed_is_left_out() {
        use GuiLayerPlacement::*;
        let order = GuiLayers::place(vec![
            layer("minecraft:hotbar", AboveAll),
            layer("minecraft:mine", AboveAll),
            layer("no_namespace", AboveAll),
            layer("compass:bar", AboveAll),
            layer("compass:bar", BelowAll),
            // Against a layer pushed later: not in the list yet.
            layer("compass:early", Below("compass:late".into())),
            layer("compass:late", AboveAll),
            layer("compass:lost", Above("minecraft:no_such_layer".into())),
        ]);
        let mods: Vec<&str> = order.ids().filter(|id| !VANILLA.contains(id)).collect();
        assert_eq!(mods, ["compass:bar", "compass:late"]);
        assert_eq!(
            order.position("compass:bar"),
            Some(VANILLA.len()),
            "the first registration of an id is kept"
        );
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
        let order = GuiLayers::place(
            staged_ids
                .into_iter()
                .map(|id| {
                    layer(
                        "compass:bar",
                        GuiLayerPlacement::Above(format!("minecraft:{id}")),
                    )
                })
                .collect(),
        );
        assert!(!order.has_mod_layers());
        assert!(
            order
                .ids()
                .map(|id| id.strip_prefix("minecraft:").unwrap())
                .eq([
                    "crosshair",
                    "hotbar",
                    "effects",
                    "boss_overlay",
                    "vehicle_health",
                    "contextual_info_bar",
                    "player_health",
                    "selected_item_name",
                    "sleep_overlay",
                    "overlay_message",
                    "title",
                    "scoreboard_sidebar",
                    "subtitle_overlay",
                    "chat",
                    "tab_list",
                ])
        );
        assert!(order.mod_layer(0).is_none());
        assert!(!NONE.has_mod_layers() && NONE.is_empty());
    }
}
