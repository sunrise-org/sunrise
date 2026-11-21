# Client networking

Call `host.disconnect(reason)` through `ClientHost` to leave the joined server or world at the end of the frame, outside hooks. Pass `Some(Text)` to show a disconnect reason or `None` for the normal leave flow. The disconnect screen renders the reason in the player's language, like other translatable text: a translatable component shows its translated text and a plain-text reason is unchanged. The call returns `Result<(), DisconnectError>`: `Unavailable` means no world is joined, it is closing, or a save is pending, and `Text` reports an invalid reason (a malformed or too-deep component is refused by that name and nothing changes). A pending request belongs to its current world epoch and cannot leave a later world. Server code continues to use the separate player disconnect operation.

## Holding outgoing play packets

Call `host.hold_packets()` to start an ordered hold and receive its opaque `HoldId`.
While a hold is active, another `hold_packets()` returns
`HoldError::Limit { packets: 256, bytes: 1_048_576 }` and leaves the first hold intact.
Uncancelled packets from `client_packet_send` queue in creation order;
mod packets accepted by `send_packet` join the queue when their root transaction commits.
`release_held(id)` sends them oldest first and returns the published count. `drop_held(id)`
discards the queue. Neither operation replays the cancellation hook or exposes the packet bodies.

The bounds are **256 packets** and **1,048,576 encoded bytes**, including packet type ids.
Overflow ends the hold and flushes it oldest first, including the packet that exceeded the bound.
Later packets send normally. The next release or drop of that id returns
`HoldError::Limit { packets: 256, bytes: 1_048_576 }`; subsequent calls return `Unknown`.
A failed release returns `HoldError::Send { sent, error }`, using `SendPacketError` for the reason,
and drops the unsent tail.

Disconnect, a protocol incarnation change, or an aborted dimension crossing discards the hold;
its id then returns `Unknown`. Starting without a joined physical play connection returns `NoWorld`.
Keep-alives, chunk batch acknowledgements, configuration packets, cookies and resource-pack
answers bypass holds. Standalone cookies and transfers run on the main thread. While the receive
queue has room, the receiver answers standalone keep-alives and chunk batches without waiting for
queued bundle handlers. Inside a bundle, each handler runs inline on the main thread in member
order, including registered NETWORK payload handlers.
Those handlers still cannot borrow a host directly; use queued work for that. In either case,
`client_packet` hears neither the batch's start nor its finish (it does hear the chunks between
them), and `client_packet_send` neither sees nor can cancel the acknowledgement.
No packet inspection or editing beyond the existing `ClientPacket` view is provided.

## Configuration tasks before play

The server's `register_configuration_tasks` hook runs once per connecting client,
after the built-in registry, tag and resource-pack steps. Append
`ConfigurationTask::new(type_id)` to its task list. Every peer gets these tasks,
including vanilla clients.

`configuration_task_start` runs once for the active type. Use the event's
`ConfigurationConnection` to check `has_channel`, send a registered configuration
payload with `send_payload`, and `finish(host, task_type)`. Keep the connection
capability if a payload reply handler will finish the task later. In a main-thread
payload handler, obtain the server host from `IPayloadContext::server_host`.
Alternatively, `configuration_task_tick` checks the task each server tick and
sets `finished` when it is complete. The next task starts only after completion.

A wrong task type, refused payload send, explicit `fail`, or callback panic closes
that client with the configuration-error reason. Check the channel before sending
to an optional or vanilla peer, then finish immediately or explain why it cannot
join. Sends use the existing negotiated payload transport and publish on transaction
commit. A callback panic aborts its transaction.

There is **no per-task timeout**. A client answering keep-alives can remain in
configuration indefinitely while its task waits. The connection's ordinary idle
and keep-alive checks still apply.

## Typed payloads

A typed payload sends a value of your own type on a registered channel. Its body is the value's
`PayloadCodec` encoding (vanilla's network encoding), with nothing around it, so a peer reads it
as an ordinary registered payload.

Derive `PayloadCodec` for the value, register it in `Hooks::register`, and keep the token the
registrar returns:

```rust,ignore
use mod_api::*;
use std::sync::OnceLock;

#[derive(PayloadCodec)]
struct Counter {
    counter: VarInt,
}

static REPLY: OnceLock<ToClient<Counter>> = OnceLock::new();

fn register(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    // The server answers each counter with the next one.
    view.play_to_server_typed::<Counter>(PayloadId("example:count".into()), |value, cx| {
        let counter = value.counter.0.checked_add(1).ok_or("counter is too large")?;
        let next = Counter { counter: VarInt(counter) };
        REPLY.get().unwrap().reply(cx, &next).map(drop).map_err(|e| format!("{e:?}"))
    });
    let reply = view.play_to_client_typed::<Counter>(
        PayloadId("example:count_reply".into()),
        |value, _| Ok(log::info!("server counted {}", value.counter.0)),
    );
    let _ = REPLY.set(reply);
}
```

- **Registration.** `play_to_server_typed`, `play_to_client_typed` and `play_bidirectional_typed`,
  and their `configuration_*` and `common_*` forms, sit beside the raw methods on the same
  registrar and take its version, `optional()`, `droppable()` and `executes_on(...)`. The handler receives the
  decoded value and the connection's `IPayloadContext`. A typed id has one declaration: the same
  id in another protocol refuses load.
- **Tokens.** Registration returns a `ToServer<T>` (client to server) or `ToClient<T>` (server to
  client), both for a bidirectional one. Keep it and send with it. A token grants nothing on its own: each send checks it.
- **Sending and replying.** `ToServer::send(host, &value)` sends from the client
  (`ClientHost::send_payload_to_server`), `ToClient::send(host, player, &value)` from the server
  (`PlayerRef::send_payload`), and `reply(cx, &value)` on either answers on the connection a
  handler received on. They return the raw sends' results: `Queued` means accepted locally.
  Sending the wrong direction does not compile. To address the single-player owner,
  use the ordinary player ref from the [players service](../entities/server-services-and-players.md).
- **Sending to a group.** A `ToClient<T>` also sends to a set of players without you building
  the set, each checking the token and encoding once, then sending the same bytes to each
  player and returning how many sends were accepted: `send_to_all(host, &value)` (every
  server player), `send_in_level(host, level, &value)`, `send_near(host, level, pos, radius,
  except, &value)` (players in the level, not `except`, strictly closer to `pos` than
  `radius`) and `send_tracking_chunk(host, chunk, &value)` (the chunk's watchers). They
  are NeoForge's `PacketDistributor` `sendToAllPlayers`, `sendToPlayersInDimension`,
  `sendToPlayersNear` and `sendToPlayersTrackingChunk`. A player whose send is refused (no
  channel, gone since the list was read, backpressure) is skipped and not counted; only a
  stale token or an encode failure is an `Err`, before anything is sent.
- **To the players tracking an entity.** `send_tracking_entity(host, entity, and_self, &value)`
  sends to the entity's trackers (`ChunkMap.TrackedEntity.seenBy`, NeoForge's
  `sendToPlayersTrackingEntity`; `EntityRef::trackers(host)` is the same list), and
  also to the entity itself when it is a player and `and_self` is set
  (`sendToPlayersTrackingEntityAndSelf`), each player once. A stale or absent entity has no
  trackers, so nothing is sent and the count is 0.
- **The one-call send underneath.** `PayloadsServerHost::players_send_payload(players, id, bytes)`
  queues the same bytes on each listed player's connection with one boundary call, where
  `player_send_payload` crosses once per player; the group sends above use it. Each connection's
  queue owns its own copy of the bytes. Every player is judged by `player_send_payload`'s rules on
  its own: a player whose send is refused is skipped and not counted, and the result is how many
  were accepted (0 for an empty list). A player listed twice is sent to twice. Like every send,
  it is published when the root transaction commits, and an abort publishes nothing.
- **Schema negotiation.** The derive gives the value type a schema fingerprint (field types in
  order, variant positions; not names), and it is part of the negotiated version. A peer whose
  type has another shape is refused at join when the payload is required; when both sides
  declared it `optional()`, it is absent and a send returns `UnsupportedPeer`.
- **Named errors.** `PayloadSendError::StaleToken` for a token the registrar did not hand out
  as it claims (forged, from another load, read as another type, or its mod off);
  `EncodeFailed` for a value the peer could not decode (a bound or the allocation budget),
  before anything is queued; `WrongProtocol` for a send in a phase the id is not declared in. A
  received body that is not one value of the type closes the session, naming the mod, and the
  handler does not run. A handler that panics is disabled: its transaction rolls back, nothing it
  staged is sent, the session closes, and it is not called again.
- **Raw channel failures stay local.** A raw channel (`play_raw`) carries bytes from any client,
  so a declaring mod's handler that returns `Failed` does not close the session: the game logs the mod and the channel, rolls back that delivery's staged
  work (a staged disconnect included), and still delivers the other declaring mods' copies. A
  handler that succeeds and asks to disconnect still disconnects, and so does a refusal by the
  main thread's work queue. A typed payload's failure closes the session as above.

## Syncing remote item cooldowns

Vanilla sends an item cooldown to the affected player, not to players tracking
that player. To display a remote shield's cooldown, register a clientbound play
payload on both ends and have the server send the authoritative state. For
example, read `PlayerRef::item_cooldowns(host)` after the change has committed;
`player_shield_disable` is a pre-cooldown notification, not proof that it started.
A subsequent `player_item_cooldown` listener can still veto the start.

Send changes (including an early end) with `ToClient::send_tracking_entity` and
send the current snapshot directly to each new observer in
`player_start_tracking`. Key the client state by the holder's entity ID in the
current level, so [held-item tinting](model-drawing.md#third-person-held-items)
can look it up. Your mod owns expiry, interpolation and cleanup when an entity
stops being tracked or when the client leaves the level or session. Without an
authoritative snapshot or update, the remote cooldown is unknown; leave the
model's tint unchanged rather than guessing from its pose or combat animation.

## Droppable delivery: audio, live data

A payload channel is `Reliable` by default: a queued send is delivered, in order, or the connection
fails. Declare `registrar.droppable()` for data that is worthless when late (voice, a live
position). The packets are the same vanilla `minecraft:custom_payload`; only what happens to a send
that cannot go out yet changes.

**Use it when** a newer message replaces an older one, or a gap is better than a delay. **Don't** for
anything a handler counts on (inventories, state changes): keep those `Reliable`.

**Rules.**
- Play channels only (a configuration or common declaration refuses load). The delivery is part of
  the negotiated version: a peer declaring the other delivery is refused like another version.
- A send waits in the engine's own queue, before framing, compression and encryption, and moves to
  the connection when its writer has room. It is dropped, and counted, when it has waited more
  than 250 ms, when more than 64 KiB wait (the oldest goes), at the switch from configuration to
  play, and when it is over the byte budget. A packet that was handed to the connection is never
  taken back, and a `Droppable` payload is never put in a bundle. The send still returns `Queued`:
  that was never a delivery acknowledgement.
- **Budgets** per connection: 256 KiB/s clientbound and 16 KiB/s serverbound by default, with one
  second of burst. The server's operator raises or lowers them with `droppable_clientbound` and
  `droppable_serverbound` under `[engine]` in `server.toml` ([hosting](https://github.com/sunrise-org/sunrise/blob/main/docs/hosting.md#droppable-mod-payloads-droppable_clientbound-and-droppable_serverbound)); 0 turns
  `Droppable` delivery off in that direction. A client sending over the serverbound budget loses those
  sends, and the server counts them as budget drops; nobody is kicked and `Reliable` traffic is never
  dropped. Over budget, stale sends go first, oldest first, then the new send.
- **Counts:** `payload_drop_counts(&id)` reads the drops of a channel by cause (`stale`,
  `watermark`, `budget`, `switch`) over this game's connections.
- **Ordering:** per channel within one sending thread. A `Network`-thread `send_to` and a
  main-thread send on one channel have no order relative to each other. Vanilla packets are never
  reordered by this.
- **Group send from a network handler:** `IPayloadContext::send_to(players, id, bytes)` on the server
  for `Droppable` channels (others refuse with `NotDroppable`), so a relay forwards a frame without
  waiting for the main tick. `players` is your own snapshot: a `PlayerRef` is a plain value, so keep a
  list in the handler's state behind a lock and replace it from a tick or join hook on the main
  thread. A player who left, or whose connection is not in play, is skipped. A network-thread send is published at once, not with the handler's staged replies.
- **Vanilla's packet-rate limit** (`rate-limit`, off by default) counts every packet, `Droppable`
  payloads included. Voice at 20 ms frames is 50 payloads a second: a mod may pack two frames into
  one payload.
- **Known limit:** the game connection is TCP. Under packet loss a lost segment stalls everything
  behind it (head-of-line blocking), including chunks and `Droppable` payloads already on the wire; the
  watermark and the 250 ms limit bound what piles up behind it, not what is stuck in it. The
  engine adds no loss concealment: carry a sequence number in your payload and let your codec
  conceal gaps.
