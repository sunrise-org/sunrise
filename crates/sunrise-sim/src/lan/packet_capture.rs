//! Per-connection outgoing play packet capture behind `net-trace` and `SUNRISE_PACKET_CAPTURE`.
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use sunrise_protocol::connection::Framing;

const CHANNEL_CAPACITY: usize = 8192;
const MAX_QUEUED_BYTES: usize = 32 << 20;
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
const CLAIM_FILE: &str = ".capture-claim";

fn split_packet_id(bytes: &[u8]) -> (i32, &[u8]) {
    let mut value: u32 = 0;
    let mut shift: u32 = 0;
    for (idx, &b) in bytes.iter().take(5).enumerate() {
        value |= u32::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return (value as i32, &bytes[idx + 1..]);
        }
        shift += 7;
    }
    (-1, bytes)
}

fn sanitize_filename(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "player".to_owned()
    } else {
        clean
    }
}

fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\t' | '\r' | '\n' => ' ',
            other => other,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PacketMeta {
    pub(super) id: i32,
    pub(super) len: usize,
    pub(super) hash: u128,
}

impl PacketMeta {
    pub(super) fn from_body(body: &[u8]) -> Self {
        let (id, payload) = split_packet_id(body);
        Self {
            id,
            len: payload.len(),
            hash: xxhash_rust::xxh3::xxh3_128(payload),
        }
    }
}

pub(super) enum CapturedBytes {
    Owned(Vec<u8>),
    Shared(Arc<[u8]>),
}

impl CapturedBytes {
    fn retained_bytes(&self) -> usize {
        match self {
            Self::Owned(bytes) => bytes.capacity(),
            Self::Shared(bytes) => bytes.len(),
        }
    }
}

impl AsRef<[u8]> for CapturedBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

struct ByteBudget {
    used: AtomicUsize,
    limit: usize,
}

impl ByteBudget {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit,
        })
    }

    fn try_reserve(self: &Arc<Self>, bytes: usize) -> Option<ByteLease> {
        if bytes == 0 {
            return Some(ByteLease {
                budget: Arc::clone(self),
                bytes: 0,
            });
        }
        let mut cur = self.used.load(Ordering::Relaxed);
        loop {
            let next = cur.checked_add(bytes)?;
            if next > self.limit {
                return None;
            }
            match self
                .used
                .compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => {
                    return Some(ByteLease {
                        budget: Arc::clone(self),
                        bytes,
                    });
                }
                Err(actual) => cur = actual,
            }
        }
    }
}

struct ByteLease {
    budget: Arc<ByteBudget>,
    bytes: usize,
}

impl Drop for ByteLease {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.budget.used.fetch_sub(self.bytes, Ordering::Release);
        }
    }
}

pub(super) enum OpenSession {
    Opened(Arc<AtomicI64>, SessionWriter),
    Sealed,
}

enum Cmd {
    Open {
        session_id: usize,
        conn: u32,
        ordinal: u32,
        player: String,
        filename: String,
    },
    Packet {
        session_id: usize,
        seq: u64,
        tick: i64,
        bytes: CapturedBytes,
        prepared_framing: Option<Framing>,
        _lease: ByteLease,
    },
    Close {
        session_id: usize,
        expected_count: u64,
        incomplete_reason: Option<&'static str>,
    },
    Shutdown,
}

#[derive(Clone)]
struct SessionEntry {
    conn: u32,
    ordinal: u32,
    player: String,
    filename: String,
    status: String,
    count: u64,
    fallback_close: Option<(u64, &'static str)>,
}

#[derive(Default)]
struct RunState {
    sealed: bool,
    active_sessions: usize,
    player_ordinals: HashMap<String, u32>,
    sessions: Vec<SessionEntry>,
}

fn claim_empty_dir(dir: &Path, run_id: &str) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let claim_path = dir.join(CLAIM_FILE);
    let mut claim = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&claim_path)?;
    let has_other = match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.any(|entry| match entry {
            Ok(entry) => entry.file_name() != CLAIM_FILE,
            Err(_) => true,
        }),
        Err(e) => {
            drop(claim);
            let _ = std::fs::remove_file(&claim_path);
            return Err(e);
        }
    };
    if has_other {
        drop(claim);
        let _ = std::fs::remove_file(&claim_path);
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "packet capture directory must be empty",
        ));
    }
    if let Err(e) = writeln!(claim, "{run_id}").and_then(|()| claim.flush()) {
        drop(claim);
        let _ = std::fs::remove_file(&claim_path);
        return Err(e);
    }
    Ok(())
}

pub(super) struct CaptureRun {
    clock: Arc<AtomicI64>,
    state: Arc<Mutex<RunState>>,
    closed_cvar: Arc<Condvar>,
    byte_budget: Arc<ByteBudget>,
    overflow_noted: Arc<AtomicBool>,
    tx: SyncSender<Cmd>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl CaptureRun {
    pub(super) fn from_env(lan_incarnation: u64) -> io::Result<Option<Arc<Self>>> {
        let Some(path) = std::env::var_os("SUNRISE_PACKET_CAPTURE") else {
            return Ok(None);
        };
        if path.is_empty() {
            return Ok(None);
        }
        Self::open_in(Path::new(&path), lan_incarnation).map(Some)
    }

    pub(super) fn open_in(dir: &Path, lan_incarnation: u64) -> io::Result<Arc<Self>> {
        Self::open_in_with_limits(
            dir,
            lan_incarnation,
            CHANNEL_CAPACITY,
            MAX_QUEUED_BYTES,
            #[cfg(test)]
            None,
        )
    }

    #[cfg(test)]
    pub(super) fn open_in_with_gate(
        dir: &Path,
        lan_incarnation: u64,
        channel_capacity: usize,
        max_queued_bytes: usize,
        start_gate: Option<Receiver<()>>,
    ) -> io::Result<Arc<Self>> {
        Self::open_in_with_limits(
            dir,
            lan_incarnation,
            channel_capacity,
            max_queued_bytes,
            start_gate,
        )
    }

    fn open_in_with_limits(
        dir: &Path,
        lan_incarnation: u64,
        channel_capacity: usize,
        max_queued_bytes: usize,
        #[cfg(test)] start_gate: Option<Receiver<()>>,
    ) -> io::Result<Arc<Self>> {
        let run_id = format!("{}-{lan_incarnation}", std::process::id());
        claim_empty_dir(dir, &run_id)?;
        let state = Arc::new(Mutex::new(RunState::default()));
        let closed_cvar = Arc::new(Condvar::new());
        let byte_budget = ByteBudget::new(max_queued_bytes);
        let overflow_noted = Arc::new(AtomicBool::new(false));
        let (tx, rx) = sync_channel(channel_capacity);
        let thread_dir = dir.to_path_buf();
        let thread_run_id = run_id;
        let thread_state = state.clone();
        let thread_overflow = overflow_noted.clone();
        let spawned = std::thread::Builder::new()
            .name("sunrise-pkt-cap".into())
            .spawn(move || {
                writer_loop(
                    thread_dir,
                    thread_run_id,
                    thread_state,
                    thread_overflow,
                    rx,
                    #[cfg(test)]
                    start_gate,
                )
            });
        let handle = match spawned {
            Ok(handle) => handle,
            Err(e) => {
                let _ = std::fs::remove_file(dir.join(CLAIM_FILE));
                return Err(e);
            }
        };
        Ok(Arc::new(Self {
            clock: Arc::new(AtomicI64::new(0)),
            state,
            closed_cvar,
            byte_budget,
            overflow_noted,
            tx,
            writer: Mutex::new(Some(handle)),
        }))
    }

    pub(super) fn set_tick(&self, tick: i64) {
        self.clock.store(tick, Ordering::Relaxed);
    }

    pub(super) fn seal(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.sealed = true;
    }

    pub(super) fn is_sealed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sealed
    }

    pub(super) fn open_session(&self, conn: u32, player: &str) -> OpenSession {
        let clean_player = sanitize_field(player);
        let filename = format!("{}-{conn}.pkt", sanitize_filename(player));
        let (session_id, ordinal) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.sealed {
                return OpenSession::Sealed;
            }
            let ordinal = state
                .player_ordinals
                .entry(clean_player.clone())
                .and_modify(|n| *n += 1)
                .or_insert(1);
            let ordinal = *ordinal;
            let session_id = state.sessions.len();
            state.sessions.push(SessionEntry {
                conn,
                ordinal,
                player: clean_player.clone(),
                filename: "-".to_owned(),
                status: "incomplete:unclosed".to_owned(),
                count: 0,
                fallback_close: None,
            });
            state.active_sessions += 1;
            (session_id, ordinal)
        };
        let mut overflowed = false;
        if let Err(err) = self.tx.try_send(Cmd::Open {
            session_id,
            conn,
            ordinal,
            player: clean_player,
            filename,
        }) {
            overflowed = true;
            let reason = match err {
                TrySendError::Full(_) => {
                    self.overflow_noted.store(true, Ordering::Relaxed);
                    "overflow"
                }
                TrySendError::Disconnected(_) => "writer_closed",
            };
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.sessions[session_id].status = format!("incomplete:{reason}");
            state.sessions[session_id].fallback_close = Some((0, reason));
        }
        OpenSession::Opened(
            self.clock.clone(),
            SessionWriter {
                session_id,
                seq: 0,
                overflowed,
                incomplete_override: None,
                closed: false,
                clock: self.clock.clone(),
                byte_budget: self.byte_budget.clone(),
                overflow_noted: self.overflow_noted.clone(),
                tx: self.tx.clone(),
                state: self.state.clone(),
                closed_cvar: self.closed_cvar.clone(),
            },
        )
    }

    pub(super) fn finish(&self) {
        self.finish_with_timeout(SHUTDOWN_WAIT);
    }

    pub(super) fn finish_with_timeout(&self, wait_timeout: Duration) {
        let handle = {
            let mut guard = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
            guard.take()
        };
        let Some(handle) = handle else {
            return;
        };
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.sealed = true;
            let deadline = Instant::now() + wait_timeout;
            while state.active_sessions > 0 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let (guard, result) = self
                    .closed_cvar
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(PoisonError::into_inner);
                state = guard;
                if result.timed_out() {
                    break;
                }
            }
        }
        let _ = self.tx.send(Cmd::Shutdown);
        let _ = handle.join();
    }
}

impl Drop for CaptureRun {
    fn drop(&mut self) {
        self.finish();
    }
}

pub(super) struct SessionWriter {
    session_id: usize,
    seq: u64,
    overflowed: bool,
    incomplete_override: Option<&'static str>,
    closed: bool,
    clock: Arc<AtomicI64>,
    byte_budget: Arc<ByteBudget>,
    overflow_noted: Arc<AtomicBool>,
    tx: SyncSender<Cmd>,
    state: Arc<Mutex<RunState>>,
    closed_cvar: Arc<Condvar>,
}

impl SessionWriter {
    pub(super) fn current_tick(&self) -> i64 {
        self.clock.load(Ordering::Relaxed)
    }

    pub(super) fn mark_incomplete(&mut self, reason: &'static str) {
        self.incomplete_override.get_or_insert(reason);
    }

    pub(super) fn record_raw(&mut self, tick: i64, bytes: CapturedBytes) {
        self.record_inner(tick, bytes, None);
    }

    pub(super) fn record_prepared(&mut self, tick: i64, bytes: CapturedBytes, framing: Framing) {
        self.record_inner(tick, bytes, Some(framing));
    }

    pub(super) fn record_prepared_slice(&mut self, tick: i64, bytes: &[u8], framing: Framing) {
        let seq = self.seq;
        self.seq += 1;
        if self.overflowed {
            return;
        }
        let Some(lease) = self.byte_budget.try_reserve(bytes.len()) else {
            self.mark_overflow();
            return;
        };
        let mut owned = Vec::new();
        if owned.try_reserve_exact(bytes.len()).is_err() {
            self.mark_overflow();
            return;
        }
        owned.extend_from_slice(bytes);
        self.send_packet(seq, tick, CapturedBytes::Owned(owned), Some(framing), lease);
    }

    fn record_inner(&mut self, tick: i64, bytes: CapturedBytes, prepared_framing: Option<Framing>) {
        let seq = self.seq;
        self.seq += 1;
        if self.overflowed {
            return;
        }
        let retained = bytes.retained_bytes();
        let Some(lease) = self.byte_budget.try_reserve(retained) else {
            self.mark_overflow();
            return;
        };
        self.send_packet(seq, tick, bytes, prepared_framing, lease);
    }

    fn send_packet(
        &mut self,
        seq: u64,
        tick: i64,
        bytes: CapturedBytes,
        prepared_framing: Option<Framing>,
        lease: ByteLease,
    ) {
        match self.tx.try_send(Cmd::Packet {
            session_id: self.session_id,
            seq,
            tick,
            bytes,
            prepared_framing,
            _lease: lease,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.mark_overflow();
            }
        }
    }

    fn mark_overflow(&mut self) {
        self.overflowed = true;
        self.overflow_noted.store(true, Ordering::Relaxed);
    }

    pub(super) fn close(mut self, reason: Option<&'static str>) {
        self.finish_close(reason);
    }

    fn finish_close(&mut self, reason: Option<&'static str>) {
        if self.closed {
            return;
        }
        self.closed = true;
        let incomplete_reason = reason
            .or(self.incomplete_override)
            .or_else(|| self.overflowed.then_some("overflow"));
        let cmd = Cmd::Close {
            session_id: self.session_id,
            expected_count: self.seq,
            incomplete_reason,
        };
        let sent = self.tx.try_send(cmd).is_ok();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !sent {
            let fallback_reason = incomplete_reason.unwrap_or("overflow");
            state.sessions[self.session_id].fallback_close = Some((self.seq, fallback_reason));
        }
        state.active_sessions = state.active_sessions.saturating_sub(1);
        if state.active_sessions == 0 && state.sealed {
            self.closed_cvar.notify_all();
        }
    }
}

impl Drop for SessionWriter {
    fn drop(&mut self) {
        if !self.closed {
            self.finish_close(Some("dropped"));
        }
    }
}

struct OpenFile {
    out: BufWriter<File>,
    written: u64,
    io_error: bool,
    decode_error: bool,
}

fn decode_captured_meta(
    bytes: &[u8],
    prepared_framing: Option<Framing>,
    inflater: &mut sunrise_protocol::frame::Inflater,
) -> sunrise_protocol::Result<PacketMeta> {
    match prepared_framing {
        None | Some(Framing::Memory | Framing::Network(None)) => Ok(PacketMeta::from_body(bytes)),
        Some(Framing::Network(Some(threshold))) => {
            match sunrise_protocol::frame::body(bytes, Some(threshold), inflater)? {
                sunrise_protocol::frame::Body::Plain(at) => Ok(PacketMeta::from_body(&bytes[at..])),
                sunrise_protocol::frame::Body::Inflated(raw) => Ok(PacketMeta::from_body(&raw)),
            }
        }
    }
}

fn finalize_file(
    mut file: OpenFile,
    expected_count: u64,
    incomplete_reason: Option<&'static str>,
) -> (String, u64) {
    let reason = if file.io_error {
        Some("write_error")
    } else if file.decode_error {
        Some("decode_error")
    } else if let Some(reason) = incomplete_reason {
        Some(reason)
    } else if file.written != expected_count {
        Some("count_mismatch")
    } else {
        None
    };
    if let Some(reason) = reason {
        if !file.io_error {
            let _ = writeln!(file.out, "# end\t{}\tincomplete\t{reason}", file.written);
            let _ = file.out.flush();
        }
        return (format!("incomplete:{reason}"), file.written);
    }
    let ok = writeln!(file.out, "# end\t{expected_count}\tok")
        .and_then(|()| file.out.flush())
        .is_ok();
    if ok {
        ("ok".to_owned(), expected_count)
    } else {
        ("incomplete:write_error".to_owned(), file.written)
    }
}

fn write_manifest_atomic(dir: &Path, run_id: &str, sessions: &[SessionEntry]) {
    let run_status = if sessions.iter().all(|s| s.status == "ok") {
        "ok"
    } else {
        "incomplete"
    };
    let tmp_path = dir.join(format!(".manifest.{run_id}.tmp"));
    let manifest_path = dir.join("manifest.tsv");
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .and_then(|file| {
            let mut out = BufWriter::new(file);
            writeln!(out, "# manifest v2\t{run_id}\t{run_status}")?;
            for entry in sessions {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    entry.conn,
                    entry.ordinal,
                    entry.player,
                    entry.filename,
                    entry.status,
                    entry.count
                )?;
            }
            writeln!(out, "# end\t{}\t{run_status}", sessions.len())?;
            out.flush()
        })
        .and_then(|()| std::fs::rename(&tmp_path, &manifest_path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp_path);
        log::warn!(
            "[lan] packet capture manifest write {}: {e}",
            manifest_path.display()
        );
    }
}

fn writer_loop(
    dir: PathBuf,
    run_id: String,
    state: Arc<Mutex<RunState>>,
    overflow_noted: Arc<AtomicBool>,
    rx: Receiver<Cmd>,
    #[cfg(test)] start_gate: Option<Receiver<()>>,
) {
    #[cfg(test)]
    if let Some(gate) = start_gate {
        let _ = gate.recv_timeout(Duration::from_secs(5));
    }
    let mut files: HashMap<usize, OpenFile> = HashMap::new();
    let mut inflater = sunrise_protocol::frame::Inflater::default();
    let mut logged_write_err = false;
    let mut logged_overflow = false;
    let mut maybe_log_overflow = |noted: &AtomicBool| {
        if !logged_overflow && noted.load(Ordering::Relaxed) {
            logged_overflow = true;
            log::warn!(
                "[lan] packet capture channel or byte budget overflowed; marking run incomplete"
            );
        }
    };
    while let Ok(cmd) = rx.recv() {
        maybe_log_overflow(&overflow_noted);
        match cmd {
            Cmd::Open {
                session_id,
                conn,
                ordinal,
                player,
                filename,
            } => {
                let path = dir.join(&filename);
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(file) => {
                        let mut out = BufWriter::new(file);
                        let header_err =
                            writeln!(out, "# v2\t{run_id}\t{conn}\t{ordinal}\t{player}").err();
                        if let Some(e) = &header_err
                            && !logged_write_err
                        {
                            logged_write_err = true;
                            log::warn!("[lan] packet capture header write {}: {e}", path.display());
                        }
                        {
                            let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
                            st.sessions[session_id].filename = filename;
                        }
                        files.insert(
                            session_id,
                            OpenFile {
                                out,
                                written: 0,
                                io_error: header_err.is_some(),
                                decode_error: false,
                            },
                        );
                    }
                    Err(e) => {
                        log::warn!("[lan] packet capture open {}: {e}", path.display());
                        let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
                        st.sessions[session_id].filename = "-".to_owned();
                        st.sessions[session_id].status = "incomplete:open_failed".to_owned();
                    }
                }
            }
            Cmd::Packet {
                session_id,
                seq,
                tick,
                bytes,
                prepared_framing,
                _lease,
            } => {
                let Some(file) = files.get_mut(&session_id) else {
                    continue;
                };
                if file.io_error || file.decode_error {
                    continue;
                }
                let meta =
                    match decode_captured_meta(bytes.as_ref(), prepared_framing, &mut inflater) {
                        Ok(meta) => meta,
                        Err(e) => {
                            file.decode_error = true;
                            if !logged_write_err {
                                logged_write_err = true;
                                log::warn!("[lan] packet capture prepared decode failed: {e}");
                            }
                            continue;
                        }
                    };
                if let Err(e) = writeln!(
                    file.out,
                    "{seq}\t{tick}\t{}\t{}\t{:032x}",
                    meta.id, meta.len, meta.hash
                ) {
                    file.io_error = true;
                    if !logged_write_err {
                        logged_write_err = true;
                        log::warn!("[lan] packet capture write failed: {e}");
                    }
                } else {
                    file.written += 1;
                }
            }
            Cmd::Close {
                session_id,
                expected_count,
                incomplete_reason,
            } => {
                if let Some(file) = files.remove(&session_id) {
                    let (status, count) = finalize_file(file, expected_count, incomplete_reason);
                    let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
                    st.sessions[session_id].status = status;
                    st.sessions[session_id].count = count;
                }
            }
            Cmd::Shutdown => break,
        }
    }
    maybe_log_overflow(&overflow_noted);
    let fallbacks: Vec<(usize, Option<(u64, &'static str)>)> = {
        let st = state.lock().unwrap_or_else(PoisonError::into_inner);
        files
            .keys()
            .map(|&sid| (sid, st.sessions[sid].fallback_close))
            .collect()
    };
    let mut finalized: Vec<(usize, String, u64)> = Vec::with_capacity(files.len());
    for (session_id, fallback) in fallbacks {
        if let Some(file) = files.remove(&session_id) {
            let (expected, reason) = fallback.unwrap_or((file.written, "unclosed"));
            let (status, count) = finalize_file(file, expected, Some(reason));
            finalized.push((session_id, status, count));
        }
    }
    let sessions = {
        let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
        for (session_id, status, count) in finalized {
            st.sessions[session_id].status = status;
            st.sessions[session_id].count = count;
        }
        st.sessions.clone()
    };
    write_manifest_atomic(&dir, &run_id, &sessions);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_or_panic(run: &CaptureRun, conn: u32, player: &str) -> SessionWriter {
        match run.open_session(conn, player) {
            OpenSession::Opened(_, writer) => writer,
            OpenSession::Sealed => panic!("expected open session"),
        }
    }

    #[test]
    fn split_packet_id_and_meta_hash_payload_after_id() {
        assert_eq!(split_packet_id(&[46, 0, 1]), (46, &[0, 1][..]));
        assert_eq!(split_packet_id(&[0x80, 0x01, 9]), (128, &[9][..]));
        assert_eq!(split_packet_id(&[0]), (0, &[][..]));
        assert_eq!(split_packet_id(&[]), (-1, &[][..]));

        let meta = PacketMeta::from_body(&[0xAC, 0x02, 10, 20, 30]);
        assert_eq!(meta.id, 300);
        assert_eq!(meta.len, 3);
        assert_eq!(meta.hash, xxhash_rust::xxh3::xxh3_128(&[10, 20, 30]));
    }

    #[test]
    fn rejects_nonempty_capture_directory_and_concurrent_claim() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stale.pkt"), b"old\n").unwrap();
        let err = match CaptureRun::open_in(dir.path(), 1) {
            Ok(_) => panic!("expected nonempty directory refusal"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(!dir.path().join(CLAIM_FILE).exists());

        let race_dir = tempfile::tempdir().unwrap();
        let (go1_tx, go1_rx) = std::sync::mpsc::channel();
        let (go2_tx, go2_rx) = std::sync::mpsc::channel();
        let (res1_tx, res1_rx) = std::sync::mpsc::channel();
        let (res2_tx, res2_rx) = std::sync::mpsc::channel();
        let p1 = race_dir.path().to_path_buf();
        let p2 = race_dir.path().to_path_buf();
        let t1 = std::thread::spawn(move || {
            go1_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("timed out waiting for concurrent claim start signal 1");
            let _ = res1_tx.send(CaptureRun::open_in(&p1, 10));
        });
        let t2 = std::thread::spawn(move || {
            go2_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("timed out waiting for concurrent claim start signal 2");
            let _ = res2_tx.send(CaptureRun::open_in(&p2, 11));
        });
        go1_tx.send(()).unwrap();
        go2_tx.send(()).unwrap();
        let r1 = res1_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for concurrent claim opener 1");
        let r2 = res2_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for concurrent claim opener 2");
        t1.join().unwrap();
        t2.join().unwrap();
        let ok_count = usize::from(r1.is_ok()) + usize::from(r2.is_ok());
        assert_eq!(ok_count, 1, "exactly one concurrent opener must claim dir");
        if let Err(e) = &r1 {
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        }
        if let Err(e) = &r2 {
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        }
        drop((r1, r2));
        // Reusing a completed capture directory must also be refused.
        assert!(CaptureRun::open_in(race_dir.path(), 12).is_err());
    }

    #[test]
    fn writes_complete_footer_and_manifest_v2_with_ordinals() {
        let dir = tempfile::tempdir().unwrap();
        let run = CaptureRun::open_in(dir.path(), 3).unwrap();
        run.set_tick(12);
        let mut first = open_or_panic(&run, 1, "Alice");
        first.record_raw(12, CapturedBytes::Owned(vec![8u8, 1, 2]));
        first.close(None);
        let mut second = open_or_panic(&run, 4, "Alice");
        second.record_raw(15, CapturedBytes::Owned(vec![50u8, 9]));
        second.close(None);
        run.finish();

        let manifest = std::fs::read_to_string(dir.path().join("manifest.tsv")).unwrap();
        let mlines: Vec<&str> = manifest.lines().collect();
        assert_eq!(mlines.len(), 4);
        assert!(mlines[0].starts_with("# manifest v2\t"));
        assert!(mlines[0].ends_with("\tok"));
        assert_eq!(mlines[1], "1\t1\tAlice\tAlice-1.pkt\tok\t1");
        assert_eq!(mlines[2], "4\t2\tAlice\tAlice-4.pkt\tok\t1");
        assert_eq!(mlines[3], "# end\t2\tok");

        let claim = std::fs::read_to_string(dir.path().join(CLAIM_FILE)).unwrap();
        let run_id = mlines[0].split('\t').nth(1).unwrap();
        assert_eq!(claim.trim(), run_id);

        let pkt1 = std::fs::read_to_string(dir.path().join("Alice-1.pkt")).unwrap();
        assert!(pkt1.ends_with("# end\t1\tok\n"));
        let pkt2 = std::fs::read_to_string(dir.path().join("Alice-4.pkt")).unwrap();
        assert!(pkt2.ends_with("# end\t1\tok\n"));
    }

    #[test]
    fn byte_budget_and_channel_overflow_mark_run_incomplete_and_refund_budget() {
        // Byte-budget overflow is deterministic regardless of writer scheduling.
        let dir = tempfile::tempdir().unwrap();
        let run = CaptureRun::open_in_with_gate(dir.path(), 20, 16, 32, None).unwrap();
        let mut session = open_or_panic(&run, 1, "ByteCap");
        session.record_raw(1, CapturedBytes::Owned(vec![8u8; 64]));
        session.close(None);
        run.finish();
        assert_eq!(run.byte_budget.used.load(Ordering::Acquire), 0);
        let pkt = std::fs::read_to_string(dir.path().join("ByteCap-1.pkt")).unwrap();
        assert!(pkt.ends_with("# end\t0\tincomplete\toverflow\n"), "{pkt}");
        let manifest = std::fs::read_to_string(dir.path().join("manifest.tsv")).unwrap();
        assert!(manifest.contains("incomplete:overflow"));
        assert!(manifest.ends_with("# end\t1\tincomplete\n"));

        // Channel-capacity overflow while writer is gated.
        let dir2 = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel();
        let run2 =
            CaptureRun::open_in_with_gate(dir2.path(), 21, 2, 1 << 20, Some(gate_rx)).unwrap();
        // Move after `run2` so an unwind drops `release_gate` before `CaptureRun::drop` joins.
        let release_gate = gate_tx;
        let mut session2 = open_or_panic(&run2, 1, "ChanCap");
        // Capacity 2 holds Cmd::Open + 1 packet; the second packet overflows deterministically.
        session2.record_raw(1, CapturedBytes::Owned(vec![8u8, 1]));
        session2.record_raw(2, CapturedBytes::Owned(vec![8u8, 2]));
        session2.close(None);
        drop(release_gate);
        run2.finish();
        assert_eq!(run2.byte_budget.used.load(Ordering::Acquire), 0);
        let pkt2 = std::fs::read_to_string(dir2.path().join("ChanCap-1.pkt")).unwrap();
        assert!(pkt2.ends_with("# end\t1\tincomplete\toverflow\n"), "{pkt2}");
        let manifest2 = std::fs::read_to_string(dir2.path().join("manifest.tsv")).unwrap();
        assert!(manifest2.contains("incomplete:overflow"));
        assert!(manifest2.ends_with("# end\t1\tincomplete\n"));
    }

    #[test]
    fn shutdown_waits_for_delayed_worker_seals_late_logins_and_times_out_stalled_session() {
        // Delayed worker finishes before deadline: tail is preserved and marked ok.
        let dir = tempfile::tempdir().unwrap();
        let run = CaptureRun::open_in(dir.path(), 30).unwrap();
        let mut delayed = open_or_panic(&run, 1, "Delayed");
        let run_for_thread = run.clone();
        let (finish_done_tx, finish_done_rx) = std::sync::mpsc::channel();
        let finish_thread = std::thread::spawn(move || {
            run_for_thread.finish_with_timeout(Duration::from_secs(2));
            let _ = finish_done_tx.send(());
        });
        // Wait until shutdown has sealed registration.
        let seal_deadline = Instant::now() + Duration::from_secs(2);
        while !run.is_sealed() {
            assert!(Instant::now() < seal_deadline, "shutdown did not seal");
            std::thread::yield_now();
        }
        // A late login reaching play after seal is refused cleanly.
        assert!(matches!(
            run.open_session(2, "LateLogin"),
            OpenSession::Sealed
        ));
        std::thread::sleep(Duration::from_millis(40));
        delayed.record_raw(7, CapturedBytes::Owned(vec![12u8, 3, 4]));
        delayed.close(None);
        finish_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for delayed CaptureRun::finish_with_timeout");
        finish_thread.join().unwrap();

        let manifest = std::fs::read_to_string(dir.path().join("manifest.tsv")).unwrap();
        let lines: Vec<&str> = manifest.lines().collect();
        assert_eq!(lines.len(), 3, "{manifest}");
        assert!(lines[0].ends_with("\tok"));
        assert_eq!(lines[1], "1\t1\tDelayed\tDelayed-1.pkt\tok\t1");
        assert_eq!(lines[2], "# end\t1\tok");
        let pkt = std::fs::read_to_string(dir.path().join("Delayed-1.pkt")).unwrap();
        assert!(pkt.ends_with("# end\t1\tok\n"));

        // Stalled session past deadline is marked incomplete:unclosed and late drop does not block.
        let dir2 = tempfile::tempdir().unwrap();
        let run2 = CaptureRun::open_in(dir2.path(), 31).unwrap();
        let mut stalled = open_or_panic(&run2, 1, "Stalled");
        stalled.record_raw(1, CapturedBytes::Owned(vec![12u8, 1]));
        run2.finish_with_timeout(Duration::from_millis(30));
        let manifest2 = std::fs::read_to_string(dir2.path().join("manifest.tsv")).unwrap();
        assert!(manifest2.contains("incomplete:unclosed"), "{manifest2}");
        assert!(manifest2.ends_with("# end\t1\tincomplete\n"));
        let pkt2 = std::fs::read_to_string(dir2.path().join("Stalled-1.pkt")).unwrap();
        assert!(pkt2.ends_with("# end\t1\tincomplete\tunclosed\n"), "{pkt2}");
        // Dropping the late handle after shutdown finishes immediately without stalling.
        drop(stalled);
    }
}
