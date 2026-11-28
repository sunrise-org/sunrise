use super::ServerState;
use crate::game::ServerGame;
use std::{
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::Dim;

#[derive(Default)]
pub(super) struct SceneTicks {
    remaining: u32,
    barrier: i32,
}

impl SceneTicks {
    pub(super) fn from_env() -> Option<Self> {
        std::env::var("SUNRISE_TEST_SCENE_TICKS")
            .is_ok_and(|value| value == "1")
            .then(Self::default)
    }

    pub(super) fn before_tick(
        &mut self,
        game: &mut ServerGame<'_>,
        state: &mut ServerState,
    ) -> Result<bool> {
        while self.remaining == 0 {
            if game.stop_now(state) {
                return Ok(false);
            }
            let line = match state.console.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Ok(false),
            };
            let words: Vec<_> = line.split_whitespace().collect();
            let host = game.server.lan.as_ref();
            match words.as_slice() {
                ["scene-ticks", count] => {
                    let count: u32 = count.parse()?;
                    if count == 0 {
                        return Err("scene-ticks requires a positive count".into());
                    }
                    self.fence(game, false)?;
                    self.remaining = count;
                }
                ["scene-config", name] => host
                    .ok_or("scene-config: no network host")?
                    .hub
                    .lan
                    .scene_wait_configuration(name)?,
                ["scene-join", name] => host
                    .ok_or("scene-join: no network host")?
                    .hub
                    .lan
                    .scene_wait_join(name)?,
                ["scene-expect", name, count] => {
                    let host = host.ok_or("scene-expect: no network host")?;
                    let guest = host
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.placed && g.name == *name)
                        .ok_or_else(|| format!("scene-expect: {name} is not placed"))?;
                    let played = host
                        .play
                        .get(&guest.conn)
                        .ok_or("scene-expect: no player state")?
                        .stats
                        .get("custom", "minecraft:play_time");
                    let ticks = host.hub.ticks - guest.placed_at + 1;
                    let expected: u64 = count.parse()?;
                    if ticks != expected {
                        return Err(
                            format!("{name}: placement age {ticks}, expected {expected}").into(),
                        );
                    }
                    for (counter, value) in [
                        ("minecraft:play_time", played),
                        ("effects_tick", guest.effects.tick_count),
                    ] {
                        if u64::try_from(value) != Ok(expected) {
                            return Err(
                                format!("{name}: {counter} {value}, expected {expected}").into()
                            );
                        }
                    }
                }
                ["scene-warm", dimension] => {
                    let dim = Dim::from_id(dimension);
                    let back = game.common.dim;
                    game.enter_level(dim)?;
                    let deadline = Instant::now() + Duration::from_secs(120);
                    game.wait_for_ticket_chunks(deadline);
                    loop {
                        game.server_update_world();
                        let streaming = &mut game.server.streaming;
                        if Instant::now() >= deadline {
                            return Err(format!(
                                "scene-warm: chunks did not settle in {dimension}"
                            )
                            .into());
                        }
                        if !streaming.busy() && !streaming.has_pending_light(&game.common.world) {
                            break;
                        }
                        streaming.wait(Duration::from_millis(5));
                    }
                    game.enter_level(back)?;
                }
                _ => game.console_line(state, &line),
            }
        }
        self.remaining -= 1;
        Ok(true)
    }

    pub(super) fn fence(&mut self, game: &ServerGame<'_>, send_input: bool) -> Result<()> {
        let Some(host) = game.server.lan.as_ref() else {
            return Ok(());
        };
        let guests = || {
            host.hub
                .guests
                .iter()
                .filter(|g| g.placed && g.participates_online())
        };
        // Both barriers drain packets; only the odd nonce asks bots for their next tick's input.
        let nonce = i32::MIN + self.barrier * 2 + i32::from(send_input);
        self.barrier += 1;
        for guest in guests() {
            if !guest.sender.try_send_play_ping(nonce) {
                return Err(format!("scene barrier {nonce}: could not ping {}", guest.name).into());
            }
        }
        sunrise_sim::lan::flush_all(guests().map(|g| &g.sender));
        host.hub
            .lan
            .scene_wait_pongs(nonce, guests().map(|g| g.conn))?;
        Ok(())
    }
}
