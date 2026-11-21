//! Native hook implementations, split by catalog area.
//!
//! Hook documentation and contract versions live in the catalog, not a shared changelog.
//!
//! Native mods are built against an exact game revision, rather than a hook API revision.
//! Bodies have no `cfg` and no macros: horizon-mixins refuses a target with
//! either (`tests::hook_points_stay_hookable`).

/// Generated dispatch entry points, not hand-written hook implementations.
mod fire;
pub(in crate::game) use build_creative_mode_tab_contents::CreativeCache;
pub(in crate::game) use client_pick::ClientPick;
pub(in crate::game) use fire::{
    BlockDropsCall, BlockScreenEffectDrawCall, ConnectsToCall, EntityChangeBlockPreCall,
    ItemAfterVerticalCollisionCall, PreRemoveSideEffectsCall, UpdateShapeCall,
};
use mod_api::{BlockEntityDataSource, BlockEntityRef, ClockAdjustment};

use super::native_data_maps::DataMapType;
use super::{
    Game, GameLevel, LevelDimension, Screen,
    execution_role::{self, ExecutionRole},
    mod_api_host,
    native_block_entity_draw::BlockEntityDraw,
    native_configs::ConfigFile,
    native_mob_draw::{MobRender, PlayerRender},
    native_panel_recipes::RecipePage,
};
use mod_api::CommandSource;
use mod_api::GameMode;
use mod_api::GameTests;
pub use mod_api::game::{PhantomSpawnResult, Shake, SleepProblem};
use mod_api::{
    self, Answered, BlockPlacement, BlockTintSources, CameraDrawContext, CauldronFluidContents,
    CauldronInteractions, ClientAudioOptions, ClientChatOptions, ClientGraphicsOptions,
    ClientGuiOptions, ClientInputOptions, ClientMiningPrediction, ClientModifiers,
    ClientReloadListeners, ColorResolvers, CommandInvocation, EnchantmentDamageEffects,
    EnchantmentInfo, EntityLayers, EntityRef, EntityRenderers, GameRuleActor, HandRender,
    InsideEffects, InteractionAttempt, InteractionKey, ItemEntityRef, JsonAnimationType,
    LayerDefinition, LifecycleContext, LivingRef, LoadConditions, NoteBlockChange,
    ParticleProviders, PathComputationType, Placement, PlayerRef, PoiTypeExtensions,
    RecipeBookCategory, ScreenId, ScreenOpening, ScreenTarget, ScreenshotImage,
    ServerReloadListeners, ServerTickPhase, SpecialModelRenderers, TicketController, ToolUse,
    VillagerProfessions, WorldPhase,
};
use mod_api::{
    ActiveEffect, AttachmentStore, BlockClick, BlockUpdateFlags, BonemealSource, ChunkRef,
    FluidStack, ItemClick, ItemDrop, LevelRef, LogicalSide, NeighbourUpdate, TooltipElement,
    TriState, UsePhase,
};
use mod_api::{
    BlockKey, DebugEntry, DebugRenderer, EffectKey, GuiLayer, HiddenParts, MapDecorationType,
    ParticleGroup, PosedParts, SkullModel, TaskId, TextureAtlasRegistration,
};
pub(crate) use mod_api::{
    BlockRim, ClientPacket, ClientPayload, ColoredLight, ContainerLootFilled, ExplosionStart,
    ItemFished, LootDrops, PanelEvent, PortalShape, ScreenView,
};
use mod_api::{
    CameraType, Despawn, Identifier, MobRef, PermissionLevel, RailShape, SoundPlay, SoundRef,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use sunrise_client::block_contact::Pending;
#[cfg(test)]
use sunrise_client::mobs::Damage;
use sunrise_client::{
    atlas_sources::qualified,
    biome::{BiomeColors, SpawnEntry},
    block_entities::Pos,
    chat::ChatSource,
    containers::{
        BlockEntity, MOD_MENUS, ModBlockEntityFields, ModContainer, ModEnergyBuffer, ModFluidTanks,
    },
    crafting::{Ingredient, recipe_result},
    extras::Corner,
    extras_hud::Corners,
    fast_hash::FxHashMap,
    first_person::Hand,
    health::{
        Attack, DamageContainer, DamageHooks, DamageLevel, HurtSource, Hurting, Knock, ShieldBlock,
        Victim,
    },
    hud::{HudBatch, rgba},
    hud_layout::GuiDraw,
    items::ItemStack,
    keybinds,
    merchant::MerchantOffer,
    mobs::Who,
    mod_list::{self, Installed, KeyMapping, NativePage, NativeSetting, NativeSettingValue},
    music::{Music, MusicLayer},
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use sunrise_client_core::gamerules::Value as GameRuleValue;
use sunrise_client_core::suggestion::Suggestion;
use sunrise_core::{
    connection_kind::ConnectionKind,
    geometry::Direction,
    position::BlockPos,
    ticks::{ScheduleResult, TickPriority},
};
use sunrise_world::level::{BonemealCheck, PlaceContext};
use winit::{event::MouseButton, keyboard::KeyCode};
use {
    sunrise_core::{
        nbt::{Compound, Tag},
        use_animation::UseAnimation,
    },
    sunrise_world::{
        fluid::FluidKind,
        level::{LevelAccess, PlacementContext, Ticks},
        registry::{BlockRegistry, BlockStateId},
    },
};

include!(concat!(env!("OUT_DIR"), "/native_hook_modules.rs"));
pub(in crate::game) use add_attribute_tooltips::HookedAttributeTooltips;
pub(in crate::game) use base::LevelView;
pub(in crate::game) use bounce_restitution::install_block_contact_hooks;
pub(in crate::game) use living_change_target::NativeHookHost;
use mod_api::ModBlockEntity;
use sound_and_shader_helpers::{finite, finite_params, local_sound_ok};
pub(super) use sound_and_shader_helpers::{level_sound_playable, mod_sound_ok, sound_id};
include!("shared.rs");
