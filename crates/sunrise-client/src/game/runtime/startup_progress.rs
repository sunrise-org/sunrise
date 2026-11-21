//! Mods' bars and lines on the startup loading screen (`RegistrationContext::progress` and
//! `message`, NeoForge's `StartupNotificationManager`): kept here, drawn under the stage bar by
//! the startup menu, or logged on a dedicated server, which has no screen.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How long a mod's line stays on the screen.
const LINE_TIME: Duration = Duration::from_secs(5);
/// The most bars drawn, newest first; older ones wait for a newer one to close.
const SHOWN_BARS: usize = 4;
/// The most lines drawn, newest last.
const SHOWN_LINES: usize = 3;
/// Bars open at once: past this, opening a bar returns `TooManyBars`.
const MAX_BARS: usize = 64;
/// Lines kept for drawing.
const MAX_LINES: usize = 32;
/// A label or line is cut to this many bytes when it is kept.
const MAX_TEXT: usize = 256;

struct Bar {
    label: String,
    done: u32,
    steps: u32,
    /// The tenths of the steps a dedicated server has logged.
    logged: u64,
}

struct Meter {
    screen: Arc<ModProgress>,
    bar: Arc<Mutex<Bar>>,
}

#[derive(Default)]
struct State {
    bars: Vec<Arc<Mutex<Bar>>>,
    lines: VecDeque<(String, Instant)>,
}

pub(crate) struct ModProgress {
    dedicated: bool,
    state: Mutex<State>,
}

static PROGRESS: OnceLock<Arc<ModProgress>> = OnceLock::new();

/// Hand mods the startup screen, before they register.
pub(crate) fn install(dedicated: bool) {
    let progress = PROGRESS.get_or_init(|| Arc::new(ModProgress::new(dedicated)));
    if let Err(error) = mod_api::__private::game::install_startup_screen(progress.clone()) {
        log::warn!("Sunrise: {error}");
    }
}

/// The bars to draw, newest first, as their label and done fraction, and the lines still shown.
pub(crate) fn shown() -> (Vec<(String, f32)>, Vec<String>) {
    PROGRESS
        .get()
        .map(|progress| progress.shown(Instant::now()))
        .unwrap_or_default()
}

/// `text` cut to [`MAX_TEXT`] bytes, on one line, without allocating.
fn one_line(text: &str) -> impl Iterator<Item = char> + Clone + '_ {
    text[..text.floor_char_boundary(MAX_TEXT)]
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
}

fn log_line(line: impl Iterator<Item = char> + Clone) {
    let line = std::fmt::from_fn(|f| {
        line.clone()
            .try_for_each(|c| std::fmt::Write::write_char(f, c))
    });
    log::info!("{line}");
}

impl ModProgress {
    fn new(dedicated: bool) -> Self {
        Self {
            dedicated,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn shown(&self, now: Instant) -> (Vec<(String, f32)>, Vec<String>) {
        let mut state = self.state();
        state
            .lines
            .retain(|(_, at)| now.saturating_duration_since(*at) < LINE_TIME);
        let bars = state
            .bars
            .iter()
            .rev()
            .take(SHOWN_BARS)
            .map(|bar| {
                let bar = bar.lock().unwrap_or_else(|p| p.into_inner());
                let done = if bar.steps == 0 {
                    0.0
                } else {
                    bar.done as f32 / bar.steps as f32
                };
                (bar.label.clone(), done)
            })
            .collect();
        let old = state.lines.len().saturating_sub(SHOWN_LINES);
        let lines = state
            .lines
            .iter()
            .skip(old)
            .map(|(line, _)| line.clone())
            .collect();
        (bars, lines)
    }
}

impl mod_api::StartupScreen for ModProgress {
    fn open(
        self: Arc<Self>,
        label: &str,
        steps: u32,
    ) -> Result<Box<dyn mod_api::ProgressMeter>, mod_api::StartupProgressError> {
        let mut state = self.state();
        if state.bars.len() >= MAX_BARS {
            return Err(mod_api::StartupProgressError::TooManyBars);
        }
        let label = one_line(label);
        let bar = Arc::new(Mutex::new(Bar {
            label: label.clone().collect(),
            done: 0,
            steps,
            logged: 0,
        }));
        state.bars.push(bar.clone());
        drop(state);
        if self.dedicated {
            log_line(label);
        }
        Ok(Box::new(Meter { screen: self, bar }))
    }

    fn message(&self, text: &str) {
        let text = one_line(text);
        if self.dedicated {
            log_line(text);
            return;
        }
        let mut state = self.state();
        state.lines.push_back((text.collect(), Instant::now()));
        if state.lines.len() > MAX_LINES {
            state.lines.pop_front();
        }
    }
}

impl Meter {
    /// Update just this bar, without scanning or locking the screen's list.
    fn update(&self, change: impl FnOnce(u32) -> u32) {
        let mut bar = self.bar.lock().unwrap_or_else(|p| p.into_inner());
        bar.done = change(bar.done).min(bar.steps);
        if self.screen.dedicated && bar.steps > 0 {
            let tenths = u64::from(bar.done) * 10 / u64::from(bar.steps);
            if tenths > bar.logged {
                bar.logged = tenths;
                let label = bar.label.clone();
                drop(bar);
                log::info!("{label}: {}%", tenths * 10);
            }
        }
    }
}

impl mod_api::ProgressMeter for Meter {
    fn step(&self) {
        self.update(|done| done.saturating_add(1));
    }

    fn set(&self, done: u32) {
        self.update(|_| done);
    }

    fn label(&self, text: &str) {
        let label = one_line(text);
        let mut bar = self.bar.lock().unwrap_or_else(|p| p.into_inner());
        if label.clone().eq(bar.label.chars()) {
            return;
        }
        bar.label.clear();
        bar.label.extend(label.clone());
        drop(bar);
        if self.screen.dedicated {
            log_line(label);
        }
    }

    fn message(&self, text: &str) {
        mod_api::StartupScreen::message(self.screen.as_ref(), text);
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.screen
            .state()
            .bars
            .retain(|kept| !Arc::ptr_eq(kept, &self.bar));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::StartupScreen;

    #[test]
    fn multibyte_labels_stop_at_the_last_complete_character() {
        let screen = Arc::new(ModProgress::new(false));
        let bar = screen
            .clone()
            .open(&format!("{}界", "é".repeat(127)), 1)
            .unwrap();
        assert_eq!(screen.shown(Instant::now()).0[0].0, "é".repeat(127));
        bar.label(&"é".repeat(129));
        assert_eq!(screen.shown(Instant::now()).0[0].0, "é".repeat(128));
        bar.message(&"é".repeat(128));
        assert_eq!(screen.shown(Instant::now()).1, ["é".repeat(128)]);
    }

    #[test]
    fn sixty_fifth_bar_is_refused_until_a_meter_is_dropped() {
        let screen = Arc::new(ModProgress::new(false));
        let mut meters: Vec<_> = (0..64)
            .map(|_| screen.clone().open("kept", 1).unwrap())
            .collect();
        assert!(matches!(
            screen.clone().open("refused", 1),
            Err(mod_api::StartupProgressError::TooManyBars)
        ));
        drop(meters.pop());
        meters.push(screen.clone().open("replacement", 1).unwrap());
        assert!(matches!(
            screen.clone().open("refused", 1),
            Err(mod_api::StartupProgressError::TooManyBars)
        ));
    }

    #[test]
    fn zero_step_bar_remains_finite_and_clamped() {
        let screen = Arc::new(ModProgress::new(false));
        let bar = screen.clone().open("empty", 0).unwrap();
        bar.step();
        bar.set(u32::MAX);
        assert_eq!(screen.shown(Instant::now()).0, [("empty".into(), 0.0)]);
    }

    #[test]
    fn thirty_third_message_keeps_the_newest_lines_and_expires() {
        let screen = ModProgress::new(false);
        for n in 0..33 {
            screen.message(&format!("message {n}"));
        }
        assert_eq!(screen.state().lines.len(), 32);
        let now = Instant::now();
        assert_eq!(
            screen.shown(now).1,
            ["message 30", "message 31", "message 32"]
        );
        assert!(screen.shown(now + Duration::from_secs(5)).1.is_empty());
    }

    #[test]
    fn bars_read_their_steps_newest_first_and_leave_when_dropped() {
        let screen = Arc::new(ModProgress::new(false));
        let first = screen.clone().open("Baking\nmodels", 10).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    first.step();
                    first.step();
                });
            }
        });
        let now = Instant::now;
        assert_eq!(
            screen.shown(now()).0,
            [("Baking models".to_owned(), 0.4_f32)]
        );
        first.set(99);
        assert_eq!(screen.shown(now()).0[0].1, 1.0);
        // Other bars with the same label still refer to distinct meters.
        let mut more: Vec<_> = (0..4)
            .map(|_| screen.clone().open("Models", 2).unwrap())
            .collect();
        more[2].step();
        assert_eq!(
            screen.shown(now()).0,
            [
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.5),
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.0),
            ]
        );
        // Dropping the updated bar leaves all its namesakes and reveals the older bar.
        drop(more.remove(2));
        assert_eq!(
            screen.shown(now()).0,
            [
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.0),
                ("Baking models".to_owned(), 1.0),
            ]
        );
        drop(more);
        drop(first);
        assert!(screen.shown(now()).0.is_empty());
    }
}
