# NeoForge counterparts

Each hook's `neoforge = ...` key in `crates/sunrise-mod-vocab/src/hooks/` names the NeoForge 26.3 event or method it
matches, since a counterpart's name and contract are the rule for a hook. This page lists the keys as declared.
The book build renders it directly from those declarations; there is no generated Markdown snapshot to update.
A key names the event class (`PlayerEvent.Clone`) or runtime callback (`IPayloadHandler.handle`);
notes about cancelling or the event bus belong in the hook's `///` doc, not in the key.

765 hooks: 455 name a counterpart, 301 say they have none.
This is a declaration reference, not an independent verification of every mapping. Registration events and the
callbacks they register are different moments: packet receipt names `IPayloadHandler.handle`, and per-draw item
decoration names `IItemDecorator.render`, not their registration events. A `none` explains why no event or method
matches that hook's runtime contract.

## Hooks with a counterpart

| Hook | Side | NeoForge |
| --- | --- | --- |
| `add_attribute_tooltips` | client | `AddAttributeTooltipsEvent` |
| `add_client_reload_listeners` | client | `AddClientReloadListenersEvent` |
| `add_destroy_effects` | client | `IClientBlockExtensions.addDestroyEffects` |
| `add_hit_effects` | client | `IClientBlockExtensions.addHitEffects` |
| `add_pack_finders` | both | `AddPackFindersEvent` |
| `add_section_geometry` | client | `AddSectionGeometryEvent` |
| `add_server_reload_listeners` | server | `AddServerReloadListenersEvent` |
| `adjacent_block_path_type` | server | `IBlockExtension.getAdjacentBlockPathType` |
| `alter_ground` | server | `AlterGroundEvent` |
| `animal_tame` | server | `AnimalTameEvent` |
| `anvil_craft_post` | server | `AnvilCraftEvent.Post` |
| `anvil_craft_pre` | server | `AnvilCraftEvent.Pre` |
| `anvil_update` | both | `AnvilUpdateEvent` |
| `apply_enchantments` | server | `IItemExtension.applyEnchantments` |
| `arm_pose_transform` | client | `HumanoidModel.ArmPose` |
| `armor_hurt` | server | `ArmorHurtEvent` |
| `arrow_loose` | server | `ArrowLooseEvent` |
| `arrow_nock` | server | `ArrowNockEvent` |
| `attack_entity` | server | `AttackEntityEvent` |
| `baby_entity_spawn` | server | `BabyEntitySpawnEvent` |
| `beacon_color_multiplier` | both | `IBlockExtension.getBeaconColorMultiplier, IBlockStateExtension.getBeaconColorMultiplier` |
| `block_drops` | server | `BlockDropsEvent` |
| `block_entity_on_load` | both | `IBlockEntityExtension.onLoad` |
| `block_entity_render_bounding_box` | client | `IBlockEntityRendererExtension.getRenderBoundingBox` |
| `block_entity_submit` | client | `EntityRenderersEvent.RegisterRenderers` |
| `block_entity_type_add_blocks` | both | `BlockEntityTypeAddBlocksEvent` |
| `block_grow_feature` | server | `BlockGrowFeatureEvent` |
| `block_highlight` | client | `ExtractBlockOutlineRenderStateEvent` |
| `block_path_type` | server | `IBlockExtension.getBlockPathType` |
| `block_tool_modification` | both | `BlockEvent.BlockToolModificationEvent` |
| `blocks` | both | `RegisterEvent` |
| `bonemeal` | both | `BonemealEvent` |
| `bounce_restitution` | both | `IBlockExtension.getBounceRestitution` |
| `break_block` | server | `BreakBlockEvent` |
| `build_creative_mode_tab_contents` | both | `BuildCreativeModeTabContentsEvent` |
| `calculate_player_turn` | client | `CalculatePlayerTurnEvent` |
| `camera_distance` | client | `CalculateDetachedCameraDistanceEvent` |
| `can_be_connected_to` | server | `IBlockExtension.canBeConnectedTo` |
| `can_be_hydrated` | server | `IBlockExtension.canBeHydrated` |
| `can_continue_sleeping` | server | `CanContinueSleepingEvent` |
| `can_continue_using` | both | `IItemExtension.canContinueUsing` |
| `can_drop_from_explosion` | server | `IBlockExtension.canDropFromExplosion` |
| `can_entity_destroy` | server | `IBlockExtension.canEntityDestroy` |
| `can_equip` | both | `IItemExtension.canEquip` |
| `can_fit_inside_container_items` | both | `IItemExtension.canFitInsideContainerItems` |
| `can_grindstone_repair` | both | `IItemExtension.canGrindstoneRepair` |
| `can_harvest_block` | both | `IBlockExtension.canHarvestBlock` |
| `can_perform_action` | both | `IItemExtension.canPerformAction` |
| `can_player_sleep` | server | `CanPlayerSleepEvent` |
| `can_rider_interact` | client | `IEntityExtension.canRiderInteract` |
| `can_start_swimming` | both | `IEntityExtension.canStartSwimming` |
| `can_stick_to` | server | `IBlockExtension.canStickTo` |
| `can_sustain_plant` | both | `IBlockExtension.canSustainPlant` |
| `chunk_data_load` | server | `ChunkDataEvent.Load` |
| `chunk_data_save` | server | `ChunkDataEvent.Save` |
| `chunk_load` | server | `ChunkEvent.Load` |
| `chunk_sent` | server | `ChunkWatchEvent.Sent` |
| `chunk_ticket_level_updated` | server | `ChunkTicketLevelUpdatedEvent` |
| `chunk_unload` | server | `ChunkEvent.Unload` |
| `chunk_unwatch` | server | `ChunkWatchEvent.UnWatch` |
| `chunk_watch` | server | `ChunkWatchEvent.Watch` |
| `client_attack_target` | client | `AttackEntityEvent` |
| `client_chat` | client | `ClientChatEvent` |
| `client_chat_received` | client | `ClientChatReceivedEvent, ClientChatReceivedEvent.Player, ClientChatReceivedEvent.System` |
| `client_chunk_load` | client | `ChunkEvent.Load` |
| `client_chunk_unload` | client | `ChunkEvent.Unload` |
| `client_information_updated` | server | `ClientInformationUpdatedEvent` |
| `client_level_load` | client | `LevelEvent.Load` |
| `client_level_unload` | client | `LevelEvent.Unload` |
| `client_pause_change_post` | client | `ClientPauseChangeEvent.Post` |
| `client_pause_change_pre` | client | `ClientPauseChangeEvent.Pre` |
| `client_player_change_game_type` | client | `ClientPlayerChangeGameTypeEvent` |
| `client_player_clone` | client | `ClientPlayerNetworkEvent.Clone` |
| `client_player_logging_in` | client | `ClientPlayerNetworkEvent.LoggingIn` |
| `client_player_logging_out` | client | `ClientPlayerNetworkEvent.LoggingOut` |
| `client_resources_reloaded` | client | `ClientResourceLoadFinishedEvent` |
| `client_setup` | client | `FMLClientSetupEvent` |
| `client_stopped` | client | `ClientStoppedEvent` |
| `client_stopping` | client | `ClientStoppingEvent` |
| `client_tick_post` | client | `ClientTickEvent.Post` |
| `client_tick_pre` | client | `ClientTickEvent.Pre` |
| `clone_item_stack` | server | `IBlockExtension.getCloneItemStack` |
| `component_types` | both | `RegisterEvent` |
| `compute_camera_angles` | client | `ViewportEvent.ComputeCameraAngles` |
| `compute_fog_color` | client | `ViewportEvent.ComputeFogColor` |
| `compute_fov` | client | `ViewportEvent.ComputeFov` |
| `compute_fov_modifier` | client | `ComputeFovModifierEvent` |
| `crafting_remainder` | server | `IItemExtension.getCraftingRemainder(ItemStack)` |
| `create_fluid_source` | server | `CreateFluidSourceEvent` |
| `create_skull_models` | client | `EntityRenderersEvent.CreateSkullModels` |
| `create_spawn_position` | server | `LevelEvent.CreateSpawnPosition` |
| `creator_mod_id` | both | `IItemExtension.getCreatorModId` |
| `critical_hit` | both | `CriticalHitEvent` |
| `crop_grow_post` | server | `CropGrowEvent.Post` |
| `crop_grow_pre` | server | `CropGrowEvent.Pre` |
| `custom_click_action` | server | `CustomClickActionEvent` |
| `customize_boss_event_progress` | client | `CustomizeGuiOverlayEvent.BossEventProgress` |
| `customize_chat_overlay` | client | `CustomizeGuiOverlayEvent.Chat` |
| `damage_item` | server | `IItemExtension.damageItem` |
| `data_map_types` | server | `RegisterDataMapTypesEvent` |
| `data_maps_updated` | both | `DataMapsUpdatedEvent` |
| `data_pack_files` | server | `AddPackFindersEvent` |
| `datapack_sync` | server | `OnDatapackSyncEvent` |
| `default_components_bound` | both | `DefaultDataComponentsBoundEvent` |
| `difficulty_changed` | server | `DifficultyChangeEvent` |
| `effect_particle_modification` | server | `EffectParticleModificationEvent` |
| `empty_contents` | server | `IDispensibleContainerItemExtension.emptyContents` |
| `enchant_power_bonus` | both | `IBlockExtension.getEnchantPowerBonus, IBlockStateExtension.getEnchantPowerBonus` |
| `enchanted_block_loot` | server | `EnchantedBlockLootEvent` |
| `enchanted_entity_loot` | server | `EnchantedEntityLootEvent` |
| `enchantment_level_set` | server | `EnchantmentLevelSetEvent` |
| `ender_entity_teleport` | server | `EntityTeleportEvent.EnderEntity` |
| `ender_pearl_teleport` | server | `EntityTeleportEvent.EnderPearl` |
| `enderman_anger` | server | `EndermanAngerEvent` |
| `entity_constructing` | both | `EntityEvent.EntityConstructing` |
| `entity_entering_section` | both | `EntityEvent.EnteringSection` |
| `entity_interact` | both | `PlayerInteractEvent.EntityInteract` |
| `entity_interact_specific` | server | `PlayerInteractEvent.EntityInteractSpecific` |
| `entity_invulnerability_check` | both | `EntityInvulnerabilityCheckEvent` |
| `entity_join_level` | server | `EntityJoinLevelEvent` |
| `entity_leave_level` | server | `EntityLeaveLevelEvent` |
| `entity_mob_griefing` | server | `EntityMobGriefingEvent` |
| `entity_mount` | server | `EntityMountEvent` |
| `entity_place` | server | `BlockEvent.EntityPlaceEvent, BlockEvent.EntityMultiPlaceEvent` |
| `entity_size` | both | `EntityEvent.Size` |
| `entity_struck_by_lightning` | server | `EntityStruckByLightningEvent` |
| `entity_tick_post` | both | `EntityTickEvent.Post` |
| `entity_tick_pre` | both | `EntityTickEvent.Pre` |
| `entity_travel_to_dimension` | server | `EntityTravelToDimensionEvent` |
| `exp_drop` | server | `IBlockExtension.getExpDrop` |
| `explosion_detonate` | server | `ExplosionEvent.Detonate` |
| `explosion_knockback` | server | `ExplosionKnockbackEvent` |
| `explosion_resistance` | server | `IBlockExtension.getExplosionResistance` |
| `explosion_start` | server | `ExplosionEvent.Start` |
| `extend_poi_types` | server | `ExtendPoiTypesEvent` |
| `extract_block_screen_effect` | client | `ExtractBlockScreenEffectEvent` |
| `fall_on` | server | `LivingFallEvent` |
| `falling_tick` | server | `IFallableExtension.fallingTick` |
| `farmland_trample` | server | `BlockEvent.FarmlandTrampleEvent` |
| `finalize_spawn` | server | `FinalizeSpawnEvent` |
| `fire_odds` | both | `IBlockExtension.getFlammability, IBlockExtension.getFireSpreadSpeed, IBlockExtension.isFlammable` |
| `flip_frame` | client | `FlipFrameEvent` |
| `fluid_place_block` | server | `BlockEvent.FluidPlaceBlockEvent` |
| `fluid_tooltip` | client | `FluidTooltipEvent` |
| `frame_graph_setup` | client | `FrameGraphSetupEvent` |
| `friction` | both | `IBlockExtension.getFriction` |
| `game_rule_changed` | server | `GameRuleChangedEvent` |
| `game_shutting_down` | both | `GameShuttingDownEvent` |
| `gather_effect_screen_tooltips` | client | `GatherEffectScreenTooltipsEvent` |
| `gather_skipped_attribute_tooltips` | client | `GatherSkippedAttributeTooltipsEvent` |
| `get_arm_pose` | client | `IClientItemExtensions.getArmPose` |
| `get_enchantment_level` | both | `GetEnchantmentLevelEvent` |
| `get_equipment_drop_chance` | server | `GetEquipmentDropChanceEvent` |
| `grindstone_place` | server | `GrindstoneEvent.OnPlaceItem` |
| `grindstone_take` | server | `GrindstoneEvent.OnTakeItem` |
| `gui_layer_post` | client | `RenderGuiLayerEvent.Post` |
| `gui_layer_pre` | client | `RenderGuiLayerEvent.Pre` |
| `hand_drawn` | client | `RenderHandEvent` |
| `highlight_tip` | client | `IItemExtension.getHighlightTip` |
| `ignited_by_lava` | server | `IBlockExtension.ignitedByLava` |
| `ingredient` | both | `ICustomIngredient` |
| `input_key` | client | `InputEvent.Key` |
| `interaction_key_mapping_triggered` | client | `InputEvent.InteractionKeyMappingTriggered` |
| `is_combine_repairable` | both | `IItemExtension.isCombineRepairable` |
| `is_fertile` | server | `IBlockExtension.isFertile` |
| `is_fire_source` | server | `IBlockExtension.isFireSource` |
| `is_ladder` | both | `IBlockExtension.isLadder` |
| `is_not_replaceable_by_pick_action` | server | `IItemExtension.isNotReplaceableByPickAction` |
| `is_piglin_currency` | both | `IItemExtension.isPiglinCurrency` |
| `is_primary_item_for` | server | `IItemExtension.isPrimaryItemFor` |
| `is_shearable` | both | `IShearable.isShearable` |
| `is_slime_block` | server | `IBlockExtension.isSlimeBlock` |
| `is_sticky_block` | server | `IBlockExtension.isStickyBlock` |
| `item_attribute_modifier` | both | `ItemAttributeModifierEvent` |
| `item_consumption_teleport` | server | `EntityTeleportEvent.ItemConsumption` |
| `item_crafted` | server | `PlayerEvent.ItemCraftedEvent` |
| `item_decorations` | client | `IItemDecorator.render` |
| `item_destroyed` | server | `IItemExtension.onDestroyed` |
| `item_expire` | server | `ItemExpireEvent` |
| `item_fished` | server | `ItemFishedEvent` |
| `item_model_property` | client | `RegisterConditionalItemModelPropertyEvent` |
| `item_name` | both | `Item.getName` |
| `item_picked_up` | server | `ItemEntityPickupEvent.Pre` |
| `item_picked_up_post` | server | `ItemEntityPickupEvent.Post` |
| `item_smelted` | both | `PlayerEvent.ItemSmeltedEvent` |
| `item_stacked_on_other` | both | `ItemStackedOnOtherEvent` |
| `item_tooltip` | client | `ItemTooltipEvent` |
| `item_toss` | server | `ItemTossEvent` |
| `items` | both | `RegisterEvent` |
| `key_mapping_pre` | client | `InputEvent.InteractionKeyMappingTriggered, InputEvent.Key` |
| `left_click_block` | server | `PlayerInteractEvent.LeftClickBlock` |
| `left_click_empty` | client | `PlayerInteractEvent.LeftClickEmpty` |
| `level_load` | server | `LevelEvent.Load` |
| `level_save` | server | `LevelEvent.Save` |
| `level_tick_post` | both | `LevelTickEvent.Post` |
| `level_tick_pre` | both | `LevelTickEvent.Pre` |
| `level_unload` | server | `LevelEvent.Unload` |
| `light_emission` | both | `IBlockExtension.getLightEmission, IBlockStateExtension.getLightEmission` |
| `living_breathe` | server | `LivingBreatheEvent` |
| `living_change_target` | server | `LivingChangeTargetEvent` |
| `living_conversion_post` | server | `LivingConversionEvent.Post` |
| `living_conversion_pre` | server | `LivingConversionEvent.Pre` |
| `living_damage_post` | server | `LivingDamageEvent.Post` |
| `living_damage_pre` | server | `LivingDamageEvent.Pre` |
| `living_death` | server | `LivingDeathEvent` |
| `living_destroy_block` | server | `LivingDestroyBlockEvent` |
| `living_drops` | server | `LivingDropsEvent` |
| `living_drown` | server | `LivingDrownEvent` |
| `living_entity_use_item_finish` | both | `LivingEntityUseItemEvent.Finish` |
| `living_entity_use_item_start` | both | `LivingEntityUseItemEvent.Start` |
| `living_entity_use_item_stop` | both | `LivingEntityUseItemEvent.Stop` |
| `living_entity_use_item_tick` | both | `LivingEntityUseItemEvent.Tick` |
| `living_equipment_change` | server | `LivingEquipmentChangeEvent` |
| `living_experience_drop` | server | `LivingExperienceDropEvent` |
| `living_fall` | both | `LivingFallEvent` |
| `living_get_projectile` | both | `LivingGetProjectileEvent` |
| `living_heal` | both | `LivingHealEvent` |
| `living_incoming_damage` | server | `LivingIncomingDamageEvent` |
| `living_jump` | both | `LivingEvent.LivingJumpEvent` |
| `living_knockback` | server | `LivingKnockBackEvent` |
| `living_shield_block` | server | `LivingShieldBlockEvent` |
| `living_use_totem` | server | `LivingUseTotemEvent` |
| `living_visibility` | server | `LivingEvent.LivingVisibilityEvent` |
| `loot_drops` | server | `LivingDropsEvent, BlockDropsEvent` |
| `loot_table_load` | server | `LootTableLoadEvent` |
| `makes_open_trapdoor_above_climbable` | both | `IBlockExtension.makesOpenTrapdoorAboveClimbable` |
| `makes_piglins_neutral` | server | `IItemExtension.makesPiglinsNeutral` |
| `map_decoration_types` | both | `RegisterEvent` |
| `map_decorations` | client | `MapDecorationType` |
| `may_place_on` | both | `IBlockExtension.canSustainPlant` |
| `menu_extra_data` | server | `IPlayerExtension.openMenu` |
| `mob_despawn` | server | `MobDespawnEvent` |
| `mob_effect_added` | server | `MobEffectEvent.Added` |
| `mob_effect_applicable` | server | `MobEffectEvent.Applicable` |
| `mob_effect_expired` | server | `MobEffectEvent.Expired` |
| `mob_effect_remove` | server | `MobEffectEvent.Remove` |
| `mob_spawn_placement_check` | server | `MobSpawnEvent.SpawnPlacementCheck` |
| `mob_spawn_position_check` | server | `MobSpawnEvent.PositionCheck` |
| `mob_split` | server | `MobSplitEvent` |
| `mod_mismatch` | both | `ModMismatchEvent` |
| `model_baking_completed` | client | `ModelEvent.BakingCompleted` |
| `modify_custom_spawners` | server | `ModifyCustomSpawnersEvent` |
| `modify_default_components` | both | `ModifyDefaultComponentsEvent` |
| `mouse_button_post` | client | `InputEvent.MouseButton.Post` |
| `mouse_button_pre` | client | `InputEvent.MouseButton.Pre` |
| `mouse_scrolling` | client | `InputEvent.MouseScrollingEvent` |
| `movement_input_update` | client | `MovementInputUpdateEvent` |
| `music` | client | `SelectMusicEvent` |
| `neighbor_notify` | server | `BlockEvent.NeighborNotifyEvent` |
| `note_block_change` | server | `NoteBlockEvent.Change` |
| `note_block_play` | server | `NoteBlockEvent.Play` |
| `on_block_exploded` | server | `IBlockExtension.onBlockExploded` |
| `on_block_placed_by` | server | `BlockItemExtension.onBlockPlacedBy` |
| `on_block_state_change` | both | `IBlockExtension.onBlockStateChange` |
| `on_caught_fire` | server | `IBlockExtension.onCaughtFire` |
| `on_chunk_unloaded` | both | `IBlockEntityExtension.onChunkUnloaded` |
| `on_destroyed_by_player` | server | `IBlockExtension.onDestroyedByPlayer` |
| `on_destroyed_by_push_reaction` | server | `IBlockExtension.onDestroyedByPushReaction` |
| `on_dropped_by_player` | server | `IItemExtension.onDroppedByPlayer` |
| `on_entity_item_update` | server | `IItemExtension.onEntityItemUpdate` |
| `on_entity_swing` | both | `IItemExtension.onEntitySwing` |
| `on_item_use_first` | both | `IItemExtension.onItemUseFirst` |
| `on_left_click_entity` | server | `IItemExtension.onLeftClickEntity` |
| `on_neighbor_change` | server | `IBlockExtension.onNeighborChange` |
| `on_sheared` | both | `IShearable.onSheared` |
| `on_stop_using` | both | `IItemExtension.onStopUsing` |
| `on_tree_grow` | server | `IBlockExtension.onTreeGrow` |
| `on_use_tick` | both | `LivingEntityUseItemEvent.Tick` |
| `permissions_changed` | server | `PermissionsChangedEvent` |
| `pickup_xp` | server | `PlayerXpEvent.PickupXp` |
| `piston_post` | server | `PistonEvent.Post` |
| `piston_pre` | server | `PistonEvent.Pre` |
| `piston_push_reaction` | both | `IBlockExtension.getPistonPushReaction` |
| `play_break_sound` | client | `IClientBlockExtensions.playBreakSound` |
| `play_hit_sound` | client | `IClientBlockExtensions.playHitSound` |
| `play_level_sound_at_entity` | both | `PlayLevelSoundEvent.AtEntity` |
| `play_level_sound_at_position` | both | `PlayLevelSoundEvent.AtPosition` |
| `play_sound` | client | `PlaySoundEvent` |
| `play_sound_source` | client | `PlaySoundSourceEvent` |
| `play_streaming_source` | client | `PlayStreamingSourceEvent` |
| `player_advancement_done` | server | `AdvancementEvent.AdvancementEarnEvent` |
| `player_advancement_progress` | server | `AdvancementEvent.AdvancementProgressEvent` |
| `player_bed_leave` | server | `PlayerWakeUpEvent` |
| `player_break_speed` | both | `PlayerEvent.BreakSpeed` |
| `player_brewed_potion` | server | `PlayerBrewedPotionEvent` |
| `player_changed_dimension` | server | `PlayerEvent.PlayerChangedDimensionEvent` |
| `player_clone` | server | `PlayerEvent.Clone (not cancellable)` |
| `player_command` | server | `CommandEvent` |
| `player_container_close` | server | `PlayerContainerEvent.Close` |
| `player_container_open` | server | `PlayerContainerEvent.Open` |
| `player_destroy_item` | both | `PlayerDestroyItemEvent` |
| `player_enchant_item` | server | `PlayerEnchantItemEvent` |
| `player_flyable_fall` | server | `PlayerFlyableFallEvent` |
| `player_game_mode_changed` | server | `PlayerEvent.PlayerChangeGameModeEvent` |
| `player_harvest_check` | both | `PlayerEvent.HarvestCheck` |
| `player_heart_type` | client | `PlayerHeartTypeEvent` |
| `player_item_smithed` | server | `PlayerEvent.ItemSmithedEvent` |
| `player_load_from_file` | server | `PlayerEvent.LoadFromFile (not cancellable)` |
| `player_logged_in` | server | `PlayerEvent.PlayerLoggedInEvent` |
| `player_logged_out` | server | `PlayerEvent.PlayerLoggedOutEvent` |
| `player_name_format` | both | `PlayerEvent.NameFormat` |
| `player_respawn_position` | server | `PlayerRespawnPositionEvent` |
| `player_respawned` | server | `PlayerEvent.PlayerRespawnEvent` |
| `player_save_to_file` | server | `PlayerEvent.SaveToFile (not cancellable)` |
| `player_set_spawn` | server | `PlayerSetSpawnEvent` |
| `player_spawn_phantoms` | server | `PlayerSpawnPhantomsEvent` |
| `player_start_tracking` | server | `PlayerEvent.StartTracking` |
| `player_stat_award` | server | `StatAwardEvent` |
| `player_stop_tracking` | server | `PlayerEvent.StopTracking` |
| `player_swap_hands` | server | `LivingSwapItemsEvent.Hands` |
| `player_switch_hotbar_slot_post` | server | `PlayerSwitchHotbarSlotEvent.Post` |
| `player_switch_hotbar_slot_pre` | server | `PlayerSwitchHotbarSlotEvent.Pre` |
| `player_tab_list_name_format` | server | `PlayerEvent.TabListNameFormat` |
| `player_tick_post` | both | `PlayerTickEvent.Post` |
| `player_tick_pre` | both | `PlayerTickEvent.Pre` |
| `player_xp_change` | server | `PlayerXpEvent.XpChange` |
| `player_xp_level_change` | server | `PlayerXpEvent.LevelChange` |
| `portal_spawn` | server | `BlockEvent.PortalSpawnEvent` |
| `potential_spawns` | server | `LevelEvent.PotentialSpawns` |
| `potion_brew_post` | server | `PotionBrewEvent.Post` |
| `potion_brew_pre` | server | `PotionBrewEvent.Pre` |
| `prepare_render_buffers` | client | `PrepareRenderBuffersEvent` |
| `projectile_impact` | server | `ProjectileImpactEvent` |
| `rail_can_make_slopes` | server | `IBaseRailBlockExtension.canMakeSlopes` |
| `rail_direction` | both | `IBaseRailBlockExtension.getRailDirection` |
| `rail_flexible` | server | `IBaseRailBlockExtension.isFlexibleRail` |
| `rail_valid_shape` | server | `IBaseRailBlockExtension.isValidRailShape` |
| `recipe_property_sets` | both | `RegisterRecipePropertiesEvent` |
| `recipes_received` | client | `RecipesReceivedEvent` |
| `register_block_state_models` | client | `RegisterBlockStateModels` |
| `register_block_tint_sources` | client | `RegisterColorHandlersEvent.BlockTintSources` |
| `register_cauldron_fluid_content` | server | `RegisterCauldronFluidContentEvent` |
| `register_cauldron_interaction` | both | `RegisterCauldronInteractionEvent` |
| `register_client_commands` | client | `RegisterClientCommandsEvent` |
| `register_client_item_extensions` | client | `RegisterClientExtensionsEvent` |
| `register_client_tooltip_component_factories` | client | `RegisterClientTooltipComponentFactoriesEvent` |
| `register_color_resolvers` | client | `RegisterColorHandlersEvent.ColorResolvers` |
| `register_commands` | server | `RegisterCommandsEvent` |
| `register_configuration_tasks` | server | `RegisterConfigurationTasksEvent` |
| `register_debug_entries` | client | `RegisterDebugEntriesEvent` |
| `register_debug_renderers` | client | `RegisterDebugRenderersEvent` |
| `register_dimension_transition_screen` | client | `RegisterDimensionTransitionScreenEvent` |
| `register_enchantment_effects` | server | `RegisterEvent` |
| `register_entity_layers` | client | `EntityRenderersEvent.AddLayers` |
| `register_entity_renderers` | client | `EntityRenderersEvent.RegisterRenderers` |
| `register_entity_spectator_shaders` | client | `RegisterEntitySpectatorShadersEvent` |
| `register_fluid_models` | client | `RegisterFluidModelsEvent` |
| `register_game_tests` | server | `RegisterGameTestsEvent` |
| `register_gui_layers` | client | `RegisterGuiLayersEvent` |
| `register_item_models` | client | `RegisterItemModelsEvent` |
| `register_item_tint_sources` | client | `RegisterColorHandlersEvent.ItemTintSources` |
| `register_json_animation_types` | client | `RegisterJsonAnimationTypesEvent` |
| `register_layer_definitions` | client | `EntityRenderersEvent.RegisterLayerDefinitions` |
| `register_load_conditions` | server | `RegisterEvent` |
| `register_model_loaders` | client | `ModelEvent.RegisterLoaders` |
| `register_particle_groups` | client | `RegisterParticleGroupsEvent` |
| `register_particle_providers` | client | `RegisterParticleProvidersEvent` |
| `register_pipeline_modifiers` | client | `RegisterPipelineModifiersEvent` |
| `register_preset_editors` | client | `RegisterPresetEditorsEvent` |
| `register_range_select_item_model_property` | client | `RegisterRangeSelectItemModelPropertyEvent` |
| `register_render_pipelines` | client | `RegisterRenderPipelinesEvent` |
| `register_select_item_model_property` | client | `RegisterSelectItemModelPropertyEvent` |
| `register_special_model_renderer` | client | `RegisterSpecialModelRendererEvent` |
| `register_sprite_sources` | client | `RegisterSpriteSourcesEvent` |
| `register_texture_atlases` | client | `RegisterTextureAtlasesEvent` |
| `register_villager_professions` | server | `RegisterEvent` |
| `render_arm` | client | `RenderArmEvent` |
| `render_fog` | client | `ViewportEvent.RenderFog` |
| `render_frame_post` | client | `RenderFrameEvent.Post` |
| `render_frame_pre` | client | `RenderFrameEvent.Pre` |
| `render_gui_post` | client | `RenderGuiEvent.Post` |
| `render_gui_pre` | client | `RenderGuiEvent.Pre` |
| `render_hand` | client | `RenderHandEvent` |
| `render_item_in_frame` | client | `RenderItemInFrameEvent` |
| `render_level_stage` | client | `RenderLevelStageEvent` |
| `render_living_post` | client | `RenderLivingEvent.Post` |
| `render_living_pre` | client | `RenderLivingEvent.Pre` |
| `render_name_tag_can_render` | client | `RenderNameTagEvent.CanRender` |
| `render_name_tag_do_render` | client | `RenderNameTagEvent.DoRender` |
| `render_player_post` | client | `RenderPlayerEvent.Post` |
| `render_player_pre` | client | `RenderPlayerEvent.Pre` |
| `render_tooltip_gather_components` | client | `RenderTooltipEvent.GatherComponents` |
| `render_tooltip_pre` | client | `RenderTooltipEvent.Pre` |
| `render_tooltip_texture` | client | `RenderTooltipEvent.Texture` |
| `respawn_position` | server | `IBlockExtension.getRespawnPosition, IBlockStateExtension.getRespawnPosition` |
| `right_click_block` | both | `PlayerInteractEvent.RightClickBlock` |
| `right_click_empty` | client | `PlayerInteractEvent.RightClickEmpty` |
| `right_click_item` | both | `PlayerInteractEvent.RightClickItem` |
| `rotate` | server | `IBlockExtension.rotate` |
| `screen_char_typed_post` | client | `ScreenEvent.CharacterTyped.Post` |
| `screen_char_typed_pre` | client | `ScreenEvent.CharacterTyped.Pre` |
| `screen_closing` | client | `ScreenEvent.Closing` |
| `screen_init_post` | client | `ScreenEvent.Init.Post` |
| `screen_key_pressed_post` | client | `ScreenEvent.KeyPressed.Post` |
| `screen_key_pressed_pre` | client | `ScreenEvent.KeyPressed.Pre` |
| `screen_key_released_post` | client | `ScreenEvent.KeyReleased.Post` |
| `screen_key_released_pre` | client | `ScreenEvent.KeyReleased.Pre` |
| `screen_mouse_button_pressed_post` | client | `ScreenEvent.MouseButtonPressed.Post` |
| `screen_mouse_button_pressed_pre` | client | `ScreenEvent.MouseButtonPressed.Pre` |
| `screen_mouse_button_released_post` | client | `ScreenEvent.MouseButtonReleased.Post` |
| `screen_mouse_button_released_pre` | client | `ScreenEvent.MouseButtonReleased.Pre` |
| `screen_mouse_dragged_post` | client | `ScreenEvent.MouseDragged.Post` |
| `screen_mouse_dragged_pre` | client | `ScreenEvent.MouseDragged.Pre` |
| `screen_mouse_scrolled_post` | client | `ScreenEvent.MouseScrolled.Post` |
| `screen_mouse_scrolled_pre` | client | `ScreenEvent.MouseScrolled.Pre` |
| `screen_opening` | client | `ScreenEvent.Opening` |
| `screen_preedit_post` | client | `ScreenEvent.Preedit.Post` |
| `screen_preedit_pre` | client | `ScreenEvent.Preedit.Pre` |
| `screen_render_background` | client | `ScreenEvent.Render.Background` |
| `screen_render_foreground` | client | `ScreenEvent.Render.Foreground` |
| `screen_render_inventory_mob_effects` | client | `ScreenEvent.RenderInventoryMobEffects` |
| `screen_render_post` | client | `ScreenEvent.Render.Post` |
| `screen_render_pre` | client | `ScreenEvent.Render.Pre` |
| `screen_transparent_background` | client | `ScreenEvent.Render.Pre (transparent background)` |
| `screenshot` | client | `ScreenshotEvent` |
| `server_about_to_start` | server | `ServerAboutToStartEvent` |
| `server_chat` | server | `ServerChatEvent` |
| `server_started` | server | `ServerStartedEvent` |
| `server_starting` | server | `ServerStartingEvent` |
| `server_stopped` | server | `ServerStoppedEvent` |
| `server_stopping` | server | `ServerStoppingEvent` |
| `server_tick` | server | `ServerTickEvent.Pre, ServerTickEvent.Post` |
| `setting_changed` | both | `ModConfigEvent.Reloading` |
| `should_cause_block_break_reset` | client | `IItemExtension.shouldCauseBlockBreakReset` |
| `should_cause_reequip_animation` | client | `IItemExtension.shouldCauseReequipAnimation` |
| `should_check_weak_power` | server | `IBlockExtension.shouldCheckWeakPower` |
| `should_display_fluid_overlay` | client | `IBlockExtension.shouldDisplayFluidOverlay` |
| `should_hide_adjacent_fluid_face` | client | `IBlockExtension.shouldHideAdjacentFluidFace` |
| `sleep_finished_time` | server | `SleepFinishedTimeEvent` |
| `sound_engine_load` | client | `SoundEngineLoadEvent` |
| `sound_type` | both | `IBlockExtension.getSoundType` |
| `spawn_cluster_size` | server | `SpawnClusterSizeEvent` |
| `spread_players_teleport` | server | `EntityTeleportEvent.SpreadPlayersCommand` |
| `standalone_models` | client | `ModelEvent.RegisterStandalone` |
| `state_at_viewpoint` | client | `IBlockExtension.getStateAtViewpoint, IBlockStateExtension.getStateAtViewpoint` |
| `submit_custom_geometry` | client | `SubmitCustomGeometryEvent` |
| `supports_enchantment` | both | `IItemExtension.supportsEnchantment` |
| `sweep_attack` | both | `SweepAttackEvent` |
| `tags_updated` | both | `TagsUpdatedEvent` |
| `teleport_command` | server | `EntityTeleportEvent.TeleportCommand` |
| `texture_atlas_stitched` | client | `TextureAtlasStitchedEvent` |
| `toast_add` | client | `ToastAddEvent` |
| `tool_modified_state` | both | `IBlockExtension.getToolModifiedState` |
| `tooltip` | client | `ItemTooltipEvent` |
| `trade_with_villager` | server | `TradeWithVillagerEvent` |
| `use_item` | both | `PlayerInteractEvent.RightClickItem` |
| `use_item_on_block` | both | `UseItemOnBlockEvent` |
| `use_on` | both | `UseItemOnBlockEvent` |
| `vanilla_game_event` | server | `VanillaGameEvent` |
| `village_siege` | server | `VillageSiegeEvent` |
| `weak_changes` | server | `IBlockExtension.getWeakChanges` |
| `window_resize` | client | `WindowResizeEvent` |
| `world_drawn` | client | `RenderLevelStageEvent.AfterLevel` |
| `xp_orb_targeting` | server | `XpOrbTargetingEvent` |
| `xp_repair_ratio` | server | `IItemExtension.getXpRepairRatio` |

## Hooks that declare no counterpart

A hook with no NeoForge counterpart is judged on its merits like any other. The note is the key's own reason.

| Hook | Side | Reason |
| --- | --- | --- |
| `affect_neighbors_after_removal` | server | vanilla Block.affectNeighborsAfterRemoval; NeoForge adds no event or extension for it |
| `analog_output` | server | vanilla BlockBehaviour.getAnalogOutputSignal; NeoForge adds no event or extension for it |
| `animate_tick` | client | vanilla Block.animateTick from ClientLevel.animateTick; NeoForge adds no event |
| `anvil_damaged` | server | NeoForge has no such event |
| `apply_effect_tick` | server | vanilla MobEffect.applyEffectTick; NeoForge adds no event or extension for it |
| `armor_stand_manipulate` | server | Bukkit's PlayerArmorStandManipulateEvent |
| `beacon_state_change` | server | NeoForge has no such event |
| `bell_ring` | server | NeoForge has no such event; Bukkit's BellRingEvent |
| `biome_colors` | client | NeoForge adds ColorResolvers with RegisterColorHandlersEvent.ColorResolvers; it does not retint a biome's own colours |
| `block_break_progress_update` | server | NeoForge has no such event |
| `block_burn` | server | NeoForge has no such event; Bukkit's BlockBurnEvent |
| `block_changed` | server | NeoForge has no set-block event; IBlockExtension.onBlockStateChange is the block part hook on_block_state_change, at the end of the write |
| `block_cook` | server | NeoForge has no such event |
| `block_damage_abort` | server | NeoForge has no such event; Bukkit's BlockDamageAbortEvent |
| `block_dispense_armor` | server | NeoForge has no such event |
| `block_entity` | both | vanilla EntityBlock.newBlockEntity; NeoForge adds no event or extension for it |
| `block_entity_client_tick` | client | vanilla BlockEntityTicker from EntityBlock.getTicker with the client level; NeoForge adds no event |
| `block_entity_data_received` | client | post-merge notice; IBlockEntityExtension.onDataPacket and handleUpdateTag are related loading paths, not counterparts |
| `block_entity_load_additional` | server | vanilla BlockEntity.loadAdditional; NeoForge adds no event for this moment |
| `block_entity_save_additional` | server | vanilla BlockEntity.saveAdditional; NeoForge adds no event for this moment |
| `block_entity_tick` | server | vanilla BlockEntityTicker; NeoForge adds no event or extension for it |
| `block_entity_update_tag` | server | vanilla BlockEntity.getUpdateTag; NeoForge adds no event or extension for it); IBlockEntityExtension.handleUpdateTag is the client half |
| `block_entity_view_distance` | client | vanilla BlockEntityRenderer.getViewDistance; NeoForge adds no event or extension for it |
| `block_fade` | server | NeoForge has no such event |
| `block_failed_dispense` | server | NeoForge has no such event |
| `block_form` | server | NeoForge has no such event |
| `block_ignite` | server | NeoForge has no such event; Bukkit's BlockIgniteEvent |
| `block_jump_factor` | both | vanilla Block.getJumpFactor; NeoForge adds no event or extension for it |
| `block_lock_check` | server | NeoForge has no such event |
| `block_receive_game` | server | NeoForge has no such event |
| `block_render_visibility` | client | Sunrise-only; NeoForge has no counterpart |
| `block_shear_entity` | server | NeoForge has no such event |
| `block_signal` | server | vanilla BlockBehaviour.getSignal and getDirectSignal; NeoForge adds no event or extension for it |
| `block_speed_factor` | both | vanilla Block.getSpeedFactor; NeoForge adds no event or extension for it |
| `block_spread` | server | BlockEvent.FluidPlaceBlockEvent covers only fluids making stone and the like, not flow or fire spread |
| `bounce_on` | server | vanilla Block.bounceOn; IBlockExtension.getBounceRestitution only returns a factor |
| `break_progress_positions` | client | vanilla LevelRenderer.destroyBlockProgress |
| `brewing_fuel` | server | NeoForge has no such event |
| `brewing_start` | server | NeoForge has no such event |
| `broadcast_message` | server | NeoForge has no such event; Bukkit's BroadcastMessageEvent |
| `bucket_empty` | server | vanilla BucketItem.emptyContents; NeoForge has no matching event |
| `camera_position` | client | vanilla Camera.setPosition; NeoForge has no camera position event |
| `camera_submersion` | client | vanilla Camera.getFluidInCamera; NeoForge adds no event for it |
| `camera_type_cycle` | client | NeoForge has no camera type event |
| `campfire_start` | server | NeoForge has no such event |
| `can_eat` | both | vanilla Player.canEat; NeoForge adds no event or extension for it |
| `can_place_item` | server | vanilla Container.canPlaceItem; the item capability wraps this decision |
| `can_place_liquid` | both | vanilla LiquidBlockContainer.canPlaceLiquid |
| `can_survive` | both | vanilla BlockBehaviour.canSurvive; NeoForge adds no event or extension for it |
| `can_take_item_through_face` | server | vanilla WorldlyContainer.canTakeItemThroughFace; the sided item capability wraps this decision |
| `cauldron_level_changed` | server | NeoForge has no such event; Bukkit's CauldronLevelChangeEvent |
| `chat_recipient` | server | NeoForge has no per-receiver server chat event |
| `chat_suggestion_applied` | client | vanilla command suggestions; NeoForge adds no event |
| `client_block_changed` | client | vanilla ClientLevel.setBlock; NeoForge adds no client event for it |
| `client_break_progress` | client | vanilla ClientLevel.destroyBlockProgress; NeoForge adds no client event for it |
| `client_identity` | client | NeoForge patches ClientBrandRetriever.getClientModName to use BrandingControl.getClientBranding, without a per-address event |
| `client_mining_predict` | client | client mining prediction |
| `client_packet` | client | NeoForge has no raw play-packet receive event |
| `client_packet_send` | client | NeoForge has no raw play-packet send event |
| `client_pick` | client | vanilla Minecraft.pick, before tick inputs and frame extraction |
| `client_take_item_entity` | client | vanilla ClientPacketListener.handleTakeItemEntity; NeoForge's pickup events are server-side |
| `collision_shape` | both | vanilla BlockBehaviour.getCollisionShape with its CollisionContext; NeoForge adds no event or extension for it |
| `component_hover` | client | vanilla GuiGraphicsExtractor.componentHoverEffect; NeoForge adds no event |
| `compost_item` | server | NeoForge has no such event |
| `connects_to` | server | vanilla FenceBlock.connectsTo / WallBlock.connectsTo / IronBarsBlock.attachsTo |
| `console_line` | server | NeoForge has no log event; mods add a Log4j appender |
| `consume_effect` | server | runtime ConsumeEffect.apply; RegisterEvent registers effect types, not invocations |
| `contained_fluid` | client | FluidUtil.getFirstStackContained over Capabilities.Fluid.ITEM, read by DynamicFluidContainerModel; Sunrise's ITEM_FLUIDS needs a server place, so the client model asks the item |
| `container_opened` | server | not NeoForge's PlayerContainerEvent.Open: this hook can cancel the opening |
| `contents_changed` | server | Container.setChanged has no bus event |
| `crafter_craft` | server | NeoForge has no such event |
| `creative_slot` | server | vanilla ServerGamePacketListenerImpl.handleSetCreativeModeSlot; NeoForge adds no event or extension for it |
| `creeper_ignite` | server | NeoForge has no such event |
| `creeper_power` | server | NeoForge has no such event |
| `destroy_progress` | both | vanilla BlockBehaviour.getDestroyProgress; NeoForge adds no event or extension for it |
| `dispense_item` | server | vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it |
| `dispense_method` | server | vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it |
| `dragon_fight_tick` | server | NeoForge has no dragon fight event |
| `dragon_fight_won` | server | NeoForge has no dragon fight event |
| `dragon_respawn_stage` | server | NeoForge has no dragon fight event |
| `emissive` | client | Sunrise-only; NeoForge has no counterpart |
| `enchant_item` | server | NeoForge has no such event |
| `end_crystal_destroyed` | server | NeoForge has no dragon fight event |
| `entities_load` | server | NeoForge has no such event |
| `entities_unload` | server | NeoForge has no such event |
| `entity_break_door` | server | NeoForge has no such event |
| `entity_breed` | server | NeoForge has no such event |
| `entity_change_block` | server | NeoForge has no such event |
| `entity_change_block_pre` | server | NeoForge has no such event; Bukkit's EntityChangeBlockEvent is cancellable |
| `entity_drop_item` | server | NeoForge has no such event; Bukkit's EntityDropItemEvent |
| `entity_enter_block` | server | NeoForge has no such event |
| `entity_enter_love_mode` | server | NeoForge has no such event |
| `entity_inside` | server | vanilla Block.entityInside; NeoForge adds no event or extension for it |
| `entity_move` | server | fork addition, #2308 |
| `entity_outline` | client | NeoForge 26.3 has no glowing or team colour override |
| `entity_portal` | server | NeoForge has no such event |
| `entity_pose_change` | server | NeoForge has no such event |
| `entity_tame` | server | NeoForge has no such event |
| `entity_teleport` | server | NeoForge has no such event |
| `entity_toggle_glide` | server | NeoForge has no such event |
| `entity_unleash` | server | NeoForge has no such event; Bukkit's EntityUnleashEvent |
| `exp_bottle` | server | NeoForge has no such event |
| `experience_orb_can_merge` | server | vanilla ExperienceOrb.canMerge; NeoForge adds no event or extension for it |
| `experience_orb_merge` | server | vanilla ExperienceOrb.tryMergeToExisting; NeoForge adds no event or extension for it |
| `explosion_prime` | server | NeoForge has no such event; Bukkit's ExplosionPrimeEvent |
| `fence_gate_in_wall` | both | vanilla FenceGateBlock.isWall |
| `fill_bucket` | server | NeoForge 26.3 has no bucket-fill event |
| `firework_explode` | server | NeoForge has no such event |
| `fishing_medium` | both | NeoForge's ItemFishedEvent fires after the loot roll, in FishingHook.retrieve |
| `fluid_flow` | both | vanilla FlowingFluid.getFlow; NeoForge adds no event or extension for it |
| `fluid_level_change` | server | NeoForge has no such event |
| `fluid_passage` | server | NeoForge has no such event |
| `fluid_rules` | server | BaseFlowingFluid.getTickDelay, getDropOff, getSlopeFindDistance and FluidType.canConvertToSource are overrides; NeoForge adds no event |
| `fluid_tick` | server | NeoForge has no such event |
| `food_level_changed` | server | vanilla: FoodData; Bukkit's FoodLevelChangeEvent |
| `framerate_limit` | client | vanilla FramerateLimitTracker.getFramerateLimit fires no event |
| `furnace_burn` | server | NeoForge has no such event |
| `furnace_smelt` | server | NeoForge has no such event |
| `furnace_start_smelt` | server | NeoForge has no such event |
| `general_entity_tick` | server | vanilla Entity.tick has no part-scoped counterpart; NeoForge's EntityTickEvent is the global pre/post pair |
| `hanging_break` | server | vanilla hanging entity breaking; NeoForge adds no event or extension for it); AttackEntityEvent fires for players only |
| `hanging_place` | server | vanilla HangingEntityItem.useOn; NeoForge adds no event or extension for it); PlayerInteractEvent.RightClickBlock fires before it |
| `has_analog_output` | server | vanilla BlockBehaviour.hasAnalogOutputSignal; NeoForge adds no event or extension for it |
| `held_item_tint` | client | vanilla ItemInHandLayer.submitArmWithItem and PlayerItemInHandLayer.submitArmWithItem |
| `hopper_inventory_search` | server | NeoForge has no such event; Bukkit's InventoryMoveItemEvent |
| `hopper_pickup_item` | server | NeoForge has no direct equivalent; Bukkit's InventoryPickupItemEvent |
| `humanoid_setup_anim` | client | vanilla HumanoidModel.setupAnim; NeoForge has no mid-setupAnim event |
| `hurt_enemy` | server | vanilla Item.hurtEnemy; NeoForge adds no event or extension for it); LivingDamageEvent.Post comes later |
| `inventory_block_start` | server | NeoForge has no such event |
| `inventory_move_item` | server | NeoForge has no such event; Bukkit's InventoryMoveItemEvent |
| `inventory_tick` | server | vanilla Item.inventoryTick; NeoForge adds no event or extension for it); PlayerTickEvent ticks the player, not each stack |
| `is_bonemeal_success` | server | vanilla BonemealableBlock.isBonemealSuccess; NeoForge adds no event or extension for it |
| `is_pathfindable` | server | vanilla BlockBehaviour.isPathfindable; NeoForge 26.3 has no IBlockExtension.isPathfindable, IBlockExtension.getBlockPathType answers a different question |
| `is_possible_to_respawn_in` | server | vanilla Block.isPossibleToRespawnInThis |
| `is_signal_source` | server | vanilla BlockBehaviour.isSignalSource; NeoForge adds no event or extension for it |
| `is_valid_bonemeal_target` | both | vanilla BonemealableBlock.isValidBonemealTarget; NeoForge adds no event or extension for it); BonemealEvent fires when meal is used, before the block's contract |
| `is_valid_book_shelf` | both | EnchantingTableBlock.isValidBookShelf; NeoForge's extension is the per-block IBlockExtension.getEnchantPowerBonus and it has no hook here |
| `item_after_vertical_collision` | both | Block.updateEntityAfterFallOn; dropped-item contact |
| `item_bar` | client | vanilla Item.isBarVisible/getBarWidth/getBarColor; RegisterItemDecorationsEvent registers decorators, not bar queries |
| `item_interact_mob` | both | Item.interactLivingEntity |
| `item_spawn` | server | NeoForge has no such event |
| `leaves_decay` | server | NeoForge has no such event |
| `lightning_strike` | server | NeoForge has no such event |
| `living_death_message` | server | vanilla TamableAnimal.die owner message; separate from living death |
| `living_death_post` | server | LivingDeathEvent fires before the death |
| `living_death_protection` | server | vanilla LivingEntity.checkTotemDeathProtection; accessory mods inject there |
| `living_flying_speed` | both | vanilla LivingEntity.getFlyingSpeed; NeoForge adds no event or extension for it |
| `living_jump_power` | both | vanilla LivingEntity.getJumpPower; not LivingEvent.LivingJumpEvent |
| `living_speed` | both | vanilla LivingEntity.getSpeed; NeoForge adds no event or extension for it |
| `living_travel` | both | fork addition, #2308 |
| `menu_background` | client | NeoForge has no menu blur event |
| `menu_button` | server | NeoForge has no such event; vanilla AbstractContainerMenu.clickMenuButton |
| `menu_button_valid` | server | vanilla menu button ids are handled by their menu implementation |
| `menu_clicked` | server | a Java menu overrides AbstractContainerMenu.clicked; no event |
| `menu_links` | server | Sunrise mod menus expose linked container slots |
| `menu_quick_move` | server | a Java menu overrides AbstractContainerMenu.quickMoveStack; no event |
| `menu_removed` | server | vanilla AbstractContainerMenu.removed, which each menu overrides |
| `menu_slot_changed` | server | vanilla Container.setItem and AbstractContainerMenu.slotsChanged; NeoForge adds no event |
| `menu_slot_rules` | both | vanilla Slot.mayPlace and mayPickup; NeoForge adds no event or extension for it |
| `menu_slot_take` | server | a Java menu's slot overrides Slot.onTake; NeoForge adds no event |
| `menu_still_valid` | server | vanilla AbstractContainerMenu.stillValid, which each menu overrides |
| `menu_transfer` | server | JEI IRecipeTransferHandler, which each mod implements for its menus |
| `minecart_pass` | server | Forge's IBaseRailExtension.onMinecartPass is gone in NeoForge 26.3 |
| `mod_container_viewers` | server | vanilla ContainerOpenersCounter; NeoForge adds no event or extension for it); PlayerContainerEvent.Open and Close fire per player |
| `moisture_change` | server | NeoForge has no such event |
| `music_layers` | client | SelectMusicEvent swaps the one track; NeoForge has no layered music |
| `must_survive` | both | vanilla BlockItem.mustSurvive |
| `neighbor_changed` | server | vanilla Block.neighborChanged; NeoForge adds no event or extension for it); BlockEvent.NeighborNotifyEvent is the neighbour broadcast, a different point |
| `on_effect_added` | server | vanilla MobEffect.onEffectAdded; NeoForge adds no event or extension for it |
| `on_place` | server | vanilla BlockBehaviour.onPlace; NeoForge adds no event or extension for it |
| `panel_event` | client | Sunrise-only; NeoForge has no counterpart |
| `perform_bonemeal` | server | vanilla BonemealableBlock.performBonemeal; NeoForge adds no event or extension for it |
| `piston_carries_block_entity` | both | NeoForge has no counterpart; IBlockExtension.getRelocability is unrelated |
| `place_liquid` | server | vanilla LiquidBlockContainer.placeLiquid |
| `placement_context` | both | vanilla BlockItem.updatePlacementContext |
| `player_arm_swing` | server | NeoForge has no such event; Bukkit's PlayerAnimationEvent |
| `player_attack_cooldown_reset` | server | NeoForge has no such event |
| `player_bed_enter` | server | NeoForge has no such event; Bukkit's PlayerBedEnterEvent |
| `player_bucket_entity` | server | NeoForge has no such event |
| `player_bundle_item_select` | server | NeoForge has no such event; Bukkit's PlayerBundleItemSelect |
| `player_change_beacon_effect` | server | NeoForge has no such event |
| `player_client_loaded_world` | server | NeoForge has no such event; Bukkit's PlayerClientLoadedWorld |
| `player_commands_sent` | server | NeoForge 26.3 has no send-time filter event |
| `player_deep_sleep` | server | NeoForge has no such event |
| `player_destroyed_block` | both | NeoForge has no event for Block.destroy; BreakEvent fires before the removal |
| `player_edit_book` | server | NeoForge has no such event; Bukkit's PlayerEditBookEvent |
| `player_egg_throw` | server | NeoForge has no such event |
| `player_elytra_boost` | server | NeoForge has no such event |
| `player_exp_cooldown_change` | server | NeoForge has no such event |
| `player_fish` | server | NeoForge's ItemFishedEvent fires after the loot roll, in FishingHook.retrieve; Bukkit's PlayerFishEvent |
| `player_flower_pot_manipulate` | server | NeoForge has no such event |
| `player_input` | server | NeoForge has no such event; Bukkit's PlayerInputEvent |
| `player_insert_lectern_book` | server | NeoForge has no such event |
| `player_inventory_changed` | server | vanilla InventoryChangeTrigger; NeoForge adds no event |
| `player_item_cooldown` | server | NeoForge has no such event; Paper's PlayerItemCooldownEvent |
| `player_item_frame_change` | server | NeoForge has no such event |
| `player_item_mend` | server | NeoForge has no such event |
| `player_kicked` | server | NeoForge has no such event; Bukkit's PlayerKickEvent |
| `player_lectern_page_change` | server | NeoForge has no such event |
| `player_level_change` | server | NeoForge has no such event; Bukkit's PlayerLevelChangeEvent |
| `player_login` | server | Bukkit's PlayerLoginEvent; NeoForge's PlayerNegotiationEvent is not cancellable |
| `player_login_profile` | server | NeoForge's PlayerNegotiationEvent shows the profile but can't edit it before it is sent |
| `player_loom_pattern_select` | server | NeoForge has no such event |
| `player_map_filled` | server | NeoForge has no such event |
| `player_move` | server | NeoForge has no player move event; PlayerTickEvent covers the tick |
| `player_name_entity` | server | NeoForge has no such event |
| `player_open_sign` | server | NeoForge has no such event |
| `player_passage_cross` | server | NeoForge has no such event; Immersive Portals' Portal.canTeleportEntity |
| `player_pick_item` | server | NeoForge has no such event; Paper's PlayerPickItemEvent |
| `player_pickup_arrow` | server | NeoForge has no such event |
| `player_recipe_book_click` | server | NeoForge has no such event |
| `player_recipe_book_settings_change` | server | NeoForge has no such event; Bukkit's PlayerRecipeBookSettingsChange |
| `player_recipe_discovered` | server | NeoForge has no such event; Bukkit's PlayerRecipeDiscoverEvent |
| `player_riptide` | server | NeoForge has no such event; Bukkit's PlayerRiptideEvent |
| `player_shear_entity` | server | NeoForge has no such event; Bukkit's PlayerShearEntityEvent |
| `player_shield_disable` | server | NeoForge has no such event |
| `player_sprint` | server | NeoForge has no sprint event |
| `player_start_spectating_entity` | server | NeoForge has no such event |
| `player_stays_on_ground_surface` | both | vanilla Player.isStayingOnGroundSurface; NeoForge adds no event or extension for it |
| `player_stonecutter_recipe_select` | server | NeoForge has no such event |
| `player_stop_spectating_entity` | server | NeoForge has no such event |
| `player_swap_with_equipment_slot` | server | NeoForge has no such event |
| `player_take_lectern_book` | server | NeoForge has no such event |
| `player_teleport_end_gateway` | server | NeoForge has no such event |
| `player_teleported` | server | EntityTeleportEvent.TeleportCommand fires before the move and can cancel it; this hook is told after the move |
| `player_toggle_entity_age_lock` | server | NeoForge has no such event |
| `player_toggle_flight` | server | NeoForge has no flight toggle event; Bukkit's PlayerToggleFlightEvent |
| `player_toggle_sneak` | server | NeoForge has no such event; Bukkit's PlayerToggleSneakEvent |
| `player_turn` | client | vanilla Entity.turn; CalculatePlayerTurnEvent covers mouse sensitivity/cinematic state only |
| `player_unleash_entity` | server | NeoForge has no such event |
| `player_velocity` | server | NeoForge has no such event |
| `portal_create` | server | NeoForge has no such event |
| `potion_splash` | server | NeoForge has no such event |
| `pre_remove_side_effects` | server | vanilla BlockEntity.preRemoveSideEffects; NeoForge adds no event or extension for it |
| `prepare_hopper_transfer` | server | NeoForge has no hopper event |
| `prepare_item_craft` | server | NeoForge has no such event |
| `prepare_item_enchant` | server | NeoForge has no such event |
| `prepare_smithing` | server | NeoForge has no such event |
| `raid_spawn_wave` | server | NeoForge has no such event |
| `raid_state_change` | server | NeoForge has no such event |
| `random_tick` | server | vanilla BlockBehaviour.randomTick; NeoForge adds no event or extension for it |
| `recipe_book_categories` | both | RegisterRecipeBookSearchCategoriesEvent registers search groups only, not a recipe type's categories |
| `recipe_book_ghost` | client | vanilla RecipeBookComponent.fillGhostRecipe, which each book component overrides; no event |
| `recipe_book_place` | server | vanilla RecipeBookMenu.handlePlacement, which each menu overrides; no event |
| `recipe_sources` | server | Sunrise mod crafting menus lend ingredient containers |
| `register_behaviors` | server | vanilla LivingEntity.makeBrain |
| `register_goals` | server | vanilla Mob.registerGoals |
| `remove_block` | server | vanilla Level.removeBlock; no NeoForge event covers this call |
| `render_needs` | client | Sunrise-only; NeoForge has no counterpart |
| `render_view_transform` | client | vanilla GameRenderer.bobHurt/bobView; NeoForge adds no event for it |
| `resolve_player_skin` | client | vanilla AbstractClientPlayer.getSkin |
| `resource_pack_status` | server | NeoForge has no resource-pack status event |
| `screen_exclusion_areas` | client | NeoForge has no counterpart in the game; recipe viewers define their own API |
| `screen_room` | client | a NeoForge mod reads AbstractContainerScreen.leftPos and cannot move it |
| `sculk_bloom` | server | NeoForge has no such event |
| `section_visibility` | client | verified |
| `server_tick_rate_changed` | server | vanilla MinecraftServer.onTickRateChanged; NeoForge adds no event or extension for it |
| `set_placed_by` | server | vanilla Block.setPlacedBy |
| `should_apply_effect_tick_this_tick` | server | vanilla MobEffect.shouldApplyEffectTickThisTick |
| `should_redstone_wire_connect_to` | server | vanilla BlockBehaviour.shouldRedstoneWireConnectTo; NeoForge adds no event or extension for it |
| `sign_changed` | server | NeoForge has no sign-editing event |
| `sleep_skip_night` | server | SleepFinishedTimeEvent cancels only the clock move |
| `sleep_status_announce` | server | NeoForge has no such event |
| `sleepers_needed` | server | vanilla SleepStatus.sleepersNeeded; NeoForge adds no event |
| `smoke_source` | both | vanilla CampfireBlock.isSmokeSource; NeoForge adds no event or extension for it |
| `spawn_change` | server | NeoForge has no such event |
| `sponge_absorb` | server | NeoForge has no such event |
| `state_for_placement` | both | vanilla Block.getStateForPlacement; NeoForge adds no event or extension for it |
| `stateful_block_entity` | both | static factory ownership for vanilla EntityBlock.newBlockEntity |
| `step_on` | both | vanilla Block.stepOn |
| `substitute_inline_images` | client | vanilla ChatComponent and AbstractSignRenderer.submitSignText |
| `tab_complete` | server | NeoForge has no generic tab completion event |
| `target_hit` | server | NeoForge has no such event |
| `tick_block` | server | vanilla BlockBehaviour.tick; NeoForge adds no event or extension for it |
| `tick_target_millis` | client | vanilla Minecraft.getTickTargetMillis; NeoForge adds no event or extension for it |
| `time_skip` | server | NeoForge has no such event |
| `tnt_primed` | server | NeoForge has no TNT priming event; the primed TNT entity joins with EntityJoinLevelEvent |
| `tooltip_image` | client | vanilla Item.getTooltipImage |
| `trade_select` | server | NeoForge has no such event |
| `update_shape` | server | vanilla BlockBehaviour.updateShape; NeoForge adds no event or extension for it |
| `use_animation` | both | vanilla Item.getUseAnimation; NeoForge adds no event or extension for it |
| `use_duration` | both | vanilla Item.getUseDuration; NeoForge adds no event or extension for it); LivingEntityUseItemEvent.Start can set a use's duration |
| `use_item_on` | both | vanilla BlockBehaviour.useItemOn; PlayerInteractEvent.RightClickBlock cancels or answers before it |
| `use_without_item` | both | vanilla BlockBehaviour.useWithoutItem; PlayerInteractEvent.RightClickBlock cancels or answers before it |
| `vault_change_state` | server | NeoForge has no such event |
| `vault_loot` | server | NeoForge has no such event; Bukkit's BlockDispenseLoot |
| `vehicle_create` | server | NeoForge has no such event; Bukkit's VehicleCreateEvent |
| `vehicle_damage` | server | NeoForge has no such event; Bukkit's VehicleDamageEvent |
| `vehicle_destroy` | server | NeoForge has no such event; Bukkit's VehicleDestroyEvent |
| `vehicle_moved` | server | NeoForge has no vehicle move event; EntityTickEvent covers the tick |
| `viewed_level_sound` | client | NeoForge has no viewed levels |
| `visuals` | client | Sunrise-only; NeoForge has no counterpart |
| `weather_change` | server | NeoForge has no such event |
| `whitelist_state_update` | server | NeoForge has no such event; Bukkit's WhitelistToggleEvent |
| `wind_burst` | server | vanilla wind-charge block interaction; NeoForge adds no event |
| `world_border_bounds_change_finish` | server | NeoForge has no such event; Bukkit's WorldBorderBoundsChangeFinish |
| `world_border_bounds_changed` | server | NeoForge has no such event |
| `world_border_center_changed` | server | NeoForge has no such event |
| `world_saved` | server | NeoForge has no such event |
| `world_view_drawn` | client | Sunrise-only; NeoForge has no counterpart |
| `world_views` | client | Sunrise-only; NeoForge has no counterpart |
