//! `RetiredPackBuilds` (#6721): a pack build dropped before it ended is kept until it ends, one
//! that already ended goes at once, and the renderer's drop waits for every one but those on a
//! lost device. Each wait here has a deadline, so a build joined too early fails its test instead
//! of hanging it.
use super::{BuildOrigin, PackBuild, Retired, RetiredPackBuilds, pack_post};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a test waits for something that must happen: far longer than a thread takes to
/// start and end on any machine CI runs.
const DEADLINE: Duration = Duration::from_secs(30);

/// A build that ends when `release` sends (or is dropped), on a device lost or not.
fn build_on(lost: bool) -> (Retired, mpsc::Sender<()>) {
    let (thread, release) = thread();
    let origin = BuildOrigin {
        device_lost: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(lost)),
        #[cfg(feature = "test-support")]
        watch: Default::default(),
    };
    (Retired { thread, origin }, release)
}

fn build() -> (Retired, mpsc::Sender<()>) {
    build_on(false)
}

fn thread() -> (PackBuild, mpsc::Sender<()>) {
    let (release, wait) = mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        let _ = wait.recv();
        Ok((
            Err(pack_post::Refused {
                reason: "test build".into(),
                notes: Vec::new(),
            }),
            Vec::new(),
            None,
        ))
    });
    (thread, release)
}

fn count(retired: &RetiredPackBuilds) -> usize {
    retired.lock().len()
}

/// `count`, failing after `DEADLINE` instead of blocking while something holds the list.
fn count_within_deadline(retired: &RetiredPackBuilds) -> usize {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Ok(builds) = retired.builds_for_test().try_lock() {
            return builds.len();
        }
        assert!(Instant::now() < deadline, "the retired builds stay locked");
        std::thread::yield_now();
    }
}

/// Waits for `ended` to hold, failing after `DEADLINE`.
fn wait_for(what: &str, ended: impl Fn() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !ended() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::yield_now();
    }
}

/// Runs `work` on a thread of its own and returns once it is done, failing after `DEADLINE`
/// (the thread is left blocked then, and released when the test's senders drop).
fn within_deadline(what: &str, work: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    assert!(finished.recv_timeout(DEADLINE).is_ok(), "{what}");
}

#[test]
fn reap_keeps_running_builds_and_joins_ended_ones() {
    let retired = RetiredPackBuilds::default();
    let (running, keep) = build();
    let (ended, release) = build();
    retired.retire(running);
    retired.retire(ended);
    let reaping = retired.clone();
    within_deadline("reap waited for a running build", move || reaping.reap());
    assert_eq!(count(&retired), 2, "neither build has ended");
    release.send(()).unwrap();
    wait_for("the released build did not end", || {
        retired.lock().iter().any(|b| b.thread.is_finished())
    });
    let reaping = retired.clone();
    within_deadline("reap waited for a running build", move || reaping.reap());
    assert_eq!(count(&retired), 1, "only the ended build is joined");
    drop(keep);
    retired.join_all();
    assert_eq!(count(&retired), 0);
}

#[test]
fn a_build_that_already_ended_is_joined_as_it_is_retired() {
    let retired = RetiredPackBuilds::default();
    let (ended, release) = build();
    release.send(()).unwrap();
    wait_for("the released build did not end", || {
        ended.thread.is_finished()
    });
    retired.retire(ended);
    assert_eq!(count(&retired), 0, "an ended build keeps its pack");
}

#[test]
fn join_all_waits_for_every_build() {
    let retired = RetiredPackBuilds::default();
    let (first, first_release) = build();
    let (second, second_release) = build();
    retired.retire(first);
    retired.retire(second);
    first_release.send(()).unwrap();
    let (done, finished) = mpsc::channel();
    let joining = retired.clone();
    let waiter = std::thread::spawn(move || {
        joining.join_all();
        let _ = done.send(());
    });
    // `join_all` took the builds: it runs.
    wait_for("join_all did not start", || {
        count_within_deadline(&retired) == 0
    });
    // And waits for the second, still held: it must not return until that ends.
    assert!(
        finished.recv_timeout(Duration::from_millis(200)).is_err(),
        "join_all returned before every build ended"
    );
    second_release.send(()).unwrap();
    assert!(
        finished.recv_timeout(DEADLINE).is_ok(),
        "join_all did not return once every build ended"
    );
    waiter.join().unwrap();
}

#[test]
fn join_all_lets_a_lost_devices_build_go() {
    let retired = RetiredPackBuilds::default();
    let (lost, keep) = build_on(true);
    retired.retire(lost);
    let joining = retired.clone();
    within_deadline("join_all waited for a lost device's build", move || {
        joining.join_all()
    });
    assert_eq!(count(&retired), 0);
    drop(keep);
}
