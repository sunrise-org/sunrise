//! The delayed tasks mods scheduled in the running world (`schedule_task`): a queue counted in
//! server ticks, owned by the world's lifecycle and dropped with it.

use mod_api::{MAX_PENDING_TASKS, ScheduleError, ServerHost, TaskId};
use std::{
    collections::{BTreeMap, HashMap},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
};

pub(super) type ServerTask = Box<dyn FnOnce(&mut dyn ServerHost)>;
/// The owning mod's `[mod] id` and its callable, detached from the scheduler.
pub(super) type PendingTask = (&'static str, ServerTask);

/// Runs every item `next` yields through `run`, even when one run panics: the other items still
/// run, and the first panic comes back for the caller to put back in flight once it has restored
/// its own state. `state` is passed to both so each may borrow the whole of it in turn.
pub(super) fn run_all<S, T>(
    state: &mut S,
    next: fn(&mut S) -> Option<T>,
    run: fn(&mut S, T),
) -> Option<Box<dyn std::any::Any + Send>> {
    let mut panic = None;
    while let Some(item) = next(state) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| run(state, item))) {
            if panic.is_none() {
                panic = Some(payload);
            } else if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
                // A later panic's payload may itself panic on drop. Keep draining, and never
                // run another destructor on behalf of a payload we aren't going to report.
                std::mem::forget(payload);
            }
        }
    }
    panic
}

#[derive(Default)]
pub(super) struct ScheduledTasks {
    /// Ticks advanced since the world opened.
    now: u64,
    next_id: u64,
    /// Waiting tasks by due tick, then id (the order they were scheduled).
    queue: BTreeMap<(u64, u64), PendingTask>,
    /// Each waiting task's due tick, to find it again on cancel.
    due: HashMap<u64, u64>,
    /// How many tasks each mod has waiting.
    pending: HashMap<&'static str, usize>,
}

impl ScheduledTasks {
    /// A task for `owner` due `delay_ticks` ticks from now (0 counts as 1: never this tick).
    pub(super) fn schedule(
        &mut self,
        owner: Option<&'static str>,
        delay_ticks: u32,
        task: ServerTask,
    ) -> Result<TaskId, ScheduleError> {
        let owner = owner.ok_or(ScheduleError::NotInMod)?;
        let waiting = self.pending.entry(owner).or_insert(0);
        if *waiting >= MAX_PENDING_TASKS {
            return Err(ScheduleError::TooMany);
        }
        *waiting += 1;
        self.next_id += 1;
        let id = self.next_id;
        let due = self.now.saturating_add(u64::from(delay_ticks.max(1)));
        self.queue.insert((due, id), (owner, task));
        self.due.insert(id, due);
        Ok(TaskId(id))
    }

    /// Withdraw `id` if `owner` scheduled it and it is still waiting. The caller gets the
    /// callable back only once all bookkeeping is done, so dropping its captures is safe.
    pub(super) fn cancel(&mut self, owner: &'static str, id: TaskId) -> Option<ServerTask> {
        let &due = self.due.get(&id.0)?;
        let &(waiting_owner, _) = self.queue.get(&(due, id.0))?;
        if waiting_owner != owner {
            return None;
        }
        self.remove(due, id.0).map(|(_, task)| task)
    }

    /// A server tick starts: "now" moves to it, before any hook of the tick runs, so a task
    /// scheduled by one is measured from this tick and never fires in it.
    pub(super) fn begin_tick(&mut self) {
        self.now += 1;
    }

    /// The next task due by now, taken off the queue before any of its mod code runs. A task
    /// scheduled while others run is due on a later tick.
    pub(super) fn pop_due(&mut self) -> Option<PendingTask> {
        let (&(due, id), _) = self.queue.first_key_value()?;
        if due > self.now {
            return None;
        }
        self.remove(due, id)
    }

    /// The world closed or opened: detach the old queue before running any capture destructors.
    pub(super) fn clear(&mut self) {
        let mut retired = std::mem::take(self).queue.into_values();
        let panic = run_all(&mut retired, Iterator::next, |_, (owner, task)| {
            let _call = mod_api::__private::enter(owner, "schedule_task");
            drop(task);
        });
        if let Some(payload) = panic {
            resume_unwind(payload);
        }
    }

    fn remove(&mut self, due: u64, id: u64) -> Option<PendingTask> {
        let task = self.queue.remove(&(due, id))?;
        self.due.remove(&id);
        if let Some(waiting) = self.pending.get_mut(task.0) {
            *waiting -= 1;
        }
        Some(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::testing::FakeHost;
    use std::{cell::RefCell, rc::Rc};

    const A: Option<&str> = Some("a");
    const B: Option<&str> = Some("b");

    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("destructor panicked");
        }
    }

    fn advance(tasks: &mut ScheduledTasks) -> Vec<PendingTask> {
        tasks.begin_tick();
        std::iter::from_fn(|| tasks.pop_due()).collect()
    }

    #[test]
    fn tasks_run_after_their_delay_in_the_order_they_were_scheduled() {
        let mut tasks = ScheduledTasks::default();
        let heard = Rc::new(RefCell::new(Vec::new()));
        for (owner, delay, value) in [(A, 2, 20), (A, 1, 10), (B, 2, 21), (B, 0, 0)] {
            let heard = Rc::clone(&heard);
            tasks
                .schedule(
                    owner,
                    delay,
                    Box::new(move |_| heard.borrow_mut().push(value)),
                )
                .unwrap();
        }
        let mut host = FakeHost::new();
        for (_, task) in advance(&mut tasks) {
            task(&mut host);
        }
        assert_eq!(*heard.borrow(), [10, 0]);
        for (_, task) in advance(&mut tasks) {
            task(&mut host);
        }
        assert_eq!(*heard.borrow(), [10, 0, 20, 21]);
        assert!(advance(&mut tasks).is_empty());
    }

    #[test]
    fn only_the_owner_cancels_a_waiting_task() {
        let mut tasks = ScheduledTasks::default();
        let capture = Rc::new(());
        let owned = Rc::clone(&capture);
        let id = tasks
            .schedule(A, 3, Box::new(move |_| drop(owned)))
            .unwrap();
        assert!(tasks.cancel("b", id).is_none());
        assert_eq!(Rc::strong_count(&capture), 2);
        assert!(tasks.cancel("a", id).is_some());
        assert_eq!(Rc::strong_count(&capture), 1);
        assert!(tasks.cancel("a", id).is_none());
        assert!(tasks.cancel("a", TaskId(99)).is_none());
        for _ in 0..3 {
            assert!(advance(&mut tasks).is_empty());
        }
        let fired = tasks.schedule(A, 1, Box::new(|_| {})).unwrap();
        assert_eq!(advance(&mut tasks).len(), 1);
        assert!(tasks.cancel("a", fired).is_none());
    }

    #[test]
    fn a_mod_outside_its_hooks_cannot_schedule_and_one_mod_is_capped() {
        let mut tasks = ScheduledTasks::default();
        assert_eq!(
            tasks.schedule(None, 1, Box::new(|_| {})),
            Err(ScheduleError::NotInMod)
        );
        let first = tasks.schedule(A, 5, Box::new(|_| {})).unwrap();
        for _ in 1..MAX_PENDING_TASKS {
            tasks.schedule(A, 5, Box::new(|_| {})).unwrap();
        }
        assert_eq!(
            tasks.schedule(A, 5, Box::new(|_| {})),
            Err(ScheduleError::TooMany)
        );
        assert!(tasks.schedule(B, 5, Box::new(|_| {})).is_ok());
        assert!(tasks.cancel("a", first).is_some());
        assert!(tasks.schedule(A, 5, Box::new(|_| {})).is_ok());
        for _ in 0..5 {
            advance(&mut tasks);
        }
        assert!(tasks.schedule(A, 1, Box::new(|_| {})).is_ok());
    }

    #[test]
    fn the_largest_delay_does_not_wrap() {
        let mut tasks = ScheduledTasks::default();
        tasks.schedule(A, u32::MAX, Box::new(|_| {})).unwrap();
        assert!(advance(&mut tasks).is_empty());
    }

    #[test]
    fn closing_the_world_detaches_all_tasks_before_dropping_captures() {
        let mut tasks = ScheduledTasks::default();
        let panics = PanicOnDrop;
        tasks
            .schedule(A, 1, Box::new(move |_| drop(panics)))
            .unwrap();
        let capture = Rc::new(());
        let owned = Rc::clone(&capture);
        tasks
            .schedule(B, 1, Box::new(move |_| drop(owned)))
            .unwrap();
        let panic = catch_unwind(AssertUnwindSafe(|| tasks.clear())).unwrap_err();
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"destructor panicked"));
        assert_eq!(Rc::strong_count(&capture), 1);
        assert!(advance(&mut tasks).is_empty());
        assert_eq!(tasks.schedule(A, 1, Box::new(|_| {})).unwrap(), TaskId(1));
    }

    #[test]
    fn a_panicking_task_does_not_drop_the_others() {
        let mut state = (ScheduledTasks::default(), FakeHost::new());
        state
            .0
            .schedule(A, 1, Box::new(|_| panic!("first")))
            .unwrap();
        state
            .0
            .schedule(A, 1, Box::new(|_| std::panic::panic_any(PanicOnDrop)))
            .unwrap();
        let heard = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&heard);
        state
            .0
            .schedule(B, 1, Box::new(move |_| log.borrow_mut().push("third")))
            .unwrap();
        state.0.begin_tick();
        let panic = run_all(&mut state, |s| s.0.pop_due(), |s, (_, task)| task(&mut s.1)).unwrap();
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"first"));
        assert_eq!(*heard.borrow(), ["third"]);
        assert!(state.0.pop_due().is_none());
    }
}
