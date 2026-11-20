use super::Game;
use mod_api::__private::{
    ResolvedPlayerLayerTexture, player_layer_selection, resolved_player_layer_texture,
};
use mod_api::{EntityUuid, PlayerLayer, PlayerLayerRefusal, PlayerLayerSelection, PlayerLayerView};

impl Game<'_> {
    fn native_resolve_player_layer(
        &self,
        player: EntityUuid,
        local: bool,
        view: PlayerLayerView,
        layer: PlayerLayer,
        selection: &mut PlayerLayerSelection<'_>,
    ) {
        self.fire_resolve_player_layer(player, local, view, layer, selection);
    }

    pub(in crate::game) fn resolve_player_layer_texture(
        &self,
        entity: u32,
        local: bool,
        view: PlayerLayerView,
        layer: PlayerLayer,
    ) -> ResolvedPlayerLayerTexture {
        let player = if local {
            self.own_uuid()
        } else if let Some(uuid) = self
            .remote
            .as_ref()
            .and_then(|remote| remote.player_uuid(entity))
        {
            uuid
        } else {
            return ResolvedPlayerLayerTexture::Keep;
        };
        let resolve = |subscriber: &str, id: &mod_api::Identifier| {
            let rect = self
                .dynamic_textures
                .owned_region(
                    self.gui.atlas.dynamic_texture_pool.as_ref(),
                    Some(subscriber),
                    id,
                )
                .map_err(PlayerLayerRefusal::Texture)?;
            // Both built-in meshes sample the 64×32 layout: PlayerCapeModel's
            // 64×64 layer definition halves the cape cube's UV height.
            if !rect.w.is_multiple_of(64) || rect.w / 2 != rect.h {
                return Err(PlayerLayerRefusal::IncompatibleLayout);
            }
            Ok([rect.u0, rect.v0, rect.u1, rect.v1])
        };
        let mut selection = player_layer_selection(layer, &resolve);
        self.native_resolve_player_layer(EntityUuid(player), local, view, layer, &mut selection);
        resolved_player_layer_texture(selection)
    }
}
