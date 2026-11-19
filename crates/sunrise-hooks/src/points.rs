//! Extension points (#2510): sealed registries whose registrants an Extras toggle ([`set_active`]) switches.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::TooLate;
use crate::registry::Registry;

/// One registry of registrants implementing `T`, a trait with empty default methods.
pub struct Point<T: ?Sized + 'static> {
    list: Registry<Entry<T>>,
}

struct Entry<T: ?Sized + 'static> {
    id: &'static str,
    value: &'static T,
}

impl<T: ?Sized + 'static> Clone for Entry<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized + 'static> Copy for Entry<T> {}

impl<T: ?Sized + 'static> Point<T> {
    pub const fn new() -> Self {
        Self {
            list: Registry::new(),
        }
    }

    /// Adds `value` under the toggle `id` (`<crate>.<name>`), before the game starts; dispatched in registration order.
    pub fn register(&self, id: &'static str, value: &'static T) -> Result<(), TooLate> {
        // Held across the check and the insertion, and by `engine::seal`, so nothing is added after the seal returns.
        let _seal = SEAL.lock().unwrap_or_else(|e| e.into_inner());
        if SEALED.load(Ordering::Acquire) {
            return Err(TooLate);
        }
        self.list.register(Entry { id, value })
    }
}

impl<T: ?Sized + 'static> Default for Point<T> {
    fn default() -> Self {
        Self::new()
    }
}

static SEALED: AtomicBool = AtomicBool::new(false);
/// Orders registrations against [`engine::seal`].
static SEAL: Mutex<()> = Mutex::new(());
/// Toggle ids switched off.
static INACTIVE: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
/// Bumped on every change of [`INACTIVE`].
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Switches every registrant under `id`, in every point, from the call sites' next refresh; ids start active.
pub fn set_active(id: &'static str, on: bool) {
    let mut inactive = INACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let at = inactive.iter().position(|&off| off == id);
    match (on, at) {
        (true, Some(at)) => {
            inactive.swap_remove(at);
        }
        (false, None) => inactive.push(id),
        _ => return,
    }
    GENERATION.fetch_add(1, Ordering::Release);
}

/// The setting last given for `id`; a frame already running finishes with the list it had.
pub fn is_active(id: &str) -> bool {
    let inactive = INACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    !inactive.contains(&id)
}

/// The engine's side: where it seals the points and keeps the lists it dispatches to.
pub mod engine {
    use super::*;

    /// Refuses registrations from now on. [`crate::engine::seal`] calls it.
    pub fn seal() {
        let _seal = SEAL.lock().unwrap_or_else(|e| e.into_inner());
        SEALED.store(true, Ordering::Release);
    }

    /// A call site's list of a point's active registrants.
    pub struct Active<T: ?Sized + 'static> {
        seen: u64,
        /// The active entries' places in the sealed list.
        indices: Vec<usize>,
        list: Vec<&'static T>,
    }

    impl<T: ?Sized + 'static> Active<T> {
        pub const fn new() -> Self {
            Self {
                seen: u64::MAX,
                indices: Vec::new(),
                list: Vec::new(),
            }
        }

        /// Follows the toggles since the last call; true only when this point's active list changed.
        pub fn refresh(&mut self, point: &'static Point<T>) -> bool {
            let generation = GENERATION.load(Ordering::Acquire);
            if generation == self.seen {
                return false;
            }
            self.seen = generation;
            let inactive = INACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            let all = point.list.all();
            let active = (0..all.len()).filter(|&i| !inactive.contains(&all[i].id));
            if active.clone().eq(self.indices.iter().copied()) {
                return false;
            }
            self.indices = active.collect();
            self.list = self.indices.iter().map(|&i| all[i].value).collect();
            true
        }

        /// The active registrants, in registration order.
        pub fn list(&self) -> &[&'static T] {
            &self.list
        }
    }

    impl<T: ?Sized + 'static> Default for Active<T> {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::engine::Active;
    use super::*;
    use std::sync::atomic::AtomicUsize;

    trait Counted: Sync {
        fn call(&self);
    }

    struct Counter(AtomicUsize);

    impl Counted for Counter {
        fn call(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn an_inactive_registrant_is_never_dispatched() {
        static POINT: Point<dyn Counted> = Point::new();
        static ON: Counter = Counter(AtomicUsize::new(0));
        static OFF: Counter = Counter(AtomicUsize::new(0));
        POINT.register("test.points.on", &ON).unwrap();
        POINT.register("test.points.off", &OFF).unwrap();
        set_active("test.points.off", false);
        let mut active = Active::new();
        assert!(active.refresh(&POINT));
        for _ in 0..3 {
            active.refresh(&POINT);
            active.list().iter().for_each(|r| r.call());
        }
        assert_eq!(ON.0.load(Ordering::Relaxed), 3);
        assert_eq!(OFF.0.load(Ordering::Relaxed), 0);

        set_active("test.points.unrelated", false);
        assert!(
            !active.refresh(&POINT),
            "another id's toggle leaves this list alone"
        );
        set_active("test.points.off", true);
        assert!(active.refresh(&POINT));
        active.list().iter().for_each(|r| r.call());
        assert_eq!(OFF.0.load(Ordering::Relaxed), 1);
        assert!(is_active("test.points.off"));
        set_active("test.points.on", false);
        assert!(active.refresh(&POINT), "an active registrant switched off");
        active.list().iter().for_each(|r| r.call());
        assert_eq!(
            (ON.0.load(Ordering::Relaxed), OFF.0.load(Ordering::Relaxed)),
            (4, 2)
        );
    }
}
