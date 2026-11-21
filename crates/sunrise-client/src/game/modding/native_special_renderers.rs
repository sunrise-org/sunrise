//! Game-side stack and draw context for special item renderers. The client item renderer owns
//! the scoped submission, including rollback and per-resource-generation failures.
use super::{
    Game,
    api_values::{TryIntoApi, engine_display_context, stack_value},
};
use mod_api::{ItemDisplayContext, ItemStack, SpecialModelContext};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    sync::{Mutex, PoisonError},
};
use sunrise_assets::item_model::{Context, DisplayContext, StackView};
use sunrise_client::item_render::{BakedSpecialModel, SpecialModelInput, stack_view};

pub(in crate::game) fn draw_input<'a>(
    game: &'a Game<'_>,
    model: &BakedSpecialModel,
    stack: &StackView,
    context: &Context<'_>,
) -> Option<SpecialModelInput<'a>> {
    let input = mod_api::__private::item_key(&stack.item)
        .zip(u32::try_from(stack.count).ok().and_then(NonZeroU32::new));
    let Some((item, count)) = input else {
        // A stack the API can't read is refused alone: the renderer isn't at fault.
        static SEEN: Mutex<BTreeMap<String, BTreeSet<String>>> = Mutex::new(BTreeMap::new());
        let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
        if !seen
            .get(model.kind.as_ref())
            .is_some_and(|items| items.contains(&stack.item))
        {
            seen.entry(model.kind.to_string())
                .or_default()
                .insert(stack.item.clone());
            drop(seen);
            log::warn!(
                "Sunrise: a stack of {} can't be read for the special model renderer of {}; it draws the missing model",
                stack.item,
                model.kind
            );
        }
        return None;
    };
    let Ok(display) = context.display.try_into_api();
    Some(SpecialModelInput {
        host: game,
        item,
        count,
        context: SpecialModelContext::new(
            display,
            context.level.map(|level| level.game_time),
            game.frame_world_partial_tick() as f32,
        ),
    })
}

/// Convert only an explicitly submitted nested stack; ordinary input stays borrowed.
pub(in crate::game) fn nested_stack(
    stack: &ItemStack,
    display: ItemDisplayContext,
) -> Option<(StackView, DisplayContext)> {
    let display = engine_display_context(display)?;
    // The item resolver's stack count is signed.
    i32::try_from(stack.count()).ok()?;
    let stack = stack_value(stack).ok()?;
    Some((stack_view(&stack), display))
}

#[cfg(test)]
#[path = "native_special_renderers/cost_tests.rs"]
mod cost_tests;
#[cfg(test)]
#[path = "native_special_renderers/frame_tests.rs"]
pub(in crate::game) mod frame_tests;
