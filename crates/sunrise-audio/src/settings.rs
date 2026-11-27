//! The sound sources an event plays on and the volume of each category
//! (`SoundSource`): the audio side of the player's settings. The options screen
//! and `options.txt` stay in `sunrise-client-render`, which re-exports these
//! under its old `settings` paths.
pub use sunrise_client_core::sound_source::SOUND_SOURCES;

/// Per-source volumes for the mixer (soundCategory_* except master).
static SOURCE_VOLUMES: std::sync::RwLock<[f32; 11]> = std::sync::RwLock::new([1.0; 11]);
pub fn set_source_volumes(volumes: [f32; 11]) {
    *SOURCE_VOLUMES.write().unwrap_or_else(|e| e.into_inner()) = volumes;
}
/// The SoundSource an event plays on. Vanilla callers pick the source; this
/// follows their conventions by event family.
pub fn sound_source(event: &str) -> usize {
    let e = event.trim_start_matches("minecraft:");
    let index = |name: &str| SOUND_SOURCES.iter().position(|n| *n == name).unwrap_or(0);
    const HOSTILE: [&str; 34] = [
        "zombie",
        "skeleton",
        "creeper",
        "spider",
        "cave_spider",
        "enderman",
        "witch",
        "slime",
        "magma_cube",
        "blaze",
        "ghast",
        "drowned",
        "husk",
        "stray",
        "phantom",
        "pillager",
        "vindicator",
        "evoker",
        "illusioner",
        "ravager",
        "guardian",
        "elder_guardian",
        "silverfish",
        "zombified_piglin",
        "wither_skeleton",
        "bogged",
        "breeze",
        "endermite",
        "parched",
        "warden",
        "zombie_villager",
        "vex",
        "wither",
        // Monster's common hurt/death (e.g. Giant) and other shared sounds.
        "hostile",
    ];
    if e == "block.potent_sulfur.noxious_gas" {
        // PotentSulfurBlock.animateTick, its only caller, plays it as AMBIENT.
        index("ambient")
    } else if e.starts_with("music_disc.") {
        index("record")
    } else if e.starts_with("item.goat_horn.") {
        // InstrumentItem.play: SoundSource.RECORDS.
        index("record")
    } else if e.starts_with("music.") {
        index("music")
    } else if e.starts_with("weather.") || e.starts_with("entity.lightning_bolt") {
        index("weather")
    } else if e.starts_with("block.") {
        index("block")
    } else if e.starts_with("ambient.") {
        index("ambient")
    } else if e.starts_with("ui.") {
        index("ui")
    } else if e.starts_with("entity.player.")
        || e.starts_with("item.")
        || e.starts_with("entity.item.")
        || e.starts_with("entity.experience_orb")
        || e.starts_with("entity.arrow")
    {
        index("player")
    } else if let Some(rest) = e.strip_prefix("entity.") {
        let mob = rest.split('.').next().unwrap_or("");
        if HOSTILE.contains(&mob) {
            index("hostile")
        } else {
            index("neutral")
        }
    } else {
        index("master")
    }
}
/// Volume multiplier of a sound source by index.
pub fn source_volume(i: usize) -> f32 {
    if i == 0 {
        1.0
    } else {
        SOURCE_VOLUMES.read().unwrap_or_else(|e| e.into_inner())[i]
    }
}
