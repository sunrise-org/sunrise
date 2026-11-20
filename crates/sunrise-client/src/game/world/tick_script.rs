//! `SUNRISE_TICK_SCRIPT=PATH` (tools/test_wire_stream_parity.py): a server's client plays a tick-indexed
//! script, the one the vanilla client plays in `VanillaTickScript.java`, so the two clients' serverbound
//! streams can be compared tick by tick. The harness writes the scenario to PATH (JSON `{"name", "ticks",
//! "steps": {"TICK": [LINE...]}}`, LINE a sunrise-ctl line: `key w down`, `mouse left click`, `look 90 0`,
//! or one of the script's own: `click-slot`, `mouse-move`, `menu-button`, `select-trade`, `beacon`,
//! `open-stats`, `close`, below), then creates PATH with the extension `go`. From the next client tick the script
//! plays: its first tick sends `/wire-mark`, which marks it in the capture, then each tick's lines act
//! before the tick runs, as a player's input in the frame before it; when its ticks have run, the client
//! writes the scenario's name to PATH with the extension `done`.
use super::Game;
use super::event_loop::EventLoopCtl;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Default)]
pub(super) struct TickScript {
    /// PATH, when the variable is set.
    path: Option<PathBuf>,
    running: Option<Running>,
}

struct Running {
    name: String,
    ticks: u32,
    steps: BTreeMap<u32, Vec<String>>,
    tick: u32,
}

impl TickScript {
    pub(super) fn from_env() -> Self {
        Self {
            path: std::env::var_os("SUNRISE_TICK_SCRIPT").map(PathBuf::from),
            running: None,
        }
    }
    /// Whether a scenario is playing.
    pub(super) fn playing(&self) -> bool {
        self.running.is_some()
    }
    /// Whether the variable is set (the frame asks before every tick).
    pub(super) fn armed(&self) -> bool {
        self.path.is_some()
    }
}

/// The scenario at `text`: its name, its length in ticks and each tick's lines.
fn parse(text: &str) -> Option<Running> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut steps = BTreeMap::new();
    for (tick, lines) in json.get("steps")?.as_object()? {
        let lines = lines
            .as_array()?
            .iter()
            .filter_map(|l| l.as_str().map(str::to_owned))
            .collect();
        steps.insert(tick.parse().ok()?, lines);
    }
    Some(Running {
        name: json.get("name")?.as_str()?.to_owned(),
        ticks: u32::try_from(json.get("ticks")?.as_u64()?).ok()?,
        steps,
        tick: 0,
    })
}

impl Game<'_> {
    /// Before a client tick: start the scenario the harness handed over, or play this tick's lines.
    pub(super) fn tick_script_step(&mut self, event_loop: &dyn EventLoopCtl) {
        let Some(path) = self.tick_script.path.clone() else {
            return;
        };
        if self.tick_script.running.is_none() {
            let go = path.with_extension("go");
            if self.remote.is_none() || !go.exists() {
                return;
            }
            let _ = std::fs::remove_file(&go);
            let Some(script) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t)) else {
                log::warn!("Sunrise: tick script {} is not a scenario", path.display());
                return;
            };
            self.tick_script.running = Some(script);
            self.send_signed_chat_input("/wire-mark");
        }
        let Some(running) = self.tick_script.running.as_mut() else {
            return;
        };
        let tick = running.tick;
        running.tick += 1;
        let lines = running.steps.remove(&tick).unwrap_or_default();
        let finished = (running.tick > running.ticks).then(|| running.name.clone());
        // A tick's clicks go to the menu together, as a screen's drag does at its release (one
        // menu session, so a quick craft keeps its state from start to end).
        let mut clicks = Vec::new();
        for line in lines {
            if let Some(click) = tick_script_click(&line) {
                clicks.extend(click);
                continue;
            }
            // Any other line acts after the clicks before it, in the script's order (a beacon's payment
            // click goes before its confirm).
            if !clicks.is_empty() {
                self.remote_menu_clicks(&std::mem::take(&mut clicks));
            }
            // `mouse-move DX DY`: the motion a grabbed pointer adds between frames (MouseHandler.onMove), which
            // the frame turns the view by, not sunrise-ctl's glide (one at a time), so a move every tick counts.
            // `menu-button N`: what a station screen does with its button (a stonecutter's recipe, a loom's
            // pattern): the menu's clickMenuButton, then handleInventoryButtonClick when the menu took it.
            if let Some(button) = line
                .strip_prefix("menu-button ")
                .and_then(|n| n.trim().parse::<i32>().ok())
            {
                let mut taken = false;
                let flag = &mut taken;
                self.menu_session(|menu, env, _| {
                    *flag = menu.button(env, button);
                    Some(*flag)
                });
                if taken {
                    self.remote_station_button(button);
                }
                continue;
            }
            // `select-trade N`, `beacon EFFECT`, `open-stats`: a merchant's offer button, the beacon's confirm
            // with a primary power, the pause menu's Statistics button.
            if let Some(n) = line
                .strip_prefix("select-trade ")
                .and_then(|n| n.trim().parse::<usize>().ok())
            {
                // Only on a merchant's screen, as the vanilla driver presses a MerchantScreen's button.
                if self.station.as_ref().is_some_and(|s| s.merchant.is_some()) {
                    self.try_move_items(n);
                }
                continue;
            }
            if let Some(effect) = line.strip_prefix("beacon ") {
                let effect = effect.trim();
                let open = self
                    .station
                    .as_ref()
                    .is_some_and(|s| s.kind == super::stations::Kind::Beacon);
                if let Some(i) = super::beacon::EFFECTS
                    .iter()
                    .position(|e| open && e.0 == effect)
                {
                    self.confirm_beacon([Some(i), None]);
                    self.close_beacon_screen();
                }
                continue;
            }
            if line.trim() == "open-stats" {
                self.open_stats();
                continue;
            }
            // `close`: the open screen's own close, as the vanilla driver's `screen.onClose()` (a sign
            // editor sends its text, a container its close), and nothing with no screen open.
            // sunrise-ctl's `close` sends a container close whatever is open.
            if line.trim() == "close" {
                self.close_screen();
                continue;
            }
            if let Some((dx, dy)) = tick_script_mouse_move(&line) {
                self.mouse.add_motion(dx, dy);
                self.agent_pointer = true;
                continue;
            }
            self.replay_line(&line, event_loop);
        }
        if !clicks.is_empty() {
            self.remote_menu_clicks(&clicks);
        }
        if let Some(name) = finished {
            self.tick_script.running = None;
            let _ = std::fs::write(path.with_extension("done"), name);
        }
    }
}

/// A `mouse-move DX DY` line's motion.
fn tick_script_mouse_move(line: &str) -> Option<(f64, f64)> {
    let mut words = line.split_whitespace();
    if words.next()? != "mouse-move" {
        return None;
    }
    Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?))
}

/// `click-slot SLOT [right] [shift] [INPUT] [bBUTTON]` without a renderer: sunrise-ctl's
/// version clicks where the slot is drawn, and `--headless` draws nothing. The vanilla driver
/// clicks the menu itself (`MultiPlayerGameMode.handleContainerInput`), and so does this.
/// `SLOT` is the menu's own slot number, -999 outside it; `INPUT` is a `ContainerInput` name
/// (`SWAP`, `THROW`, `PICKUP_ALL`, `QUICK_CRAFT`...).
/// The line's click: `None` for another line, `Some(None)` for a slot out of range.
fn tick_script_click(line: &str) -> Option<Option<(i16, i32, sunrise_client::menu::Input)>> {
    use sunrise_client::menu::Input;
    let words: Vec<&str> = line.split_whitespace().collect();
    let (Some(&"click-slot"), Some(Ok(slot))) =
        (words.first(), words.get(1).map(|w| w.parse::<i32>()))
    else {
        return None;
    };
    let named = words[2..].iter().find_map(|w| Input::parse(w));
    let input = named.unwrap_or(if words.contains(&"shift") {
        Input::QuickMove
    } else {
        Input::Pickup
    });
    let button = words[2..]
        .iter()
        .find_map(|w| w.strip_prefix('b')?.parse().ok())
        .unwrap_or(i32::from(words.contains(&"right")));
    Some(i16::try_from(slot).ok().map(|slot| (slot, button, input)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scenario_names_its_ticks_and_their_lines() {
        let s = parse(r#"{"name":"walk","ticks":40,"steps":{"0":["key w down"],"20":["key w up","look 90 0"]}}"#)
            .unwrap();
        assert_eq!((s.name.as_str(), s.ticks), ("walk", 40));
        assert_eq!(s.steps[&20], ["key w up", "look 90 0"]);
        assert!(parse(r#"{"name":"x","ticks":1,"steps":{"a":[]}}"#).is_none());
    }

    #[test]
    fn a_click_line_names_the_menu_slot_its_input_and_button() {
        use sunrise_client::menu::Input;
        assert_eq!(
            tick_script_click("click-slot 36 SWAP b2"),
            Some(Some((36, 2, Input::Swap)))
        );
        assert_eq!(
            tick_script_click("click-slot -999 THROW"),
            Some(Some((-999, 0, Input::Throw)))
        );
        assert_eq!(
            tick_script_click("click-slot 3 right shift"),
            Some(Some((3, 1, Input::QuickMove)))
        );
        assert_eq!(tick_script_click("click-slot 40000"), Some(None));
        assert_eq!(tick_script_click("key w down"), None);
    }
}
