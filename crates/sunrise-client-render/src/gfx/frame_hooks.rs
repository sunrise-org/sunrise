//! E1, frame hooks (#2510): what changes a world frame, called by the renderer on the render thread.
use sunrise_hooks::TooLate;
use sunrise_hooks::points::Point;
use sunrise_hooks::points::engine::Active;

/// What changes a world frame; `seat` is the window's ([`sunrise_hooks::seat`]), and state per seat is the registrant's.
pub trait FrameHook: Sync {
    /// Read once per world frame: changes whenever what the registrant asks of the frame changes.
    fn generation(&self, _seat: usize) -> u64 {
        0
    }
}

static HOOKS: Point<dyn FrameHook> = Point::new();

/// Adds `hook` under the toggle `id` (`<crate>.<name>`), before the game starts.
pub fn register(id: &'static str, hook: &'static dyn FrameHook) -> Result<(), TooLate> {
    HOOKS.register(id, hook)
}

/// The renderer's side: the active hooks and the generation each gave last.
#[derive(Default)]
pub(crate) struct FrameHooks {
    active: Active<dyn FrameHook>,
    generations: Vec<u64>,
}

impl FrameHooks {
    /// Once per world frame: true when a hook was switched or changed its generation since the last call.
    pub(crate) fn poll(&mut self, seat: usize) -> bool {
        Self::poll_point(&mut self.active, &mut self.generations, &HOOKS, seat)
    }

    fn poll_point(
        active: &mut Active<dyn FrameHook>,
        generations: &mut Vec<u64>,
        point: &'static Point<dyn FrameHook>,
        seat: usize,
    ) -> bool {
        let mut changed = active.refresh(point);
        if changed {
            generations.clear();
            generations.resize(active.list().len(), 0);
        }
        for (hook, seen) in active.list().iter().zip(generations.iter_mut()) {
            let generation = hook.generation(seat);
            changed |= generation != *seen;
            *seen = generation;
        }
        changed
    }

    /// How many hooks are active.
    pub(crate) fn len(&self) -> usize {
        self.active.list().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    struct Counting {
        calls: AtomicUsize,
        generation: AtomicU64,
    }

    impl FrameHook for Counting {
        fn generation(&self, _seat: usize) -> u64 {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.generation.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn an_inactive_hook_is_never_called_and_only_its_own_changes_count() {
        static POINT: Point<dyn FrameHook> = Point::new();
        static ON: Counting = Counting {
            calls: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
        };
        static OFF: Counting = Counting {
            calls: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
        };
        POINT.register("test.frame_hooks.on", &ON).unwrap();
        POINT.register("test.frame_hooks.off", &OFF).unwrap();
        sunrise_hooks::points::set_active("test.frame_hooks.off", false);
        let (mut active, mut generations) = (Active::new(), Vec::new());
        let mut poll = || FrameHooks::poll_point(&mut active, &mut generations, &POINT, 0);
        assert!(poll(), "the first frame merges every hook");
        assert!(!poll());
        ON.generation.store(1, Ordering::Relaxed);
        assert!(poll());
        assert!(!poll());
        sunrise_hooks::points::set_active("test.frame_hooks.unrelated", false);
        assert!(!poll(), "another id's toggle is no change to this frame");
        assert_eq!(ON.calls.load(Ordering::Relaxed), 5);
        assert_eq!(OFF.calls.load(Ordering::Relaxed), 0);
    }
}
