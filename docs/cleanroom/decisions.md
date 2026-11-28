# Implementation decisions

Behaviours the spec ([shader-packs-spec.md](shader-packs-spec.md), revision 1) leaves open, each with the decision,
its reason and the tests that hold it ([README](README.md), "Decisions on record"). An entry is never taken from another
implementation, its code, its issue tracker or its changelog. Decisions taken before this file existed (on the spec's
open questions, OQ-25, OQ-26 and others) are in the implementation plan, which was not published, so this file is not
the full list.

## D-1: which bolt `lightningBoltPosition` gives

- **Open point:** spec rev 1, 5.2, line 692 gives "player-space position of one rendered lightning bolt, w = 1; zero
  if none", without saying which bolt when several are drawn. It is not one of section 9's open questions.
- **Decision:** the last bolt in Sunrise's draw list: the frame's bolts (`bolt_instances`), which are the ones within
  vanilla's draw distance (`LightningBolt.shouldRenderAtSqrDistance`; `LightningBoltRenderer.affectedByCulling` is
  false), in the order the game keeps them (a bolt added later goes last). Its offset from the frame's `cameraPosition`
  (player space, spec line 607), subtracted in double precision by the same function that places the bolt's geometry
  (`gfx::lightning::offset`); zeros when the list is empty.
- **Reason:** the spec asks for one rendered bolt, and the draw list is what Sunrise renders, so the uniform and the
  drawn geometry agree. The order bolts are submitted in doesn't decide whose pixels end up on screen last, so this is
  not a claim about which flash the player sees last; the uniform only follows the existing list. Which bolts are
  visible and how they sort is separate rendering work. A bolt never moves, so its render position is where it
  struck.
- **Tests:** `gfx::pack_uniforms::lightning_bolt_test::the_last_bolt_in_the_draw_list_reaches_the_pack_in_player_space`
  (`sunrise-client-render`) and the daily `game::lightning_bolt_uniform_tests::a_summoned_bolt_reaches_the_pack_in_player_space`
  (`sunrise-client`).

## D-2: which player booleans programs read, from which vanilla method, and their type

- **Open point:** spec rev 1, 3.4, line 552 lists fourteen player booleans (`is_alive` … `is_wet`) for expressions,
  "state of the player (the camera entity)", and adds "`[IR]` also exposes six of them as uniforms". The uniform
  table of 5.2 (line 664) lists only those six (`is_burning is_hurt is_invisible is_on_ground is_sneaking
  is_sprinting`). The bullet at line 558 ("the player booleans as normal uniforms too") can be read as all fourteen.
  The spec names no vanilla method for any of them, and no type beyond "bool". None of this is one of section 9's
  open questions.
- **Decision 1, expression-only:** custom uniforms (`uniform.<type>.<name> = <expression>`) read all fourteen. A
  program that declares one of the eight others (`is_alive is_child is_glowing is_in_lava is_in_water is_ridden
  is_riding is_wet`) gets no built-in value: it reads 0, unless the pack defines a custom uniform of that name, which
  binds instead. Compute programs and quad programs follow the same rule. The basis is line 552's "six of them" and
  the six-name table at line 664, which agree; line 558 is the looser of the three and is read through them. The 3.4
  heading ("Parameters only for expressions (Tier A) and as uniforms (Tier E)") is no basis: `biome`, `temperature`
  and the rest sit under it and are program uniforms.
- **Decision 2, the vanilla method behind each name** (Minecraft 26.3, the window's player as a client answers;
  `LocalPlayer` and `AbstractClientPlayer` override none of them):
  - `is_alive`: `LivingEntity.isAlive` (`!isRemoved() && getHealth() > 0`; the window's player is never removed
    while it is the camera);
  - `is_child`: `LivingEntity.isBaby`, which `Player` keeps, so always 0;
  - `is_glowing`: `Entity.isCurrentlyGlowing` on a client, which skips the effect check and reads shared flag 6: the
    flag a server sends, or in single player the Glowing effect the flag is set from (a local `Glowing` tag is not
    modelled). A server's flag resets with a new `LocalPlayer` (a login, a respawn without `KEEP_ENTITY_DATA`), as
    the player's other shared flags do;
  - `is_in_lava`: `Entity.isInLava` (`!firstTick && isInFluid(LAVA)`);
  - `is_in_water`: `Entity.isInWater` (`wasTouchingWater`), not `isUnderWater`;
  - `is_ridden`: `Entity.isVehicle` (`!passengers.isEmpty()`), so a dying passenger counts until it is removed;
  - `is_riding`: `Entity.isPassenger` (`getVehicle() != null`);
  - `is_wet`: `Entity.isInWaterOrRain` (`isInWater() || isInRain()`, rain at `blockPosition` or at the box's top);
  - the six of the table keep the methods they already had.
- **Decision 3, bool only:** the eight are `bool` built-ins, like the six, so `uniform.float.x = is_wet` reads 0 as
  `uniform.float.x = is_sneaking` does. Only the six were probed; the eight are bool by inference from the spec's
  "bool" and from the six, not by measurement.
- **Not covered:** the spec's "the camera entity". While spectating another entity all fourteen read the window's
  player, as the six did before this entry.
- **Tests:** `gfx::pack_uniforms::initial_history_tests::custom_uniforms_read_every_player_boolean`,
  `gfx::pack_uniforms::initial_history_tests::programs_read_only_six_player_booleans` and
  `gfx::pack_post::player_boolean_tests::compute_and_quad_programs_read_only_the_listed_player_booleans`
  (`sunrise-client-render`); the daily `game::server_side::shader_packs_server::player_status_tests::pack_status_follows_the_player`,
  `game::server_side::shader_packs_server::player_status_tests::pack_status_leaves_queued_effects_alone`,
  `game::client_side::shader_packs::player_status_tests::pack_status_on_a_server_follows_what_it_sent` and
  `game::client_side::shader_packs::player_status_tests::pack_status_starts_over_with_a_new_local_player` (`sunrise-client`).

## D-3: an image whose format qualifier differs from its buffer's format

- **Open point:** spec rev 1, 5.2, lines 783-784 declare images as `layout(<format>) uniform image2D <name>;` over the
  buffer they name, and 6.7, lines 984-996 list the formats a buffer, raw texture or image may have. The spec doesn't
  say what happens when the qualifier names another format than the buffer's (`layout(rgba16)` on a buffer whose
  `colortex8Format` is `RGBA16F`), and it is not one of section 9's open questions. wgpu binds a storage image only
  in its texture's exact format, and the shader must declare that same format, so a differing qualifier is either
  replaced by the texture's format or its program is refused (#548).
- **Decision:** a 16-bit normalized qualifier (`rgba16`, `rg16`, `r16` and their `_snorm` forms) on the 16-bit float
  format of the same channels (`RGBA16F`, `RG16F`, `R16F`), which is the same size per texel, binds the texture's
  float format, and the program gets a note naming it, the image, the qualifier and the format bound. The same holds
  for the float format Sunrise makes of a 16-bit unorm or snorm buffer on a device without 16-bit normalized
  formats. Every other differing pair is refused, with the note naming the program, the image, the qualifier and
  the texture's format: another size per texel (`rgba8` on `RGBA16F`), other channels, an integer qualifier on a
  float or normalized texture or the reverse, unorm against snorm, and a float qualifier on a normalized texture.
  The `r11f_g11f_b10f` qualifier on Sunrise's `RGBA16F` stand-in for an `R11F_G11F_B10F` buffer or image keeps its
  earlier rule, which is the same in kind, and now gets the same note.
- **Semantics:** a rebound image is accessed as the texture's values, with the conversion of the texture's format,
  not the qualifier's. A store of 2.0 through `layout(rgba16)` stays 2.0 in an `RGBA16F` buffer, and a store of 0.5
  reads back as 0.5. So the same program can store differently by device: over an `RGBA16` buffer that the device
  holds as `RGBA16` (with `TEXTURE_FORMAT_16BIT_NORM`), the qualifier matches, and the store of 2.0 is clamped to 1.0;
  over the `RGBA16F` that Sunrise makes of that buffer without the feature, it stays 2.0. The note says when this
  happens. Sunrise does not reinterpret the texture's bits as the qualifier's format; doing so would leave the
  unorm16 bits of 0.5 (0x8000), which a half float reads as -0.0.
- **Reason:** the spec leaves the pair open, and wgpu binds a storage texture only in its own format, with the shader
  declaring that same format; so the program can keep its image only in the texture's format. The buffer's format is
  what the pack chose for its data. The shader reads and writes floating-point vectors through unorm, snorm and float
  formats alike, so taking the texture's format changes no expression of the program, only the conversion at the
  texture. The pairs are limited to equal texel sizes so that a qualifier is never taken over a buffer of another
  precision or channel count.
- **Tests:** `pack_post::tests::only_16_bit_normalized_qualifiers_bind_their_float_texture`,
  `pack_post::tests::a_rebound_image_declares_its_textures_format_alone` and
  `pack_post::tests::a_rebound_image_is_noted_and_an_exact_one_is_not` (`sunrise-render-shaderpack`); the daily
  `gfx::pack_post::normalized_image_tests::an_rgba16_image_of_a_float_buffer_keeps_the_floats_stored`,
  `gfx::pack_post::normalized_image_tests::an_rgba16_snorm_image_of_a_float_buffer_keeps_the_floats_a_quad_program_stores`,
  `gfx::pack_post::normalized_image_tests::an_rgba16f_image_of_a_float_buffer_binds_without_a_note` and
  `gfx::pack_post::normalized_image_tests::an_rgba8_image_of_a_float_buffer_still_refuses_its_program`
  (`sunrise-client-render`).

## D-4: native terrain quads of block entities, entities and the hand under a pack

- **Open point:** spec rev 1, 2.3 (line 294) gives `gbuffers_block` "block entities, sign text" and `gbuffers_entities`
  "entities", and 6.1.4 sends a program's outputs to the buffers its `RENDERTARGETS`/`DRAWBUFFERS` list names.
  Sunrise's mods can draw, inside a block entity's or an entity's run, terrain-sheet geometry that Sunrise draws
  natively from its sprite arrays: still fluids and terrain models (`FLUID_OVERLAY`, `TERRAIN_MODEL_*_OVERLAY`; a
  block entity's renderer, a mod's model copies, a mod's entity renderer or layer, and a mod's `render_hand` listener
  in the first-person hand all draw through the same context). The spec has no such geometry, so it does not say which
  program draws it, which texture it reads, which maps its `normals` and `specular` are, or in what order its
  translucent quads draw.
- **Decision:** the program of its run draws it, as the rest of that block entity or entity: `gbuffers_block` or
  `gbuffers_entities`, and for the translucent kinds (a still fluid, a translucent terrain model) that program's
  translucent one (`gbuffers_block_translucent`, `gbuffers_entities_translucent`) when the pack has it, with the run's
  `blockEntityId` or `entityId`. It samples the pack's block atlas, the one the pack's terrain samples (`pack_atlas`):
  each quad's UVs are placed in its sprite's cell of that atlas, clamped to the sprite per vertex as the pack's
  terrain clamps them. Its `normals` and `specular` read the stand-ins a program reads with no map bound: these
  programs read those maps as the shared atlas's paged maps, whose pages say nothing of the block atlas. A run's
  translucent quads, wherever they lie in it, are one draw after its other quads, far to near by the distance and tie
  order Sunrise's own draw sorts them with (`QuadSort`); separate runs stay separate draws, in the order they came.
  The pack's shadow program draws what the run's program draws and its shadow pass casts (an entity's translucent
  parts cast nothing, as its other translucent parts). A quad whose sprite the block atlas lacks (a layer past the
  atlas's table, or a fluid layer below 0, NaN or infinite), a pack without the run's program (even one with a shadow
  program: nothing casts the quad then, as before), or one without a block atlas leaves the quad to Sunrise.
  - In the first-person hand, `gbuffers_hand` draws them as the hand's other geometry, and the translucent kinds
    `gbuffers_hand_water` (or its fallback) after deferred, with the hand's other translucent quads; each hand run's
    translucent ones together after its other translucent quads, far to near as above. They sample the block atlas
    with the same stand-in maps. Sunrise draws nothing of a hand that a pack's `gbuffers_hand` draws, so a quad the
    block atlas lacks (or every one of them, without a block atlas) is not drawn there: its UVs and layer mean
    nothing in the shared atlas either.
- **Reason:** 2.3's programs for block entities and entities are the only ones the spec ties to this geometry, and
  6.1.4 then puts it in the buffers the pack's `final` reads; drawn natively, it reached only `colortex0`. The block
  atlas already holds every terrain sprite, as the pack's terrain reads them, so no texture is copied; the renderer
  has no table from a terrain sprite layer to its place in the shared atlas. Paged-map reads of the block atlas's UVs
  would return another texture's material. Sunrise's own draw sorts these translucent kinds on upload, across every
  run; the pack's draws sort each run, since one draw carries one `blockEntityId` or `entityId`. In the hand, 2.3
  (lines 301 and 304) gives `gbuffers_hand` "the hand and opaque held objects" and `gbuffers_hand_water` "translucent
  held objects", drawn after deferred.
- **Tests:** `pack_entities::material_route_tests::native_terrain_quads_take_the_runs_program_where_the_block_atlas_holds_them`
  and `pack_entities::material_route_tests::the_block_atlas_places_native_terrain_quads` (`sunrise-render-shaderpack`);
  the daily `pack_entities::gpu_span_tests::native_terrain_quads_draw_in_the_pack_programs_from_the_block_atlas`,
  `pack_entities::gpu_span_tests::a_runs_translucent_native_quads_draw_once_far_to_near` and
  `pack_entities::gpu_span_tests::an_entitys_native_terrain_quads_draw_in_the_entity_programs`
  (`sunrise-render-shaderpack`), and `pack_block_entity_runs` and
  `gfx::pack_post::block_atlas_group_tests::a_resize_drops_the_portal_and_block_atlas_groups` (`sunrise-client-render`);
  for the hand, `pack_hand::tests::native_terrain_quads_take_the_block_atlas` and
  `pack_hand::tests::without_a_block_atlas_native_terrain_quads_are_not_drawn` (`sunrise-render-shaderpack`) and the
  daily `pack_hand_terrain_quads` (`sunrise-client-render`).

## D-5: which faces a `layer.*` rule moves, and what wins

- **Open point:** spec rev 1, 4.2, lines 590-593 say `layer.solid`, `layer.cutout`, E `layer.cutout_mipped` and
  `layer.translucent` in `block.properties` "move blocks to another terrain layer", and that full opaque cubes cannot
  move `[OF-SH]` (followed as written: such a state keeps its layer, with a load note naming the rule and the states;
  the issue's own control moves one, so that conflict is asked in `questions/Q-001.md`). 4.1, lines 575-579 give the
  names, state filters and E `%<tag>` entries, and say that "individual entries win over tags when both match". The
  spec doesn't say which rule wins when several match one state, what `cutout_mipped` is where the terrain has one
  alpha-tested pass, whether a rule also moves the fluid a block holds, or whether it wins over the options that choose
  a block's layer (`cutoutLeaves`, `opaqueIce`). None of these is one of section 9's open questions.
- **Decision:**
  - A rule naming the block wins over a rule naming its tag; then the rule with the most state filters that all match;
    then the first in the file. This is the order 4.1 gives entries, and the one `block.<id>` lines already use.
  - `layer.cutout_mipped` is `layer.cutout`: the terrain has one alpha-tested pass, the mesher's solid quads without
    `OPAQUE_LAYER` (`Mesher::quad_as`).
  - An unknown `layer.<name>` and a word that is no block name are skipped with a load note naming them.
  - A rule moves the state's own model faces, with their connected-texture tiles and overlays, and wins over the
    face's own sprite and over `cutoutLeaves` and `opaqueIce`. Fluid faces (a liquid block, the water of a waterlogged
    block), the models a mod attaches to a block and a mod's section geometry keep their own layer.
  - Solid draws with no alpha test and no blending; cutout keeps the alpha test even where every texel is opaque;
    translucent blends and is drawn by `gbuffers_water` (and in the shadow pass by `shadow_water`).
  - Without a pack, with a pack without `layer.*` rules, and with rules that move no state of the game, every state
    keeps its own layer and the terrain meshes as without them.
- **Reason:** the precedence is the one 4.1 states and the id rules use, so one file reads one way. The options and
  the face's sprite are the game's own choices of a layer, which the pack's rule replaces. The fluid has a layer of its
  own, the fluid's, which no rule names.
- **Tests:** `block_ids::tests::layer_rules_move_named_states_and_tags` and `block_ids::tests::layer_rule_precedence`
  (`sunrise-shaders`); the daily `mesher::layer_override_tests::translucent_moves_only_the_named_state`,
  `mesher::layer_override_tests::cutout_clears_the_opaque_bit_of_the_named_state_only`,
  `mesher::layer_override_tests::rules_win_over_the_sprite_and_the_options` and
  `mesher::layer_override_tests::no_moved_state_meshes_to_the_byte`, and
  `mesher::quarter_tests::a_tile_over_a_moved_face_takes_its_layer` (`sunrise-client-render`);
  `game::pack_layers::tests::states_get_their_rules_layer_by_id`,
  `game::pack_layers::tests::full_opaque_cubes_keep_their_layer_with_a_note` and the daily
  `game::pack_layer_frame_tests::layer_rules_move_states_between_the_packs_terrain_programs` and
  `game::pack_layer_frame_tests::layer_rules_move_states_between_the_packs_shadow_programs` (`sunrise-client`).

## D-6: a sampler whose class differs from its image's format

- **Open point:** spec rev 1, 5.2, line 786 samples an image through its `image.*` sampler as `sampler2D` for float
  and normalized formats and `isampler2D`/`usampler2D` for integer formats. It doesn't say what happens when the
  sampler's class differs from the image's format (`usampler2D` over an image whose format is `R32F`), and it is not
  one of section 9's open questions. wgpu makes a pipeline only when a sampled texture's class matches the
  shader's, so such a program failed at pipeline creation with wgpu's error (#5223).
- **Decision:** the program is refused by Sunrise's own check before its pipeline is made, with a note naming the
  program, the sampler's type and name, the image and the texture's format. This holds for every pair of classes
  that differ: an integer sampler over a float or normalized image, a float sampler over an integer image, and a
  signed sampler over an unsigned image or the reverse. The rest of the pack keeps loading.
- **Reason:** D-3 refuses an integer storage qualifier on a float texture for the same cause. The two other ways to
  keep the program were rejected: a second texture in the sampler's class, kept in step with the image, needs a copy
  after each writer and a rule for which copy a store reaches (Vulkan has no view between `R32_SFLOAT` and
  `R32_UINT`); a stand-in of the sampler's class would let the program run while it reads nothing the pack wrote,
  hiding the fault instead of naming it. The refusal skips no program that ran before: such a program never got a
  pipeline.
- **Tests:** `pack_post::tests::a_sampler_of_another_class_than_its_image_refuses_the_program`
  (`sunrise-render-shaderpack`).

## D-7: which feature flags get `IRIS_FEATURE_<flag>`, and which reads see it

- **Open point:** spec rev 1, 3.2, lines 372-373: `iris.features.optional` asks to "enable those supported and define
  `IRIS_FEATURE_<flag>` for each"; `iris.features.required` asks to refuse the pack when one is unsupported "and enable
  them", without naming a macro. 1.5.1, line 167 lists `IRIS_FEATURE_<flag>` ("an optional feature flag … is
  supported"). The spec doesn't say whether a required flag gets the macro, which case the macro's name takes when the
  pack writes the flag in another one, or which reads of the pack see it: `shaders.properties` holds the flag lists
  and may itself test the macros. Until 2026-11-26 OQ-2 in the [README](README.md) kept every `IRIS_FEATURE_*` undefined;
  it was rewritten on [#6054](https://github.com/sunrise-org/sunrise/issues/6054).
- **Decision:**
  - `IRIS_FEATURE_<NAME>` is defined, as `1`, for each flag Sunrise supports (`FEATURES` in
    `crates/sunrise-shaders/src/macros.rs`) that the pack lists in `iris.features.optional` or in
    `iris.features.required`. A flag matches in any case, and the macro takes the name as `FEATURES` writes it
    (`ssbo` listed gives `IRIS_FEATURE_SSBO`, not `IRIS_FEATURE_ssbo`). A flag Sunrise lacks, and a flag the pack
    doesn't list, stay undefined. A required flag Sunrise lacks keeps its note, and the pack still loads, as before.
  - *Added 2026-11-28 ([#6921](https://github.com/sunrise-org/sunrise/issues/6921)):* `PER_BUFFER_BLENDING` is
    supported only on a device that draws colour targets differing in blend (wgpu's
    `DownlevelFlags::INDEPENDENT_BLEND`; `Macros::set_per_buffer_blending`). There it is answered as the flags of
    `FEATURES` are: listed in either list, in any case, it gets `IRIS_FEATURE_PER_BUFFER_BLENDING`, in the same two
    reads below. On a device without it, and in a read made without the renderer's answer, the macro stays undefined
    and a required `PER_BUFFER_BLENDING` keeps its note. Before, the macro was never defined. A macro set told its
    device lacks it drops the answer at once, and each definition of the answers drops the ones the set held before.
    The Shader Options page, on the title screen as in game, reads with the renderer's macros where a renderer
    exists, and with the standard set alone before one is made. Once a renderer on another device is installed (a
    device recovery, any newly opened device), a pack refused before is tried again.
  - A supported flag Sunrise can't keep on every draw it may take loads without a note but gets no macro:
    `BLOCK_EMISSION_ATTRIBUTE` (`UNANSWERED` in `macros.rs`). Terrain drawn from its vertex attributes alone (when
    neither the arena's pages nor the normal stream fit the pack's vertex stage) has no source block, so
    `at_midBlock.w` is `0` there and a pack's emission branch would lose every block's light. Keeping the flag
    unanswered is the smaller correct choice: the vertex data on that path carries no emitted light to supply.
  - With `FADE_VARIABLE` answered, `mc_chunkFade` is given on every gbuffers draw, as spec 5.1 line 626 says: the
    section's fade on terrain, `-1.0` on every other draw (sky, celestial bodies, entities and block entities with
    their particles, translucent and shadow draws, clouds, weather, lines, the hand, mods' pipelines;
    `post::off_terrain_fade`). A program that declares the name keeps its own declaration.
  - `shaders.properties` is read twice, in one fixed order: first with the standard macros, the renderer's and the
    pack's options and no feature macro (any answer a reused macro set still holds is dropped), which settles the two
    lists; then the answers are defined and the file is read again, and that second read is the one the game uses. A list that only a feature macro reaches is not read.
    The programs and `block.properties`, `entity.properties` and `item.properties` see the answers. A pack that lists
    no supported flag is read once, with the macros it had before, but for one exception that holds for every pack:
    a line declaring an `IRIS_FEATURE_*` name (`#define IRIS_FEATURE_SSBO 1 // [0 1]`, `//#define
    IRIS_FEATURE_SSBO`) is no longer one of its options (below).
  - A mod's pack binding may not define an `IRIS_FEATURE_*` name: registration refuses it with a diagnostic, as it
    refuses `GL_` names (`PackBinding::validate_syntax`). Before this change such a define was accepted and reached
    the pack; with the read above dropping every `IRIS_FEATURE_*` before it settles the answers, it would be lost
    without a word, so registration names it instead. This is a compatibility change for mods
    (`crates/sunrise-mod-vocab/changes/shader-pack-iris-feature-defines-6054.md`).
  - What is decided by the lists after the read (the eight shadow colour buffers of `HIGHER_SHADOWCOLOR`) takes the
    lists of the first read, the ones the answers came from, even where the second read would hide or change them.
    They keep their place in the file's order among the other entries.
  - `IRIS_FEATURE_*` names are Sunrise's answers alone: a pack's line declaring such a name is not an option. It is
    not listed among the pack's options, not defined for its property files, doesn't count as on in
    `program.<name>.enabled`, and a saved choice never rewrites it (a commented-out `//#define` stays commented out).
  - The pack's own uncommented directives are read as written. In a program, a `#define` or `#undef` of such a name
    applies, as any directive does, to the rest of that stage's preprocessed source: the lines after it, the files
    included after it and the including file after their `#include` line. `shaders.properties` is read line by line,
    with no `#include`, by each of the two reads: there a `#define` or `#undef` applies from its line to the end of
    the file, to the conditionals (`#if`, `#ifdef`, `#elif`) and to the macros that read expands in the lines after
    it (custom `uniform.*` / `variable.*` lines, and `size.buffer.*` / `image.*` values). So a `#define` lets the
    first read reach a list behind it, and an `#undef` hides Sunrise's answer from those later lines, not from the
    programs nor from the id files. The values the load expands again with its own macros (`size.buffer.*`,
    `image.*`, `program.<name>.enabled`) are expanded with the answers, as with every other macro of the load, so an
    `#undef` earlier in the file doesn't hide the answer there.
  - A flag list set twice is noted as the lists were settled: the note that a later list "counts" comes from the first
    read for the two flag-list keys, and from the second read for every other key.
  - The same read serves every place that reads a pack's `shaders.properties`: the load, the HDR read made before
    the frame, and the Shader Options page.
  - No setting turns it off: it is the format's answer to what the pack asks.
- **Reason:** a required flag is one the pack needs, so it is at least as much a request as an optional one, and a
  pack that tests the macro after requiring the flag means the feature it required. The pack's spelling of a flag is
  matched in any case already (the notes), and the macro needs one spelling for every program to agree on. Reading the
  lists before any answer exists makes the answer independent of the answer itself, so a list guarded by a feature
  macro has one result.
- **Tests:** `options::tests::listed_features_sunrise_has_are_defined_for_properties_programs_and_id_files`,
  `options::tests::a_pack_that_lists_no_feature_sunrise_has_gets_none`,
  `options::tests::flag_lists_are_settled_before_the_answers_are_defined`,
  `options::tests::properties_are_read_with_the_options_then_define_the_features`,
  `options::tests::the_settled_lists_decide_the_shadow_colour_buffers`,
  `options::tests::a_packs_option_named_as_a_feature_answer_changes_no_answer`,
  `options::tests::a_saved_choice_never_rewrites_a_feature_answer`,
  `options::tests::a_packs_own_directive_in_shaders_properties_applies_from_its_line`,
  `options::tests::a_repeated_flag_list_is_noted_as_the_lists_were_settled`,
  `options::tests::a_repeat_the_second_read_cannot_see_is_still_noted`,
  `options::tests::the_settled_lists_keep_their_place_in_the_file`,
  `options::tests::an_undef_in_shaders_properties_hides_the_answer_from_conditionals_only`,
  `options::tests::a_reused_macro_set_carries_no_answer_to_the_next_pack`,
  `options::tests::block_emission_is_supported_but_not_answered`,
  `post::tests::non_terrain_programs_read_the_fade_as_minus_one`, `post::tests::sunrises_test_pack_loads`, and
  in `tests/it`
  `reversed_culling_feature::requested_reversed_culling_loads_and_packs_take_the_path_it_guards` and
  `terrain_fade::fade_feature_negotiation_ignores_case_and_defines_the_written_macro` (`sunrise-shaders`);
  `game::client_side::shader_packs::native_pack_tests::the_hdr_read_and_the_options_page_see_the_features_the_pack_lists`,
  `game::client_side::shader_packs::native_pack_tests::the_loads_id_files_see_the_features_the_pack_lists`; the refusal by
  `client::render_pipeline::tests::pack_binding_syntax_rejects_bad_identifiers_and_multiline_defines`
  (`sunrise-mod-vocab`)
  (`sunrise-client-gameplay`); `profiles_behind_answered_feature_flags_are_dumped` (`sunrise-render-shaderpack`, `spirv_dump`).

## D-8: which packs get `IS_IRIS` and `IRIS_VERSION`, and when

- **Open point:** spec rev 1, 1.5.1, line 168 lists `IS_IRIS`, `IRIS_VERSION`, `IRIS_TAG_SUPPORT`,
  `IRIS_HAS_TRANSLUCENCY_SORTING` and `IRIS_HAS_CONNECTED_TEXTURES` as macros that "identify a specific loader", and
  leaves whether to define them to a product decision (OQ-2, line 1122). The spec gives no value for `IRIS_VERSION`.
  Until 2026-11-26 OQ-2 in the [README](README.md) kept them undefined; it was rewritten on
  [#6054](https://github.com/sunrise-org/sunrise/issues/6054) (PR B), which reads the two as what a pack uses them
  for: choosing the path it wrote for Iris.
- **Decision** (2026-11-26, the shader lead on #6054):
  - Each pack has a Sunrise switch, Iris compatibility, with Auto (the default), On and Off, on Sunrise Settings ›
    Shaders under the picked pack's row. It is kept by the pack's name in `shaderpacks/` (the installed path), in
    `sunrise-graphics.txt` (`shaderPackIrisCompat:<on|off>:<name>`), apart from the pack's options and profiles: a
    pack replaced under the same name keeps it, and choosing Auto again forgets it.
  - While it is on, the pack gets `IS_IRIS` as `1` and `IRIS_VERSION` as `11103`, for its programs and its
    `shaders.properties` (`Macros::set_identity`). Off, and on Auto for a pack that doesn't ask, neither is defined.
    On gives them to any pack the player turns it on for. `IRIS_TAG_SUPPORT` and the `IRIS_HAS_*` names stay
    undefined.
  - **`IRIS_VERSION` is 11103**, the value #6054's observations were made with (its setup defined `IS_IRIS` and
    `IRIS_VERSION 11103` for the "identity" mode). It follows no vanilla behaviour (vanilla has no shader packs) and
    no other loader's: no loader's source or documentation was read for it, and the spec names the macro with no
    value. It promises only that a pack testing it takes its Iris path; whether that path's needs are met is shown
    per pack by the load's notes. The value changes only by a new dated decision.
  - A pack's line declaring `IS_IRIS` or `IRIS_VERSION` is no option, as a feature answer's isn't (D-7): it is not
    listed among the pack's options, not defined for its property files, and a saved choice never rewrites it, so no
    option or profile changes the answer, On's value or Off's absence. The pack's own uncommented directive keeps a
    directive's scope.
  - On Auto it is on for a pack that asks: evaluating a conditional directive looks up either name. That is the name
    `#ifdef` / `#ifndef` tests, a `defined` operand, or a name the `#if` / `#elif` condition's expansion leaves (a
    name a macro pastes together counts; an argument a macro drops, or the `FOO` of `defined FOO` whatever `FOO`
    holds, doesn't). It is looked for in the stage files the loader reads: those of its programs
    (`post::program_stages`, from the loader's own program lists, the programs mods request among them, each program's
    stages picked by the loader's own selection, `post::program_read` and `post::compute_paths`), found as
    `Pack::exists` finds them (a link to a file included), in the base folder and each dimension folder (those
    `dimension.properties` maps, or else `world0`, `world-1` and `world1`), their includes with them. The files are
    read as the load's preprocessor reads them, code lines included (so `__LINE__` and the expansion work spent are
    the same), without output, with the standard macros, the pack's saved options and its feature answers (D-7) and no
    identity macro, and the read stops at the first lookup. The preprocessor only evaluates the branches those
    settings reach, so the answer is fixed for those files and options. The renderer's macros are not among them, so
    the title screen, the HDR read before the frame and the load, which run with and without a renderer, get the same
    answer.
  - The scan runs before anything is translated, once for the pack's files (their fingerprint), its saved options
    and the programs mods request: its answer is kept and used again until one changes; a pack whose fingerprint can't
    be taken is scanned at every read. It runs on a thread of its own, never on the frame: the load and the HDR read
    before the frame go on without it until it has landed (the game draws without the pack meanwhile, and the HDR
    request is read without the identity macros and read again once it has); the Shader Options page waits for it. It
    is asked at the load (in game), when the pick's options are read (picking a pack or saving its options, in game
    and on the title screen), when the pack list is read again, and by the HDR read, and the same answer feeds all of
    them.
  - A load (running or still building) made with another switch than the pick's, or with another answer than the
    pick's as last read (a pack replaced under its name), is dropped before the poll that would install it, and the
    pack loads again: a build started before the player turned it off is never installed. The loader's refusals are
    kept per switch: the same files refused with one switch are tried again with another.
  - Where it applies, the pack's row says it runs with Iris compatibility, and the load logs a line saying so and why
    (the switch, or the file and line where the pack asks). No toast.
  - A mod's pack binding may not define `IS_IRIS` or `IRIS_VERSION`: registration refuses it, as it refuses `GL_` and
    `IRIS_FEATURE_*` names (`crates/sunrise-mod-vocab/changes/shader-pack-iris-identity-defines-6054.md`).
- **Reason:** a pack that tests the names asks which path to take; one that never tests them gets nothing, so its
  output can't change. The player decides last, per pack, where the answer would be wrong for that pack.
- **Tests:** `identity::tests::*` and `post::tests::program_stages_are_the_stages_the_loader_reads` (`sunrise-shaders`:
  what asks, which files are read, options and features, the answer reaching `shaders.properties` and the programs),
  `condition::tests::the_names_a_condition_looks_up_are_noted` (`sunrise-glsl`);
  `game::client_side::shader_packs::native_pack_tests::iris_compatibility_follows_the_switch_for_the_options_page_and_the_hdr_read`,
  `…::a_load_from_other_inputs_than_the_picks_is_dropped`,
  `…::a_pack_replaced_under_its_name_with_another_answer_loads_again`,
  `…::a_pack_without_a_fingerprint_is_scanned_every_time`, `…::the_frame_never_runs_a_cold_identity_scan`,
  `…::a_refusal_under_one_switch_is_tried_again_under_another` and
  `native_shaders::tests::outcomes_and_refusals_survive_renderer_resets` (`sunrise-client-gameplay`);
  `graphics::tests::iris_compatibility_is_kept_by_pack_name`,
  `shader_ui::tests::the_picked_pack_has_an_iris_compatibility_switch_beside_its_row` and the daily
  `pack_terrain_ids::a_stale_pack_build_is_never_installed` (`sunrise-client-render`);
  `client::render_pipeline::tests::pack_binding_syntax_rejects_bad_identifiers_and_multiline_defines`
  (`sunrise-mod-vocab`).

## D-9: which tags `%<tag>` id entries read, and when

- **Open point:** spec rev 1, 4.1, line 579 says E `%<tag>` "selects every block of a block tag" and that "individual
  entries win over tags when both match"; lines 570-572 give `block.properties`, `item.properties` and
  `entity.properties` one line form. The spec doesn't say whether `%<tag>` is read in `item.properties` and
  `entity.properties`, which binding of the tags an entry reads (the game's own data, a server's, a reload's), when
  a change to them reaches the ids, or which entry wins when several tags list one name. None of these is one of
  section 9's open questions. Whether to define `IRIS_TAG_SUPPORT` is OQ-2 (line 168), not decided here: it stays
  undefined.
- **Decision:**
  - The word form is the same in the three files: `%<tag>` names a block tag in `block.properties` (with state
    filters, as a block name takes them), an item tag in `item.properties` and an entity type tag in
    `entity.properties`, `[namespace:]` defaulting to `minecraft`.
  - An entry reads the tags of the world being played: connected to a server (a single-player world's own server
    included), the tags it sent in its tags packets (`remote.registries.tags`); otherwise the world's tables. When
    they are bound again (a server's tags packet, which `/reload` sends, another world), the ids are resolved again
    before anything reads them, the terrain's included.
  - A name's own entry wins over a tag's; then the entry with the most state filters that all match; then the first
    in the file. A tag the world lacks, an empty tag and an empty `%` give no name an id, without a note.
  - A file whose entries name no tag is read as before: its ids never follow the tags.
- **Reason:** the three files share one line form and one parser, so one word reads one way in each. The tags are the
  server's current ones, so a pack's ids follow a `/reload` in single player too. The precedence is D-5's, the one
  4.1 states.
- **Tests:** `block_ids::tests::tag_words_give_their_members_ids` and `block_ids::tests::plain_names_take_tags`
  (`sunrise-shaders`); `game::client_side::shader_packs::terrain_source_tests::terrain_ids_reach_a_tags_members` and
  `game::client_side::shader_packs::terrain_source_tests::pack_ids_follow_the_worlds_tags`
  (`sunrise-client-gameplay`); the daily
  `cut_layout::game::pack_tag_id_frame_tests::a_tags_id_reaches_the_terrain_and_follows_the_tags`,
  `cut_layout::game::pack_tag_id_frame_tests::a_tags_id_reaches_the_terrain_through_the_normal_stream` and
  `cut_layout::game::client_side::remote_tags::tests::a_shader_packs_tag_ids_follow_the_servers_tags`
  (`sunrise-client`).

## D-10: what `indirect.<pass>` names, and what a key or counts that can't run do

- **Open point:** spec rev 1, 3.2, line 398 gives `indirect.<pass>` = `<ssbo index> <byte offset>`, "read the compute
  dispatch size (a `uvec3`) from an SSBO", and line 104 points there from the work-group counts of 1.3. It doesn't say
  which compute shader `<pass>` names when a program has several (`<program>.csh`, `<program>_a.csh` ..., line 98), and
  says nothing of a value that is malformed, names a buffer the pack doesn't declare (6.5, line 973) or an offset the
  counts don't fit at, nor of counts the device can't dispatch. It is not one of section 9's open questions.
- **Decision:** `<pass>` is one compute shader's file name without `.csh`, in the folder being loaded: `composite`
  names `composite.csh` only, `composite_a` names `composite_a.csh`. Its counts are the three 4-byte words at the
  offset, read as the shader is about to run, after every earlier pass of the frame; its `workGroups` and
  `workGroupsRender` are not read or checked. A value that is not two unsigned 32-bit integers, a buffer the pack doesn't
  declare, an offset that is not a multiple of 4, or one where the three words end past a fixed-size buffer
  (`<bytes>` or `<bytes> <file>`) skips that compute shader, with a note naming the key; the rest of the program
  and pack loads. A buffer sized by the render size is checked as the shader runs: counts that end past it, or with
  one over the device's `max_compute_workgroups_per_dimension`, run no work group. Zero in any count runs none. A key
  naming no compute shader of the folder does nothing.
- **Reason:** naming each shader by its own name lets every compute shader of a program have its own counts, and a
  key for the unsuffixed name reads the same under either reading of line 98. The direct path already does not run a
  `workGroupsRender` dispatch over the device's limit; the indirect one matches it, and the check happens on the GPU
  because the counts only exist there. A shader skipped for a key it can't honour runs nothing, rather than a
  per-pixel default dispatch the pack never asked for. Multiples of 4: the counts are `uint`s, and wgpu takes an
  indirect dispatch only at a 4-byte aligned offset. A key naming no shader gets no note because a folder is loaded
  on its own: the key may name another dimension folder's shader.
- **Tests:** `post::indirect_tests` (`sunrise-shaders`) and the daily
  `gfx::pack_post::indirect_dispatch_tests` (`sunrise-client-render`).

## D-12: which boss event `bossBattle` gives

- **Open point:** spec rev 1, 5.2, line 699 gives `bossBattle` as an `int`, "1 custom, 2 ender dragon, 3 wither,
  4 raid", updated every frame. It doesn't say what it reads with no boss event, which event counts when several
  bars are shown, or how the game tells the kinds apart. It is not one of section 9's open questions. A client knows
  only the boss bars the server sends (`ClientboundBossEventPacket`): a name, a progress, a colour, an overlay and
  the darken-screen, boss-music and world-fog flags. No field names the kind.
- **Decision:** 0 while the client shows no boss bar. Otherwise each bar is matched against the colour, overlay and
  flags vanilla 26.3 creates each event with: `EnderDragonFight`'s `dragonEvent` (pink, progress, boss music and
  world fog) is 2, `WitherBoss`'s `bossEvent` (purple, progress, darken screen) is 3, and `Raid`'s `raidEvent` (red,
  10 notches, no flags) is 4 when its name is also one of the titles `Raid` gives it: `event.minecraft.raid` alone,
  the same followed by `" - "` and `event.minecraft.raid.raiders_remaining` with one argument (one or two raiders
  left), `event.minecraft.raid.victory.full` or `event.minecraft.raid.defeat.full`, each a translatable component
  with nothing else set. Any other bar is 1 (custom). All five fields must match; the dragon's and wither's names
  are not read. The value is the kind of the first bar, in the order the bars show (the order their add packets
  arrived), that is not custom, else 1 when only custom bars are shown. It is read from the client's bars each
  frame, so a removed bar or a cleared overlay (leaving the world or the server) counts from the next frame. Bars
  the screen has no room for, or that the HUD doesn't draw (F1), still count.
- **Reason:** the bars are the only boss state a client has, and vanilla never changes those five fields after it
  creates the three events (only their name, progress and visibility), so the match holds for the whole event. The
  dragon's and wither's names are left out because either takes a custom name, and `/bossbar` can't set the
  darken-screen, boss-music or world-fog flags (`BossBarCommands` has setters for name, colour, style, value, max,
  visibility and players only), so those two signatures need no name. A raid's flags are all off,
  as a new `/bossbar`'s are, and `/bossbar set ... color red` and `style notched_10` give its colour and overlay, so
  its name is checked too; `Raid` sets no other name outside its debug mode (`SharedConstants.DEBUG_RAIDS`). A custom bar that copies a signature still reads as that
  kind: a mod's bar, a saved custom bar's data or a server's own boss event can set any flags, and a server can send a bar with the raid's
  exact title (a `/bossbar` named `{"translate":"event.minecraft.raid"}`), so on a remote server such a bar stays
  ambiguous; nothing a client receives tells them apart. A custom bar is skipped while a boss bar is shown, so a
  server's standing custom bar does not hide a fight from the pack; between two boss bars the first shown wins, as
  it is the top one on screen.
- **Tests:** `boss_overlay::tests::boss_battle_*` (`sunrise-client-lib`: each kind, none, the fields one by one,
  names, the raid's titles and ordinary red notched bars, ordinals past the enum, several bars, removal, style and
  flag updates), `pack_uniforms::boss_battle_test::*`
  (`sunrise-render-shaderpack`: the value and its type, in programs and in custom uniforms) and the daily
  `game::client_side::boss_battle_tests::boss_battle_follows_the_servers_boss_bars` (`sunrise-client-gameplay`: packets to the frame's
  inputs).
