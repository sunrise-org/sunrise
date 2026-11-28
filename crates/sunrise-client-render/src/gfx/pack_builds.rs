//! The threads `shaders/` packs build on (`Renderer::load_shader_pack`), once nobody waits for
//! their pack: each holds the device while the driver finishes the programs it was compiling, so
//! it is joined before the renderer drops, and handed on when another renderer takes over.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The thread a `shaders/` pack builds on: what it builds (`PackBuilt`), or the panic that
/// stopped it.
pub(crate) type PackBuild = std::thread::JoinHandle<std::result::Result<super::PackBuilt, String>>;

/// Where a pack build runs: its device's lost flag (`Renderer::device_lost`), and in tests the
/// watch of the renderer that started it.
#[derive(Clone)]
pub(crate) struct BuildOrigin {
    pub(crate) device_lost: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "test-support")]
    pub(crate) watch: PackBuildWatch,
}

impl BuildOrigin {
    fn lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A build nobody takes the pack of any more, with where it runs.
pub(crate) struct Retired {
    pub(crate) thread: PackBuild,
    pub(crate) origin: BuildOrigin,
}

/// Pack builds cancelled before anyone took their pack (`PackLoading` dropped). One that has
/// already ended is joined when it is retired, so its pack and the GPU objects in it go at once;
/// the others are joined as they end (`reap`, every `poll_shader_pack`, title frames included),
/// and before the renderer drops (`join_all`), except those whose device is lost: a compile in a
/// lost device's driver may never return, so they are left to end on their own. A cancelled
/// build stops at its next program: the wait is for the programs the driver was compiling then,
/// at most one per compile thread (`pack_post::compile`).
#[derive(Clone, Default)]
pub(crate) struct RetiredPackBuilds {
    builds: Arc<Mutex<Vec<Retired>>>,
    #[cfg(feature = "test-support")]
    pub(crate) watch: PackBuildWatch,
}

impl RetiredPackBuilds {
    pub(crate) fn retire(&self, build: Retired) {
        if build.thread.is_finished() {
            self.join(build);
        } else {
            self.lock().push(build);
        }
    }
    /// Joins the builds that ended; the others are kept.
    pub(crate) fn reap(&self) {
        let ended: Vec<Retired> = {
            let mut builds = self.lock();
            if builds.is_empty() {
                return;
            }
            let (ended, running) = builds.drain(..).partition(|b| b.thread.is_finished());
            *builds = running;
            ended
        };
        for build in ended {
            self.join(build);
        }
    }
    /// Waits for every build to end, but those whose device is lost, which are let go.
    pub(crate) fn join_all(&self) {
        for build in self.take() {
            if !build.origin.lost() {
                // Tests: a held build goes on once something waits for it.
                #[cfg(feature = "test-support")]
                build.origin.watch.release();
                self.join(build);
            }
        }
    }
    /// Every build, for another renderer to keep (`Renderer::take_pack_builds`).
    pub(crate) fn take(&self) -> Vec<Retired> {
        std::mem::take(&mut *self.lock())
    }
    fn join(&self, build: Retired) {
        // A cancelled build's pack, refusal or panic: nobody wants it any more.
        let _ = build.thread.join();
        #[cfg(feature = "test-support")]
        self.watch
            .0
            .joined
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    pub(crate) fn lock(&self) -> MutexGuard<'_, Vec<Retired>> {
        self.builds.lock().unwrap_or_else(PoisonError::into_inner)
    }
    #[cfg(test)]
    pub(crate) fn builds_for_test(&self) -> &Mutex<Vec<Retired>> {
        &self.builds
    }
}

/// The pack builds a renderer hands to the one replacing it (`Renderer::take_pack_builds`,
/// `Renderer::adopt_pack_builds`), so that a renderer swap doesn't wait for them; each keeps its
/// own device's lost flag.
pub struct PackBuilds(pub(crate) Vec<Retired>);

/// Tests: a renderer's pack builds as they run and end (`Renderer::pack_build_watch`), and a gate
/// that holds every build it starts before its first program, until released (`release`, or a
/// renderer about to wait for one of them, `RetiredPackBuilds::join_all`), so a test decides when
/// a build can end.
#[cfg(feature = "test-support")]
#[derive(Clone, Default)]
pub struct PackBuildWatch(Arc<WatchState>);

#[cfg(feature = "test-support")]
#[derive(Default)]
struct WatchState {
    running: std::sync::atomic::AtomicUsize,
    joined: std::sync::atomic::AtomicUsize,
    held: Mutex<bool>,
    opened: std::sync::Condvar,
    timed_out: std::sync::atomic::AtomicBool,
}

/// How long a held build waits before it goes on by itself (`PackBuildWatch::timed_out`): a test
/// that waits for a held build then fails with that flag instead of hanging. Test-only, well under
/// the game fixtures' 900 s child-case limit.
#[cfg(feature = "test-support")]
const GATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(feature = "test-support")]
impl PackBuildWatch {
    /// Builds started from now on wait before their first program.
    pub fn hold(&self) {
        *self.held() = true;
    }
    /// Lets every held build go on.
    pub fn release(&self) {
        *self.held() = false;
        self.0.opened.notify_all();
    }
    /// Builds started and not ended (their thread is still running).
    pub fn running(&self) -> usize {
        self.0.running.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Retired builds joined.
    pub fn joined(&self) -> usize {
        self.0.joined.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Whether a held build gave up waiting (`GATE_LIMIT`): something waited for it while it
    /// was held.
    pub fn timed_out(&self) -> bool {
        self.0.timed_out.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Counts a build as running until the guard drops, at the end of its thread.
    pub(crate) fn started(&self) -> RunningBuild {
        self.0
            .running
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        RunningBuild(self.clone())
    }
    /// Waits while builds are held.
    pub(crate) fn wait_gate(&self) {
        let held = self.held();
        let (_held, wait) = self
            .0
            .opened
            .wait_timeout_while(held, GATE_LIMIT, |held| *held)
            .unwrap_or_else(PoisonError::into_inner);
        if wait.timed_out() {
            self.0
                .timed_out
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn held(&self) -> MutexGuard<'_, bool> {
        self.0.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A build counted by `PackBuildWatch::running` while it lives.
#[cfg(feature = "test-support")]
pub(crate) struct RunningBuild(PackBuildWatch);

#[cfg(feature = "test-support")]
impl Drop for RunningBuild {
    fn drop(&mut self) {
        self.0
            .0
            .running
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
