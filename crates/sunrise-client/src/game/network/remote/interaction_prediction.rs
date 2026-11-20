use super::*;

impl Game<'_> {
    /// `Minecraft.startUseItem`'s entity turn for `hand`: Interact goes to
    /// the server, and whether the click ends there is what this client's
    /// copy of the entity answers (`MultiPlayerGameMode.interact`).
    pub(super) fn remote_use_entity(&mut self, hand: Hand) -> bool {
        // A menu this use opens (a chest minecart's) is no clicked block's.
        if let Some(r) = self.remote.as_mut() {
            r.use_target = None;
        }
        if self.uses_stored_pick() {
            let Some(mod_api::CrosshairTarget::Entity { entity, .. }) = self.crosshair_target()
            else {
                return false;
            };
            let Some([lo, hi]) = self.entity_bounding_box(entity) else {
                return false;
            };
            let Some(eye) = self.eye_position() else {
                return false;
            };
            // Client interact range: closest box point, strict bound and no server buffer.
            // Refuse this use without choosing another target or the block behind it.
            let distance_squared = (0..3)
                .map(|i| (eye[i] - eye[i].clamp(lo[i], hi[i])).powi(2))
                .sum::<f64>();
            if distance_squared >= self.entity_reach.powi(2) {
                return false;
            }
        }
        let sneaking = self.remote_shift_down();
        // A boat, minecart, item frame, armor stand or painting nearer than
        // any mob: the server uses it (AbstractBoat.interact boards it).
        if let Some(hit) = self.remote_object_hit(false)
            && self.mob_hit().is_none_or(|(_, d, _)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // Interaction.interact answers SUCCESS when it responds and
            // CONSUME otherwise: both end the click (nothing behind the box
            // is used), but only SUCCESS swings; the packet carries the hand.
            let response = self.interactions.get(&hit.id).map(|i| i.response);
            // Objects whose interact passes (paintings, empty or fixed item frames,
            // boats sneaked at or out of control, carts that pass) leave each hand's
            // turn to its item: a crossbow aimed at it shoots.
            let is_painting = self.paintings.iter().any(|p| p.id as i32 == hit.id);
            let held = self.with_hand(hand, |g| g.held().cloned());
            let frame_passes = self
                .item_frames
                .iter()
                .find(|f| f.id as i32 == hit.id)
                .is_some_and(|f| {
                    item_frame_interact_passes(f.fixed, f.item.is_some(), held.is_some())
                });
            let boat_passes = self
                .boats
                .iter()
                .enumerate()
                .find(|(_, b)| b.id as i32 == hit.id)
                .is_some_and(|(i, b)| {
                    boat_interact_passes(
                        b.chest,
                        sneaking,
                        b.out_of_control >= 60.0,
                        self.boat_has_room(i),
                    )
                });
            let (cart_passes, is_rideable_cart) = self
                .carts
                .iter()
                .find(|c| c.id as i32 == hit.id)
                .map(|c| {
                    let ridden = self.riding == Some(c.id)
                        || self.mobs.iter().any(|m| m.extra.mount == Some(c.id));
                    (
                        c.kind
                            .interact_passes(sneaking, ridden, self.can_use_game_master_blocks()),
                        c.kind == crate::game::minecarts::CartKind::Rideable,
                    )
                })
                .unwrap_or((false, false));
            // Cushion.interact on a client: PASS when sneaked at or when
            // something already sits on it, CONSUME (no swing) otherwise.
            let is_cushion = self.cushions.iter().any(|c| c.id as i32 == hit.id);
            let cushion_passes = is_cushion
                && (sneaking
                    || self.riding.is_some_and(|r| r as i32 == hit.id)
                    || self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.has_passengers(hit.id as u32)));
            // EndCrystal and EnderDragonPart keep Entity.interact's PASS.
            let crystal_passes = self.crystals.iter().any(|c| c.id as i32 == hit.id)
                || self.level_dragon_part(hit.id as u32).is_some();
            let passes = self.shown_remote_general(hit.id).is_some()
                || is_painting
                || frame_passes
                || boat_passes
                || cart_passes
                || cushion_passes
                || crystal_passes;
            // Otherwise the main hand's turn takes it; there is no off hand turn.
            if response.is_none() && hand == Hand::Off && !passes {
                return false;
            }
            self.send(S::Interact(play::Interact {
                id: hit.id,
                hand: hand_id(hand),
                location: hit.location,
                secondary: sneaking,
            }));
            if let Some(done) = self.entity_interaction(hand, hit.id) {
                return done;
            }
            if passes {
                return false;
            }
            // Minecart.interact resets the passenger's easing baseline on
            // a successful boarding click, not on SetPassengers (commands
            // can mount a player without this interaction).
            if let Some(i) = self.carts.iter().position(|c| {
                c.id as i32 == hit.id && c.kind == crate::game::minecarts::CartKind::Rideable
            }) {
                self.carts
                    .update(i, |c| c.player_rotation_offset = c.rotation_offset);
            }
            // MinecartCommandBlock.interact on the client: the editor opens.
            if self.carts.iter().any(|c| {
                c.id as i32 == hit.id && c.kind == crate::game::minecarts::CartKind::CommandBlock
            }) {
                self.open_cart_command_block(hit.id as u32);
            }
            // Minecart.interact boards the cart and answers CONSUME on the client,
            // which ends the click without an arm swing. AbstractBoat.interact
            // answers SUCCESS there, which swings. Cushion.interact answers
            // CONSUME there too.
            if remote_object_swings(response, is_rideable_cart || is_cushion) {
                self.with_hand(hand, Game::swing_arm);
            }
            return true;
        }
        if let Some((i, distance, location)) = self.mob_hit()
            && self.before_block(distance)
        {
            let id = self.mobs[i].id as i32;
            self.send(S::Interact(play::Interact {
                id,
                hand: hand_id(hand),
                location,
                secondary: sneaking,
            }));
            if let Some(done) = self.entity_interaction(hand, id) {
                return done;
            }
            // Player.interactOn: another player passes, and so does a mob
            // the held item means nothing to: the item is used instead.
            let player = self
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id));
            if !player && self.remote_mob_takes(i, hand) {
                self.remote_feed_particles(i, hand);
                // As for the objects above, only SUCCESS swings the hand: a
                // CONSUME ends the click without one.
                if self.remote_mob_swings(i, hand) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            }
            if !player
                && self.with_hand(hand, |g| {
                    g.item_interact_mob(Some(crate::game::InteractTarget { id, location }))
                })
            {
                return true;
            }
        }
        false
    }
    /// MultiPlayerGameMode.useItemOn for `hand`: whether the click ends there
    /// (the block reacts, or a block item is placed).
    pub(super) fn remote_use_item_on(&mut self, hand: Hand) -> bool {
        let held = self.with_hand(hand, |g| g.held().cloned());
        let held_block = held
            .as_ref()
            .and_then(|h| self.items().get(&h.item))
            .is_some_and(|i| i.block.is_some());
        let debug_stick = held
            .as_ref()
            .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
        if let Some(hit) = self.pick() {
            use mod_api::InteractionResult as Answer;
            use mod_api::{LogicalSide, TriState};
            use sunrise_world::level::UseBranch;
            // MultiPlayerGameMode.useItemOn syncs the original slot before prediction.
            // A Part can select a different slot, but not for this request.
            self.remote_send_carried();
            // startPrediction reserves one sequence for the request, even if
            // the client does not apply a prediction.
            let seq = self.next_sequence();
            let spectator = self.spectator();
            // RightClickBlock on this client, where performUseItemOn starts,
            // for a spectator too (whose use only opens menus on the server):
            // it may force the block's use or the item's useOn, or answer for
            // both.
            let (mut canceled, mut use_block, mut use_item) =
                (None, TriState::Default, TriState::Default);
            match self.right_click_block(&hit, hand, held.as_ref()) {
                crate::game::native_hooks::BlockClickAnswer::Go {
                    use_block: block,
                    use_item: item,
                } => (use_block, use_item) = (block, item),
                crate::game::native_hooks::BlockClickAnswer::Canceled(answer) => {
                    canceled = Some(answer);
                }
            }
            // Before-block event, then the held item's first refusal. A
            // canceled PASS skips that Part but still goes on to the block.
            if canceled.is_none()
                && !spectator
                && use_item.or(true)
                && let Some(stack) = held.as_ref()
            {
                let Ok(answer) = self.item_use_first(&hit, hand, stack) else {
                    self.send_use_item_on(&hit, hand, seq);
                    return true;
                };
                canceled = answer.filter(|answer| *answer != Answer::Pass);
            }
            // Player.isSecondaryUseActive with something in either hand
            // keeps the block from reacting by default; an explicit block
            // allow overrides that skip, and False always skips the block.
            let block_used =
                canceled.is_none() && use_block.or(!self.remote_secondary_use_skips_block());
            let item_forced = use_item == TriState::True;
            let item_used = canceled.is_none()
                && use_item.or(true)
                && (self.may_build()
                    || self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos)))
                && held
                    .as_ref()
                    .is_none_or(|stack| self.items().get(&stack.item).is_some())
                && (item_forced
                    || held.as_ref().is_some_and(|stack| {
                        self.items().get(&stack.item).is_some_and(|info| {
                            !self
                                .cooldowns
                                .is_on_cooldown(&sunrise_client::cooldowns::group(
                                    stack,
                                    info.use_cooldown.as_ref(),
                                ))
                        })
                    }));
            let [x, y, z] = hit.pos;
            let state = self.world.block(x, y, z);
            let block_answer = if !spectator && block_used {
                if let Some(answer) =
                    self.use_item_on_block(&hit, hand, held.as_ref(), mod_api::UsePhase::Block)
                {
                    Some(answer)
                } else {
                    let Ok(answer) = self.predict_block_part(
                        &hit,
                        state,
                        hand,
                        held.as_ref(),
                        UseBranch::ItemOn,
                    ) else {
                        self.send_use_item_on(&hit, hand, seq);
                        return true;
                    };
                    answer
                }
            } else {
                None
            };
            let vanilla_block = block_used && block_answer.is_none();
            // ShelfBlock.useItemOn consumes a nonempty main-hand click client-side
            // without predicting the inventory swap or swinging.
            let shelf_reacts = hand == Hand::Main
                && held.is_some()
                && vanilla_block
                && self.shelf_hit(&hit).is_some();
            let (result, changes) = if let Some(answer) = block_answer {
                let result =
                    crate::game::native_hooks::block_use_result(answer, LogicalSide::Client);
                (
                    result.or((hand == Hand::Off).then_some(InteractionResult::Pass)),
                    DryUse::default(),
                )
            } else if shelf_reacts {
                (Some(InteractionResult::Consume), DryUse::default())
            } else if hand == Hand::Main && !spectator && vanilla_block {
                self.remote_block_shortcut(&hit, held.as_ref())
                    .map(|(result, changes)| (Some(result), changes))
                    .unwrap_or_else(|| {
                        // With no fallback Part, keep both vanilla methods
                        // on the same dry level as an ordinary click.
                        let split = crate::game::api_values::block_state::block_listens(
                            mod_api::HookId::UseWithoutItem,
                            state,
                        );
                        self.remote_block_template(
                            &hit,
                            state,
                            held.as_ref(),
                            split.then_some(UseBranch::ItemOn),
                            true,
                        )
                    })
            } else {
                // useItemOn runs for the off hand too (useWithoutItem doesn't):
                // RespawnAnchorBlock.useItemOn charges an anchor with its
                // glowstone, SUCCESS; the blocks of `client_block_answer` answer too.
                let charges = vanilla_block && self.anchor_takes(hit.pos, held.as_ref());
                let result = if charges {
                    InteractionResult::Success
                } else if let Some(answer) = (vanilla_block && !spectator)
                    .then(|| self.client_block_answer(&hit, held.as_ref()))
                    .flatten()
                {
                    answer
                } else {
                    InteractionResult::Pass
                };
                (Some(result), DryUse::default())
            };
            // Campfire dousing is an item-on-block behaviour, not a use
            // without the item. If it takes the click there is no fallback.
            let result = if !spectator
                && vanilla_block
                && !result.is_some_and(InteractionResult::consumes_action)
                && self.with_hand(hand, |g| g.predict_campfire_douse(&hit, held.as_ref(), seq))
            {
                Some(InteractionResult::Success)
            } else {
                result
            };
            // Only the main-hand block branch can select the empty method.
            // That method still sees the held stack, and has no broadcast phase.
            let mut empty_answer = None;
            let (result, changes) = if let Some(result) = result {
                (result, changes)
            } else {
                let Ok(answer) = self.predict_block_part(
                    &hit,
                    state,
                    hand,
                    held.as_ref(),
                    UseBranch::WithoutItem,
                ) else {
                    self.send_use_item_on(&hit, hand, seq);
                    return true;
                };
                empty_answer = answer;
                if let Some(answer) = answer {
                    (
                        crate::game::native_hooks::block_use_result(answer, LogicalSide::Client)
                            .unwrap_or(InteractionResult::Pass),
                        DryUse::default(),
                    )
                } else {
                    let (result, changes) = self.remote_block_template(
                        &hit,
                        state,
                        held.as_ref(),
                        Some(UseBranch::WithoutItem),
                        true,
                    );
                    (result.unwrap_or(InteractionResult::Pass), changes)
                }
            };
            // AbstractCauldronBlock.useItemOn, either hand: CauldronInteractions
            // answers on the client as on the server and only the server acts,
            // so a bucket the cauldron takes is not used again (UseItem).
            // The main-hand path already asked above; only the off-hand fallback asks here.
            let result =
                if result.consumes_action() || spectator || !vanilla_block || hand == Hand::Main {
                    result
                } else {
                    match self.with_hand(hand, |game| game.cauldron_outcome(hit.pos, held.as_ref()))
                    {
                        sunrise_client::cauldron::Outcome::Success => InteractionResult::Success,
                        sunrise_client::cauldron::Outcome::Consume => InteractionResult::Consume,
                        sunrise_client::cauldron::Outcome::Pass => result,
                    }
                };
            // useItemOn on this client, unless a sneak with something in hand
            // skips it: RedStoneOreBlock's dust, and the wick smoke of a lit
            // candle an empty hand snuffs (CandleBlock.useItemOn) or a lit
            // candle cake (CandleCakeBlock.useItemOn).
            if hand == Hand::Main && !spectator && vanilla_block {
                self.redstone_ore_particles(hit.pos);
                let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                let name = self.block_name(hit.pos);
                let lit =
                    self.pack.assets.registry.state(state).is_some_and(|s| {
                        s.properties.get("lit").map(String::as_str) == Some("true")
                    });
                let snuffs = if name.ends_with("candle_cake") {
                    hit.point[1] - f64::from(hit.pos[1]) > 0.5
                } else {
                    name.ends_with("candle") && self.may_build()
                };
                if held.is_none() && lit && snuffs {
                    self.candle_smoke(hit.pos, state);
                }
            }
            let item_answer = if !spectator && !result.consumes_action() && item_used {
                self.use_item_on_block(&hit, hand, held.as_ref(), mod_api::UsePhase::ItemAfterBlock)
                    .or_else(|| {
                        held.as_ref()
                            .and_then(|stack| self.item_on_interaction(&hit, hand, stack))
                    })
            } else {
                None
            };
            let item_used =
                item_used && !spectator && !result.consumes_action() && item_answer.is_none();
            // DebugStickItem.useOn once the block passed: SUCCESS on the client
            // too (the hand swings below); only the server changes the block.
            let (result, changes) = if debug_stick && item_used {
                (InteractionResult::Success, DryUse::default())
            } else {
                (result, changes)
            };
            let reacts = result.consumes_action();
            // ArmorStandItem.useOn runs in startPrediction before the packet
            // goes out (the server may spawn the stand at once).
            let stand_fits = item_used
                && held
                    .as_ref()
                    .is_some_and(|s| &*s.item == "minecraft:armor_stand")
                && held
                    .as_ref()
                    .is_some_and(|s| self.armor_stand_spawn(&hit, s).is_some());
            self.send_use_item_on(&hit, hand, seq);
            // MultiPlayerGameMode.performUseItemOn: a spectator's use predicts
            // nothing and ends the click (CONSUME, no swing); the server opens
            // the block's menu. A cancelled RightClickBlock answers instead,
            // as for any player (below).
            if spectator && canceled.is_none() {
                return true;
            }
            // A cancelled RightClickBlock's answer, once the packet went out
            // (startPrediction sends it whatever the use): SUCCESS swings
            // here, and PASS goes on to useItem and the other hand.
            if let Some(answer) = canceled {
                if answer == Answer::Success {
                    self.with_hand(hand, Game::swing_arm);
                }
                return answer.consumes_action() || answer == Answer::Fail;
            }
            // Only a predicted placement (predict_place) marks a cell whose
            // resend plays a place sound: an empty hand or a non-block item
            // places nothing (BlockItem.place is where the sound comes from).
            if let Some(r) = self.remote.as_mut() {
                r.use_target = Some(hit.pos);
            }
            if let Some(answer) = item_answer {
                if answer == Answer::Success {
                    self.with_hand(hand, Game::swing_arm);
                }
                return answer.consumes_action() || answer == Answer::Fail;
            }
            if shelf_reacts {
                return true;
            }
            if reacts {
                self.predict_use(&changes, seq);
                // LeverBlock.useWithoutItem on the client: the predicted pull puffs.
                if empty_answer.is_none()
                    && (vanilla_block || block_answer == Some(Answer::TryWithEmptyHand))
                {
                    if &*self.block_name(hit.pos) == "minecraft:lever" {
                        self.lever_puff(hit.pos);
                    }
                    self.open_command_block_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
                    self.open_block_editor_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
                }
                // MultiPlayerGameMode.useItemOn: SUCCESS SwingSource.CLIENT
                // (a chest, barrel, furnace, …). CONSUME does not swing.
                if result.should_swing() {
                    self.with_hand(hand, Game::swing_arm);
                }
            } else if item_used
                && held
                    .as_ref()
                    .is_some_and(|s| &*s.item == "minecraft:end_crystal")
            {
                // EndCrystalItem.useOn on a client level: SUCCESS where the
                // crystal fits (the server spawns it), this hand swings; FAIL
                // elsewhere, no swing. Either ends the click
                // (Minecraft.startUseItem), so no UseItem and no other hand.
                if self.end_crystal_fits(hit.pos) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            } else if item_used && held.as_ref().is_some_and(|s| &*s.item == "minecraft:brush") {
                // BrushItem.useOn: brushing starts here (CONSUME, no swing); the
                // server brushes and the dust is this client's own.
                self.with_hand(hand, Game::start_using_item);
                return true;
            } else if item_used
                && let Some(stack) = held.as_ref().filter(|s| &*s.item == "minecraft:bone_meal")
                && self.with_hand(hand, |g| g.remote_bone_meal_succeeds(&hit, stack))
            {
                // BoneMealItem.useOn succeeded on this client's level (SUCCESS;
                // the server grows the plant): this hand swings and the click
                // ends, so the other hand (a torch) is never tried.
                self.with_hand(hand, Game::swing_arm);
                return true;
            } else if stand_fits {
                // ArmorStandItem.useOn on a client level: SUCCESS once the
                // stand fits, before the server spawns it. This hand swings
                // and the click ends.
                self.with_hand(hand, Game::swing_arm);
                return true;
            } else if let Some(stack) = held
                .as_ref()
                .filter(|_| item_used)
                .filter(|s| s.item.ends_with("_spawn_egg"))
            {
                // SpawnEggItem.useOn on a client level: FAIL for a mob that
                // cannot spawn (the click ends, no swing), else SUCCESS (swing).
                // Either way no UseItem follows, so the server does not spawn
                // a second mob from the fluid ray. ItemStack.useOn's adventure
                // predicate was checked before selecting this phase.
                if self.spawn_egg_allowed(&stack.item) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            } else if let Some(stack) = held.as_ref().filter(|_| item_used).filter(|s| {
                matches!(
                    &*s.item,
                    "minecraft:flint_and_steel" | "minecraft:fire_charge"
                )
            }) {
                if self.predict_fire_starter(stack, &hit, hand, seq) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            } else if let Some(stack) = held.as_ref().filter(|_| item_used)
                && &*stack.item == "minecraft:potion"
                && stack.potion() == Some("minecraft:water")
                && hit.face != [0, -1, 0]
                && self
                    .named_block_in_tag("minecraft:convertible_to_mud", &self.block_name(hit.pos))
                && (self.may_build()
                    || self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos)))
            {
                // PotionItem.useOn on a client level: SUCCESS. The bottle is
                // poured out into the hand and the block turns to mud here at
                // once; the sounds (`playSound(null, ...)`) and the splash
                // particles (`sendParticles`) are the server's.
                if let Some(mud) = self.pack.assets.registry.default_state("minecraft:mud") {
                    let pos =
                        sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
                    self.predict_blocks(&[(pos, mud)], seq);
                }
                self.with_hand(hand, |g| {
                    g.filled_result(ItemStack::new("minecraft:glass_bottle", 1))
                });
                self.with_hand(hand, Game::swing_arm);
                return true;
            } else if let Some(stack) = held.as_ref().filter(|_| item_used)
                && (self.with_hand(hand, |g| g.predict_item_on_block(stack, &hit))
                    || self.predict_map_on_banner(stack, &hit))
            {
                // Item.useOn succeeded here (InteractionResult.SUCCESS):
                // this hand swings and the click ends, so the other hand
                // (a shield, a torch) is never tried.
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            if item_used && held_block {
                let consumable = held
                    .as_ref()
                    .and_then(|stack| self.items().get(&stack.item).cloned())
                    .filter(|info| info.consumable.is_some());
                let placed = if reacts {
                    None
                } else {
                    held.as_ref()
                        .and_then(|stack| self.remote_can_place(&hit, stack))
                };
                // BlockItem.useOn: a consumable that could not be placed is used
                // (Item.use), which the server does for this same packet. CONSUME
                // does not swing, and a full player's FAIL still ends the click.
                // Adventure's can_place_on leaves the placing to the server.
                if !reacts
                    && placed.is_none()
                    && self.may_build()
                    && let Some(info) = &consumable
                {
                    self.with_hand(hand, |g| g.start_eating(info));
                }
                if let Some((placed, stack)) = placed.as_ref().zip(held.as_ref()) {
                    self.predict_place(placed, stack, seq);
                    self.with_hand(hand, |g| g.consume_placed(stack));
                    self.with_hand(hand, Game::swing_arm);
                }
                // BlockItem.place returns FAIL when placement is rejected:
                // stop this click without trying the item or the other hand.
                return true;
            }
            return reacts;
        }
        false
    }

    /// Send the ordinary block-use request, even for a discarded prediction.
    /// The carried slot was already synced before prediction; don't sync it again here.
    fn send_use_item_on(&self, hit: &crate::game::Hit, hand: Hand, sequence: i32) {
        let cursor = [0, 1, 2].map(|i| (hit.point[i] - f64::from(hit.pos[i])) as f32);
        self.send_play(S::UseItemOn(play::UseItemOn {
            hand: hand_id(hand),
            pos: hit.pos,
            face: i32::from(face_direction(hit.face)),
            cursor,
            inside: hit.inside,
            world_border: false,
            sequence,
        }));
    }
}
