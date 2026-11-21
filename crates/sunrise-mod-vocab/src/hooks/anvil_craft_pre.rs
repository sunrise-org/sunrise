//! The `anvil_craft_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0238

pub trait Hooks {
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`), before the levels are charged, the inputs used up and the anvil worn. `inputs` borrows the input stacks and result from before the take. Set `cancel` to refuse the take: the result stays in its slot and the inputs, levels, inventory and cursor are as before the click.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_anvil_craft_pre",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilCraftEvent.Pre",
        cancellable = true,
        version = 2
    )]
    fn anvil_craft_pre(
        player: u128,
        name: &str,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilCraftInputs<'_>,
        cancel: &mut bool,
    );
}
