//! The game's log, as vanilla's log4j configuration writes it (client-1.21.2.xml, the one
//! 26.3's version manifest names): `logs/latest.log` in the game folder with the pattern
//! `[%d{HH:mm:ss}] [%t/%level]: %msg%n`, rolled over at start (OnStartupTriggeringPolicy, a
//! non-empty file only) and when the day changes (TimeBasedTriggeringPolicy) to
//! `logs/%d{yyyy-MM-dd}-%i.log.gz`, dated by the old file's last write. DefaultRolloverStrategy
//! keeps 7 archives a day, the newest with the highest index.
//!
//! Messages go through the `log` facade. Sunrise's own still print their plain text on stderr,
//! as they did before the log existed. Other crates using the game's facade reach the file
//! through INFO, with their target prefixed. The native mod API submits directly to this same
//! writer with the manifest ID, not through a mod's own copy of the facade.
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};
use mod_api::LogOutcome;

/// DefaultRolloverStrategy's default `max`: archives kept for one date.
pub const MAX_ARCHIVES_PER_DAY: u32 = 7;
/// Formatted lines kept in memory for crash reports.
const RECENT: usize = 50;
/// Warnings and errors one line of code may log in a [`FLOOD_WINDOW`]; past that they are counted,
/// not written, so a warning repeated every packet can't fill the disk. Vanilla has no such limit:
/// below it the log is vanilla's, line for line.
const FLOOD_LINES: u32 = 100;
const FLOOD_WINDOW: Duration = Duration::from_secs(60);

/// The platform data folder, shared with launcher command preparation.
pub fn data_dir() -> PathBuf {
    sunrise_mod_package::selection::data_dir_with(|key| std::env::var_os(key))
}

/// The resolved game and overlay cache homes, set once at startup.
static HOME: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();

/// Records the game home and the base folder for the content overlay cache.
pub fn set_game_home(home: PathBuf, cache_home: PathBuf) {
    let _ = HOME.set((home, cache_home));
}

/// `--player-dir DIR` (#4952), set once at start: one player's own files, when several games
/// play in one game folder at the same time (the launcher's `play --account`).
static PLAYER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Makes [`player_dir`] return `dir` for the rest of the run (`--player-dir`), creating it.
/// What is a player's own goes there: `options.txt` and the settings files beside it,
/// `logs/`, `crash-reports/`, `sunrise-player.txt`, `servers.dat`, `hotbar.nbt` and
/// `screenshots/`. What the game folder's players share stays in it: saves, resource and
/// shader packs, mods and their config, caches.
pub fn set_player_dir(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let _ = PLAYER.set(dir);
}

/// The folder of this player's own files: `--player-dir`, else the game folder (the working
/// directory, where the launcher runs the game).
pub fn player_dir() -> PathBuf {
    PLAYER
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// `--player-dir`, if given.
pub fn player_dir_override() -> Option<&'static Path> {
    PLAYER.get().map(PathBuf::as_path)
}

/// Where the files all players share live, for settings kept beside `options.txt` in
/// `options_dir` (the enabled mods): that folder, unless `--player-dir` moved `options.txt`
/// out of the game folder.
pub fn shared_dir(options_dir: &Path) -> PathBuf {
    shared_dir_of(PLAYER.get().map(PathBuf::as_path), options_dir, || {
        std::env::current_dir().unwrap_or_default()
    })
}

/// [`shared_dir`] for a given `--player-dir` and game folder.
fn shared_dir_of(
    player: Option<&Path>,
    options_dir: &Path,
    game_dir: impl FnOnce() -> PathBuf,
) -> PathBuf {
    match player {
        Some(player) if player == options_dir => game_dir(),
        _ => options_dir.to_path_buf(),
    }
}

/// Where the launcher runs the game and keeps its saves: `--home`, else `SUNRISE_HOME`, else
/// `sunrise` in the platform data folder (https://github.com/sunrise-org/sunrise-launcher/blob/main/src/backend.rs `game_home`).
pub fn game_home() -> PathBuf {
    if let Some((home, _)) = HOME.get() {
        return home.clone();
    }
    match std::env::var_os("SUNRISE_HOME").filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => data_dir().join("sunrise"),
    }
}

/// The content overlay's `.sunrise-cache`: in the private home when `--home` or
/// `SUNRISE_HOME` names one, else in the game folder. Without startup (e.g. in tests),
/// use a named `SUNRISE_HOME` or the working directory.
pub fn cache_dir() -> PathBuf {
    if let Some((_, cache_home)) = HOME.get() {
        return cache_home.join(".sunrise-cache");
    }
    match std::env::var_os("SUNRISE_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => std::env::current_dir().unwrap_or_default(),
    }
    .join(".sunrise-cache")
}

/// `logs/` and `crash-reports/` of a game folder.
pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("logs")
}
pub fn crash_reports_dir(home: &Path) -> PathBuf {
    home.join("crash-reports")
}

/// `2026-10-14-3.log.gz`
pub fn archive_name(date: NaiveDate, index: u32) -> String {
    format!("{}-{index}.log.gz", date.format("%Y-%m-%d"))
}

/// The archive index of `name` if it is one of `date`'s archives.
fn archive_index(name: &str, date: NaiveDate) -> Option<u32> {
    let prefix = format!("{}-", date.format("%Y-%m-%d"));
    name.strip_prefix(&prefix)?
        .strip_suffix(".log.gz")?
        .parse()
        .ok()
        .filter(|&i| i > 0)
}

/// What one rollover does to the archives already there.
#[derive(Debug, PartialEq, Eq)]
pub struct Rollover {
    pub delete: Option<String>,
    /// In order: each target is free when its rename runs.
    pub renames: Vec<(String, String)>,
    pub archive: String,
}

/// DefaultRolloverStrategy with fileIndex "max": the new archive takes the next index; once
/// `max` are there the lowest goes and the others move down one.
pub fn rollover(existing: &[String], date: NaiveDate, max: u32) -> Rollover {
    let mut present: Vec<u32> = existing
        .iter()
        .filter_map(|n| archive_index(n, date))
        .collect();
    present.sort_unstable();
    present.dedup();
    let highest = present.last().copied().unwrap_or(0);
    if highest < max {
        return Rollover {
            delete: None,
            renames: Vec::new(),
            archive: archive_name(date, highest + 1),
        };
    }
    let delete = Some(archive_name(date, present[0]));
    let kept = &present[1..];
    let renames = kept
        .iter()
        .enumerate()
        .map(|(i, &from)| (from, i as u32 + 1))
        .filter(|(from, to)| from != to)
        .map(|(from, to)| (archive_name(date, from), archive_name(date, to)))
        .collect();
    Rollover {
        delete,
        renames,
        archive: archive_name(date, kept.len() as u32 + 1),
    }
}

/// `[12:04:31] [Render thread/INFO]: message`
pub fn format_line(
    time: &DateTime<Local>,
    thread: &str,
    level: log::Level,
    message: &str,
) -> String {
    format!(
        "[{}] [{thread}/{level}]: {message}",
        time.format("%H:%M:%S")
    )
}

/// Gzips `text` into the next archive for `date` in `dir`.
fn archive(dir: &Path, date: NaiveDate, text: &[u8]) -> io::Result<()> {
    let names: Vec<String> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let plan = rollover(&names, date, MAX_ARCHIVES_PER_DAY);
    if let Some(name) = &plan.delete {
        fs::remove_file(dir.join(name))?;
    }
    for (from, to) in &plan.renames {
        fs::rename(dir.join(from), dir.join(to))?;
    }
    let mut gz = flate2::write::GzEncoder::new(
        File::create(dir.join(&plan.archive))?,
        flate2::Compression::default(),
    );
    gz.write_all(text)?;
    gz.finish()?.sync_all()
}

/// Moves what `file` holds into an archive dated `date` and empties it. The file stays open
/// (and locked), so this works on Windows too.
fn roll(file: &mut File, dir: &Path, date: NaiveDate) -> io::Result<()> {
    let mut text = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_end(&mut text)?;
    if !text.is_empty() {
        archive(dir, date, &text)?;
    }
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// The open `latest.log` and the lock that keeps it this game's.
pub struct Latest {
    pub file: File,
    _lock: File,
}

/// Opens `dir/latest.log` for this run, archiving the last run's. None when another Sunrise
/// using the same folder is writing it: that game keeps it, this one logs to stderr only. The
/// lock is a separate file (`.lock`): Windows locks are mandatory, and a locked latest.log could
/// not be read while the game runs.
pub fn open_latest(dir: &Path) -> io::Result<Option<Latest>> {
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Ok(None),
        Err(fs::TryLockError::Error(e)) => return Err(e),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("latest.log"))?;
    let modified = file.metadata()?.modified()?;
    let date = DateTime::<Local>::from(modified).date_naive();
    roll(&mut file, dir, date)?;
    Ok(Some(Latest { file, _lock: lock }))
}

/// What the writer thread is told.
enum Out {
    /// Lines for the file, dated, and the message for stderr (Sunrise's own).
    Write {
        date: NaiveDate,
        lines: Vec<String>,
        stderr: Option<String>,
    },
    /// Answer once everything before is written.
    Flush(mpsc::SyncSender<()>),
}

/// Owns the only receiver and clears availability even if its thread unwinds or fails to start.
struct WriterInbox {
    receiver: mpsc::Receiver<Out>,
    file_available: Arc<AtomicBool>,
}

impl Drop for WriterInbox {
    fn drop(&mut self) {
        self.file_available.store(false, Ordering::Relaxed);
    }
}

/// The file side of the log, on its own thread: a line costs the thread that logs it a
/// format and a channel send, never a write that can wait (a full stderr pipe, a slow disk):
/// written on the game thread, a line could hold the tick for milliseconds while the CPU sat
/// idle.
struct Writer {
    file: Option<Latest>,
    dir: PathBuf,
    /// The day the open file's lines belong to.
    date: NaiveDate,
}

impl Writer {
    fn run(mut self, inbox: WriterInbox) {
        let stderr = io::stderr();
        for msg in &inbox.receiver {
            match msg {
                Out::Write {
                    date,
                    lines,
                    stderr: message,
                } => {
                    if let Some(message) = message {
                        let _ = writeln!(stderr.lock(), "{message}");
                    }
                    if date != self.date {
                        if let Some(latest) = &mut self.file
                            && let Err(e) = roll(&mut latest.file, &self.dir, self.date)
                        {
                            let _ = writeln!(
                                stderr.lock(),
                                "Sunrise: could not roll over the log: {e}"
                            );
                        }
                        self.date = date;
                    }
                    if let Some(latest) = &mut self.file {
                        let mut text = lines.join("\n");
                        text.push('\n');
                        let _ = latest.file.write_all(text.as_bytes());
                    }
                }
                Out::Flush(done) => {
                    if let Some(latest) = &mut self.file {
                        let _ = latest.file.flush();
                    }
                    let _ = done.send(());
                }
            }
        }
    }
}

struct Logger {
    out: mpsc::Sender<Out>,
    /// The last lines, kept here (not on the writer) so a crash report has them at once.
    recent: Mutex<VecDeque<String>>,
    /// What the main thread is called in the log: "Render thread", or "Server thread" for the
    /// dedicated server, as in vanilla.
    main_thread: &'static str,
    file_available: Arc<AtomicBool>,
    floods: Mutex<Floods>,
}

/// What the flood limit does with one warning or error.
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    /// Write it, after saying how many lines like it were left out in the window before.
    Write {
        left_out: u32,
    },
    /// Write it, then say the next ones from this line of code wait for the next window.
    Last,
    Skip,
}

/// Warnings and errors per line of code, in windows of [`FLOOD_WINDOW`].
#[derive(Default)]
struct Floods {
    sites: HashMap<(Option<&'static str>, Cow<'static, str>, u32), Flood>,
}

struct Flood {
    since: Instant,
    written: u32,
    left_out: u32,
}

impl Floods {
    fn gate(
        &mut self,
        mod_id: Option<&'static str>,
        file: Cow<'static, str>,
        line: u32,
        now: Instant,
    ) -> Gate {
        let flood = self.sites.entry((mod_id, file, line)).or_insert(Flood {
            since: now,
            written: 0,
            left_out: 0,
        });
        let mut left_out = 0;
        if now.duration_since(flood.since) >= FLOOD_WINDOW {
            left_out = flood.left_out;
            *flood = Flood {
                since: now,
                written: 0,
                left_out: 0,
            };
        }
        flood.written += 1;
        match flood.written.cmp(&FLOOD_LINES) {
            std::cmp::Ordering::Less => Gate::Write { left_out },
            std::cmp::Ordering::Equal => Gate::Last,
            std::cmp::Ordering::Greater => {
                flood.left_out += 1;
                Gate::Skip
            }
        }
    }
}

static LOGGER: std::sync::OnceLock<Logger> = std::sync::OnceLock::new();

/// The most lines the tap keeps between two drains (`take_tapped`); older ones are dropped.
pub const TAP_LINES: usize = 4096;

/// The most bytes of a line's message the tap keeps; the rest is cut off at a character boundary.
pub const TAP_MESSAGE_BYTES: usize = 8192;
/// The most bytes of a thread name the tap keeps; the rest is cut off at a character boundary.
pub const TAP_THREAD_BYTES: usize = 128;

/// A line the log tap kept: its level, the thread it names and the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TappedLine {
    pub level: log::Level,
    pub thread: String,
    pub message: String,
}

/// Whether the tap keeps lines (`set_tap`), read before its lock is taken.
static TAPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The lines kept for the native mods' `console_line` hook, oldest first.
static TAP: Mutex<VecDeque<TappedLine>> = Mutex::new(VecDeque::new());

thread_local! {
    /// Set while this thread runs `untapped`: its lines are not kept.
    static UNTAPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Keeps every line logged from now on for `take_tapped` (`on`), or stops and forgets them.
pub fn set_tap(on: bool) {
    TAPPING.store(on, std::sync::atomic::Ordering::Relaxed);
    if !on {
        TAP.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// The lines the tap kept since the last call, oldest first.
pub fn take_tapped() -> Vec<TappedLine> {
    TAP.lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect()
}

/// Runs `f` without tapping the lines this thread logs meanwhile (a `console_line`
/// listener's own lines).
pub fn untapped<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            UNTAPPED.set(self.0);
        }
    }
    let _restore = Restore(UNTAPPED.replace(true));
    f()
}

/// `text` cut to at most `max` bytes, at a character boundary.
fn capped(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn tap(thread: &str, level: log::Level, message: &str) {
    if !TAPPING.load(std::sync::atomic::Ordering::Relaxed) || UNTAPPED.get() {
        return;
    }
    let mut lines = TAP.lock().unwrap_or_else(|e| e.into_inner());
    if lines.len() >= TAP_LINES {
        lines.pop_front();
    }
    lines.push_back(TappedLine {
        level,
        thread: capped(thread, TAP_THREAD_BYTES),
        message: capped(message, TAP_MESSAGE_BYTES),
    });
}

fn ours(target: &str) -> bool {
    target.starts_with("sunrise")
}

/// Crates logged from ERROR only, their warnings being known noise. `calloop` 0.13, winit 0.30's
/// Wayland event loop, warns once per key press: smithay-client-toolkit re-registers the key
/// repeat timer on each press and the removed timer's last event still arrives ("Received an
/// event for non-existence source"). Neither crate has a release without it.
const ERRORS_ONLY: &[&str] = &["calloop"];
const DEFAULT_LEVEL: log::Level = log::Level::Info;

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        let target = meta.target();
        let floor = if ERRORS_ONLY.iter().any(|name| {
            target
                .strip_prefix(name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
        }) {
            log::Level::Error
        } else {
            DEFAULT_LEVEL
        };
        meta.level() <= floor
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.enqueue(record, None);
        }
    }

    /// Waits (a second at most) until every line logged before is in the file.
    fn flush(&self) {
        let (done, wait) = mpsc::sync_channel(1);
        if self.out.send(Out::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(1));
        }
    }
}

impl Logger {
    fn new(main_thread: &'static str, has_file: bool) -> (Self, WriterInbox) {
        let (out, receiver) = mpsc::channel();
        let file_available = Arc::new(AtomicBool::new(has_file));
        (
            Self {
                out,
                recent: Mutex::default(),
                main_thread,
                file_available: Arc::clone(&file_available),
                floods: Mutex::default(),
            },
            WriterInbox {
                receiver,
                file_available,
            },
        )
    }

    fn mod_enabled(&self, level: log::Level) -> bool {
        self.file_available.load(Ordering::Relaxed) && level <= DEFAULT_LEVEL
    }

    fn log_mod(
        &self,
        id: &'static str,
        level: log::Level,
        message: std::fmt::Arguments<'_>,
        site: &'static std::panic::Location<'static>,
    ) -> LogOutcome {
        if !self.file_available.load(Ordering::Relaxed) {
            return LogOutcome::Unavailable;
        }
        if level > DEFAULT_LEVEL {
            return LogOutcome::Filtered;
        }
        self.enqueue(
            &log::Record::builder()
                .args(message)
                .level(level)
                .target(id)
                .file_static(Some(site.file()))
                .line(Some(site.line()))
                .build(),
            Some(id),
        )
    }

    fn enqueue(&self, record: &log::Record, mod_id: Option<&'static str>) -> LogOutcome {
        let level = record.level();
        let gate = if level <= log::Level::Warn {
            let mut floods = self.floods.lock().unwrap_or_else(|e| e.into_inner());
            floods.gate(
                mod_id,
                record.file_static().map_or_else(
                    || Cow::Owned(record.file().unwrap_or(record.target()).to_owned()),
                    Cow::Borrowed,
                ),
                record.line().unwrap_or(0),
                Instant::now(),
            )
        } else {
            Gate::Write { left_out: 0 }
        };
        if gate == Gate::Skip {
            return LogOutcome::Filtered;
        }
        let current = std::thread::current();
        let thread = match current.name() {
            Some("main") => self.main_thread,
            Some(name) => name,
            None => "unnamed",
        };
        if let Gate::Write { left_out } = gate
            && left_out > 0
        {
            self.send(
                thread,
                level,
                &format!(
                    "{left_out} more lines like the next one were not logged in the last minute"
                ),
                None,
                mod_id.is_some(),
            );
        }
        let sunrise = mod_id.is_none() && ours(record.target());
        let message = if sunrise {
            record.args().to_string()
        } else {
            format!("[{}] {}", record.target(), record.args())
        };
        let stderr = sunrise.then(|| message.clone());
        let queued = self.send(thread, level, &message, stderr, mod_id.is_some());
        if queued || mod_id.is_none() {
            tap(thread, level, &message);
        }
        if gate == Gate::Last && (queued || mod_id.is_none()) {
            self.send(
                thread,
                level,
                &format!("{FLOOD_LINES} lines like this one in a minute: the next ones are counted, not logged"),
                None,
                mod_id.is_some(),
            );
        }
        if queued {
            LogOutcome::Queued
        } else {
            LogOutcome::Unavailable
        }
    }

    fn write(&self, thread: &str, level: log::Level, message: &str) {
        self.send(thread, level, message, None, false);
    }

    fn send(
        &self,
        thread: &str,
        level: log::Level,
        message: &str,
        stderr: Option<String>,
        require_queue: bool,
    ) -> bool {
        let now = Local::now();
        // As log4j writes a multi-line message: the pattern once, the rest as it is.
        let lines: Vec<String> = message
            .lines()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format_line(&now, thread, level, line)
                } else {
                    line.to_owned()
                }
            })
            .collect();
        let tail: smallvec::SmallVec<[String; 1]> = lines
            .iter()
            .skip(lines.len().saturating_sub(RECENT))
            .cloned()
            .collect();
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        let result = self.out.send(Out::Write {
            date: now.date_naive(),
            lines,
            stderr,
        });
        if result.is_ok() || !require_queue {
            for line in tail {
                if recent.len() == RECENT {
                    recent.pop_front();
                }
                recent.push_back(line);
            }
        }
        drop(recent);
        match result {
            Ok(()) => true,
            Err(mpsc::SendError(Out::Write {
                stderr: Some(message),
                ..
            })) => {
                // Without a writer the terminal still gets Sunrise's messages.
                eprintln!("{message}");
                false
            }
            Err(_) => false,
        }
    }
}

/// Starts the log in `home/logs`. The main thread shows as `main_thread` in it. Messages are
/// still printed when the file cannot be opened.
pub fn init(home: &Path, main_thread: &'static str) {
    let dir = logs_dir(home);
    let file = match open_latest(&dir) {
        Ok(Some(latest)) => Some(latest),
        Ok(None) => {
            eprintln!(
                "Sunrise: another game is writing {}; this one logs to the terminal only",
                dir.join("latest.log").display()
            );
            None
        }
        Err(e) => {
            eprintln!(
                "Sunrise: could not open {}: {e}",
                dir.join("latest.log").display()
            );
            None
        }
    };
    let logger = LOGGER.get_or_init(|| {
        let (logger, inbox) = Logger::new(main_thread, file.is_some());
        let writer = Writer {
            file,
            dir,
            date: Local::now().date_naive(),
        };
        if let Err(e) = std::thread::Builder::new()
            .name("Log writer".into())
            .spawn(move || writer.run(inbox))
        {
            eprintln!("Sunrise: could not start the log writer: {e}");
        }
        logger
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(DEFAULT_LEVEL.to_level_filter());
    }
    mod_api::__private::game::install_mod_log_sink(
        |level| LOGGER.get().is_some_and(|logger| logger.mod_enabled(level)),
        |id, level, message, site| {
            LOGGER.get().map_or(LogOutcome::Unavailable, |logger| {
                logger.log_mod(id, level, message, site)
            })
        },
    );
}

/// A line in the log from code that prints it elsewhere itself (the dedicated server console).
pub fn write_only(level: log::Level, message: &str) {
    let current = std::thread::current();
    let main = LOGGER.get().map_or("main", |logger| logger.main_thread);
    let thread = match current.name() {
        Some("main") | None => main,
        Some(name) => name,
    };
    tap(thread, level, message);
    if let Some(logger) = LOGGER.get() {
        logger.write(thread, level, message);
    }
}

/// Writes `message` to the log file only, under the thread vanilla names for it (`Server thread` for
/// a login refused on a connection's own thread), as the console line beside it shows.
pub fn write_as(thread: &str, level: log::Level, message: &str) {
    tap(thread, level, message);
    if let Some(logger) = LOGGER.get() {
        logger.write(thread, level, message);
    }
}

/// The last lines written, oldest first (for crash reports).
pub fn recent_lines() -> Vec<String> {
    LOGGER.get().map_or_else(Vec::new, |l| {
        l.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    })
}

/// Flushes the log file (before the process exits).
pub fn flush() {
    if let Some(logger) = LOGGER.get() {
        log::Log::flush(logger);
    }
}

/// The last `lines` lines of a text file.
pub fn tail(path: &Path, lines: usize) -> io::Result<Vec<String>> {
    let text = fs::read(path)?;
    let text = String::from_utf8_lossy(&text);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| l.to_string())
        .collect())
}

/// The newest `crash-*.txt` in a game folder's crash-reports (names sort by time).
pub fn newest_crash_report(home: &Path) -> Option<PathBuf> {
    fs::read_dir(crash_reports_dir(home))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .max_by_key(|p| p.file_name().map(|n| n.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tap keeps lines only while on, never a line logged inside `untapped`, and at most
    /// `TAP_LINES`, dropping the oldest.
    #[test]
    fn the_tap_keeps_lines_while_on_and_skips_untapped_ones() {
        let probes = |lines: Vec<TappedLine>| -> Vec<String> {
            lines
                .into_iter()
                .filter(|l| l.message.starts_with("tap probe"))
                .map(|l| format!("{}/{}: {}", l.thread, l.level, l.message))
                .collect()
        };
        write_as("Server thread", log::Level::Info, "tap probe off");
        set_tap(true);
        write_as("Server thread", log::Level::Warn, "tap probe 1");
        untapped(|| write_as("Server thread", log::Level::Info, "tap probe hidden"));
        write_only(log::Level::Error, "tap probe 2");
        let thread = std::thread::current().name().unwrap_or("main").to_owned();
        assert_eq!(
            probes(take_tapped()),
            [
                "Server thread/WARN: tap probe 1".to_owned(),
                format!("{thread}/ERROR: tap probe 2"),
            ]
        );
        for i in 0..TAP_LINES + 5 {
            write_as("Server thread", log::Level::Info, &format!("tap probe {i}"));
        }
        let kept = take_tapped();
        assert!(kept.len() <= TAP_LINES);
        assert_eq!(
            kept.last().unwrap().message,
            format!("tap probe {}", TAP_LINES + 4)
        );
        // An oversized line is cut to the stated caps, at a character boundary: the message
        // ends in a three-byte character that straddles the cap, the thread name is long.
        let long = format!("tap probe long {}", "\u{20ac}".repeat(TAP_MESSAGE_BYTES));
        write_as(&"t".repeat(TAP_THREAD_BYTES + 50), log::Level::Info, &long);
        let kept = take_tapped();
        let line = kept.last().unwrap();
        assert_eq!(line.thread, "t".repeat(TAP_THREAD_BYTES));
        assert!(line.message.starts_with("tap probe long \u{20ac}"));
        assert!(line.message.len() <= TAP_MESSAGE_BYTES);
        assert!(
            TAP_MESSAGE_BYTES - line.message.len() < 3,
            "cut at the last whole character"
        );
        set_tap(false);
        write_as("Server thread", log::Level::Info, "tap probe after");
        assert!(probes(take_tapped()).is_empty());
    }

    #[test]
    fn native_mod_records_reach_the_file_with_their_targets() {
        use log::Log;
        let dir =
            std::env::temp_dir().join(format!("sunrise-native-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let writer = Writer {
            file: Some(open_latest(&dir).unwrap().unwrap()),
            dir: dir.clone(),
            date: Local::now().date_naive(),
        };
        let (logger, inbox) = Logger::new("Render thread", true);
        let handle = std::thread::spawn(move || writer.run(inbox));
        for (target, level, message) in [
            ("meridian_sunrise::save", log::Level::Warn, "save failed"),
            ("meridian_core::map", log::Level::Info, "map opened"),
            ("wgpu_core::device", log::Level::Error, "device lost"),
            ("meridian_core::map", log::Level::Debug, "debug detail"),
            ("meridian_core::map", log::Level::Trace, "trace detail"),
            ("calloop::loop_logic", log::Level::Warn, "calloop noise"),
            ("calloop::loop_logic", log::Level::Error, "calloop failed"),
            ("calloopish", log::Level::Warn, "another crate"),
        ] {
            logger.log(
                &log::Record::builder()
                    .args(format_args!("{message}"))
                    .level(level)
                    .target(target)
                    .build(),
            );
        }
        logger.flush();
        drop(logger);
        handle.join().unwrap();
        let text = fs::read_to_string(dir.join("latest.log")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert!(
            text.contains("[meridian_sunrise::save] save failed"),
            "{text}"
        );
        assert!(text.contains("[meridian_core::map] map opened"), "{text}");
        assert!(text.contains("[wgpu_core::device] device lost"), "{text}");
        assert!(!text.contains("debug detail"));
        assert!(!text.contains("trace detail"));
        assert!(!text.contains("calloop noise"), "{text}");
        assert!(
            text.contains("[calloop::loop_logic] calloop failed"),
            "{text}"
        );
        assert!(text.contains("[calloopish] another crate"), "{text}");
    }

    #[test]
    fn sunrise_stderr_and_external_file_records_stay_separate() {
        use log::Log;
        let (logger, inbox) = Logger::new("Render thread", true);
        let receive = &inbox.receiver;
        for target in ["sunrise_client::game", "meridian_core", "wgpu_hal::vulkan"] {
            logger.log(
                &log::Record::builder()
                    .args(format_args!("diagnostic"))
                    .level(log::Level::Info)
                    .target(target)
                    .build(),
            );
            let Out::Write { stderr, lines, .. } = receive.try_recv().unwrap() else {
                panic!("a diagnostic must write a record")
            };
            if ours(target) {
                assert_eq!(stderr.as_deref(), Some("diagnostic"));
                assert!(lines.iter().any(|line| line.ends_with("diagnostic")));
            } else {
                assert!(stderr.is_none());
                assert!(
                    lines
                        .iter()
                        .any(|line| line.ends_with(&format!("[{target}] diagnostic")))
                );
            }
        }
    }

    struct MustNotFormat;

    impl std::fmt::Display for MustNotFormat {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("a filtered or unavailable log must not format its message")
        }
    }

    #[test]
    fn a_level_filtered_mod_record_is_not_formatted_or_queued() {
        let (logger, inbox) = Logger::new("Render thread", true);
        assert_eq!(
            logger.log_mod(
                "example",
                log::Level::Debug,
                format_args!("{MustNotFormat}"),
                std::panic::Location::caller(),
            ),
            LogOutcome::Filtered
        );
        assert!(inbox.receiver.try_recv().is_err());
    }

    #[test]
    fn mod_logging_refuses_a_writer_without_a_log_file() {
        let (logger, inbox) = Logger::new("Render thread", false);
        assert!(!logger.mod_enabled(log::Level::Error));
        assert_eq!(
            logger.log_mod(
                "example",
                log::Level::Error,
                format_args!("{MustNotFormat}"),
                std::panic::Location::caller(),
            ),
            LogOutcome::Unavailable
        );
        assert!(inbox.receiver.try_recv().is_err());
    }

    #[test]
    fn mod_logging_refuses_a_disconnected_writer_before_filtering_or_formatting() {
        let (logger, inbox) = Logger::new("Render thread", true);
        assert!(logger.mod_enabled(log::Level::Error));
        drop(inbox);
        assert!(!logger.mod_enabled(log::Level::Error));
        for level in [log::Level::Error, log::Level::Debug] {
            assert_eq!(
                logger.log_mod(
                    "example",
                    level,
                    format_args!("{MustNotFormat}"),
                    std::panic::Location::caller(),
                ),
                LogOutcome::Unavailable
            );
        }
        assert!(logger.recent.lock().unwrap().is_empty());
    }

    #[test]
    fn mod_warnings_and_errors_share_a_flood_budget_per_mod_and_call_site() {
        let (logger, inbox) = Logger::new("Render thread", true);
        let receive = &inbox.receiver;
        let site = std::panic::Location::caller();
        for _ in 0..100 {
            assert_eq!(
                logger.log_mod("alpha", log::Level::Warn, format_args!("warning"), site),
                LogOutcome::Queued
            );
        }
        assert_eq!(
            logger.log_mod(
                "alpha",
                log::Level::Error,
                format_args!("{MustNotFormat}"),
                site
            ),
            LogOutcome::Filtered
        );
        assert_eq!(
            logger.log_mod("beta", log::Level::Error, format_args!("last error"), site),
            LogOutcome::Queued
        );
        // 100 alpha records, the existing flood notice, and one beta record.
        assert_eq!(receive.try_iter().count(), 102);
    }

    #[test]
    fn a_player_dir_keeps_the_shared_files_in_the_game_folder() {
        let (game, player) = (Path::new("/games/shared"), Path::new("/games/players/alex"));
        // options.txt moved to the player's folder: the enabled mods stay the game folder's.
        assert_eq!(shared_dir_of(Some(player), player, || game.into()), game);
        // With or without --player-dir: beside options.txt.
        assert_eq!(shared_dir_of(None, game, || unreachable!()), game);
        let profile = Path::new("/games/shared/profiles/local");
        assert_eq!(
            shared_dir_of(Some(player), profile, || unreachable!()),
            profile
        );
    }

    #[test]
    fn a_warning_repeated_every_packet_is_counted_past_a_hundred_a_minute() {
        let (mut floods, start) = (Floods::default(), Instant::now());
        let gates: Vec<Gate> = (0..140)
            .map(|i| {
                floods.gate(
                    None,
                    "lan_vehicles.rs".into(),
                    345,
                    start + Duration::from_millis(i * 7),
                )
            })
            .collect();
        assert!(
            gates[..99]
                .iter()
                .all(|g| *g == Gate::Write { left_out: 0 })
        );
        assert_eq!(gates[99], Gate::Last);
        assert!(gates[100..].iter().all(|g| *g == Gate::Skip));
        // Another line of code has its own count.
        assert_eq!(
            floods.gate(None, "lan_vehicles.rs".into(), 330, start),
            Gate::Write { left_out: 0 }
        );
        // The next minute writes again, and says how many were left out.
        assert_eq!(
            floods.gate(None, "lan_vehicles.rs".into(), 345, start + FLOOD_WINDOW),
            Gate::Write { left_out: 40 }
        );
        assert_eq!(
            floods.gate(None, "lan_vehicles.rs".into(), 345, start + FLOOD_WINDOW),
            Gate::Write { left_out: 0 }
        );
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn archives_are_named_like_vanillas() {
        assert_eq!(archive_name(day(4), 1), "2026-10-04-1.log.gz");
        let none: Vec<String> = Vec::new();
        assert_eq!(
            rollover(&none, day(14), 7),
            Rollover {
                delete: None,
                renames: vec![],
                archive: "2026-10-14-1.log.gz".into()
            }
        );
        // Another day's archives and unrelated files do not count.
        let names: Vec<String> = [
            "2026-10-13-1.log.gz",
            "2026-10-13-2.log.gz",
            "2026-10-14-1.log.gz",
            "2026-10-14-2.log.gz",
            "latest.log",
            "debug-1.log.gz",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(rollover(&names, day(14), 7).archive, "2026-10-14-3.log.gz");
        assert_eq!(rollover(&names, day(13), 7).archive, "2026-10-13-3.log.gz");
        assert_eq!(rollover(&names, day(15), 7).archive, "2026-10-15-1.log.gz");
    }

    #[test]
    fn the_eighth_archive_of_a_day_drops_the_oldest() {
        let names: Vec<String> = (1..=7).map(|i| archive_name(day(14), i)).collect();
        let plan = rollover(&names, day(14), 7);
        assert_eq!(plan.delete.as_deref(), Some("2026-10-14-1.log.gz"));
        assert_eq!(plan.renames.len(), 6);
        assert_eq!(
            plan.renames[0],
            ("2026-10-14-2.log.gz".into(), "2026-10-14-1.log.gz".into())
        );
        assert_eq!(
            plan.renames[5],
            ("2026-10-14-7.log.gz".into(), "2026-10-14-6.log.gz".into())
        );
        assert_eq!(plan.archive, "2026-10-14-7.log.gz");
    }

    #[test]
    fn lines_look_like_log4js_pattern() {
        let time = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap();
        assert_eq!(
            format_line(
                &time,
                "Render thread",
                log::Level::Info,
                "load assets: 1.20s"
            ),
            "[09:05:07] [Render thread/INFO]: load assets: 1.20s"
        );
        assert_eq!(
            format_line(&time, "Worker-1", log::Level::Warn, "x"),
            "[09:05:07] [Worker-1/WARN]: x"
        );
    }

    #[test]
    fn startup_archives_the_last_runs_log() {
        let dir = std::env::temp_dir().join(format!("sunrise-logs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Empty: nothing is archived (OnStartupTriggeringPolicy's minSize is 1 byte).
        drop(open_latest(&dir).unwrap().unwrap());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::write(
            dir.join("latest.log"),
            "[10:00:00] [Render thread/INFO]: first run\n",
        )
        .unwrap();
        let mut latest = open_latest(&dir).unwrap().unwrap();
        // A second game in the same folder leaves the file alone.
        assert!(open_latest(&dir).unwrap().is_none());
        writeln!(latest.file, "second run").unwrap();
        drop(latest);
        let today = Local::now().date_naive();
        let archive = dir.join(archive_name(today, 1));
        let mut text = String::new();
        flate2::read::GzDecoder::new(File::open(&archive).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "[10:00:00] [Render thread/INFO]: first run\n");
        assert_eq!(
            fs::read_to_string(dir.join("latest.log")).unwrap(),
            "second run\n"
        );
        assert_eq!(
            tail(&dir.join("latest.log"), 100).unwrap(),
            vec!["second run"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_crash_report_is_found_by_name() {
        let home = std::env::temp_dir().join(format!("sunrise-crash-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        assert!(newest_crash_report(&home).is_none());
        let dir = crash_reports_dir(&home);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "crash-2026-10-13_23.59.59-client.txt",
            "crash-2026-10-14_08.00.01-client.txt",
            "notes.txt",
        ] {
            fs::write(dir.join(name), "x").unwrap();
        }
        assert_eq!(
            newest_crash_report(&home).unwrap(),
            dir.join("crash-2026-10-14_08.00.01-client.txt")
        );
        let _ = fs::remove_dir_all(&home);
    }
}
