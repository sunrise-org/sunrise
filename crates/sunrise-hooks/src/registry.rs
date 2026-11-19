//! A list of values registered before the game runs and fixed when it starts: what
//! [`crate::register`], [`crate::settings::register`] and the other registries keep. Each one is
//! a `static`, usually of `&'static dyn Trait` items.
use std::sync::{Mutex, OnceLock};

use crate::TooLate;

pub(crate) struct Registry<T: Copy + 'static> {
    /// Registered before the list is sealed.
    pending: Mutex<Vec<T>>,
    /// The list the engine reads, fixed the first time it reads it.
    sealed: OnceLock<Box<[T]>>,
}

impl<T: Copy + 'static> Registry<T> {
    pub(crate) const fn new() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
            sealed: OnceLock::new(),
        }
    }

    pub(crate) fn register(&self, value: T) -> Result<(), TooLate> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // Checked under the lock: `all` seals under it too, so a value is never lost in between.
        if self.sealed.get().is_some() {
            return Err(TooLate);
        }
        pending.push(value);
        Ok(())
    }

    /// The registered values in registration order, sealing the list on the first call. Once
    /// sealed, one atomic load.
    pub(crate) fn all(&'static self) -> &'static [T] {
        if let Some(sealed) = self.sealed.get() {
            return sealed;
        }
        // Sealed under the lock `register` checks, so a value is either in the list or refused.
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        self.sealed
            .get_or_init(|| std::mem::take(&mut *pending).into_boxed_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_kept_before_the_seal_and_refused_after() {
        static LIST: Registry<u32> = Registry::new();
        let racers: Vec<_> = (0..8u32)
            .map(|i| std::thread::spawn(move || (i, LIST.register(i).is_ok())))
            .collect();
        let sealed = LIST.all();
        for racer in racers {
            let (value, accepted) = racer.join().unwrap();
            assert_eq!(
                accepted,
                sealed.contains(&value),
                "{value}: accepted means listed"
            );
        }
        assert_eq!(LIST.register(99), Err(TooLate));
        assert!(!LIST.all().contains(&99));
    }
}
