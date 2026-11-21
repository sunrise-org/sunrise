# Declaring a machine's menu data

A mod block entity has two different kinds of storage: external item access uses
transactional Parts, while internal slots use the handle methods below; integer menu data is read with
`BlockEntityRef::data_slots` and changed on a server Host. Menu data is transient:
creation and loading both start with an empty data vector, even when the saved
payload contains the machine's progress.

## State-aware creation

Declare the block entity type and its owning blocks during registration. To make
creation depend on block properties, answer `stateful_block_entity` with `true` for
that block. The `block_entity` hook then receives the actual placed
state, not just the block identity. Read its properties through the Host's state
registry and supply the initial saved compound. The registered declaration owns
the entity type; an unanswered state-aware creation makes no entity.

Creation runs inside the block write, after placement side effects, on servers
and clients. Changing properties within the same block preserves an existing
entity even when a fresh instance of the new state would have no entity.

## Size the data vector

Call `entity.set_data_slots(host, initial_values)` with the complete initial
vector when preparing a newly created or loaded machine. For a progress display,
use one element initialized from the progress value in its saved payload.
Do not reset the vector to its initial values on every tick.

The method accepts at most the game's mod-menu data-slot limit (256).
It returns `Ok(true)` when it changes the vector, `Ok(false)` for identical values,
or `BlockEntityDataError::TooManySlots` with the submitted count and limit. It
never silently clamps a vector. `BlockEntityDataError::Unavailable` means the
handle cannot reach a writable mod holder in the server level in place.

## Update individual values

After sizing, call `entity.set_data_slot(host, index, value)` for scalar updates.
This method does not resize: an absent index returns `Ok(false)`. Both methods use the
same silent storage write target and journal; call `set_changed` explicitly if needed. Transactions retain their
ordinary behavior; an aborted outer transaction restores the vector that existed
before its first write, including writes committed by children. The normal menu
broadcast delivers committed changes to viewers. To draw those values on the client, see
[Draw a progress bar and a fuel gauge](../client/menu-gauges.md).

The public handle resolves one entity's lifecycle generation as well as its level
and position. Removal, replacement and chunk reload make it stale; in-place data
edits preserve it. It is not a borrowed-payload API.

## Internal items versus automation

For a configured mod container, `item_slot(host, slot)` snapshots an internal slot;
`set_item_slot(host, tx, slot, stack)` replaces it within an existing server
transaction. Any held mod block-entity handle can be used; there is no caller
ownership check. This deliberately bypasses inputs, outputs and face policy, so a
recipe can consume its input and produce its output atomically, including output
slots that automation cannot insert into. Use the `ITEMS` capability for external
insertion; it enforces automation policy. Vanilla containers are unavailable here.

Indices use the current container's declared count. Lazy slots read empty; saved
excess slots are neither exposed nor truncated. `None` clears, identical values
return `Ok(false)`. A committed change marks the chunk unsaved without notifying
comparators or neighbours. Call `set_changed` explicitly to notify. Nested abort
restores ordinary before-images, including committed-child writes. Invalid
indices, unknown/air/zero-count stacks and counts above the registered or
component-overridden maximum return `BlockEntityItemError`, never a partial write
or silent count clamp. A stale handle refuses a replacement rather than accessing
its container; acquire a new handle for the replacement.

## Skip unchanged tick write-back

A tick with no `BlockEntityPart::block_entity_tick` subscriber for the entity's
registered type does not snapshot the row, resize its items or mark its chunk
unsaved. Pending load notices are still delivered before the tick.

The `BlockEntityPart::block_entity_tick` reply defaults to `Changed`, retaining full checked payload write-back.
When no payload should be written back, set:

```rust,ignore
*event.reply = BlockEntityTickReply::Unchanged;
```

This discards all edits to `event.entity`, omits the reply snapshot encode/decode,
preserves the original revision, and causes no tick-only chunk dirty mark or
container/comparator effects. Removal or replacement during the callback prevents
restoring the original entity. Independent Host writes still have their own normal
effects. The input snapshot is still sent; this is not batching or a validity cache.

## One-shot client effects

To tell clients about something that happened once, such as a burst of particles, queue
fields instead of writing them to the saved `nbt`. In `block_entity_tick`, or another
event with a mutable entity, call
`event.entity.set_client_update_fields(fields)`, then `host.block_entity_sync(event.pos)`.
A later call replaces the batch; an empty compound clears it.

The next block change that reaches a viewer puts the fields over what
`block_entity_update_tag` gives, or sends them alone without one, in one packet for every
current viewer, and consumes the batch. A change nobody sees leaves it waiting, and neither a
chunk sent to a player nor a resend of the blocks under a single player takes it. Removing
or unloading the block entity drops it. It is never saved, cloned or compared, and it does
not change the entity's revision.

If the merged tag exceeds the 2 MiB network-NBT limit, the game drops the fields,
logs a warning, and sends the original update tag instead. A transaction that rolls
the block entity back also drops any batch queued before it.

A client merges the fields into its copy's `nbt` as any update field, and they stay there
until sent again: put a counter in them to tell one effect from the next.

## Retaining a handle

A `BlockEntityRef` names one live entity, not every entity later placed at its position. Removal,
replacement and chunk reload invalidate it; reacquire with `host.block_entity(level, position)`.
In-place data edits and temporary tick storage preserve identity. Option reads return `None` for
stale handles; writes return their error's `Stale` variant, never writing to a replacement.
`set_lock`, `set_name`, `merge_data`, `replace_data`, `set_changed` and `set_data_slot`
return `Result<bool, BlockEntityError>`: handle `Err(BlockEntityError::Stale)` by dropping
or reacquiring the handle. `Ok(false)` means unchanged data or an unsupported operation,
not a stale identity. Item and data-vector methods return `BlockEntityItemError` and
`BlockEntityDataError`, respectively.

## When its chunk unloads

A machine that keeps something outside its saved data (a running job, an open handle, its place in an
energy or pipe network) lets go of it in `on_chunk_unloaded`, NeoForge's `onChunkUnloaded`: a
`BlockEntityPart` on the type is told once for each of its block entities in a chunk that unloads, on each
side that holds a copy, before the copy is dropped.

```rust,ignore
struct Node;
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for Node {
    fn on_chunk_unloaded(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::OnChunkUnloadedEvent<'_>,
    ) {
        if let mod_api::HostSide::Server(_) = host {
            // The handle equals the one `host.block_entity` gave while it was loaded.
            NETWORK.lock().unwrap().remove(&event.block_entity);
        }
    }
}
// In Hooks::register:
// registrar.block_entity_part(&node_type, Node)?;
```

`event.pos` is where it stands and `event.entity` its data at the call: on a server what the chunk was
just saved with, on a client its copy as the server last sent it. On a server the chunk is saved before
the call, so a change made in it is not kept, and the chunk may already have left the level, so its
handle's reads find nothing there. It is not called when the world closes, nor on a client when its whole
level goes, and never for a vanilla block entity. The chunk-wide notice is `chunk_unload`, a
server hook told after the chunk's block entities; a client is told of the block entities alone
([hook notes](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/on_chunk_unloaded.rs)).

## Client ticks: a machine's hum

What a running machine does on the client alone (a looping hum, smoke from its chimney, a part eased between
two server updates) hangs off its client tick, vanilla's client `BlockEntityTicker`. A `BlockEntityPart` opts
its type in with `block_entity_client_ticks`, asked once for each of its blocks, and is then told
`block_entity_client_tick` once per client tick for each copy this client holds, while the level runs (not
paused or frozen), in vanilla's ticker order. A loaded copy ticks whether or not it is drawn: hidden behind a
wall, out of view or beyond `block_entity_view_distance`, where `block_entity_submit` is not called. It is
never called on a server, and a type that doesn't opt in costs nothing.

The event has the copy's `pos`, its `block_entity` handle, its block `state` and its `entity` data, read only,
and the host is a `ClientHost`. The data is what the frame's handling of the server's packets merged into the
copy (`block_entity_update_tag`, and `block_entity_data_received` told before): a frame handles what has
arrived, then runs its client ticks, so every tick of one frame sees the same data, and an update that arrives
during a frame is seen from the next frame's first tick.

Here a machine whose update tag carries `running` starts a looping hum on the first client tick it runs and
fades it out when it stops:

```rust,ignore
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The hums playing, by the machine they belong to.
static HUMS: LazyLock<Mutex<HashMap<mod_api::BlockEntityRef, mod_api::SoundRef>>> =
    LazyLock::new(Default::default);

struct Hum;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Hum {}
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for Hum {
    fn block_entity_client_ticks(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::BlockEntityClientTicksEvent<'_>,
    ) {
        event.ticks = true;
    }
    fn block_entity_client_tick(
        &self,
        host: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::BlockEntityClientTickEvent<'_>,
    ) {
        let running = matches!(event.entity.nbt.get(&"running".into()), Some(mod_api::Tag::Byte(1)));
        let mut hums = HUMS.lock().unwrap();
        match (running, hums.get(&event.block_entity).copied()) {
            (true, None) => {
                let [x, y, z] = event.pos.map(|c| f64::from(c) + 0.5);
                let play = mod_api::SoundPlay::at("mymod:machine.hum", mod_api::SoundSource::Blocks, [x, y, z])
                    .looping(0);
                if let Some(sound) = host.play_local_sound(&play, 0) {
                    hums.insert(event.block_entity, sound);
                }
            }
            (false, Some(sound)) => {
                // A fade to 0 stops the sound when it ends.
                sound.fade(&*host, 0.0, 20);
                hums.remove(&event.block_entity);
            }
            _ => {}
        }
    }
}

struct MyMod;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for MyMod {
    // A machine removed, replaced or whose chunk unloaded ticks no more, and its handle no
    // longer reads: its hum stops.
    fn client_tick_post(&self, host: &mut dyn mod_api::ClientHost, _: &mut mod_api::ClientTickPostEvent) {
        HUMS.lock().unwrap().retain(|machine, sound| {
            let here = machine.position(&*host).is_some();
            if !here {
                sound.stop(&*host);
            }
            here
        });
    }
}
```

The copies are told in the order they arrived, a chunk's own in its map's order, as vanilla's client ticker
list gives. One removed, replaced by another type, changed to a block state without a client ticker or whose
chunk unloads is not told again from that tick on, so a looping sound it started plays on until something else
stops it: the example stops a hum once its machine's handle no longer reads (removal, replacement and unload
all end a handle), and a type with states that don't tick would also stop it when the state changes. Leaving
or changing levels stops every sound. Add particles from the same tick with `host.add_particle`
([hook notes](../../reference/hook-notes.md#client-ticks-hums-smoke-and-moving-parts)).

`player.open_menu(host, entity)` also returns `Result<bool, BlockEntityError>`:
`Ok(true)` means opened, `Ok(false)` means a non-provider, cancelled open or
unconnected/wrong-level player, and `Err(BlockEntityError::Stale)` means the entity
does not resolve. It never opens a replacement through an old handle.

## Blockless status menus

To open a content mod menu type with no block (from an item, an entity or a command),
call `player.open_mod_menu(host, request)` with a `ModMenuOpen` request (NeoForge's
`openMenu(provider, extraDataWriter)`). `menu` is the type's `MenuKey`. `extra`, at most
32 KiB, is one-time context available through the client's `host.screen_extra_data()`;
`pos` is an optional hint returned by `screen_pos()`. Neither is live data or block backing.
Pass `Some(hand)` from an item's use to let `menu_still_valid` compare that hand's current
stack with the one at open. Other openers pass `None`. The menu's declared container slots,
if any, start empty; [`owned_slots`](../../concepts/capabilities/menu-slots.md#menu-owned-slots)
declares a temporary container that belongs to the window. Opening returns the window id or
`Ok(None)` when vetoed. Invalid type, connection or client support, opening bytes, data count or
title returns `OpenMenuError`. Neither a refused nor a cancelled open closes the old menu.

For a slotless status screen, declare `data/mymod/sunrise/menu/status.json` as
`{ "slots": 0, "player_inventory": false, "data_slots": 2 }` and attach a `MenuPart`
to `mymod:status`. Open it with two signed values, in index order:

```rust,ignore
let mut request = ModMenuOpen::new(status, Text::translate("mymod.status", vec![]));
request.data = vec![-80_000, 7];
player.open_mod_menu(host, request)?;
```

The initial values are sent after the new window opens. In `screen_draw`, read
`event.data.get(index)`; the values are not available before they arrive. The owning mod
gets the live `MenuRef` from `player.container_menu(host)` or the open event and calls, for
example, `menu.set_data_slot(host, 0, 120_000)?`. It returns `true` for a changed value,
`false` for a repeat, and refuses unsupported, stale or foreign menus and undeclared indices.
Updates go only to this player and this window. Within a transaction, another write sees the
pending value, but a full resend only includes committed values. Commit sends the final changed
values and abort discards pending writes. Closing or replacing the menu discards its data.
Buttons and validity still use `menu_button` and `menu_still_valid`; neither needs a slot.

The menu the player had open closes on the server first, as a close does: its `removed`, one
`player_container_close`, and the cursor stack and crafting grids back to the inventory. Then
the new menu is built, its screen opened and its contents sent, and `player_container_open`
tells of it synchronously. Version 2 names the type string `menu_type` (also on
`player_container_close`) and supplies `menu: Option<MenuRef>`, valid during the open callback,
as `PlayerRef::container_menu` does. It is `None` for crafting tables, merchants, workstations,
lecterns, mount inventories and hidden-slot menus. `replace` is the Java `MenuProvider`'s choice
(`shouldTriggerClientSideContainerClosingOnOpen`), as for
[`open_menu`](../../concepts/capabilities/menu-slots.md#a-mods-chest-menu):
`MenuReplace::CloseFirst`, vanilla's default, first sends the client a close for the old
window, which centres the mouse again; `MenuReplace::InPlace` sends none, so the client keeps
its screen and its mouse where they are until the new screen replaces it, as when one
screen of a machine leads to the next.

## The menu's screen

A mod menu type's screen belongs to the menu type, as a screen registered with NeoForge's
`RegisterMenuScreensEvent` does: attach a `MenuPart` to the type and answer `screen` (the
layout) and, as needed, `screen_draw`, `screen_click` and `screen_tooltip`. The same part
answers whatever opened the menu, a machine's use or `player.open_mod_menu` from an item,
an entity or a command. A mod menu type with no `screen` answer has no screen, and its
window closes again.

```rust
struct BackpackScreen;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for BackpackScreen {
    fn screen(&self, _: &dyn mod_api::Host, _: &mut mod_api::ScreenEvent<'_>) -> Option<mod_api::Screen> {
        Some(mod_api::Screen {
            width: 176.0,
            height: 133.0,
            // One row of nine; a slot left `None` is hidden.
            slots: (0..9)
                .map(|i| Some(mod_api::ScreenSlot { position: [8.0 + 18.0 * i as f32, 18.0], ..Default::default() }))
                .collect(),
            inventory_x: 8.0,
            inventory_y: 51.0,
            ..Default::default()
        })
    }
}

// In `register`:
r.menu_part("mymod:backpack", BackpackScreen);
```

Each of the four events names the menu type (`event.menu`) and the mod block whose menu it
is (`event.block`: the mod block at the menu's position whose container opens this menu type,
`None` for a menu opened without one), so blocks that share a menu type
can lay out or draw differently. A mod block whose container names one of vanilla's menus
gets a mod screen the same way, from a part attached to that vanilla menu type; it is asked
only for your own mod's blocks that open the menu, never for another mod's or vanilla's own,
so several mods' machines can share one vanilla type. One part owns each screen and alone
hears its four hooks: a mod menu type takes one part answering them, a vanilla type one per
mod, and a second is refused at load. The server asks `screen` too, whatever opened the
menu, so a slot it hides takes no shift-click there either.

For a menu type with a `MenuPart`, a button returned by `screen_click` reaches
`menu_button` on the server. A block-backed menu also supplies its backing block entity;
a menu opened without a block supplies `None` for both `event.entity` and `event.context.pos`.
Registered recipe-book screens work for blockless menus when their recipe slots have native
slot backing. The mod that registered the layout owns its slot controls and can attach a part
for drawing, clicks and tooltips; another mod's screen part is refused.

## Laying a screen's slots out again

A screen whose slots move while it is open (Tinkers' tinker station, which lays its input slots out for the
tool being built) calls `host.set_screen_slot(menu, slot, Some([x, y]))` on the client, or `None` to hide a
slot ([moving or hiding a mod screen's slots](../client/screens.md#moving-or-hiding-a-mod-screens-slots)). Only
the screen changes: the server's menu keeps every slot.

## Receiving synchronized data

Implement `block_entity_data_received` on the block entity's part to react on the
client after its synchronized tag has merged. It receives a `BlockEntityRef` and
the arriving compound; reads through the handle already see the new data. This
listen notice carries `BlockEntityDataSource::Chunk` or `UpdatePacket`. A chunk
with an empty compound sends no notice. An update packet with an empty compound
sends one notice; a packet with no tag sends none. Repeated identical packets
each notify, and the event contains only the arriving fields, while handle reads
see the merged copy.
It cannot change the arriving tag. Vanilla entities, missing tags, missing client
entities and mismatched types do not call it. Server tag production uses
`block_entity_update_tag`.
