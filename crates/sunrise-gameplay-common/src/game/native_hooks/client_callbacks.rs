use crate::game::native_living_body::BodyReads;
use mod_api::__private::LivingParty;

pub fn install_client_hooks() {
    crate::game::api_values::install_stack_hooks();
    if !sunrise_sim::native_client_hooks::install(
        sunrise_sim::native_client_hooks::NativeClientHooks {
            menu_still_valid: crate::game::api_values::menu_still_valid,
            menu_still_valid_listens: |menu| {
                mod_api::__private::menu_listens(mod_api::HookId::MenuStillValid, menu)
            },
            living_heal: |context, healed, amount, cancel| {
                if !mod_api::__private::listened(mod_api::HookId::LivingHeal) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                with_lent_view(healed, |view| {
                    mod_api::dispatch_living_heal(&context, healed.id(), view, amount, cancel);
                });
            },
            living_breathe_heard: || mod_api::__private::listened(mod_api::HookId::LivingBreathe),
            has_mods: || !mod_api::__private::mods().is_empty(),
            player_name_format: |context, username, displayname| {
                use crate::game::api_values::TryIntoApi;
                if !mod_api::__private::listened(mod_api::HookId::PlayerNameFormat) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                let Ok(username) = username.try_into_api() else {
                    return;
                };
                let Ok(mut converted) = displayname.try_into_api() else {
                    return;
                };
                mod_api::dispatch_player_name_format(&context, &username, &mut converted);
                if let Ok(converted) = sunrise_sim::api_text::to_serialized_engine(&converted) {
                    *displayname = converted;
                }
            },
            player_tab_list_name_format: |context, player, name, display_name| {
                use crate::game::api_values::TryIntoApi;
                if !mod_api::__private::listened(mod_api::HookId::PlayerTabListNameFormat) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                let Ok(mut converted) = display_name.try_into_api() else {
                    return;
                };
                mod_api::dispatch_player_tab_list_name_format(
                    &context,
                    player,
                    name,
                    &mut converted,
                );
                if let Ok(converted) = converted
                    .as_ref()
                    .map(sunrise_sim::api_text::to_serialized_engine)
                    .transpose()
                {
                    *display_name = converted;
                }
            },
            living_drown: |context,
                           entity,
                           kind,
                           is_drowning,
                           damage_amount,
                           bubble_count,
                           cancel| {
                if !mod_api::__private::listened(mod_api::HookId::LivingDrown) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                mod_api::dispatch_living_drown(
                    &context,
                    entity,
                    kind,
                    is_drowning,
                    damage_amount,
                    bubble_count,
                    cancel,
                );
            },
            living_fall: |context, entity, kind, distance, multiplier, cancel| {
                if !mod_api::__private::listened(mod_api::HookId::LivingFall) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                mod_api::dispatch_living_fall(&context, entity, kind, distance, multiplier, cancel);
            },
            living_jump: |context, entity, kind, velocity| {
                if !mod_api::__private::listened(mod_api::HookId::LivingJump) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                mod_api::dispatch_living_jump(&context, entity, kind, velocity);
            },
            living_jump_power: |context, living, power| {
                if !mod_api::__private::listened(mod_api::HookId::LivingJumpPower) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                with_lent_view(living, |view| {
                    mod_api::dispatch_living_jump_power(
                        &context,
                        living.id(),
                        living.kind(),
                        view,
                        power,
                    );
                });
            },
            living_speed: |context, living, speed| {
                if !mod_api::__private::listened(mod_api::HookId::LivingSpeed) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                with_lent_view(living, |view| {
                    mod_api::dispatch_living_speed(
                        &context,
                        living.id(),
                        living.kind(),
                        view,
                        speed,
                    );
                });
            },
            living_flying_speed: |context, living, speed| {
                if !mod_api::__private::listened(mod_api::HookId::LivingFlyingSpeed) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                with_lent_view(living, |view| {
                    mod_api::dispatch_living_flying_speed(
                        &context,
                        living.id(),
                        living.kind(),
                        view,
                        speed,
                    );
                });
            },
            block_jump_factor: |context, state, factor| {
                crate::game::block_jump_factor(context, state, factor);
            },
            block_speed_factor: |context, state, factor| {
                crate::game::block_speed_factor(context, state, factor);
            },
            is_ladder: |context, entity, pos, state, ladder| {
                crate::game::is_ladder(context, entity, pos, state, ladder);
            },
            block_interaction_range: |context, player, pos, buffer, within| {
                if !mod_api::__private::listened(mod_api::HookId::BlockInteractionRange) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                mod_api::dispatch_block_interaction_range(
                    &reader.context(),
                    player,
                    pos,
                    buffer,
                    within,
                );
            },
            makes_open_trapdoor_above_climbable:
                crate::game::native_trapdoor_ladder::makes_open_trapdoor_above_climbable,
            friction: crate::game::native_surface::friction,
            collision_shape: |context, pos, state, query, shape| {
                use crate::game::api_values::{TryIntoApi, block_state::block_listens};
                if !block_listens(mod_api::HookId::CollisionShape, state) {
                    return;
                }
                let Ok(state) = state.try_into_api() else {
                    return;
                };
                let mut answer = None;
                mod_api::dispatch_collision_shape(
                    &sunrise_level::api_context::LoopReader(context).context(),
                    pos,
                    state,
                    query,
                    &mut answer,
                );
                *shape = answer.map(mod_api::__private::voxel_shape_into_core);
            },
            player_stays_on_ground_surface: |context, entity, staying| {
                if !mod_api::__private::listened(mod_api::HookId::PlayerStaysOnGroundSurface) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                mod_api::dispatch_player_stays_on_ground_surface(&context, entity, staying);
            },
            ender_entity_teleport: |context, entity, kind, from, target, cancel| {
                if !mod_api::__private::listened(mod_api::HookId::EnderEntityTeleport) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                mod_api::dispatch_ender_entity_teleport(
                    &context, entity, kind, from, target, cancel,
                );
            },
            fluid_flow: |context, fluid, pos, flow| {
                if !mod_api::__private::listened(mod_api::HookId::FluidFlow) {
                    return;
                }
                let reader = sunrise_level::api_context::LoopReader(context);
                let context = reader.context();
                mod_api::dispatch_fluid_flow(&context, fluid, pos, flow);
            },
            sound_type: |level, pos, state, entity, cause, own| {
                use crate::game::api_values::block_state::block_listens;
                if !block_listens(mod_api::HookId::SoundType, state) {
                    return None;
                }
                let mut sound = own.clone();
                crate::game::native_surface::FrictionView(level, None)
                    .native_sound_type(pos, state, entity, cause, &mut sound);
                Some(sound)
            },
            can_grindstone_repair: crate::game::native_item_menu_parts::can_grindstone_repair,
            is_combine_repairable: crate::game::native_item_menu_parts::is_combine_repairable,
            can_fit_inside_container_items:
                crate::game::native_item_menu_parts::can_fit_inside_container_items,
            item_name: crate::game::native_item_menu_parts::item_name,
        },
    ) {
        log::warn!("Native client hooks already installed; keeping the first callbacks");
    }
}

fn with_lent_view(
    lend: sunrise_sim::health::LivingLend<'_>,
    dispatch: impl FnOnce(&mod_api::LivingView<'_>),
) {
    let Some(reads) = (lend.reads)() else {
        return;
    };
    let lent = BodyReads {
        body: lend.living_body(&reads),
        reads,
    };
    if let Some(view) = mod_api::LivingView::new(&lent, LivingParty::Subject) {
        dispatch(&view);
    }
}
