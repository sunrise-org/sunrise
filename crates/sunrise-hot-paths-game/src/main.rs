//! The game as `tools/hot_paths.py` measures it (docs/hot-path-trends.md). Without
//! `SUNRISE_HOT_PATHS_MODS=native` it starts exactly as `sunrise-game` does, with no mod. With it,
//! it is what the launcher's generated `main` builds for one native mod: `hot_paths_listener`
//! listens to the entity tick hooks and does nothing in them, so the benchmark's hook workload
//! measures a dispatch that reaches a listener. With `SUNRISE_HOT_PATHS_MODS=fanout-N`,
//! it instead registers N counting listeners as separate native mods. The tool writes the
//! matching `sunrise-native-mods/` packages beside the executable.
//!
//! A crate and not a feature of `sunrise-game`: the game players get links no mod.
#![forbid(unsafe_code)]
use mod_api::__private::native::BuiltMod;
use mod_api::Side;
use mod_api::{
    EntityTickListenedEvent, EntityTickPostEvent, EntityTickPreEvent, Hooks, HostSide, ModEntry,
    ServerHost, ServerStartedEvent,
};
use sunrise_client::game::{HotPathsMods, note_hot_paths_listener_call};

const ID: &str = "hot_paths_listener";
const BUILT: &[BuiltMod] = &[BuiltMod {
    id: ID,
    version: "1.0.0",
    side: Side::Both,
}];

struct Listener;

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn entity_tick_listened(&self, _: HostSide<'_>, event: &mut EntityTickListenedEvent) {
        event.listened = true;
    }
    fn entity_tick_pre(&self, _: HostSide<'_>, _: &mut EntityTickPreEvent<'_>) {}
    fn entity_tick_post(&self, _: HostSide<'_>, _: &mut EntityTickPostEvent<'_>) {}
}

struct FanoutListener(usize);

#[mod_api::__subscriptions]
impl Hooks for FanoutListener {
    fn entity_tick_listened(&self, _: HostSide<'_>, event: &mut EntityTickListenedEvent) {
        event.listened = true;
    }

    fn entity_tick_pre(&self, _: HostSide<'_>, _: &mut EntityTickPreEvent<'_>) {
        note_hot_paths_listener_call(self.0);
    }

    fn server_started(&self, _: &mut dyn ServerHost, _: &mut ServerStartedEvent) {
        note_hot_paths_listener_call(self.0);
    }
}

fn main() -> sunrise_client::Result<()> {
    sunrise_mod_host::prepare_main_stack();
    let startup = sunrise_client::entry::prepare();
    std::hint::black_box(&sunrise_mod_package::holder::CAPABILITY);
    let (entries, built): (Vec<_>, &'static [BuiltMod]) = match HotPathsMods::from_env()? {
        HotPathsMods::None => (Vec::new(), &[]),
        HotPathsMods::Native => (vec![ModEntry::new(ID, Box::new(Listener))], BUILT),
        HotPathsMods::Fanout(count) => {
            let built: Vec<_> = (0..count.get())
                .map(|index| BuiltMod {
                    // BuiltMod is a process-lifetime inventory, as it is for a generated main.
                    id: Box::leak(format!("hot_paths_fanout_{index}").into_boxed_str()),
                    version: "1.0.0",
                    side: Side::Both,
                })
                .collect();
            let entries = built
                .iter()
                .enumerate()
                .map(|(slot, item)| ModEntry::new(item.id, Box::new(FanoutListener(slot))))
                .collect();
            (entries, Box::leak(built.into_boxed_slice()))
        }
    };
    sunrise_mod_host::main_with(entries, built, startup);
    Ok(())
}
