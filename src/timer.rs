//! One clock: a countdown or a stopwatch that can be paused, reset and renamed.

use std::time::{Duration, Instant};

use crate::display::{Panel, Tone};
use crate::duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Countdown(Duration),
    Stopwatch,
}

/// A clock as asked for on the command line or in the form.
#[derive(Clone, Debug, PartialEq)]
pub struct ClockSpec {
    pub mode: Mode,
    pub title: Option<String>,
}

/// Elapsed time that can be paused, resumed and reset.
pub struct Clock {
    banked: Duration,
    since: Option<Instant>,
}

impl Clock {
    pub fn started() -> Self {
        Clock {
            banked: Duration::ZERO,
            since: Some(Instant::now()),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    pub fn toggle(&mut self) {
        match self.since.take() {
            Some(t) => self.banked += t.elapsed(),
            None => self.since = Some(Instant::now()),
        }
    }

    /// Back to zero, keeping whether it runs.
    pub fn reset(&mut self) {
        self.banked = Duration::ZERO;
        if self.running() {
            self.since = Some(Instant::now());
        }
    }

    /// Stops the clock at `at`, so it reads exactly that from now on.
    pub fn stop_at(&mut self, at: Duration) {
        self.since = None;
        self.banked = at;
    }
}

/// How long a clock flashes after its countdown reaches zero.
const FLASH_FOR: Duration = Duration::from_secs(4);
/// One flash on or off.
const FLASH_HALF: Duration = Duration::from_millis(250);
/// The bell rings at zero and twice more, this far apart.
const BELL_EVERY: Duration = Duration::from_secs(1);
const BELLS: u32 = 3;
/// A countdown turns red for its last seconds.
const WARN_SECS: u64 = 10;

pub struct Timer {
    pub title: Option<String>,
    pub mode: Mode,
    pub clock: Clock,
    pub done_at: Option<Instant>,
    bells_rung: u32,
    /// On the display view. At most four clocks are.
    pub shown: bool,
}

impl Timer {
    pub fn new(spec: ClockSpec, shown: bool) -> Self {
        Timer {
            title: spec.title,
            mode: spec.mode,
            clock: Clock::started(),
            done_at: None,
            bells_rung: 0,
            shown,
        }
    }

    pub fn is_countdown(&self) -> bool {
        matches!(self.mode, Mode::Countdown(_))
    }

    pub fn is_done(&self) -> bool {
        self.done_at.is_some()
    }

    pub fn is_paused(&self) -> bool {
        !self.clock.running() && !self.is_done()
    }

    /// Marks a countdown finished the moment it reaches zero.
    pub fn check_done(&mut self) {
        if let Mode::Countdown(total) = self.mode {
            if self.done_at.is_none() && self.clock.elapsed() >= total {
                self.clock.stop_at(total);
                self.done_at = Some(Instant::now());
            }
        }
    }

    /// Bells due since the countdown ended that have not rung yet.
    pub fn bells_due(&mut self) -> u32 {
        let Some(t) = self.done_at else { return 0 };
        let since = t.elapsed();
        let mut due = 0;
        while self.bells_rung < BELLS && since >= BELL_EVERY * self.bells_rung {
            self.bells_rung += 1;
            due += 1;
        }
        due
    }

    /// Pauses or resumes. A finished countdown stays finished.
    pub fn toggle(&mut self) {
        if !self.is_done() {
            self.clock.toggle();
        }
    }

    pub fn pause(&mut self) {
        if self.clock.running() && !self.is_done() {
            self.clock.toggle();
        }
    }

    pub fn resume(&mut self) {
        if !self.clock.running() && !self.is_done() {
            self.clock.toggle();
        }
    }

    /// Back to the start. A finished countdown starts over and runs.
    pub fn reset(&mut self) {
        if self.done_at.take().is_some() {
            self.restart();
        } else {
            self.clock.reset();
        }
    }

    /// From zero, running, as if just created.
    pub fn restart(&mut self) {
        self.clock = Clock::started();
        self.done_at = None;
        self.bells_rung = 0;
    }

    pub fn flashing(&self) -> bool {
        self.done_at.is_some_and(|t| {
            let since = t.elapsed();
            since < FLASH_FOR && (since.as_millis() / FLASH_HALF.as_millis()).is_multiple_of(2)
        })
    }

    pub fn face(&self) -> String {
        match self.mode {
            Mode::Countdown(total) => {
                let left = duration::remaining_secs(total, self.clock.elapsed());
                duration::countdown_face(left, total.as_secs() >= 3600)
            }
            Mode::Stopwatch => duration::stopwatch_face(self.clock.elapsed()),
        }
    }

    pub fn tone(&self) -> Tone {
        if self.is_done() {
            return Tone::Warn;
        }
        if self.is_paused() {
            return Tone::Paused;
        }
        match self.mode {
            Mode::Countdown(total)
                if duration::remaining_secs(total, self.clock.elapsed()) <= WARN_SECS =>
            {
                Tone::Warn
            }
            _ => Tone::Normal,
        }
    }

    /// The state in one word, as the display and the list show it.
    pub fn status(&self) -> &'static str {
        if self.is_done() {
            "TIME'S UP"
        } else if self.is_paused() {
            "PAUSED"
        } else {
            ""
        }
    }

    /// What the clock is called: its title, or what kind of clock it is.
    pub fn label(&self) -> String {
        match (&self.title, self.mode) {
            (Some(t), _) => t.clone(),
            (None, Mode::Countdown(d)) => duration::compact(d),
            (None, Mode::Stopwatch) => "stopwatch".into(),
        }
    }

    pub fn panel(&self) -> Panel {
        Panel {
            title: self.title.clone(),
            face: self.face(),
            tone: self.tone(),
            status: self.status(),
            flash: self.flashing(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countdown(ms: u64) -> Timer {
        let spec = ClockSpec {
            mode: Mode::Countdown(Duration::from_millis(ms)),
            title: None,
        };
        Timer::new(spec, true)
    }

    #[test]
    fn clock_pauses_and_resets() {
        let mut c = Clock::started();
        c.toggle();
        assert!(!c.running());
        let frozen = c.elapsed();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(c.elapsed(), frozen);
        c.toggle();
        assert!(c.running());
        std::thread::sleep(Duration::from_millis(20));
        assert!(c.elapsed() > frozen);
        c.reset();
        assert!(c.running());
        assert!(c.elapsed() < Duration::from_millis(15));
        c.stop_at(Duration::from_secs(5));
        assert_eq!(c.elapsed(), Duration::from_secs(5));
    }

    #[test]
    fn finished_countdown_resets_into_a_running_one() {
        let mut t = countdown(1);
        std::thread::sleep(Duration::from_millis(5));
        t.check_done();
        assert!(t.is_done());
        assert_eq!(t.status(), "TIME'S UP");
        assert_eq!(t.bells_due(), 1);
        assert_eq!(t.bells_due(), 0);
        t.toggle();
        assert!(t.is_done(), "a finished clock ignores pause");
        t.reset();
        assert!(!t.is_done());
        assert!(t.clock.running());
        assert_eq!(t.bells_rung, 0);
    }

    #[test]
    fn labels_fall_back_to_the_kind() {
        let mut t = countdown(90_000);
        assert_eq!(t.label(), "1m30s");
        t.title = Some("Tea".into());
        assert_eq!(t.label(), "Tea");
        let sw = Timer::new(
            ClockSpec {
                mode: Mode::Stopwatch,
                title: None,
            },
            false,
        );
        assert_eq!(sw.label(), "stopwatch");
    }

    #[test]
    fn pause_and_resume_are_idempotent() {
        let mut t = countdown(60_000);
        t.pause();
        t.pause();
        assert!(t.is_paused());
        assert_eq!(t.tone(), Tone::Paused);
        t.resume();
        t.resume();
        assert!(!t.is_paused());
    }
}
