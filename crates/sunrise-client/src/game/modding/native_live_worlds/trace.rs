//! Bounded, explicitly enabled observations of publication, client ticks and drawn frames.
//! This records the existing path; it never advances a pose or changes frame pacing.
use super::{LiveWorldPose, LiveWorldSample, Vec3};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    fs::OpenOptions,
    io::{BufWriter, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_TIME: Duration = Duration::from_secs(180);
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EVENTS: u64 = 200_000;
const STOP_RESERVE: u64 = 4096;
static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(true);
static NEXT_FRAME: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static FRAME: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

struct Trace {
    writer: BufWriter<std::fs::File>,
    origin: Instant,
    last_flush: Instant,
    sequence: u64,
    bytes: u64,
    stopped: bool,
}

fn unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_nanos().min(u128::from(u64::MAX)) as u64
        })
}

impl Trace {
    fn open() -> Option<Self> {
        let path = std::env::var_os("SUNRISE_LIVE_WORLD_TRACE").filter(|p| !p.is_empty())?;
        let file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) => {
                log::warn!("live-world trace was not opened: {error}");
                return None;
            }
        };
        let origin = Instant::now();
        let mut trace = Self {
            writer: BufWriter::with_capacity(64 * 1024, file),
            origin,
            last_flush: origin,
            sequence: 0,
            bytes: 0,
            stopped: false,
        };
        let result = trace.write("trace_start", json!({
            "schema": 1, "max_seconds": MAX_TIME.as_secs(), "max_bytes": MAX_BYTES,
            "max_events": MAX_EVENTS, "float_encoding": "values_and_u64_bits",
            "clock": "process-wide Instant elapsed; UNIX nanoseconds sampled per event",
            "marker": "frame_id and frame-start monotonic milliseconds, numeric top-left overlay",
            "qualification": "CPU submission observations, not GPU/display presentation receipts"
        }));
        if let Err(error) = result.and_then(|()| trace.writer.flush()) {
            log::warn!("live-world trace start failed: {error}");
            return None;
        }
        Some(trace)
    }

    fn write(&mut self, kind: &str, data: Value) -> std::io::Result<()> {
        let thread = std::thread::current();
        let event = json!({
            "seq": self.sequence, "kind": kind, "pid": std::process::id(),
            "thread": thread.name(), "frame_id": FRAME.get().0,
            "monotonic_ns": self.origin.elapsed().as_nanos() as u64,
            "unix_ns": unix_ns(), "data": data
        });
        let mut line = serde_json::to_vec(&event)?;
        line.push(b'\n');
        if self.bytes + line.len() as u64 > MAX_BYTES {
            return Err(std::io::Error::other("live-world trace byte limit"));
        }
        self.writer.write_all(&line)?;
        self.bytes += line.len() as u64;
        self.sequence += 1;
        Ok(())
    }

    fn stop(&mut self, reason: &str) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        ACTIVE.store(false, Ordering::Relaxed);
        let result = self
            .write(
                "trace_stop",
                json!({
                    "reason": reason, "events_before_stop": self.sequence,
                    "bytes_before_stop": self.bytes
                }),
            )
            .and_then(|()| self.writer.flush());
        if let Err(error) = result {
            log::warn!("live-world trace final flush failed: {error}");
        }
    }
}

/// Cached absence costs no clock reads, allocation, formatting, locking or filesystem work.
pub(in crate::game) fn enabled() -> bool {
    ACTIVE.load(Ordering::Relaxed)
        && TRACE
            .get_or_init(|| Trace::open().map(Mutex::new))
            .is_some()
}

pub(in crate::game) fn emit(kind: &str, data: impl FnOnce() -> Value) {
    if !enabled() {
        return;
    }
    let Some(mut trace) = TRACE
        .get()
        .and_then(Option::as_ref)
        .and_then(|t| t.lock().ok())
    else {
        return;
    };
    if trace.stopped {
        return;
    }
    let reason = if trace.origin.elapsed() >= MAX_TIME {
        Some("time_limit")
    } else if trace.sequence >= MAX_EVENTS - 1 {
        Some("event_limit")
    } else if trace.bytes >= MAX_BYTES - STOP_RESERVE {
        Some("byte_limit")
    } else {
        None
    };
    if let Some(reason) = reason {
        trace.stop(reason);
        return;
    }
    // Serialize once to keep the bound strict even if an individual observation grows.
    let data = data();
    let size = serde_json::to_vec(&data).map_or(MAX_BYTES, |bytes| bytes.len() as u64);
    if trace.bytes + size + STOP_RESERVE > MAX_BYTES {
        trace.stop("byte_limit");
        return;
    }
    if let Err(error) = trace.write(kind, data) {
        log::warn!("live-world trace write failed: {error}");
        trace.stop("write_error");
    } else if trace.last_flush.elapsed() >= Duration::from_secs(1) {
        if let Err(error) = trace.writer.flush() {
            log::warn!("live-world trace flush failed: {error}");
            trace.stop("flush_error");
        }
        trace.last_flush = Instant::now();
    }
}

pub(in crate::game) fn finish() {
    if let Some(mut trace) = TRACE
        .get()
        .and_then(Option::as_ref)
        .and_then(|t| t.lock().ok())
    {
        trace.stop("client_shutdown");
    }
}

pub(in crate::game) fn frame_begin(elapsed: Duration, tick: u64) {
    if !enabled() {
        return;
    }
    let id = NEXT_FRAME.fetch_add(1, Ordering::Relaxed);
    let ns = TRACE
        .get()
        .and_then(Option::as_ref)
        .and_then(|t| t.lock().ok())
        .map_or(0, |t| t.origin.elapsed().as_nanos() as u64);
    FRAME.set((id, ns));
    emit(
        "frame_begin",
        || json!({"elapsed_ns": elapsed.as_nanos() as u64, "client_tick": tick}),
    );
}

/// Appended after normal HUD/screen drawing, including when F1 hides the ordinary HUD.
pub(in crate::game) fn marker(
    hud: &mut sunrise_client::hud::HudBatch,
    atlas: &sunrise_client::hud::HudAtlas,
) {
    if !enabled() {
        return;
    }
    let (id, ns) = FRAME.get();
    let text = format!("{id:06} {:06}", ns / 1_000_000);
    let width = sunrise_client::hud::HudBatch::text_width(atlas, &text);
    hud.fill(atlas, 1.0, 1.0, width + 4.0, 12.0, 0xff00_0000);
    hud.text(atlas, &text, 3.0, 3.0, 0xffff_ffff, false);
    emit(
        "frame_marker",
        || json!({"frame_id": id, "frame_start_monotonic_ns": ns, "text": text}),
    );
}

pub(super) fn pose(pose: LiveWorldPose) -> Value {
    json!({"translation": pose.translation, "pivot": pose.pivot, "rotation": pose.rotation,
        "translation_bits": pose.translation.map(f64::to_bits), "pivot_bits": pose.pivot.map(f64::to_bits),
        "rotation_bits": pose.rotation.map(f64::to_bits)})
}

pub(super) fn sample(sample: LiveWorldSample) -> Value {
    json!({"revision": sample.revision, "previous": pose(sample.previous), "current": pose(sample.current)})
}

pub(super) fn transform(frame: &super::Frame) -> Value {
    let origin = frame.to_world(Vec3::ZERO);
    let columns = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ]
    .map(|axis| frame.to_world(axis).subtract(origin).components());
    json!({"origin": origin.components(), "origin_bits": origin.components().map(f64::to_bits),
        "axes": columns, "axes_bits": columns.map(|axis| axis.map(f64::to_bits))})
}
