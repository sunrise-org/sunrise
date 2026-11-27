//! `BossHealthOverlay` and `LerpingBossEvent`: the boss bars a client is
//! showing, in the order their add packets arrived.
use crate::{
    hud_overlays::BossBar,
    scoreboard::{Comp, comp_text, tag_to_json},
    text::Text,
};
use sunrise_protocol::packets::play::{BossAction, BossEvent};

/// `LerpingBossEvent.LERP_MILLISECONDS`.
const LERP_MS: u128 = 100;

/// `Util.getMillis`.
pub fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// One `LerpingBossEvent`.
#[derive(Clone, Debug)]
struct Event {
    id: u128,
    name: Text,
    /// The progress the lerp started from (`progress`).
    progress: f32,
    /// `targetPercent`.
    target: f32,
    /// `setTime`.
    set_at: u128,
    color: u8,
    overlay: u8,
    darken: bool,
    music: bool,
    fog: bool,
    /// The name is one of the titles vanilla's `Raid` gives its bar ([`raid_title`]).
    raid_title: bool,
}
/// `bossBattle`'s values (spec 5.2): no bar, a custom one, the ender dragon, a wither, a raid.
const BATTLE_NONE: i32 = 0;
const BATTLE_CUSTOM: i32 = 1;
const BATTLE_DRAGON: i32 = 2;
const BATTLE_WITHER: i32 = 3;
const BATTLE_RAID: i32 = 4;

impl Event {
    /// The kind `bossBattle` gives this bar. The packet names no kind, so the bar is matched
    /// against the colour, overlay and flags vanilla 26.3 makes each event with and never
    /// changes: `EnderDragonFight`'s `dragonEvent` (pink, progress, boss music and world fog),
    /// `WitherBoss`'s `bossEvent` (purple, progress, darken screen) and `Raid`'s `raidEvent`
    /// (red, 10 notches, no flags), whose name must also be one of the raid's own titles: a
    /// `/bossbar` gets red and 10 notches as easily. The dragon's and wither's names are not
    /// read: either takes a custom name. Any other bar is custom (D-12).
    fn battle_kind(&self) -> i32 {
        match (self.color, self.overlay, self.darken, self.music, self.fog) {
            (0, 0, false, true, true) => BATTLE_DRAGON,
            (5, 0, true, false, false) => BATTLE_WITHER,
            (2, 2, false, false, false) if self.raid_title => BATTLE_RAID,
            _ => BATTLE_CUSTOM,
        }
    }
    /// `LerpingBossEvent.getProgress`.
    fn displayed(&self, now: u128) -> f32 {
        let t = ((now.saturating_sub(self.set_at)) as f32 / LERP_MS as f32).clamp(0.0, 1.0);
        self.progress + (self.target - self.progress) * t
    }
    /// `LerpingBossEvent.setProgress`: the bar starts the next lerp from
    /// whatever it is showing now.
    fn set_progress(&mut self, progress: f32, now: u128) {
        self.progress = self.displayed(now);
        self.target = progress;
        self.set_at = now;
    }
}

/// Whether a bar's name is one of the titles vanilla 26.3's `Raid` gives its bar, as the packet
/// carries it: `RAID_NAME_COMPONENT` (`translatable("event.minecraft.raid")`) alone, or with
/// `" - "` and `translatable("event.minecraft.raid.raiders_remaining", n)` appended (one or two
/// raiders left, `Raid.tick`), or `RAID_BAR_VICTORY_COMPONENT` / `RAID_BAR_DEFEAT_COMPONENT`
/// (`event.minecraft.raid.victory.full`, `event.minecraft.raid.defeat.full`). A component may
/// name its own kind (`"type"`), and a plain text may come as a bare string; nothing else may
/// be set (no style, no other parts).
fn raid_title(name: &Comp) -> bool {
    let Some(parts) = name.as_object() else {
        return false;
    };
    let kind_is = |part: &serde_json::Map<String, Comp>, kind: &str| {
        part.get("type").is_none_or(|t| t.as_str() == Some(kind))
    };
    let only = |part: &serde_json::Map<String, Comp>, keys: &[&str]| {
        part.keys()
            .all(|k| k == "type" || keys.contains(&k.as_str()))
    };
    let key = parts.get("translate").and_then(Comp::as_str);
    if !kind_is(parts, "translatable") {
        return false;
    }
    match key {
        Some("event.minecraft.raid.victory.full" | "event.minecraft.raid.defeat.full") => {
            only(parts, &["translate"])
        }
        Some("event.minecraft.raid") if only(parts, &["translate"]) => true,
        Some("event.minecraft.raid") if only(parts, &["translate", "extra"]) => {
            let Some([dash, remaining]) = parts
                .get("extra")
                .and_then(Comp::as_array)
                .map(Vec::as_slice)
            else {
                return false;
            };
            let dash = match dash {
                Comp::String(s) => s == " - ",
                Comp::Object(o) => {
                    kind_is(o, "text") && only(o, &["text"]) && o.get("text") == Some(&" - ".into())
                }
                _ => false,
            };
            let remaining = remaining.as_object().is_some_and(|o| {
                kind_is(o, "translatable")
                    && only(o, &["translate", "with"])
                    && o.get("translate").and_then(Comp::as_str)
                        == Some("event.minecraft.raid.raiders_remaining")
                    && o.get("with")
                        .and_then(Comp::as_array)
                        .is_some_and(|args| args.len() == 1)
            });
            dash && remaining
        }
        _ => false,
    }
}

/// Snapshot of a displayed bar before conversion to the mod API.
pub struct ModBar {
    pub uuid: u128,
    pub name: Text,
    pub progress: f32,
    pub color: mod_api::BossBarColor,
    pub overlay: mod_api::BossBarOverlay,
    pub darken_screen: bool,
    pub play_boss_music: bool,
    pub create_world_fog: bool,
}

/// `BossHealthOverlay`: the events, plus `GameRenderer`'s world darkening.
#[derive(Clone, Debug, Default)]
pub struct BossOverlay {
    events: Vec<Event>,
    /// `bossOverlayWorldDarkening` and the previous tick's value.
    darken: f32,
    darken_o: f32,
}
impl BossOverlay {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// `ClientboundBossEventPacket.dispatch` into `BossHealthOverlay`'s
    /// handler. An id already shown keeps its place when added again.
    pub fn apply(&mut self, packet: &BossEvent, now: u128) {
        // An ordinal past the enum shows as its last value.
        let color = |c: i32| c.clamp(0, 6) as u8;
        let overlay = |o: i32| o.clamp(0, 4) as u8;
        let id = packet.id;
        if let BossAction::Add {
            name,
            progress,
            color: c,
            overlay: o,
            darken,
            music,
            fog,
        } = &packet.action
        {
            let name = tag_to_json(name);
            let event = Event {
                id,
                raid_title: raid_title(&name),
                name: comp_text(&name),
                progress: *progress,
                target: *progress,
                set_at: now,
                color: color(*c),
                overlay: overlay(*o),
                darken: *darken,
                music: *music,
                fog: *fog,
            };
            match self.events.iter_mut().find(|e| e.id == id) {
                Some(slot) => *slot = event,
                None => self.events.push(event),
            }
            return;
        }
        if let BossAction::Remove = packet.action {
            self.events.retain(|e| e.id != id);
            return;
        }
        let Some(e) = self.events.iter_mut().find(|e| e.id == id) else {
            return;
        };
        match &packet.action {
            BossAction::UpdateProgress { progress } => e.set_progress(*progress, now),
            BossAction::UpdateName { name } => {
                let name = tag_to_json(name);
                e.raid_title = raid_title(&name);
                e.name = comp_text(&name);
            }
            BossAction::UpdateStyle {
                color: c,
                overlay: o,
            } => {
                e.color = color(*c);
                e.overlay = overlay(*o);
            }
            BossAction::UpdateProperties { darken, music, fog } => {
                (e.darken, e.music, e.fog) = (*darken, *music, *fog);
            }
            BossAction::Add { .. } | BossAction::Remove => {}
        }
    }
    /// `GameRenderer.tick` while the level runs.
    pub fn tick_darken(&mut self, running: bool) {
        if !running {
            return;
        }
        self.darken_o = self.darken;
        if self.should_darken() {
            self.darken = (self.darken + 0.05).min(1.0);
        } else if self.darken > 0.0 {
            self.darken -= 0.0125;
        }
    }
    /// `GameRenderer.bossOverlayWorldDarkening`.
    pub fn darken_amount(&self, partial: f32) -> f32 {
        let t = partial.clamp(0.0, 1.0);
        self.darken_o + (self.darken - self.darken_o) * t
    }
    pub fn should_darken(&self) -> bool {
        self.events.iter().any(|e| e.darken)
    }
    pub fn should_play_music(&self) -> bool {
        self.events.iter().any(|e| e.music)
    }
    pub fn should_create_world_fog(&self) -> bool {
        self.events.iter().any(|e| e.fog)
    }
    /// The `bossBattle` shader-pack uniform (spec 5.2): 0 with no bar, else the kind of the
    /// first bar, in the order they show, that is not a custom one, else 1 (custom). Decision
    /// D-12 (`docs/cleanroom/decisions.md`).
    pub fn boss_battle(&self) -> i32 {
        if self.events.is_empty() {
            return BATTLE_NONE;
        }
        self.events
            .iter()
            .map(Event::battle_kind)
            .find(|&k| k != BATTLE_CUSTOM)
            .unwrap_or(BATTLE_CUSTOM)
    }
    /// The bars `extractRenderState` walks, with the lerped progress.
    pub fn bars(&self, now: u128) -> Vec<BossBar> {
        self.events
            .iter()
            .map(|e| BossBar {
                name: e.name.clone(),
                progress: e.displayed(now),
                color: e.color as usize,
                overlay: e.overlay as usize,
            })
            .collect()
    }
    /// The bar at `index` of `bars(now)` as `customize_boss_event_progress`
    /// hands it to mods.
    pub fn mod_bar(&self, index: usize, now: u128) -> Option<ModBar> {
        let e = self.events.get(index)?;
        Some(ModBar {
            uuid: e.id,
            name: e.name.clone(),
            progress: e.displayed(now),
            color: mod_api::BossBarColor::ALL[e.color as usize],
            overlay: mod_api::BossBarOverlay::ALL[e.overlay as usize],
            darken_screen: e.darken,
            play_boss_music: e.music,
            create_world_fog: e.fog,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;

    fn add(id: u128, name: &str, progress: f32, darken: bool, fog: bool) -> BossEvent {
        BossEvent {
            id,
            action: BossAction::Add {
                name: Tag::String(name.into()),
                progress,
                color: 5,
                overlay: 0,
                darken,
                music: false,
                fog,
            },
        }
    }
    fn progress(id: u128, progress: f32) -> BossEvent {
        BossEvent {
            id,
            action: BossAction::UpdateProgress { progress },
        }
    }

    #[test]
    fn progress_lerps_over_a_tenth_of_a_second() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&add(1, "W", 0.0, true, false), 0);
        overlay.apply(&progress(1, 1.0), 0);
        assert!((overlay.bars(50)[0].progress - 0.5).abs() < 1.0e-5);
        assert!((overlay.bars(100)[0].progress - 1.0).abs() < 1.0e-5);
        // A new target starts from the value on screen.
        overlay.apply(&progress(1, 0.0), 50);
        assert!((overlay.bars(150)[0].progress - 0.0).abs() < 1.0e-5);
    }

    #[test]
    fn darken_ramps_and_fades() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&add(1, "W", 1.0, true, false), 0);
        overlay.tick_darken(true);
        assert!((overlay.darken_amount(1.0) - 0.05).abs() < 1.0e-5);
        overlay.apply(
            &BossEvent {
                id: 1,
                action: BossAction::Remove,
            },
            0,
        );
        overlay.tick_darken(true);
        assert!((overlay.darken - 0.0375).abs() < 1.0e-5);
        assert!(!overlay.should_play_music());
    }

    #[test]
    fn bars_keep_arrival_order() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&add(2, "b", 1.0, false, true), 0);
        overlay.apply(&add(1, "a", 1.0, false, false), 0);
        overlay.apply(&add(2, "c", 0.5, false, true), 0);
        // An update for a bar this client never got is ignored.
        overlay.apply(&progress(9, 0.5), 0);
        let names: Vec<_> = overlay
            .bars(0)
            .into_iter()
            .map(|b| b.name.plain())
            .collect();
        assert_eq!(names, ["c", "a"]);
        assert!(overlay.should_create_world_fog());
        overlay.apply(
            &BossEvent {
                id: 2,
                action: BossAction::Remove,
            },
            0,
        );
        assert!(!overlay.should_create_world_fog());
    }

    /// An add packet with vanilla's wire fields: colour and overlay ordinals, then the darken,
    /// music and fog flags.
    fn shown(id: u128, name: Tag, color: i32, overlay: i32, flags: [bool; 3]) -> BossEvent {
        let [darken, music, fog] = flags;
        BossEvent {
            id,
            action: BossAction::Add {
                name,
                progress: 1.0,
                color,
                overlay,
                darken,
                music,
                fog,
            },
        }
    }
    /// A plain text name, as the packet carries `Component.literal`.
    fn lit(text: &str) -> Tag {
        Tag::String(text.into())
    }
    /// `Component.translatable(key)`.
    fn key(key: &str) -> Tag {
        Tag::compound([("translate", Tag::String(key.into()))])
    }
    /// `Raid.RAID_NAME_COMPONENT`.
    fn title() -> Tag {
        key("event.minecraft.raid")
    }
    /// `RAID_NAME_COMPONENT.copy().append(" - ").append(translatable(RAIDERS_REMAINING, n))`,
    /// with `dash` as the second part and `args` as the remaining line's arguments.
    fn remaining(dash: Tag, args: Vec<Tag>) -> Tag {
        Tag::compound([
            ("translate", Tag::String("event.minecraft.raid".into())),
            (
                "extra",
                Tag::List(vec![
                    dash,
                    Tag::compound([
                        (
                            "translate",
                            Tag::String("event.minecraft.raid.raiders_remaining".into()),
                        ),
                        ("with", Tag::List(args)),
                    ]),
                ]),
            ),
        ])
    }
    fn remove(id: u128) -> BossEvent {
        BossEvent {
            id,
            action: BossAction::Remove,
        }
    }
    /// `EnderDragonFight`'s, `WitherBoss`'s and `Raid`'s events, and `/bossbar add`'s
    /// defaults (white, progress, no flags), each with a name of its own.
    fn dragon(id: u128) -> BossEvent {
        shown(id, lit("Ender Dragon"), 0, 0, [false, true, true])
    }
    fn wither(id: u128) -> BossEvent {
        shown(id, lit("Wither"), 5, 0, [true, false, false])
    }
    fn raid(id: u128) -> BossEvent {
        shown(id, title(), 2, 2, [false, false, false])
    }
    fn custom(id: u128) -> BossEvent {
        shown(id, lit("Demo"), 6, 0, [false, false, false])
    }

    #[test]
    fn boss_battle_is_zero_without_a_bar() {
        assert_eq!(BossOverlay::default().boss_battle(), 0);
    }

    #[test]
    fn boss_battle_gives_each_kind() {
        for (event, kind) in [(custom(1), 1), (dragon(1), 2), (wither(1), 3), (raid(1), 4)] {
            let mut overlay = BossOverlay::default();
            overlay.apply(&event, 0);
            assert_eq!(overlay.boss_battle(), kind, "{event:?}");
        }
    }

    #[test]
    fn boss_battle_ignores_the_name() {
        // A dragon or wither with a custom name, a raid's victory bar: the kind stays.
        for (event, kind) in [
            (shown(1, lit("Bob"), 0, 0, [false, true, true]), 2),
            (shown(1, lit("Steve"), 5, 0, [true, false, false]), 3),
            (
                shown(
                    1,
                    key("event.minecraft.raid.victory.full"),
                    2,
                    2,
                    [false, false, false],
                ),
                4,
            ),
            (shown(1, lit("Wither"), 6, 0, [false, false, false]), 1),
        ] {
            let mut overlay = BossOverlay::default();
            overlay.apply(&event, 0);
            assert_eq!(overlay.boss_battle(), kind, "{event:?}");
        }
    }

    #[test]
    fn boss_battle_needs_every_field_of_a_kind() {
        // One field off each vanilla event: a custom bar.
        for event in [
            shown(1, lit("d"), 0, 0, [false, true, false]),
            shown(1, lit("d"), 0, 0, [false, false, true]),
            shown(1, lit("d"), 0, 1, [false, true, true]),
            shown(1, lit("d"), 1, 0, [false, true, true]),
            shown(1, lit("d"), 0, 0, [true, true, true]),
            shown(1, lit("w"), 5, 0, [false, false, false]),
            shown(1, lit("w"), 5, 1, [true, false, false]),
            shown(1, lit("w"), 5, 0, [true, true, false]),
            shown(1, lit("w"), 5, 0, [true, false, true]),
            shown(1, lit("w"), 6, 0, [true, false, false]),
            shown(1, title(), 2, 0, [false, false, false]),
            shown(1, title(), 2, 3, [false, false, false]),
            shown(1, title(), 3, 2, [false, false, false]),
            shown(1, title(), 2, 2, [true, false, false]),
            shown(1, title(), 2, 2, [false, true, false]),
            shown(1, title(), 2, 2, [false, false, true]),
        ] {
            let mut overlay = BossOverlay::default();
            overlay.apply(&event, 0);
            assert_eq!(overlay.boss_battle(), 1, "{event:?}");
        }
    }

    #[test]
    fn boss_battle_reads_ordinals_as_the_bar_shows_them() {
        // Past the enum a colour shows as white and an overlay as 20 notches; below it as pink
        // and progress, so a negative colour with the dragon's flags is the dragon's bar.
        for (event, kind) in [
            (shown(1, lit("x"), 99, 0, [true, false, false]), 1),
            (shown(1, lit("x"), -3, -1, [false, true, true]), 2),
            (shown(1, title(), 2, 99, [false, false, false]), 1),
        ] {
            let mut overlay = BossOverlay::default();
            overlay.apply(&event, 0);
            assert_eq!(overlay.boss_battle(), kind, "{event:?}");
        }
    }

    #[test]
    fn boss_battle_prefers_the_first_boss_over_custom_bars() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&custom(1), 0);
        overlay.apply(&wither(2), 0);
        overlay.apply(&dragon(3), 0);
        assert_eq!(
            overlay.boss_battle(),
            3,
            "the wither came before the dragon"
        );
        overlay.apply(&remove(2), 0);
        assert_eq!(
            overlay.boss_battle(),
            2,
            "the dragon once the wither's bar is gone"
        );
        overlay.apply(&remove(3), 0);
        assert_eq!(overlay.boss_battle(), 1, "only the custom bar is left");
        overlay.apply(&remove(1), 0);
        assert_eq!(overlay.boss_battle(), 0);
        // A raid's bar ahead of a wither's.
        overlay.apply(&raid(4), 0);
        overlay.apply(&wither(5), 0);
        assert_eq!(overlay.boss_battle(), 4);
        // Re-adding a shown id keeps its place, with its new kind.
        overlay.apply(&dragon(4), 0);
        assert_eq!(overlay.boss_battle(), 2);
        overlay.reset();
        assert_eq!(overlay.boss_battle(), 0);
    }

    #[test]
    fn boss_battle_follows_style_and_property_updates() {
        let mut overlay = BossOverlay::default();
        overlay.apply(&custom(1), 0);
        let update = |action| BossEvent { id: 1, action };
        overlay.apply(
            &update(BossAction::UpdateStyle {
                color: 5,
                overlay: 0,
            }),
            0,
        );
        assert_eq!(overlay.boss_battle(), 1, "purple without darkening");
        overlay.apply(
            &update(BossAction::UpdateProperties {
                darken: true,
                music: false,
                fog: false,
            }),
            0,
        );
        assert_eq!(overlay.boss_battle(), 3, "a bar shown as the wither's");
        overlay.apply(
            &update(BossAction::UpdateStyle {
                color: 2,
                overlay: 2,
            }),
            0,
        );
        assert_eq!(overlay.boss_battle(), 1, "red and notched, but darkening");
        // An update for a bar this client was never told about changes nothing.
        overlay.apply(
            &BossEvent {
                id: 9,
                action: BossAction::UpdateProperties {
                    darken: false,
                    music: false,
                    fog: false,
                },
            },
            0,
        );
        assert_eq!(overlay.boss_battle(), 1);
        overlay.apply(&remove(9), 0);
        assert_eq!(overlay.boss_battle(), 1);
    }

    #[test]
    fn boss_battle_needs_a_raid_title() {
        let red = |name: Tag| shown(1, name, 2, 2, [false, false, false]);
        let typed = |kind: &str, mut pairs: Vec<(&str, Tag)>| {
            pairs.push(("type", Tag::String(kind.into())));
            Tag::compound(pairs)
        };
        // Every title vanilla's `Raid` gives its bar, as vanilla and Sunrise's own server write it.
        for name in [
            title(),
            remaining(lit(" - "), vec![Tag::Int(2)]),
            remaining(lit(" - "), vec![Tag::String("1".into())]),
            remaining(
                Tag::compound([("text", Tag::String(" - ".into()))]),
                vec![Tag::Int(1)],
            ),
            key("event.minecraft.raid.victory.full"),
            key("event.minecraft.raid.defeat.full"),
            typed(
                "translatable",
                vec![("translate", Tag::String("event.minecraft.raid".into()))],
            ),
        ] {
            let mut overlay = BossOverlay::default();
            overlay.apply(&red(name.clone()), 0);
            assert_eq!(overlay.boss_battle(), 4, "{name:?}");
        }
        // An ordinary `/bossbar` set to red and 10 notches, and near misses of the titles.
        for name in [
            lit("Raid"),
            lit("Demo"),
            key("event.minecraft.raid.raiders_remaining"),
            key("event.minecraft.raid.victory"),
            Tag::compound([
                ("translate", Tag::String("event.minecraft.raid".into())),
                ("color", Tag::String("gold".into())),
            ]),
            Tag::compound([
                ("translate", Tag::String("event.minecraft.raid".into())),
                ("with", Tag::List(vec![Tag::Int(1)])),
            ]),
            Tag::compound([
                (
                    "translate",
                    Tag::String("event.minecraft.raid.victory.full".into()),
                ),
                ("extra", Tag::List(vec![lit("!")])),
            ]),
            remaining(lit(" + "), vec![Tag::Int(1)]),
            remaining(lit(" - "), vec![]),
            remaining(lit(" - "), vec![Tag::Int(1), Tag::Int(2)]),
            typed(
                "text",
                vec![("translate", Tag::String("event.minecraft.raid".into()))],
            ),
            Tag::compound([("text", Tag::String("event.minecraft.raid".into()))]),
        ] {
            let mut overlay = BossOverlay::default();
            overlay.apply(&red(name.clone()), 0);
            assert_eq!(overlay.boss_battle(), 1, "{name:?}");
        }
        // A custom red bar does not win over a real boss shown after it.
        let mut overlay = BossOverlay::default();
        overlay.apply(&red(lit("Raid")), 0);
        overlay.apply(&wither(2), 0);
        assert_eq!(overlay.boss_battle(), 3);
        // A rename follows: the raid's bar renamed to a plain text, and back.
        let rename = |name| BossEvent {
            id: 1,
            action: BossAction::UpdateName { name },
        };
        overlay.apply(&rename(title()), 0);
        assert_eq!(overlay.boss_battle(), 4);
        overlay.apply(&rename(lit("Raid")), 0);
        assert_eq!(overlay.boss_battle(), 3);
        overlay.apply(&rename(remaining(lit(" - "), vec![Tag::Int(2)])), 0);
        assert_eq!(overlay.boss_battle(), 4);
    }
}
