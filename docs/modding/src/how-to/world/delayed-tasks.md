# Delayed tasks

Use `host.schedule_task` to run a closure after a delay in server ticks. Like vanilla's
`MinecraftServer.schedule(TickTask)`, it takes a callable. Move the data the task needs into the
closure; it cannot borrow the current hook's host or event. When the task runs, it receives a
fresh `&mut dyn ServerHost` on the same thread.

```rust,ignore
use mod_api::{ScheduleError, ServerHost, TaskId, Text};

fn schedule_reminder(host: &mut dyn ServerHost) -> Result<TaskId, ScheduleError> {
    let reminder = Text::translate("mymod.reminder", vec![]);
    // Keep the returned id if you might need to call host.cancel_task(id).
    host.schedule_task(20, Box::new(move |host| {
        let _ = host.messages().broadcast(reminder);
    }))
}
```

**Timing.** Delays count ticks the server runs: a paused integrated server does not advance them.
Delays of `0` and `1` both mean the next tick. Tasks due on the same tick run in the order they
were scheduled, just after the `server_tick` start phase. A task scheduled by a task or by that
start phase cannot run in the same tick.

**Ownership.** Only the scheduling mod's callback runs. It runs as that mod, including when it
schedules more work. If its owner is disabled when the task is due, the game drops the task without
calling it. `host.cancel_task(id)` returns `true` only if the calling mod owns the still-pending
task. A cancelled or discarded task releases the values it captured; ids are not reused in a world.

**Refusals.** `schedule_task` returns `ScheduleError::NotInMod` when there is no running mod call,
`ScheduleError::TooMany` when that mod already has `MAX_PENDING_TASKS` tasks waiting, and
`ScheduleError::Unavailable` outside the world phase or without an open or opening server world.

**Writes.** Each callback runs in its own transaction. Journaled writes commit on normal return or
roll back if the callback panics. Other due tasks still run; the first panic resumes after the due
work is drained.

**Lifetime.** The queue belongs to the current world and is never saved. Closing the world or a
failed open drops all its pending tasks. If a deadline needs to outlive a restart, save the deadline
in your mod's own storage and schedule a new closure when the world opens.

## Spare time in a tick

`host.server_has_time()` says whether time remains before the next server tick is due (NeoForge's `ServerTickEvent.hasTime` and `LevelTickEvent.hasTime`, vanilla's `MinecraftServer.haveTime`).
A listener of `server_tick` or of a level tick (`level_tick_pre`, `level_tick_post`) that has optional work, such
as a cache rebuild, checks it and leaves the rest for a later tick when it is false. It is always true on a
client and outside a scheduled server tick, so the same code runs on both sides.

```rust,ignore
fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
    if let HostSide::Server(host) = host
        && host.server_has_time()
    {
        self.rebuild_next_cache_entry();
    }
}
```
