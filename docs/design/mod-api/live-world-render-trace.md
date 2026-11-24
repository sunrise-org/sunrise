# Local live-world rendering trace

This opt-in diagnostic observes the existing publication, acceptance, contact and rendering
path. It does not change interpolation, packet acceptance, contacts, frame pacing or camera
inputs. Enable it only for a bounded local investigation:

```sh
SUNRISE_LIVE_WORLD_TRACE=/absolute/fresh/output.jsonl <existing-game-command>
```

The parent directory must exist and the output file must not exist. No directories are
created and existing files are never overwritten. An unset or empty variable disables all
trace output and the overlay; its cached check performs no allocation, clock read or lock.
The variable is sampled once per process. The ordinary isolated-run and real-player-data
rules in `AGENTS.md` still apply.

The writer stops at the first observed limit: 180 seconds from initialization, 64 MiB, or
200,000 JSONL events including the stop row. It reserves space for a `trace_stop` row,
flushes at initialization, approximately once per second while events arrive, and on stop.
Normal client shutdown explicitly stops and flushes it. A quiet process checks the time
limit at its next event; no timer thread is added. A crash or forced termination may leave
the final buffered interval or stop row absent. A write failure is logged and stops tracing;
the stop row itself cannot be guaranteed after an I/O failure. Reaching any limit also
disables the marker. Buffered serialization and synchronous file writes perturb the enabled
run and are not a throughput benchmark.

Every event carries a process-wide `monotonic_ns`, a separately sampled SystemTime `unix_ns`,
sequence, PID, thread name and the client thread's current `frame_id` (zero on a thread
without a redraw). UNIX nanoseconds and float-bit fields must be parsed as integers, not
rounded through a JavaScript Number. Compare elapsed intervals using monotonic time. UNIX
samples are anchors for another module/process with an independent Instant origin; they do
not establish identical origins or make the wall clock immune to adjustment.

`server_queued` observes successful queue admission, not delivery. `client_received` precedes
validation; `client_accepted` records actual stored state, or removal/retirement. Explicit
duplicate, stale, retired and unknown-ID paths emit `client_ignored`. A refused malformed or
divergent message can have a received row without accepted/ignored, followed by the ordinary
connection error. Wire frame, source revision, pose revision and projection generation have
distinct fields. Stored accepted samples, coherent visible samples, frozen status, observed
source revision/hash, contact and presentation endpoints are retained separately.

`client_contact_complete` records both sides of the existing completion write. `client_draw_pair`
records the pair selected for model drawing. `model_draw` records the actual interpolated frame,
camera-relative double matrix, first emitted cell's actual float matrix, first emitted vertex
and emitted vertex count. The first cell/vertex is a bounded witness, not a dump of all cells.
`outline_draw` records the actual interpolated outline transform and a first-edge witness.
Values that choose the pose/transform also carry IEEE bit patterns. No extra interpolation
calculation advances or replaces the one used for drawing.

`frame_begin` records the ordinary redraw interval. `frame_steps` records the scheduled client
ticks; completion and draw rows identify the actual client tick. `frame_submit` and
`frame_submit_result` bracket the normal assembled Frame's CPU renderer boundary; `frame_end`
records its returned rendered flag. These are not GPU completion, display presentation or
movie-capture receipts. Failed early rendering can lack the later rows. Special loading,
closing-save and screenshot-sequence paths do not all pass the normal assembled Frame boundary.

While active, the normal assembled Frame appends a black-backed numeric marker in the top-left
corner after HUD/screen drawing, including with the normal HUD hidden. Its first number is
`frame_id`; the second is frame-start monotonic milliseconds. `frame_marker` records the exact
text and full nanoseconds. The marker becomes observable only if that Frame is actually
captured/displayed. Repeated movie frames with the same marker can be compared with actual
submitted-frame rows; the trace alone cannot classify an absent capture frame. No FPS, time,
world, viewport or camera setting is forced.

## Regression event sequences

Use an independent generic projection fixture and preserve the pre-fix trace. Exercise an
update followed by one tick and several renders; an idle tick without a new sample; multiple
ticks in one frame; multiple accepted updates before one tick; receipt without a client tick;
duplicate/stale input; coherence freeze and recovery; retirement followed by a new generation;
and both translation and pivot rotation. Check model/outline transforms together with the
actual camera partial. A source mechanism by itself does not establish the cause of a visible
hold/jump. Physical-contact expectations remain separate from frame-smoothness observations.

## Authoring provenance

This is AI-authored local experimental instrumentation, not clean-room certification, human
DCO eligibility, maintainer approval, publication or runtime proof. The owner authorized this
local diagnostic before any interpolation fix. Only this Sunrise worktree's instructions,
contribution/licensing/provenance records and relevant own source were consulted. The supplied
requirements were generic rendering observations and investigation questions. No neighboring
personal project, port, Sable, Companion, native caller implementation, foreign engine source,
old frame audit, installed profile or private derived algorithm was inspected or used.

Inputs include `AGENTS.md`, `CONTRIBUTING.md`, `LICENSING.md`, `.githooks/pre-commit`, existing
live-world provenance/fixture authoring records, `game.rs`, the live-world projection/sync/picking/
inspection and sync-carry test modules, ordinary remote packet/tick and client-level-tick modules,
client tick steps, window handler, headless and render-frame-pre modules, `clock.rs`, protocol
`live_world_sync.rs`, core `geometry/oriented.rs`, and renderer `hud.rs`. Filename/symbol searches
were confined to this worktree. No Cargo, compiler, game, network or installation command was
run by this author while preparing the initial diagnostic source. Build/runtime validation is
the coordinator's separate next gate, with the existing assigned cache and profile.
