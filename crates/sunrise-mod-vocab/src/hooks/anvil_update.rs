//! The `anvil_update` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An anvil at `pos` made vanilla's result with a left input, for
    /// `player` (UUID) and `player_entity_id` (`AnvilUpdateEvent`, after
    /// `AnvilMenu.createResult`), whenever an input or the name changed:
    /// `inputs` lends the stacks, rename text and vanilla result; `edits`
    /// starts with `Keep` and vanilla's costs. Set an output or edit the
    /// costs without changing the inputs. Cancellation or an invalid
    /// replacement clears the result and both costs. Both sides: the
    /// server's menu and a server's client menu with the local player.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilUpdateEvent",
        cancellable = true,
        version = 2
    )]
    fn anvil_update(
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
        edits: &mut mod_api::item::menu::AnvilUpdateEdits,
        cancel: &mut bool,
    );
}
