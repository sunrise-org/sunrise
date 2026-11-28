//! The network threads of the play state: a fixed pool, one thread per two
//! cores, each polling its players' sockets (epoll, kqueue or Windows' AFD,
//! through mio), where every player used to have a reader and a writer thread
//! of its own. The game thread only queues packet bodies in a player's
//! [`Outbox`] (over the network, bounded by [`MAX_PENDING`] bytes); framing, compression,
//! encryption, the socket writes, reading and the keep-alives happen here. A
//! connection stays on one thread, so its bytes keep their order. The
//! integrated server's own player, over an in-memory pipe, is served the same
//! way: the pipe tells the thread when bytes arrive.
use super::Body;
use mio::{Events, Interest, Poll, Registry, Token, Waker};
#[cfg(feature = "net-trace")]
use std::sync::atomic::AtomicI64;
use std::{
    io::{ErrorKind, Read, Write},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use sunrise_protocol::connection::{ConnReader, ConnWriter, Pipe, Socket};

/// Packets queued for one network client beyond this many bytes disconnect it.
pub(super) const MAX_PENDING: usize = 96 << 20;
/// How often a thread looks at its connections' clocks: keep-alives,
/// timeouts and the closes that wait for a disconnect's last bytes.
const SWEEP: Duration = Duration::from_millis(100);
/// Framed bytes a connection may hold unwritten before its thread stops
/// taking more from its outbox: the rest wait there, counted in `pending`, so
/// that a client which reads slowly has the hub hold its chunks back and, past [`MAX_PENDING`], is disconnected.
const AHEAD: usize = 256 << 10;
/// How long a disconnect's last bytes are given before the socket closes.
const LINGER: Duration = Duration::from_millis(50);
const WAKER: Token = Token(usize::MAX);

/// The play state's side of a connection (conn.rs's `PlayConn`).
pub(super) trait Session: Send {
    /// A packet arrived.
    fn packet(&mut self, id: i32, payload: Vec<u8>) -> Verdict;
    /// A packet to send of its own accord now (a keep-alive).
    fn tick(&mut self, now: Instant) -> Option<Vec<u8>>;
    /// The connection is ending, for this reason: no packet arrives after it.
    fn left(&mut self, reason: String);
    /// The connection is closed, after [`Self::left`].
    fn closed(&mut self);
}

/// What becomes of a connection after one of its packets.
pub(super) enum Verdict {
    Go,
    /// Close it now, for this reason.
    Close(String),
    /// Send this body (a disconnect) after what is queued, then close, for this reason.
    Kick(String, Vec<u8>),
}

/// How an outbox ends, after the bodies queued before.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum End {
    /// Write everything queued (a disconnect last), then close.
    Disconnect,
    /// Close now: what is queued is dropped.
    Close,
}

/// Where an outbox's connection is.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum At {
    /// Not on its thread yet: the thread takes what waits when it arrives.
    #[default]
    Coming,
    Slot(usize),
    Closed,
}

#[derive(Default)]
struct Queue {
    bodies: Vec<Queued>,
    #[cfg(feature = "net-trace")]
    trace: Vec<i64>,
    end: Option<End>,
    at: At,
    /// In its thread's list of outboxes to serve.
    listed: bool,
    #[cfg(feature = "test-support")]
    test_published: usize,
    #[cfg(feature = "test-support")]
    // The framed outbox prefix and its endpoint in the complete connection stream.
    test_framed: (usize, i64),
}

// Include allocation capacity and geometric queue growth, even for empty bodies.
pub(super) fn charge(retained: usize) -> usize {
    retained.saturating_add(std::mem::size_of::<Queued>() * 2)
}

/// A pairing payload carries its own prepaid charge until the queued body is dropped.
pub(super) fn body_charge(body: &Body) -> usize {
    if body.is_prepared() {
        0
    } else {
        charge(body.retained())
    }
}

pub(super) struct Queued {
    body: Body,
    prepared: bool,
    pending: Arc<AtomicUsize>,
    cost: usize,
}
impl Drop for Queued {
    fn drop(&mut self) {
        self.pending.fetch_sub(self.cost, Ordering::Relaxed);
    }
}

/// One player's packets on their way to its network thread.
pub(super) struct Outbox {
    #[cfg(any(test, feature = "test-payload-probe"))]
    // write attempts, nonzero write successes, bytes written, WouldBlock,
    // nonempty buffers fully drained, remaining output bytes,
    // successful body-framing calls since drain, lifetime body-framing calls.
    // Body-framing calls are frames for raw payloads, not necessarily adapters.
    probe: [AtomicUsize; 8],
    queue: Mutex<Queue>,
    /// The connection's `Droppable` sends, framed by its thread when it has room.
    droppable: OnceLock<Arc<crate::raw_payloads::droppable::Queue>>,
    #[cfg(feature = "net-trace")]
    capture_clock: OnceLock<Arc<AtomicI64>>,
    #[cfg(feature = "net-trace")]
    capture: Mutex<Option<super::packet_capture::SessionWriter>>,
    /// The bytes queued and not framed yet (`PlayerSender::pending`).
    pending: Arc<AtomicUsize>,
    /// Whether [`MAX_PENDING`] applies. A client in this process (a memory connection) reads
    /// what it is sent as it comes, and a join's first view can pass the limit in one burst.
    /// The reader is this process: if it stopped reading, that would be a bug of its own.
    capped: bool,
    worker: Arc<Worker>,
}
impl Outbox {
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(super) fn diagnostics(&self) -> String {
        match self.queue.try_lock() {
            Ok(q) => format!(
                "outbox queued_items={} queued_body_bytes={} closed={} end={:?} listed={} write_attempts={} write_ok_calls={} written_bytes={} would_block={} drained_nonempty_buffers={} output_remaining_bytes={} body_frame_ok_since_drain={} body_frame_ok_total={}",
                q.bodies.len(),
                q.bodies.iter().map(|b| b.body.len()).sum::<usize>(),
                matches!(q.at, At::Closed),
                q.end,
                q.listed,
                self.probe[0].load(Ordering::Relaxed),
                self.probe[1].load(Ordering::Relaxed),
                self.probe[2].load(Ordering::Relaxed),
                self.probe[3].load(Ordering::Relaxed),
                self.probe[4].load(Ordering::Relaxed),
                self.probe[5].load(Ordering::Relaxed),
                self.probe[6].load(Ordering::Relaxed),
                self.probe[7].load(Ordering::Relaxed)
            ),
            Err(_) => "outbox busy/poisoned".into(),
        }
    }
    /// An outbox on the least busy network thread, started the first time.
    pub(super) fn new(pending: Arc<AtomicUsize>, capped: bool) -> Result<Arc<Self>, String> {
        let workers = &pool()?.workers;
        let worker = workers
            .iter()
            .min_by_key(|w| w.load.load(Ordering::Relaxed))
            .expect("a pool has a thread");
        Ok(Arc::new(Self {
            #[cfg(any(test, feature = "test-payload-probe"))]
            probe: std::array::from_fn(|_| AtomicUsize::new(0)),
            queue: Mutex::default(),
            droppable: OnceLock::new(),
            #[cfg(feature = "net-trace")]
            capture_clock: OnceLock::new(),
            #[cfg(feature = "net-trace")]
            capture: Mutex::new(None),
            pending,
            capped,
            worker: worker.clone(),
        }))
    }
    #[cfg(feature = "net-trace")]
    pub(super) fn attach_capture(
        &self,
        clock: Arc<AtomicI64>,
        writer: super::packet_capture::SessionWriter,
    ) {
        let _ = self.capture_clock.set(clock);
        *self.capture.lock().unwrap_or_else(PoisonError::into_inner) = Some(writer);
    }
    #[cfg(feature = "net-trace")]
    pub(super) fn take_capture(&self) -> Option<super::packet_capture::SessionWriter> {
        self.capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
    /// The packets queued and not taken by the thread yet.
    pub(super) fn queued(&self) -> usize {
        self.queue().bodies.len()
    }
    #[cfg(feature = "test-support")]
    pub(super) fn test_published(&self) -> usize {
        self.queue().test_published
    }

    #[cfg(feature = "test-support")]
    pub(super) fn test_framed_through(&self, published: usize) -> Option<i64> {
        let (framed, wire_packets) = self.queue().test_framed;
        (framed >= published).then_some(wire_packets)
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Queue `bodies` (already counted in `pending`), then `end`. Nothing
    /// follows an end, but a close overrides a disconnect still writing.
    /// Past [`MAX_PENDING`] a capped connection closes. The thread to wake, if
    /// this is the first news it has since it last looked.
    pub(super) fn push(&self, bodies: Vec<Body>, end: Option<End>) -> Option<&Arc<Worker>> {
        self.push_checked(bodies, end).ok().flatten()
    }
    // An accepted send may need no wake: its worker is already listed or not attached yet.
    fn push_checked(
        &self,
        bodies: Vec<Body>,
        end: Option<End>,
    ) -> Result<Option<&Arc<Worker>>, ()> {
        let bodies = bodies
            .into_iter()
            .map(|body| Queued {
                cost: body_charge(&body),
                prepared: body.is_prepared(),
                body,
                pending: self.pending.clone(),
            })
            .collect::<Vec<_>>();
        let mut q = self.queue();
        match (q.at, q.end) {
            (At::Closed, _) => return Err(()),
            (_, None) => {
                #[cfg(feature = "test-support")]
                {
                    q.test_published += bodies.len();
                }
                #[cfg(feature = "net-trace")]
                if let Some(clock) = self.capture_clock.get() {
                    let tick = clock.load(Ordering::Relaxed);
                    let new_len = q.trace.len() + bodies.len();
                    q.trace.resize(new_len, tick);
                }
                if q.bodies.is_empty() {
                    q.bodies = bodies;
                } else {
                    q.bodies.extend(bodies);
                }
                q.end = if self.capped && self.pending.load(Ordering::Relaxed) > MAX_PENDING {
                    Some(End::Close)
                } else {
                    end
                };
            }
            (_, Some(End::Disconnect)) if end == Some(End::Close) => q.end = end,
            (_, Some(_)) => return Err(()),
        }
        let At::Slot(slot) = q.at else {
            return Ok(None);
        };
        if std::mem::replace(&mut q.listed, true) {
            return Ok(None);
        }
        drop(q);
        Ok(self
            .worker
            .post(|i| i.serve.push(slot))
            .then_some(&self.worker))
    }
    /// [`Self::push`], waking the thread at once.
    pub(super) fn send(&self, bodies: Vec<Body>, end: Option<End>) {
        self.try_send(bodies, end);
    }
    /// Whether the already reserved bodies were admitted, independently of waking a worker.
    pub(super) fn try_send(&self, bodies: Vec<Body>, end: Option<End>) -> bool {
        let Ok(wake) = self.push_checked(bodies, end) else {
            return false;
        };
        if let Some(worker) = wake {
            worker.wake();
        }
        true
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(super) fn queued_contents(&self) -> Vec<Vec<u8>> {
        self.queue()
            .bodies
            .iter()
            .map(|b| b.body.to_vec())
            .collect()
    }
    pub(super) fn alive(&self) -> bool {
        let q = self.queue();
        !matches!(q.at, At::Closed) && q.end.is_none()
    }
    pub(super) fn publish(&self, body: Vec<u8>, cost: usize) -> bool {
        let packet = Queued {
            body: body.into(),
            cost,
            prepared: true,
            pending: self.pending.clone(),
        };
        let mut q = self.queue();
        if matches!(q.at, At::Closed) || q.end.is_some() {
            return false;
        }
        q.bodies.push(packet);
        #[cfg(feature = "net-trace")]
        if let Some(clock) = self.capture_clock.get() {
            q.trace.push(clock.load(Ordering::Relaxed));
        }
        #[cfg(feature = "test-support")]
        {
            q.test_published += 1;
        }
        self.list(q);
        true
    }
    /// Have the thread look at this outbox, if it is on one and not listed already.
    fn list(&self, mut q: MutexGuard<'_, Queue>) {
        let At::Slot(slot) = q.at else {
            return;
        };
        if std::mem::replace(&mut q.listed, true) {
            return;
        }
        drop(q);
        if self.worker.post(|i| i.serve.push(slot)) {
            self.worker.wake();
        }
    }
    /// Serve `queue`'s `Droppable` sends on this connection's thread.
    pub(super) fn attach_droppable(
        self: &Arc<Self>,
        queue: Arc<crate::raw_payloads::droppable::Queue>,
    ) {
        let this = Arc::downgrade(self);
        queue.set_wake(Box::new(move || {
            if let Some(outbox) = this.upgrade() {
                let q = outbox.queue();
                if !matches!(q.at, At::Closed) && q.end.is_none() {
                    outbox.list(q);
                }
            }
        }));
        let _ = self.droppable.set(queue);
    }
    /// What waits, for the thread.
    /// What waits, for the thread: the bodies and the end after them with
    /// `room` for them, else only a close. An end stays, so that nothing is
    /// queued after it.
    #[cfg(not(feature = "net-trace"))]
    fn take(&self, room: bool) -> (Vec<Queued>, Option<End>) {
        let mut q = self.queue();
        q.listed = false;
        match room {
            true => (std::mem::take(&mut q.bodies), q.end),
            false => (Vec::new(), q.end.filter(|e| *e == End::Close)),
        }
    }
    #[cfg(feature = "net-trace")]
    fn take(&self, room: bool) -> (Vec<Queued>, Vec<i64>, Option<End>) {
        let mut q = self.queue();
        q.listed = false;
        match room {
            true => (
                std::mem::take(&mut q.bodies),
                std::mem::take(&mut q.trace),
                q.end,
            ),
            false => (Vec::new(), Vec::new(), q.end.filter(|e| *e == End::Close)),
        }
    }
    fn attach(&self, slot: usize) {
        let mut q = self.queue();
        q.at = At::Slot(slot);
        q.listed = false;
    }
    fn closed(&self) {
        let mut q = self.queue();
        q.at = At::Closed;
        q.bodies = Vec::new();
        #[cfg(feature = "net-trace")]
        q.trace.clear();
        q.end = None;
    }
}

/// What a network thread is told between polls.
#[derive(Default)]
struct Inbox {
    /// Connections with something in their outbox.
    serve: Vec<usize>,
    /// Pipes with bytes to read.
    read: Vec<usize>,
    adopt: Vec<Adopted>,
}

/// One network thread, as the others see it.
pub(super) struct Worker {
    waker: Waker,
    inbox: Mutex<Inbox>,
    /// Its connections, to give a new one to the least busy.
    load: AtomicUsize,
}
impl Worker {
    /// Change the inbox: whether the thread needs waking for it (it was empty).
    fn post(&self, f: impl FnOnce(&mut Inbox)) -> bool {
        let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        let empty = inbox.serve.is_empty() && inbox.read.is_empty() && inbox.adopt.is_empty();
        f(&mut inbox);
        empty
    }
    pub(super) fn wake(&self) {
        if let Err(e) = self.waker.wake() {
            log::warn!("[lan] waking a network thread: {e}");
        }
    }
}

struct Pool {
    workers: Vec<Arc<Worker>>,
}

/// The pool, started the first time: one thread per two cores.
fn pool() -> Result<&'static Pool, String> {
    static POOL: OnceLock<Result<Pool, String>> = OnceLock::new();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().div_ceil(2));
    POOL.get_or_init(|| start(threads).map_err(|e| format!("network threads: {e}")))
        .as_ref()
        .map_err(Clone::clone)
}

fn start(threads: usize) -> std::io::Result<Pool> {
    let mut workers = Vec::new();
    for n in 0..threads {
        let poll = Poll::new()?;
        let worker = Arc::new(Worker {
            waker: Waker::new(poll.registry(), WAKER)?,
            inbox: Mutex::default(),
            load: AtomicUsize::new(0),
        });
        let mine = worker.clone();
        // Mod payload handlers run on these threads (the server's network root).
        std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name(format!("sunrise-net-{n}"))
            .spawn(move || run(poll, &mine))?;
        workers.push(worker);
    }
    Ok(Pool { workers })
}

/// A connection handed to its thread.
struct Adopted {
    socket: Socket,
    reader: ConnReader,
    writer: ConnWriter,
    outbox: Arc<Outbox>,
    session: Box<dyn Session>,
    timeout: Duration,
}

/// Serve a connection from now on, on its outbox's thread: `timeout` is how
/// long it may read nothing, or have a write wait.
pub(super) fn adopt(
    socket: Socket,
    (reader, writer): (ConnReader, ConnWriter),
    outbox: Arc<Outbox>,
    session: Box<dyn Session>,
    timeout: Duration,
) {
    let worker = outbox.worker.clone();
    worker.load.fetch_add(1, Ordering::Relaxed);
    let adopted = Adopted {
        socket,
        reader,
        writer,
        outbox,
        session,
        timeout,
    };
    if worker.post(|i| i.adopt.push(adopted)) {
        worker.wake();
    }
}

fn run(mut poll: Poll, worker: &Arc<Worker>) {
    let mut events = Events::with_capacity(1024);
    let mut conns: Vec<Option<Conn>> = Vec::new();
    let mut free: Vec<usize> = Vec::new();
    let mut touched: Vec<usize> = Vec::new();
    let mut sweep_at = Instant::now() + SWEEP;
    loop {
        let wait = sweep_at.saturating_duration_since(Instant::now());
        if let Err(e) = poll.poll(&mut events, Some(wait))
            && e.kind() != ErrorKind::Interrupted
        {
            log::warn!("[lan] network poll: {e}");
            std::thread::sleep(SWEEP);
        }
        let now = Instant::now();
        for event in &events {
            let i = event.token().0;
            let Some(Some(c)) = conns.get_mut(i) else {
                continue;
            };
            if event.is_readable() || event.is_read_closed() || event.is_error() {
                c.read(now);
            }
            if event.is_writable() || event.is_write_closed() || event.is_error() {
                c.serve(now);
            }
            touched.push(i);
        }
        let inbox =
            std::mem::take(&mut *worker.inbox.lock().unwrap_or_else(PoisonError::into_inner));
        for adopted in inbox.adopt {
            let i = free.pop().unwrap_or(conns.len());
            if i == conns.len() {
                conns.push(None);
            }
            match Conn::register(adopted, poll.registry(), worker, i, now) {
                Ok(mut c) => {
                    c.outbox.attach(i);
                    c.read(now);
                    c.serve(now);
                    conns[i] = Some(c);
                    touched.push(i);
                }
                Err(()) => free.push(i),
            }
        }
        for i in inbox.read {
            if let Some(Some(c)) = conns.get_mut(i) {
                c.memory_ready(now);
                touched.push(i);
            }
        }
        for i in inbox.serve {
            if let Some(Some(c)) = conns.get_mut(i) {
                c.serve(now);
                touched.push(i);
            }
        }
        if now >= sweep_at {
            sweep_at = now + SWEEP;
            touched.clear();
            for (i, slot) in conns.iter_mut().enumerate() {
                if let Some(c) = slot {
                    c.sweep(now);
                    touched.push(i);
                }
            }
        }
        for i in touched.drain(..) {
            if conns[i].as_ref().is_some_and(|c| c.done(now)) {
                let c = conns[i].take().expect("checked");
                c.close(poll.registry(), worker);
                free.push(i);
            }
        }
    }
}

/// A connection's stream, polled.
enum Io {
    Tcp(mio::net::TcpStream),
    Memory(Pipe),
}
impl Read for Io {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(out),
            Self::Memory(p) => p.read(out),
        }
    }
}
impl Write for Io {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(data),
            Self::Memory(p) => p.write(data),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Why a read error ends a connection: a client that closed it is
/// Connection.channelInactive without details, multiplayer.disconnect.generic.
fn read_error(e: std::io::Error) -> String {
    match e.kind() {
        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => "Disconnected".to_owned(),
        _ => sunrise_protocol::Error::from(e).to_string(),
    }
}

/// A connection on its thread.
struct Conn {
    io: Io,
    reader: ConnReader,
    writer: ConnWriter,
    /// Framed bytes, written from `written` on.
    out: Vec<u8>,
    written: usize,
    outbox: Arc<Outbox>,
    session: Box<dyn Session>,
    timeout: Duration,
    last_read: Instant,
    /// Since when `out` has waited on a client that reads nothing.
    stalled: Option<Instant>,
    /// [`Session::left`] was called: what arrives now is dropped.
    left: bool,
    /// A disconnect is in `out`: nothing more is taken from the outbox.
    draining: bool,
    /// A bundle (between two `BUNDLE_DELIMITER` packets) is open on the wire: no `Droppable` packet
    /// goes between them.
    bundling: bool,
    close_at: Option<Instant>,
    #[cfg(feature = "net-trace")]
    capture: Option<super::packet_capture::SessionWriter>,
    #[cfg(feature = "net-trace")]
    frame_error: bool,
    #[cfg(feature = "net-trace")]
    write_error: bool,
}
impl Conn {
    /// Pipe readiness covers incoming data and capacity returned by peer reads.
    fn memory_ready(&mut self, now: Instant) {
        self.read(now);
        self.serve(now);
    }

    fn register(
        a: Adopted,
        registry: &Registry,
        worker: &Arc<Worker>,
        i: usize,
        now: Instant,
    ) -> Result<Self, ()> {
        let io = match a.socket {
            Socket::Tcp(stream) => {
                let registered = stream.set_nonblocking(true).and_then(|()| {
                    let mut stream = mio::net::TcpStream::from_std(stream);
                    let interest = Interest::READABLE | Interest::WRITABLE;
                    registry.register(&mut stream, Token(i), interest)?;
                    Ok(stream)
                });
                match registered {
                    Ok(stream) => Io::Tcp(stream),
                    Err(e) => {
                        let mut session = a.session;
                        a.outbox.closed();
                        #[cfg(feature = "net-trace")]
                        if let Some(capture) = a.outbox.take_capture() {
                            capture.close(Some("register_failed"));
                        }
                        session.left(e.to_string());
                        session.closed();
                        worker.load.fetch_sub(1, Ordering::Relaxed);
                        return Err(());
                    }
                }
            }
            Socket::Memory(mut pipe) => {
                pipe.set_nonblocking(true);
                let worker = worker.clone();
                pipe.on_ready(move || {
                    if worker.post(|inbox| inbox.read.push(i)) {
                        worker.wake();
                    }
                });
                Io::Memory(pipe)
            }
        };
        #[cfg(feature = "net-trace")]
        let capture = a.outbox.take_capture();
        Ok(Self {
            io,
            reader: a.reader,
            writer: a.writer,
            out: Vec::new(),
            written: 0,
            outbox: a.outbox,
            session: a.session,
            timeout: a.timeout,
            last_read: now,
            stalled: None,
            left: false,
            draining: false,
            bundling: false,
            close_at: None,
            #[cfg(feature = "net-trace")]
            capture,
            #[cfg(feature = "net-trace")]
            frame_error: false,
            #[cfg(feature = "net-trace")]
            write_error: false,
        })
    }

    fn left(&mut self, reason: String) {
        if !std::mem::replace(&mut self.left, true) {
            self.session.left(reason);
        }
    }
    /// Close now.
    fn end(&mut self, now: Instant, reason: impl Into<String>) {
        self.left(reason.into());
        self.close_at = Some(now);
    }
    fn done(&self, now: Instant) -> bool {
        self.close_at.is_some_and(|at| now >= at)
    }

    /// Read and hand over every packet that arrived.
    fn read(&mut self, now: Instant) {
        loop {
            match self.reader.buffered() {
                Ok(Some((id, payload))) => {
                    if !self.left {
                        match self.session.packet(id, payload) {
                            Verdict::Go => {}
                            Verdict::Close(reason) => return self.end(now, reason),
                            Verdict::Kick(reason, body) => self.kick(now, reason, &body),
                        }
                    }
                    continue;
                }
                Ok(None) => {}
                Err(e) => return self.end(now, e.to_string()),
            }
            if self.close_at.is_some() {
                return;
            }
            match self.reader.read_from(&mut self.io) {
                Ok(0) => return self.end(now, "Disconnected"),
                Ok(_) => self.last_read = now,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return self.end(now, read_error(e)),
            }
        }
    }

    fn frame_one(&mut self, now: Instant, body: &Queued) -> bool {
        // Do not insert a droppable packet between a bundle's delimiters.
        if self.outbox.droppable.get().is_some()
            && !body.prepared
            && body.body.first()
                == Some(&(sunrise_protocol::ids::play::clientbound::BUNDLE_DELIMITER as u8))
        {
            self.bundling = !self.bundling;
        }
        let result = if body.prepared {
            self.writer.frame_into(&body.body, &mut self.out);
            Ok(())
        } else {
            super::conn::frame_body(&mut self.writer, &body.body, &mut self.out)
        };
        if let Err(e) = result {
            log::warn!("[lan] failed to frame outgoing packet: {e}");
            #[cfg(feature = "net-trace")]
            {
                self.frame_error = true;
            }
            self.end(now, e.to_string());
            return false;
        }
        true
    }

    #[cfg(any(test, feature = "test-payload-probe"))]
    fn record_frame_probe(&self) {
        self.outbox.probe[6].fetch_add(1, Ordering::Relaxed);
        self.outbox.probe[7].fetch_add(1, Ordering::Relaxed);
        self.outbox.probe[5].store(self.out.len() - self.written, Ordering::Relaxed);
    }

    #[cfg(feature = "net-trace")]
    fn record_queued(
        capture: &mut super::packet_capture::SessionWriter,
        framing: sunrise_protocol::connection::Framing,
        tick: i64,
        body: &mut Queued,
    ) {
        use super::packet_capture::CapturedBytes;
        if body.prepared {
            match &mut body.body {
                Body::Owned(bytes) => {
                    capture.record_prepared(
                        tick,
                        CapturedBytes::Owned(std::mem::take(bytes)),
                        framing,
                    );
                }
                Body::Shared(bytes) => {
                    capture.record_prepared(
                        tick,
                        CapturedBytes::Shared(Arc::clone(bytes)),
                        framing,
                    );
                }
                Body::Prepared(pairing) => {
                    capture.record_prepared_slice(tick, &pairing.bytes, framing);
                }
            }
        } else {
            match &mut body.body {
                Body::Owned(bytes) => {
                    capture.record_raw(tick, CapturedBytes::Owned(std::mem::take(bytes)));
                }
                Body::Shared(bytes) => {
                    capture.record_raw(tick, CapturedBytes::Shared(Arc::clone(bytes)));
                }
                Body::Prepared(_) => {
                    capture.mark_incomplete("unexpected_prepared");
                }
            }
        }
    }

    /// Frame `bodies` into `out`, `queued` ones counted off `pending`: false
    /// if one could not be.
    fn frame(
        &mut self,
        now: Instant,
        #[cfg(not(feature = "net-trace"))] bodies: Vec<Queued>,
        #[cfg(feature = "net-trace")] mut bodies: Vec<Queued>,
        #[cfg(feature = "net-trace")] trace: Vec<i64>,
        _queued: bool,
    ) -> bool {
        #[cfg(not(feature = "net-trace"))]
        for body in &bodies {
            if !self.frame_one(now, body) {
                return false;
            }
            #[cfg(any(test, feature = "test-payload-probe"))]
            self.record_frame_probe();
        }
        #[cfg(feature = "net-trace")]
        {
            let mut ticks = trace.into_iter();
            for body in &mut bodies {
                let tick = ticks.next();
                if !self.frame_one(now, body) {
                    return false;
                }
                if let Some(capture) = &mut self.capture {
                    let tick = tick.unwrap_or_else(|| capture.current_tick());
                    Self::record_queued(capture, self.writer.framing(), tick, body);
                }
                #[cfg(any(test, feature = "test-payload-probe"))]
                self.record_frame_probe();
            }
        }
        #[cfg(feature = "test-support")]
        if _queued && !bodies.is_empty() {
            let mut q = self.outbox.queue();
            q.test_framed = (
                q.test_framed.0 + bodies.len(),
                self.reader.counters.sent.load(Ordering::Relaxed),
            );
        }
        true
    }

    /// Send `disconnect` after everything queued, then close.
    fn kick(&mut self, now: Instant, reason: String, disconnect: &[u8]) {
        self.left(reason);
        if !self.draining {
            #[cfg(not(feature = "net-trace"))]
            let (bodies, end) = self.outbox.take(true);
            #[cfg(feature = "net-trace")]
            let (bodies, trace, end) = self.outbox.take(true);
            if end == Some(End::Close) {
                return self.end(now, "Disconnected");
            }
            // The hub's own disconnect, if it was the last queued, does for this one.
            let ours = (end.is_none()).then(|| Body::from(disconnect.to_vec()));
            if !self.frame(
                now,
                bodies,
                #[cfg(feature = "net-trace")]
                trace,
                true,
            ) || !self.frame(
                now,
                ours.into_iter()
                    .map(|body| Queued {
                        body,
                        prepared: false,
                        pending: self.outbox.pending.clone(),
                        cost: 0,
                    })
                    .collect(),
                #[cfg(feature = "net-trace")]
                Vec::new(),
                false,
            ) {
                return;
            }
            self.draining = true;
        }
        self.serve(now);
    }

    /// Write what is framed, and frame what the outbox holds, while the
    /// client takes it.
    fn serve(&mut self, now: Instant) {
        loop {
            if !self.flush(now) || self.close_at.is_some() {
                return;
            }
            // A close is seen even while the client takes nothing.
            let room = !self.draining && self.out.len() - self.written <= AHEAD;
            // Droppable sends go first while there is room: they are small, and late is
            // worthless; vanilla's packets keep their own order.
            if room
                && !self.bundling
                && let Some(queue) = self.outbox.droppable.get().cloned()
                && let Some(body) = queue.pop(now)
            {
                #[cfg(not(feature = "net-trace"))]
                if let Err(e) =
                    super::conn::frame_body(&mut self.writer, &Body::from(body), &mut self.out)
                {
                    return self.end(now, e.to_string());
                }
                #[cfg(feature = "net-trace")]
                {
                    let body = Body::from(body);
                    if let Err(e) = super::conn::frame_body(&mut self.writer, &body, &mut self.out)
                    {
                        self.frame_error = true;
                        return self.end(now, e.to_string());
                    }
                    if let Some(capture) = &mut self.capture {
                        let tick = capture.current_tick();
                        if let Body::Owned(bytes) = body {
                            capture.record_raw(
                                tick,
                                super::packet_capture::CapturedBytes::Owned(bytes),
                            );
                        }
                    }
                }
                continue;
            }
            #[cfg(not(feature = "net-trace"))]
            let (bodies, end) = self.outbox.take(room);
            #[cfg(feature = "net-trace")]
            let (bodies, trace, end) = self.outbox.take(room);
            if end == Some(End::Close) {
                return self.end(now, "Disconnected");
            }
            if bodies.is_empty() {
                break;
            }
            if !self.frame(
                now,
                bodies,
                #[cfg(feature = "net-trace")]
                trace,
                true,
            ) {
                return;
            }
            self.draining = end == Some(End::Disconnect);
        }
        if self.draining && self.written == self.out.len() && self.close_at.is_none() {
            self.close_at = Some(now + LINGER);
        }
    }

    /// Write what is framed until the client takes no more: false once the
    /// connection failed.
    fn flush(&mut self, now: Instant) -> bool {
        if self.close_at.is_some_and(|at| at <= now) {
            return false;
        }
        #[cfg(any(test, feature = "test-payload-probe"))]
        let had_output = self.written < self.out.len();
        while self.written < self.out.len() {
            #[cfg(any(test, feature = "test-payload-probe"))]
            self.outbox.probe[0].fetch_add(1, Ordering::Relaxed);
            match self.io.write(&self.out[self.written..]) {
                Ok(0) => {
                    #[cfg(feature = "net-trace")]
                    {
                        self.write_error = true;
                    }
                    self.end(now, "Disconnected");
                    return false;
                }
                Ok(n) => {
                    self.written += n;
                    #[cfg(any(test, feature = "test-payload-probe"))]
                    {
                        self.outbox.probe[1].fetch_add(1, Ordering::Relaxed);
                        self.outbox.probe[2].fetch_add(n, Ordering::Relaxed);
                        self.outbox.probe[5]
                            .store(self.out.len() - self.written, Ordering::Relaxed);
                    }
                    self.stalled = None;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    #[cfg(any(test, feature = "test-payload-probe"))]
                    self.outbox.probe[3].fetch_add(1, Ordering::Relaxed);
                    self.stalled.get_or_insert(now);
                    return true;
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    #[cfg(feature = "net-trace")]
                    {
                        self.write_error = true;
                    }
                    self.end(now, "Disconnected");
                    return false;
                }
            }
        }
        #[cfg(any(test, feature = "test-payload-probe"))]
        {
            if had_output {
                self.outbox.probe[4].fetch_add(1, Ordering::Relaxed);
            }
            self.outbox.probe[6].store(0, Ordering::Relaxed);
        }
        self.out.clear();
        self.written = 0;
        self.stalled = None;
        if self.out.capacity() > 4 * AHEAD {
            self.out.shrink_to(AHEAD);
        }
        true
    }

    /// The connection's clocks: a keep-alive due, a network client that sent
    /// nothing or took nothing for too long.
    fn sweep(&mut self, now: Instant) {
        if self.close_at.is_some() {
            return;
        }
        // Only a network connection times out: `ServerConnectionListener.startTcpServerListener`
        // puts a `ReadTimeoutHandler(30)` on its pipeline, while `startMemoryChannel`'s
        // (`Connection.configureInMemoryPipeline`) has none.
        if matches!(self.io, Io::Tcp(_)) {
            if self
                .stalled
                .is_some_and(|t| now.duration_since(t) > self.timeout)
            {
                return self.end(now, "Disconnected");
            }
            if !self.left && now.duration_since(self.last_read) > self.timeout {
                return self.end(now, sunrise_protocol::Error::timed_out().to_string());
            }
        }
        if !self.draining
            && let Some(body) = self.session.tick(now)
            && self.frame(
                now,
                vec![Queued {
                    body: body.into(),
                    prepared: false,
                    pending: self.outbox.pending.clone(),
                    cost: 0,
                }],
                #[cfg(feature = "net-trace")]
                Vec::new(),
                false,
            )
        {
            self.serve(now);
        }
    }

    fn close(mut self, registry: &Registry, worker: &Worker) {
        match &mut self.io {
            Io::Tcp(stream) => {
                let _ = registry.deregister(stream);
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            Io::Memory(pipe) => {
                pipe.on_ready(|| {});
                pipe.shutdown();
            }
        }
        self.outbox.closed();
        #[cfg(feature = "net-trace")]
        if let Some(capture) = self.capture.take() {
            let reason = if self.frame_error {
                Some("frame_error")
            } else if self.write_error {
                Some("write_error")
            } else if self.written < self.out.len() {
                Some("unflushed_tail")
            } else {
                None
            };
            capture.close(reason);
        }
        self.left("Disconnected".to_owned());
        self.session.closed();
        worker.load.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use sunrise_protocol::connection::Connection;

    #[derive(Debug, PartialEq)]
    enum Seen {
        Packet(i32, Vec<u8>),
        Left(String),
        Closed,
    }
    struct Probe {
        seen: Sender<Seen>,
        /// The packet id answered with a kick.
        kick_on: Option<i32>,
        /// Sent at the first sweep.
        unprompted: Option<Vec<u8>>,
    }
    impl Session for Probe {
        fn packet(&mut self, id: i32, payload: Vec<u8>) -> Verdict {
            let _ = self.seen.send(Seen::Packet(id, payload));
            if self.kick_on == Some(id) {
                Verdict::Kick("kicked".into(), body(0x7f, 3))
            } else {
                Verdict::Go
            }
        }
        fn tick(&mut self, _: Instant) -> Option<Vec<u8>> {
            self.unprompted.take()
        }
        fn left(&mut self, reason: String) {
            let _ = self.seen.send(Seen::Left(reason));
        }
        fn closed(&mut self) {
            let _ = self.seen.send(Seen::Closed);
        }
    }

    /// A packet body: a one-byte id, then `len` bytes.
    fn body(id: u8, len: usize) -> Vec<u8> {
        std::iter::once(id)
            .chain((0..len).map(|j| (j * 31 + usize::from(id)) as u8))
            .collect()
    }
    fn packet(body: &[u8]) -> (i32, Vec<u8>) {
        (i32::from(body[0]), body[1..].to_vec())
    }

    struct Served {
        outbox: Arc<Outbox>,
        pending: Arc<AtomicUsize>,
        seen: Receiver<Seen>,
    }
    impl Served {
        fn send(&self, bodies: Vec<Vec<u8>>, end: Option<End>) {
            let bodies: Vec<Body> = bodies.into_iter().map(Body::from).collect();
            let len: usize = bodies.iter().map(|b| charge(b.retained())).sum();
            self.pending.fetch_add(len, Ordering::Relaxed);
            self.outbox.send(bodies, end);
        }
        fn next(&self) -> Seen {
            self.seen.recv_timeout(Duration::from_secs(10)).unwrap()
        }
    }
    fn serve(
        server: Connection,
        probe: impl FnOnce(Sender<Seen>) -> Probe,
        timeout: Duration,
    ) -> Served {
        serve_with(server, probe, timeout, true)
    }
    fn serve_with(
        server: Connection,
        probe: impl FnOnce(Sender<Seen>) -> Probe,
        timeout: Duration,
        capped: bool,
    ) -> Served {
        let pending = Arc::new(AtomicUsize::new(0));
        let outbox = Outbox::new(pending.clone(), capped).unwrap();
        let (tx, seen) = channel();
        let (socket, reader, writer) = server.detach();
        adopt(
            socket,
            (reader, writer),
            outbox.clone(),
            Box::new(probe(tx)),
            timeout,
        );
        Served {
            outbox,
            pending,
            seen,
        }
    }
    fn quiet(seen: Sender<Seen>) -> Probe {
        Probe {
            seen,
            kick_on: None,
            unprompted: None,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_socket_carries_packets_both_ways_in_order_then_disconnects() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let mut server = Connection::from_stream(listener.accept().unwrap().0).unwrap();
        for c in [&mut client, &mut server] {
            c.set_compression(64);
            c.enable_encryption(&[5; 16]);
        }
        // The second arrives with the first, before the pool has the connection.
        client.send(&body(1, 10)).unwrap();
        client.send(&body(2, 10)).unwrap();
        assert_eq!(server.recv().unwrap(), packet(&body(1, 10)));
        let served = serve(server, quiet, Duration::from_secs(30));
        assert_eq!(served.next(), Seen::Packet(2, body(2, 10)[1..].to_vec()));
        // Small, compressed and shared (past 8 KiB) frames.
        let bodies: Vec<Vec<u8>> = (0..60)
            .map(|i| body(i as u8 + 3, [0, 63, 64, 300, 9000, 70_000][i % 6]))
            .collect();
        served.send(bodies.clone(), None);
        for b in &bodies {
            assert_eq!(client.recv().unwrap(), packet(b));
        }
        for i in 0..20u8 {
            client.send(&body(i + 3, 50)).unwrap();
        }
        for i in 0..20u8 {
            assert_eq!(
                served.next(),
                Seen::Packet(i32::from(i + 3), body(i + 3, 50)[1..].to_vec())
            );
        }
        assert_eq!(served.pending.load(Ordering::Relaxed), 0);
        served.send(vec![body(0x1d, 5)], Some(End::Disconnect));
        assert_eq!(client.recv().unwrap(), packet(&body(0x1d, 5)));
        assert!(client.recv().unwrap_err().is_end_of_stream());
        assert_eq!(served.next(), Seen::Left("Disconnected".into()));
        assert_eq!(served.next(), Seen::Closed);
    }

    /// Every packet the client reads until a closing sentinel (id 0x7e), as (id, payload).
    fn read_until_sentinel(client: &mut Connection) -> Vec<(i32, Vec<u8>)> {
        let mut all = Vec::new();
        loop {
            let packet = client.recv().unwrap();
            if packet.0 == 0x7e {
                return all;
            }
            all.push(packet);
        }
    }

    #[test]
    fn a_droppable_packet_never_lands_inside_a_bundle() {
        use crate::raw_payloads::droppable::Queue;
        let delimiter = 0u8;
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        let served = serve(Connection::local(b), quiet, Duration::from_secs(30));
        let queue = Queue::new(u32::MAX, None);
        served.outbox.attach_droppable(queue.clone());
        let counters = Arc::default();
        // The opening delimiter is on the wire and the thread is idle between batches when the
        // droppable frame arrives. The thread looks at the droppable queue before it takes the
        // next batch, so a probe packet read back proves it declined to frame the droppable one.
        served.send(vec![body(delimiter, 0)], None);
        assert_eq!(client.recv().unwrap(), packet(&body(delimiter, 0)));
        queue.push(Instant::now(), &counters, body(0x40, 4));
        served.send(vec![body(0x7d, 0)], None);
        assert_eq!(client.recv().unwrap(), packet(&body(0x7d, 0)));
        // Once the bundle closes, the waiting frame follows it.
        served.send(vec![body(0x30, 3), body(delimiter, 0)], None);
        assert_eq!(client.recv().unwrap(), packet(&body(0x30, 3)));
        assert_eq!(client.recv().unwrap(), packet(&body(delimiter, 0)));
        assert_eq!(client.recv().unwrap(), packet(&body(0x40, 4)));
    }

    #[test]
    fn a_dropped_droppable_leaves_the_framed_stream_as_if_never_sent() {
        use crate::raw_payloads::droppable::Queue;
        let run = |push: bool| {
            let (a, b) = Pipe::pair();
            let mut client = Connection::local(a);
            let served = serve(Connection::local(b), quiet, Duration::from_secs(30));
            // A zero budget drops every droppable send at the queue.
            let queue = Queue::new(0, None);
            served.outbox.attach_droppable(queue.clone());
            served.send(vec![body(3, 10)], None);
            if push {
                queue.push(Instant::now(), &Arc::default(), body(0x40, 20));
            }
            served.send(vec![body(4, 300), body(0x7e, 0)], None);
            read_until_sentinel(&mut client)
        };
        assert_eq!(run(true), run(false));
    }

    #[test]
    fn waiting_droppable_packets_are_framed_by_the_thread_with_room() {
        use crate::raw_payloads::droppable::Queue;
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        let served = serve(Connection::local(b), quiet, Duration::from_secs(30));
        let queue = Queue::new(u32::MAX, None);
        served.outbox.attach_droppable(queue.clone());
        for n in 0..3u8 {
            queue.push(Instant::now(), &Arc::default(), body(0x40 + n, 10));
        }
        for n in 0..3u8 {
            assert_eq!(client.recv().unwrap(), packet(&body(0x40 + n, 10)));
        }
    }

    #[test]
    fn a_kick_goes_after_what_was_queued_and_ends_the_packets() {
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        let probe = |seen| Probe {
            seen,
            kick_on: Some(9),
            unprompted: Some(body(0x21, 2)),
        };
        let served = serve(Connection::local(b), probe, Duration::from_secs(30));
        // The queued packet and the one the session sends of its own accord
        // are framed by different paths (`Outbox::take` and `Session::tick`
        // in `sweep`), so which of the two goes out first is the sweep's
        // timing, not an order the pool promises: take both, then tell them
        // apart. The kick still has to come after the queued one.
        served.send(vec![body(3, 10)], None);
        // The first worker sweep may send the session's unsolicited packet
        // before the test queues its packet. Both must arrive before the kick,
        // but these independent producers have no ordering between them.
        let mut received = [client.recv().unwrap(), client.recv().unwrap()];
        received.sort();
        assert_eq!(received, [packet(&body(3, 10)), packet(&body(0x21, 2))]);
        client.send(&body(9, 1)).unwrap();
        client.send(&body(10, 1)).unwrap();
        assert_eq!(client.recv().unwrap(), packet(&body(0x7f, 3)));
        assert!(client.recv().is_err());
        assert_eq!(served.next(), Seen::Packet(9, body(9, 1)[1..].to_vec()));
        assert_eq!(served.next(), Seen::Left("kicked".into()));
        assert_eq!(served.next(), Seen::Closed);
        // Nothing after the kick, and nothing queued once closed.
        assert!(served.seen.try_recv().is_err());
        assert!(served.outbox.push(vec![body(4, 1).into()], None).is_none());
    }

    #[test]
    #[ignore = "daily"]
    fn a_silent_network_client_times_out() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let server = Connection::from_stream(listener.accept().unwrap().0).unwrap();
        let served = serve(server, quiet, Duration::from_millis(300));
        assert_eq!(served.next(), Seen::Left("Timed out".into()));
        assert_eq!(served.next(), Seen::Closed);
        assert!(client.recv().is_err());
    }

    /// The integrated server's own client, over memory, has no read timeout: a silent one
    /// stays connected well past it.
    #[test]
    fn a_silent_memory_client_stays_connected() {
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        let served = serve(Connection::local(b), quiet, Duration::from_millis(100));
        assert!(
            served
                .seen
                .recv_timeout(Duration::from_millis(500))
                .is_err(),
            "a memory connection timed out"
        );
        served.send(vec![body(3, 10)], None);
        assert_eq!(client.recv().unwrap().0, 3);
    }

    /// Nor a write stall: a memory client that takes nothing while the pipe is full stays
    /// connected well past the timeout, and is served once it reads.
    #[test]
    fn a_stalled_memory_client_stays_connected() {
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        let served = serve(Connection::local(b), quiet, Duration::from_millis(100));
        // Past the pipe's capacity, so the server's writes wait for the client.
        served.send(vec![body(3, 256 << 10)], None);
        assert!(
            served
                .seen
                .recv_timeout(Duration::from_millis(500))
                .is_err(),
            "a stalled memory connection was ended"
        );
        assert_eq!(client.recv().unwrap(), packet(&body(3, 256 << 10)));
    }

    /// A client that reads nothing holds its disconnect unwritten; a close
    /// then ends the connection at once.
    #[test]
    #[ignore = "daily"]
    fn a_close_ends_a_disconnect_the_client_does_not_take() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let mut socket = listener.accept().unwrap().0;
        socket.set_nonblocking(true).unwrap();
        // Windows may buffer an entire large first write. Establish actual backpressure with
        // small framed writes before adoption, rather than assuming the batch below fills it.
        let fill = sunrise_protocol::frame::encode(&body(0, 65535), None).unwrap();
        let mut filled = 0;
        let mut offset = 0;
        let fill_until = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < fill_until, "socket prefill timed out");
            match socket.write(&fill[offset..]) {
                Ok(n) => {
                    assert!(n > 0, "socket closed during prefill");
                    filled += n;
                    assert!(filled < MAX_PENDING, "socket did not apply backpressure");
                    offset = (offset + n) % fill.len();
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => panic!("socket prefill: {e}"),
            }
        }
        socket.set_nonblocking(false).unwrap();
        let server = Connection::from_stream(socket).unwrap();
        let served = serve(server, quiet, Duration::from_secs(30));
        // Fill the socket until the outbox waits. A fixed amount may all go
        // through: Windows takes a write of any size whole while its buffer
        // has room.
        let full = || {
            std::thread::sleep(Duration::from_millis(50));
            served.pending.load(Ordering::Relaxed) > 0
        };
        for i in 0..=u8::MAX {
            if full() && full() && full() {
                break;
            }
            assert!(i < u8::MAX, "the socket took everything");
            served.send(vec![body(i, 1 << 20)], None);
        }
        served.send(Vec::new(), Some(End::Disconnect));
        std::thread::sleep(Duration::from_millis(300));
        assert!(served.seen.try_recv().is_err(), "still writing");
        served.send(Vec::new(), Some(End::Close));
        assert_eq!(served.next(), Seen::Left("Disconnected".into()));
        assert_eq!(served.next(), Seen::Closed);
        drop(client);
    }

    #[test]
    fn a_close_or_too_much_queued_drops_what_waits() {
        for overflow in [false, true] {
            let (a, b) = Pipe::pair();
            let mut client = Connection::local(a);
            let served = serve(Connection::local(b), quiet, Duration::from_secs(30));
            if overflow {
                served.pending.store(MAX_PENDING, Ordering::Relaxed);
                served.send(vec![body(3, 10)], None);
            } else {
                served.send(Vec::new(), Some(End::Close));
            }
            assert!(client.recv().is_err());
            assert_eq!(served.next(), Seen::Left("Disconnected".into()));
            assert_eq!(served.next(), Seen::Closed);
        }
    }

    /// A client in this process takes a join's whole first view at once: queued past the
    /// limit, it is served and stays connected.
    #[test]
    fn an_uncapped_connection_outlasts_too_much_queued() {
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        let served = serve_with(Connection::local(b), quiet, Duration::from_secs(30), false);
        served.pending.store(MAX_PENDING, Ordering::Relaxed);
        served.send(vec![body(3, 10)], None);
        assert_eq!(client.recv().unwrap().0, 3);
        assert!(served.seen.try_recv().is_err());
    }

    #[cfg(feature = "net-trace")]
    fn serve_captured_session(
        server: Connection,
        session: Box<dyn Session>,
        run: &crate::lan::packet_capture::CaptureRun,
        conn: u32,
        player: &str,
        seen: Receiver<Seen>,
    ) -> Served {
        let pending = Arc::new(AtomicUsize::new(0));
        let outbox = Outbox::new(pending.clone(), false).unwrap();
        let (clock, writer) = match run.open_session(conn, player) {
            crate::lan::packet_capture::OpenSession::Opened(clock, writer) => (clock, writer),
            crate::lan::packet_capture::OpenSession::Sealed => panic!("capture run sealed"),
        };
        outbox.attach_capture(clock, writer);
        let (socket, reader, writer) = server.detach();
        adopt(
            socket,
            (reader, writer),
            outbox.clone(),
            session,
            Duration::from_secs(30),
        );
        Served {
            outbox,
            pending,
            seen,
        }
    }

    #[cfg(feature = "net-trace")]
    fn serve_captured(
        server: Connection,
        probe: impl FnOnce(Sender<Seen>) -> Probe,
        run: &crate::lan::packet_capture::CaptureRun,
        conn: u32,
        player: &str,
    ) -> Served {
        let (tx, seen) = channel();
        serve_captured_session(server, Box::new(probe(tx)), run, conn, player, seen)
    }

    #[cfg(feature = "net-trace")]
    fn encoded_meta(id: i32, payload: &[u8]) -> (i32, usize, String) {
        (
            id,
            payload.len(),
            format!("{:032x}", xxhash_rust::xxh3::xxh3_128(payload)),
        )
    }

    #[cfg(feature = "net-trace")]
    fn parse_recorded(path: &std::path::Path) -> (Vec<(i32, usize, String)>, String) {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.ends_with('\n'));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() >= 2);
        assert!(lines[0].starts_with("# v2\t"));
        let footer = lines.last().unwrap().to_string();
        let mut rows = Vec::new();
        for (idx, line) in lines[1..lines.len() - 1].iter().enumerate() {
            let parts: Vec<&str> = line.split('\t').collect();
            assert_eq!(parts.len(), 5, "row {line}");
            assert_eq!(parts[0].parse::<usize>().unwrap(), idx);
            rows.push((
                parts[2].parse::<i32>().unwrap(),
                parts[3].parse::<usize>().unwrap(),
                parts[4].to_owned(),
            ));
        }
        (rows, footer)
    }

    #[cfg(feature = "net-trace")]
    fn recv_until_eof(client: &mut Connection, what: &str) -> Vec<(i32, usize, String)> {
        let mut seen = Vec::new();
        loop {
            match client.recv() {
                Ok((id, payload)) => seen.push(encoded_meta(id, &payload)),
                Err(e) if e.is_end_of_stream() => return seen,
                Err(e) => panic!("unexpected error while waiting for {what}: {e}"),
            }
        }
    }

    #[cfg(feature = "net-trace")]
    #[test]
    fn capture_order_matches_wire_with_interleaved_producers_timer_and_kick() {
        let dir = tempfile::tempdir().unwrap();
        let run = crate::lan::packet_capture::CaptureRun::open_in(dir.path(), 1).unwrap();
        run.set_tick(42);
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let probe = |seen| Probe {
            seen,
            kick_on: Some(9),
            unprompted: Some(body(45, 8)),
        };
        let served = serve_captured(Connection::local(b), probe, &run, 1, "Alice");
        let outbox_a = served.outbox.clone();
        let pending_a = served.pending.clone();
        let (go_tx, go_rx) = channel();
        let (done_tx, done_rx) = channel();
        let producer = std::thread::spawn(move || {
            go_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("timed out waiting for producer start signal");
            for i in 0..8u8 {
                let b = Body::from(body(10 + i, 4));
                pending_a.fetch_add(charge(b.retained()), Ordering::Relaxed);
                outbox_a.send(vec![b], None);
            }
            let _ = done_tx.send(());
        });
        go_tx.send(()).unwrap();
        for i in 0..8u8 {
            served.send(vec![body(20 + i, 6)], None);
        }
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for concurrent producer thread");
        producer.join().unwrap();
        // Wait for all 16 queued packets and the sweep-emitted timer packet before kicking.
        let mut peer_seen = Vec::new();
        while peer_seen.len() < 17 {
            let (id, payload) = client
                .recv()
                .expect("timed out waiting for pre-kick queued/timer packet");
            peer_seen.push(encoded_meta(id, &payload));
        }
        assert!(
            peer_seen.iter().any(|(id, _, _)| *id == 45),
            "expected timer packet 45 before kick"
        );
        // Trigger a kick after queued and timer packets; the client reads until EOF.
        client.send(&body(9, 1)).unwrap();
        peer_seen.extend(recv_until_eof(&mut client, "kick packet and EOF"));
        assert_eq!(served.next(), Seen::Packet(9, body(9, 1)[1..].to_vec()));
        assert_eq!(served.next(), Seen::Left("kicked".into()));
        assert_eq!(served.next(), Seen::Closed);
        assert_eq!(served.pending.load(Ordering::Relaxed), 0);
        run.finish();

        let (recorded, footer) = parse_recorded(&dir.path().join("Alice-1.pkt"));
        assert_eq!(peer_seen.len(), 18);
        assert_eq!(recorded, peer_seen);
        assert_eq!(footer, "# end\t18\tok");
        assert_eq!(recorded.last().unwrap().0, 0x7f);
    }

    #[cfg(feature = "net-trace")]
    #[test]
    fn capture_omits_rejected_closed_batch_and_suppressed_kick() {
        struct GateProbe {
            seen: Sender<Seen>,
            entered: Sender<()>,
            release: Receiver<()>,
        }
        impl Session for GateProbe {
            fn packet(&mut self, id: i32, payload: Vec<u8>) -> Verdict {
                let _ = self.seen.send(Seen::Packet(id, payload));
                if id == 1 {
                    // Hold the worker inside Conn::read while the test queues End::Disconnect
                    // and sends the kick trigger packet 9. Never panic on the shared worker if
                    // the test unwinds early.
                    let _ = self.entered.send(());
                    let _ = self.release.recv_timeout(Duration::from_secs(5));
                    Verdict::Go
                } else if id == 9 {
                    Verdict::Kick("kicked".into(), body(0x7f, 3))
                } else {
                    Verdict::Go
                }
            }
            fn tick(&mut self, _: Instant) -> Option<Vec<u8>> {
                None
            }
            fn left(&mut self, reason: String) {
                let _ = self.seen.send(Seen::Left(reason));
            }
            fn closed(&mut self) {
                let _ = self.seen.send(Seen::Closed);
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let run = crate::lan::packet_capture::CaptureRun::open_in(dir.path(), 2).unwrap();
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (tx, seen) = channel();
        let served = serve_captured_session(
            Connection::local(b),
            Box::new(GateProbe {
                seen: tx,
                entered: entered_tx,
                release: release_rx,
            }),
            &run,
            1,
            "Bob",
            seen,
        );
        // Pause worker inside Conn::read on packet 1, then queue disconnect and packet 9.
        client.send(&body(1, 0)).unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for worker to pause in GateProbe::packet(1)");
        served.send(vec![body(32, 4)], Some(End::Disconnect));
        client.send(&body(9, 1)).unwrap();
        release_tx.send(()).unwrap();

        let peer_seen = recv_until_eof(&mut client, "disconnect packet and EOF");
        assert_eq!(served.next(), Seen::Packet(1, Vec::new()));
        assert_eq!(served.next(), Seen::Packet(9, body(9, 1)[1..].to_vec()));
        assert_eq!(served.next(), Seen::Left("kicked".into()));
        assert_eq!(served.next(), Seen::Closed);
        assert_eq!(served.pending.load(Ordering::Relaxed), 0);

        // Sending after close with a properly reserved charge is rejected and fully refunded.
        let rejected = Body::from(body(8, 3));
        let rejected_cost = charge(rejected.retained());
        served.pending.fetch_add(rejected_cost, Ordering::Relaxed);
        assert!(!served.outbox.try_send(vec![rejected], None));
        assert_eq!(served.pending.load(Ordering::Relaxed), 0);
        run.finish();

        let (recorded, footer) = parse_recorded(&dir.path().join("Bob-1.pkt"));
        assert_eq!(recorded, peer_seen);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, 32);
        assert_eq!(footer, "# end\t1\tok");
    }

    #[cfg(feature = "net-trace")]
    #[test]
    fn capture_includes_live_droppable_and_skips_expired_in_wire_order() {
        use crate::raw_payloads::droppable::Queue;
        let dir = tempfile::tempdir().unwrap();
        let run = crate::lan::packet_capture::CaptureRun::open_in(dir.path(), 3).unwrap();
        let (a, b) = Pipe::pair();
        let mut client = Connection::local(a);
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let served = serve_captured(Connection::local(b), quiet, &run, 1, "Carol");
        let queue = Queue::new(u32::MAX, None);
        served.outbox.attach_droppable(queue.clone());
        let counters = Arc::default();
        // Open a bundle so the droppable waits until the bundle closes.
        served.send(vec![body(0, 0)], None);
        assert_eq!(
            client
                .recv()
                .expect("timed out waiting for opening bundle delimiter"),
            packet(&body(0, 0))
        );
        let expired_at = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        queue.push(expired_at, &counters, body(0x41, 4));
        queue.push(Instant::now(), &counters, body(0x42, 5));
        served.send(vec![body(0x30, 3), body(0, 0)], None);
        let mut peer_seen = vec![encoded_meta(0, &[])];
        for _ in 0..3 {
            let (id, payload) = client
                .recv()
                .expect("timed out waiting for bundle + live droppable packet");
            peer_seen.push(encoded_meta(id, &payload));
        }
        served.send(vec![body(32, 2)], Some(End::Disconnect));
        peer_seen.extend(recv_until_eof(&mut client, "disconnect packet and EOF"));
        assert_eq!(served.next(), Seen::Left("Disconnected".into()));
        assert_eq!(served.next(), Seen::Closed);
        run.finish();

        let (recorded, footer) = parse_recorded(&dir.path().join("Carol-1.pkt"));
        assert_eq!(recorded, peer_seen);
        assert!(recorded.iter().any(|(id, _, _)| *id == 0x42));
        assert!(!recorded.iter().any(|(id, _, _)| *id == 0x41));
        assert_eq!(footer, format!("# end\t{}\tok", peer_seen.len()));
    }

    #[cfg(feature = "net-trace")]
    #[test]
    fn capture_records_prepared_compressed_and_pairing_over_tcp_without_prolonging_pending() {
        let dir = tempfile::tempdir().unwrap();
        let (gate_tx, gate_rx) = channel();
        let run = crate::lan::packet_capture::CaptureRun::open_in_with_gate(
            dir.path(),
            4,
            64,
            1 << 20,
            Some(gate_rx),
        )
        .unwrap();
        // Move after `run` so any early panic drops `release_gate` before `CaptureRun::drop` joins.
        let release_gate = gate_tx;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let accept_deadline = Instant::now() + Duration::from_secs(5);
        let server_stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < accept_deadline,
                        "timed out waiting for loopback TCP accept"
                    );
                    std::thread::yield_now();
                }
                Err(e) => panic!("loopback TCP accept failed: {e}"),
            }
        };
        server_stream.set_nonblocking(false).unwrap();
        let mut server = Connection::from_stream(server_stream).unwrap();
        client.set_compression(32);
        server.set_compression(32);
        let framing = server.writer.framing();
        assert!(matches!(
            framing,
            sunrise_protocol::connection::Framing::Network(Some(32))
        ));
        let served = serve_captured(server, quiet, &run, 1, "Dave");

        // 1. Compressed prepared payload published via Outbox::publish.
        let mut raw_big = vec![24u8];
        raw_big.extend(std::iter::repeat_n(b'z', 128));
        let meta_big = crate::lan::packet_capture::PacketMeta::from_body(&raw_big);
        let prepared_big = framing.contents(&raw_big).unwrap();
        assert_ne!(prepared_big, raw_big);
        let cost_big = charge(prepared_big.capacity());
        served.pending.fetch_add(cost_big, Ordering::Relaxed);
        assert!(served.outbox.publish(prepared_big, cost_big));

        // 2. Below-threshold prepared payload published via Outbox::publish.
        let raw_small = vec![25u8, 1, 2, 3, 4];
        let meta_small = crate::lan::packet_capture::PacketMeta::from_body(&raw_small);
        let prepared_small = framing.contents(&raw_small).unwrap();
        let cost_small = charge(prepared_small.capacity());
        served.pending.fetch_add(cost_small, Ordering::Relaxed);
        assert!(served.outbox.publish(prepared_small, cost_small));

        // 3. Compressed Body::Prepared(Arc<PairingPayload>) with prepaid credit.
        // Register one declaration so Transport::new allocates a real connection and Epoch.
        let mut manifest = sunrise_protocol::payload_negotiation::Manifest::builder();
        manifest
            .register(
                sunrise_protocol::payload_negotiation::RegistrationPhase::Play,
                "test:pairing",
                sunrise_protocol::payload_negotiation::Flow::ToClient,
                "1",
                true,
            )
            .unwrap();
        let local = Arc::new(crate::raw_payloads::Local::new(
            manifest.finish().unwrap(),
            None,
        ));
        let transport = crate::raw_payloads::Transport::new(
            local,
            crate::raw_payloads::Side::Server,
            crate::lan::PAYLOAD_RECORD_OVERHEAD,
        );
        transport.accept(&[]).unwrap();
        let epoch = transport.epoch().expect("transport epoch");
        let mut raw_pair = vec![26u8];
        raw_pair.extend(std::iter::repeat_n(b'p', 128));
        let meta_pair = crate::lan::packet_capture::PacketMeta::from_body(&raw_pair);
        let prepared_pair = framing.contents(&raw_pair).unwrap();
        assert_ne!(prepared_pair, raw_pair);
        let cost_pair = charge(prepared_pair.capacity());
        served.pending.fetch_add(cost_pair, Ordering::Relaxed);
        let pairing = Arc::new(crate::raw_payloads::PairingPayload {
            bytes: prepared_pair,
            epoch,
            pending: served.pending.clone(),
            cost: cost_pair,
        });
        served.outbox.send(vec![Body::Prepared(pairing)], None);

        // 4. Multi-byte VarInt packet id (300) with a payload (2000 bytes) whose
        // uncompressed body length (2002) and compressed frame contents length are all
        // distinct multi-byte VarInts (>= 128) so no field can be confused with another.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut payload_multi = Vec::with_capacity(2000);
        for _ in 0..500 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            payload_multi.push((seed >> 56) as u8);
        }
        payload_multi.extend(std::iter::repeat_n(b'q', 1500));
        let mut raw_multi = Vec::with_capacity(2 + payload_multi.len());
        sunrise_core::vanilla_bytes::write_varint(&mut raw_multi, 300);
        raw_multi.extend_from_slice(&payload_multi);
        assert_eq!(raw_multi.len(), 2002);
        let meta_multi = crate::lan::packet_capture::PacketMeta::from_body(&raw_multi);
        assert_eq!(meta_multi.id, 300);
        assert_eq!(meta_multi.len, 2000);
        let prepared_multi = framing.contents(&raw_multi).unwrap();
        assert!(
            prepared_multi.len() >= 128 && prepared_multi.len() < payload_multi.len(),
            "compressed length {} must be a distinct multi-byte varint below {}",
            prepared_multi.len(),
            payload_multi.len()
        );
        assert_ne!(prepared_multi.len(), 300);
        let cost_multi = charge(prepared_multi.capacity());
        served.pending.fetch_add(cost_multi, Ordering::Relaxed);
        assert!(served.outbox.publish(prepared_multi, cost_multi));

        // Verify the TCP peer decoded every original uncompressed packet id and payload.
        let (id1, payload1) = client
            .recv()
            .expect("timed out waiting for compressed prepared packet 1");
        assert_eq!((id1, payload1.as_slice()), (24, &raw_big[1..]));
        let (id2, payload2) = client
            .recv()
            .expect("timed out waiting for below-threshold prepared packet 2");
        assert_eq!((id2, payload2.as_slice()), (25, &raw_small[1..]));
        let (id3, payload3) = client
            .recv()
            .expect("timed out waiting for compressed pairing packet 3");
        assert_eq!((id3, payload3.as_slice()), (26, &raw_pair[1..]));
        let (id4, payload4) = client
            .recv()
            .expect("timed out waiting for multi-byte varint compressed packet 4");
        assert_eq!((id4, payload4.as_slice()), (300, payload_multi.as_slice()));

        // While the capture writer thread is still paused, all pending charges must be refunded.
        assert_eq!(served.pending.load(Ordering::Acquire), 0);

        drop(release_gate);
        drop(client);
        assert_eq!(served.next(), Seen::Left("Disconnected".into()));
        assert_eq!(served.next(), Seen::Closed);
        run.finish();

        let (recorded, footer) = parse_recorded(&dir.path().join("Dave-1.pkt"));
        assert_eq!(
            recorded,
            [
                encoded_meta(id1, &payload1),
                encoded_meta(id2, &payload2),
                encoded_meta(id3, &payload3),
                encoded_meta(id4, &payload4),
            ]
        );
        assert_eq!(
            recorded,
            [
                (meta_big.id, meta_big.len, format!("{:032x}", meta_big.hash)),
                (
                    meta_small.id,
                    meta_small.len,
                    format!("{:032x}", meta_small.hash),
                ),
                (
                    meta_pair.id,
                    meta_pair.len,
                    format!("{:032x}", meta_pair.hash),
                ),
                (
                    meta_multi.id,
                    meta_multi.len,
                    format!("{:032x}", meta_multi.hash),
                ),
            ]
        );
        assert_eq!(recorded[0].0, 24);
        assert_eq!(recorded[0].1, 128);
        assert_eq!(recorded[3].0, 300);
        assert_eq!(recorded[3].1, 2000);
        assert_eq!(footer, "# end\t4\tok");
    }

    #[cfg(feature = "net-trace")]
    #[test]
    fn capture_marks_unflushed_tail_incomplete_on_close() {
        let dir = tempfile::tempdir().unwrap();
        let run = crate::lan::packet_capture::CaptureRun::open_in(dir.path(), 5).unwrap();
        let (a, b) = Pipe::pair();
        let client = Connection::local(a);
        let served = serve_captured(Connection::local(b), quiet, &run, 1, "Eve");
        // Send a payload larger than the memory pipe's buffer while the peer reads nothing.
        served.send(vec![body(3, 512 << 10)], None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while served.outbox.queued() > 0 {
            assert!(Instant::now() < deadline, "worker did not take batch");
            std::thread::yield_now();
        }
        // Close while framed bytes remain unwritten in Conn.out (client still alive).
        served.send(Vec::new(), Some(End::Close));
        assert_eq!(served.next(), Seen::Left("Disconnected".into()));
        assert_eq!(served.next(), Seen::Closed);
        drop(client);
        run.finish();

        let (_, footer) = parse_recorded(&dir.path().join("Eve-1.pkt"));
        assert_eq!(footer, "# end\t1\tincomplete\tunflushed_tail");
        let manifest = std::fs::read_to_string(dir.path().join("manifest.tsv")).unwrap();
        assert!(manifest.contains("incomplete:unflushed_tail"), "{manifest}");
        assert!(manifest.ends_with("# end\t1\tincomplete\n"), "{manifest}");
    }
}

#[cfg(test)]
mod memory_progress_tests;
