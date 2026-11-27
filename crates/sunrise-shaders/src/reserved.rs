//! Built-in shader-pack identifiers reserved by Sunrise's pack translator and uniform table.
//!
//! Having one table here lets registration validation (`sunrise-client-render`), uniform upload
//! (`sunrise-render-shaderpack`), and translation (`sunrise-shaders`) share one authority.

/// What programs that draw the game's geometry (gbuffers, shadow) read as `fogMode`,
/// `fogShape` and `centerDepthSmooth`: no GLSL name can hold a dot, so no pack reads these
/// directly.
pub const GBUFFERS_FOG_MODE: &str = "gbuffers.fogMode";
pub const GBUFFERS_FOG_SHAPE: &str = "gbuffers.fogShape";
pub const GBUFFERS_CENTER_DEPTH: &str = "gbuffers.centerDepthSmooth";

/// Built-in per-frame shader-pack uniform names, in the order of
/// `sunrise_render_shaderpack::pack_uniforms::FrameUniforms::values`.
pub const PACK_UNIFORM_NAMES: &[&str] = &[
    "cameraPosition",
    "previousCameraPosition",
    "eyeAltitude",
    "upPosition",
    "sunPosition",
    "moonPosition",
    "shadowLightPosition",
    "sunAngle",
    "shadowAngle",
    "worldTime",
    "worldDay",
    "moonPhase",
    "frameCounter",
    "frameTime",
    "frameTimeCounter",
    "viewWidth",
    "viewHeight",
    "aspectRatio",
    "near",
    "far",
    "rainStrength",
    "wetness",
    "thunderStrength",
    "lightningBoltPosition",
    "fogColor",
    "skyColor",
    "fogMode",
    "fogStart",
    "fogEnd",
    "fogDensity",
    "fogShape",
    "isEyeInWater",
    "blindness",
    "nightVision",
    "darknessFactor",
    "darknessLightFactor",
    "screenBrightness",
    "hideGUI",
    "eyeBrightness",
    "eyeBrightnessSmooth",
    "heldItemId",
    "heldItemId2",
    "heldBlockLightValue",
    "heldBlockLightValue2",
    "renderStage",
    "biome",
    "biome_category",
    "biome_precipitation",
    "temperature",
    "rainfall",
    "gbufferModelView",
    "gbufferModelViewInverse",
    "gbufferProjection",
    "gbufferProjectionInverse",
    "gbufferPreviousModelView",
    "gbufferPreviousProjection",
    "atlasSize",
    "shadowModelView",
    "shadowModelViewInverse",
    "shadowProjection",
    "shadowProjectionInverse",
    "cameraPositionInt",
    "cameraPositionFract",
    "previousCameraPositionInt",
    "previousCameraPositionFract",
    "eyePosition",
    "relativeEyePosition",
    "playerLookVector",
    "playerBodyVector",
    "firstPersonCamera",
    "currentPlayerHealth",
    "maxPlayerHealth",
    "currentPlayerHunger",
    "maxPlayerHunger",
    "currentPlayerAir",
    "maxPlayerAir",
    "currentPlayerArmor",
    "maxPlayerArmor",
    "isSpectator",
    "isRiding",
    "feetInWater",
    "inSwimmingAnimation",
    "is_burning",
    "is_hurt",
    "is_invisible",
    "is_sneaking",
    "is_sprinting",
    "is_alive",
    "is_child",
    "is_glowing",
    "is_in_lava",
    "is_in_water",
    "is_ridden",
    "is_riding",
    "is_wet",
    "endFlashIntensity",
    "endFlashPosition",
    "ambientLight",
    "playerMood",
    "constantMood",
    "centerDepthSmooth",
    "alphaTestRef",
    "entityId",
    "blockEntityId",
    "entityColor",
    GBUFFERS_FOG_MODE,
    GBUFFERS_FOG_SHAPE,
    GBUFFERS_CENTER_DEPTH,
    "isRightHanded",
    "is_on_ground",
    "bedrockLevel",
    "heightLimit",
    "logicalHeightLimit",
    "hasSkylight",
    "hasCeiling",
    "cloudTime",
    "bossBattle",
    // Sunrise's own: Sunrise Extras' dynamic lights (`dynamic_light.rs`), nearest first: how
    // many, and each one's camera-relative position and light level (zeros past the count).
    "sunriseDynamicLightCount",
    "sunriseDynamicLight0",
    "sunriseDynamicLight1",
    "sunriseDynamicLight2",
    "sunriseDynamicLight3",
    "sunriseDynamicLight4",
    "sunriseDynamicLight5",
    "sunriseDynamicLight6",
    "sunriseDynamicLight7",
    "sunriseDynamicLight8",
    "sunriseDynamicLight9",
    "sunriseDynamicLight10",
    "sunriseDynamicLight11",
    "sunriseDynamicLight12",
    "sunriseDynamicLight13",
    "sunriseDynamicLight14",
    "sunriseDynamicLight15",
    // Sunrise's own: the boats' water patches, for the shadow pass's water (`water_patches`):
    // how many, then each one's corner and y range, and its two horizontal edges.
    "sunriseWaterPatchCount",
    "sunriseWaterPatch0",
    "sunriseWaterPatch1",
    "sunriseWaterPatch2",
    "sunriseWaterPatch3",
    "sunriseWaterPatch4",
    "sunriseWaterPatch5",
    "sunriseWaterPatch6",
    "sunriseWaterPatch7",
    "sunriseWaterPatch8",
    "sunriseWaterPatch9",
    "sunriseWaterPatch10",
    "sunriseWaterPatch11",
    "sunriseWaterPatch12",
    "sunriseWaterPatch13",
    "sunriseWaterPatch14",
    "sunriseWaterPatch15",
    "sunriseWaterPatch16",
    "sunriseWaterPatch17",
    "sunriseWaterPatch18",
    "sunriseWaterPatch19",
    "sunriseWaterPatch20",
    "sunriseWaterPatch21",
    "sunriseWaterPatch22",
    "sunriseWaterPatch23",
    "sunriseWaterPatch24",
    "sunriseWaterPatch25",
    "sunriseWaterPatch26",
    "sunriseWaterPatch27",
    "sunriseWaterPatch28",
    "sunriseWaterPatch29",
    "sunriseWaterPatch30",
    "sunriseWaterPatch31",
    "isElytraFlying",
    "cloudHeight",
];

/// Built-in Iris/OptiFine pack GLSL sampler, image, and uniform names that mod `PackBinding`
/// declarations may not claim. Samplers, images, and value uniforms share GLSL's `uniform`
/// identifier namespace, so both `PackDepthTarget` samplers and `PackUniformDecl` names are
/// checked against this full table.
pub fn reserved_pack_identifier(name: &str) -> Option<&'static str> {
    let is_color_img = name
        .strip_prefix("colorimg")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < crate::BUFFERS);
    let is_shadow_img = name
        .strip_prefix("shadowcolorimg")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < crate::SHADOW_COLORS);
    if crate::buffer_index(name).is_some()
        || crate::shadow_color(name).is_some()
        || is_color_img
        || is_shadow_img
        || matches!(
            name,
            "depthtex0"
                | "depthtex1"
                | "depthtex2"
                | "gdepthtex"
                | "shadowtex0"
                | "shadowtex1"
                | "shadowtex0HW"
                | "shadowtex1HW"
                | "watershadow"
                | "shadow"
                | "noisetex"
                | "texture"
                | "gtexture"
                | "tex"
                | "lightmap"
                | "normals"
                | "specular"
                | crate::WATER_MASK
        )
    {
        return Some("built-in shader-pack sampler or image");
    }
    if PACK_UNIFORM_NAMES.contains(&name) {
        return Some("built-in shader-pack uniform");
    }
    None
}

/// The type of a game input a vertex stage may read without declaring it: terrain's
/// `mc_chunkFade`, and the core-profile vertex attributes (`vaPosition`...) that a
/// `#version 130` line program reads as it would in a core-profile one (Complementary
/// Reimagined r5.9.3's `gbuffers_line`).
pub fn implicit_input_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "mc_chunkFade" => "float",
        "vaPosition" | "vaNormal" => "vec3",
        "vaColor" => "vec4",
        "vaUV0" => "vec2",
        "vaUV1" | "vaUV2" => "ivec2",
        _ => return None,
    })
}
