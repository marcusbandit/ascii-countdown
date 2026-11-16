//! tock: a full screen countdown timer and stopwatch in big ASCII-art digits.

mod duration;
mod font;
mod screen;

use std::io;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use screen::{Screen, Tone, View};

const HELP: &str = "\
tock: full screen countdown timer and stopwatch in big ASCII-art digits

Usage:
  tock [options] <duration>     count down, then flash and ring the bell
  tock [options] stopwatch      count up from zero (same as --up)

Durations:
  25m  1h30m  90s  1h2m3s  10:00 (mm:ss)  1:30:00 (h:mm:ss)  45 (seconds)

Options:
  -t, --title <text>   show a label above the clock
  -u, --up             stopwatch mode, count up instead of down
      --no-bell        flash at zero without ringing the terminal bell
  -h, --help           print this help
  -V, --version        print the version

Keys:
  space   pause or resume
  r       reset
  q       quit (also Esc and Ctrl+C)
  any key quits once the countdown has finished

Exit status: 0 when the countdown finished or the stopwatch was stopped,
1 when a countdown was quit early, 2 on bad arguments.
";

#[derive(Debug, PartialEq)]
enum Mode {
    Countdown(Duration),
    Stopwatch,
}

#[derive(Debug, PartialEq)]
struct Options {
    mode: Mode,
    title: Option<String>,
    bell: bool,
}

#[derive(Debug, PartialEq)]
enum Cli {
    Run(Options),
    Help,
    Version,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut args = args.into_iter();
    let mut title = None;
    let mut up = false;
    let mut bell = true;
    let mut positional: Vec<String> = Vec::new();
    let mut only_positional = false;

    while let Some(arg) = args.next() {
        if only_positional || !arg.starts_with('-') || arg == "-" {
            positional.push(arg);
            continue;
        }
        match arg.as_str() {
            "--" => only_positional = true,
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            "-u" | "--up" => up = true,
            "--no-bell" => bell = false,
            "-t" | "--title" => {
                title = Some(args.next().ok_or(format!("{arg} needs a value"))?);
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--title=") {
                    title = Some(value.to_string());
                } else if let Some(value) = arg.strip_prefix("-t").filter(|v| !v.is_empty()) {
                    title = Some(value.to_string());
                } else {
                    return Err(format!("unknown option '{arg}'"));
                }
            }
        }
    }

    if positional.first().is_some_and(|p| p == "stopwatch") {
        up = true;
        positional.remove(0);
    }
    if positional.len() > 1 {
        return Err(format!(
            "expected one duration, got '{}'",
            positional.join(" ")
        ));
    }
    let mode = match (up, positional.pop()) {
        (true, None) => Mode::Stopwatch,
        (true, Some(d)) => return Err(format!("the stopwatch takes no duration, got '{d}'")),
        (false, Some(d)) => Mode::Countdown(duration::parse(&d)?),
        (false, None) => return Err("give a duration such as 25m, or 'stopwatch'".into()),
    };
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(Cli::Run(Options { mode, title, bell }))
}

/// Elapsed time that can be paused, resumed and reset.
struct Clock {
    banked: Duration,
    since: Option<Instant>,
}

impl Clock {
    fn started() -> Self {
        Clock {
            banked: Duration::ZERO,
            since: Some(Instant::now()),
        }
    }

    fn elapsed(&self) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    fn running(&self) -> bool {
        self.since.is_some()
    }

    fn toggle(&mut self) {
        match self.since.take() {
            Some(t) => self.banked += t.elapsed(),
            None => self.since = Some(Instant::now()),
        }
    }

    /// Back to zero, keeping whether it runs.
    fn reset(&mut self) {
        self.banked = Duration::ZERO;
        if self.running() {
            self.since = Some(Instant::now());
        }
    }

    /// Stops the clock at `at`, so it reads exactly that from now on.
    fn stop_at(&mut self, at: Duration) {
        self.since = None;
        self.banked = at;
    }
}

/// How long the screen flashes after a countdown reaches zero.
const FLASH_FOR: Duration = Duration::from_secs(4);
/// One flash on or off.
const FLASH_HALF: Duration = Duration::from_millis(250);
/// The bell rings at zero and twice more, this far apart.
const BELL_EVERY: Duration = Duration::from_secs(1);
const BELLS: u32 = 3;
/// Keys pressed this soon after zero do not quit, so a key that was already on
/// its way does not close the screen before anyone sees it.
const DONE_GRACE: Duration = Duration::from_millis(600);
/// The countdown turns red for its last seconds.
const WARN_SECS: u64 = 10;
/// How often the loop wakes up to see whether the face changed.
const TICK: Duration = Duration::from_millis(40);

enum Outcome {
    Finished,
    QuitEarly,
    Stopped(Duration),
    Interrupted,
}

fn run(opts: &Options, stop: &AtomicBool) -> io::Result<Outcome> {
    let mut screen = Screen::enter()?;
    let mut clock = Clock::started();
    let mut done_at: Option<Instant> = None;
    let mut bells_rung = 0;
    let mut last: Option<(View, (u16, u16))> = None;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(Outcome::Interrupted);
        }

        // Finish the countdown the moment it reaches zero.
        if let Mode::Countdown(total) = opts.mode {
            if done_at.is_none() && clock.elapsed() >= total {
                clock.stop_at(total);
                done_at = Some(Instant::now());
            }
        }

        if let Some(t) = done_at {
            let since = t.elapsed();
            while opts.bell && bells_rung < BELLS && since >= BELL_EVERY * bells_rung {
                screen.bell()?;
                bells_rung += 1;
            }
        }

        let view = view_for(opts, &clock, done_at);
        let size = crossterm::terminal::size()?;
        if last.as_ref() != Some(&(view.clone(), size)) {
            screen.draw(&view, size)?;
            last = Some((view, size));
        }

        if !event::poll(TICK)? {
            continue;
        }
        match event::read()? {
            Event::Resize(..) => {
                screen.clear()?;
                last = None;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(t) = done_at {
                    if is_interrupt(&key) || t.elapsed() >= DONE_GRACE {
                        return Ok(Outcome::Finished);
                    }
                    continue;
                }
                if is_interrupt(&key) {
                    return Ok(Outcome::Interrupted);
                }
                match key.code {
                    KeyCode::Char(' ') => clock.toggle(),
                    KeyCode::Char('r') | KeyCode::Char('R') => clock.reset(),
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        return Ok(match opts.mode {
                            Mode::Countdown(_) => Outcome::QuitEarly,
                            Mode::Stopwatch => Outcome::Stopped(clock.elapsed()),
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn is_interrupt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
}

/// Everything that decides what the screen shows, so it is redrawn only when
/// something visible changed.
fn view_for(opts: &Options, clock: &Clock, done_at: Option<Instant>) -> View {
    let paused = !clock.running() && done_at.is_none();
    let (face, mut tone) = match opts.mode {
        Mode::Countdown(total) => {
            let left = duration::remaining_secs(total, clock.elapsed());
            let face = duration::countdown_face(left, total.as_secs() >= 3600);
            (
                face,
                if left <= WARN_SECS {
                    Tone::Warn
                } else {
                    Tone::Normal
                },
            )
        }
        Mode::Stopwatch => (duration::stopwatch_face(clock.elapsed()), Tone::Normal),
    };
    let mut status = "";
    if paused {
        tone = Tone::Paused;
        status = "PAUSED";
    }
    let mut flash = false;
    if let Some(t) = done_at {
        let since = t.elapsed();
        tone = Tone::Warn;
        status = "TIME'S UP";
        flash = since < FLASH_FOR && (since.as_millis() / FLASH_HALF.as_millis()).is_multiple_of(2);
    }
    View {
        title: opts.title.clone(),
        face,
        tone,
        status,
        flash,
    }
}

fn main() -> ExitCode {
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run(opts)) => opts,
        Ok(Cli::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("tock {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("tock: {e}\nTry 'tock --help'.");
            return ExitCode::from(2);
        }
    };

    // SIGTERM, SIGHUP and a SIGINT from outside set a flag instead of killing
    // the process, so the loop can put the terminal back before exiting.
    let stop = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        if let Err(e) = signal_hook::flag::register(sig, Arc::clone(&stop)) {
            eprintln!("tock: cannot watch signal {sig}: {e}");
        }
    }
    screen::restore_on_panic();

    // The screen is dropped, and the terminal restored, before anything prints.
    match run(&opts, &stop) {
        Ok(Outcome::Finished) => ExitCode::SUCCESS,
        Ok(Outcome::QuitEarly) => ExitCode::from(1),
        Ok(Outcome::Stopped(elapsed)) => {
            println!("{}", duration::stopwatch_face(elapsed));
            ExitCode::SUCCESS
        }
        Ok(Outcome::Interrupted) => ExitCode::from(130),
        Err(e) if e.raw_os_error() == Some(6) => {
            // ENXIO from opening /dev/tty: no controlling terminal.
            eprintln!("tock: needs a terminal to draw in");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("tock: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Result<Cli, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    fn run_opts(args: &[&str]) -> Options {
        match cli(args) {
            Ok(Cli::Run(o)) => o,
            other => panic!("{args:?} gave {other:?}"),
        }
    }

    #[test]
    fn countdown_args() {
        let o = run_opts(&["25m"]);
        assert_eq!(o.mode, Mode::Countdown(Duration::from_secs(1500)));
        assert_eq!(o.title, None);
        assert!(o.bell);

        let o = run_opts(&["-t", "Tea", "--no-bell", "3m"]);
        assert_eq!(o.title.as_deref(), Some("Tea"));
        assert!(!o.bell);
        assert_eq!(
            run_opts(&["--title=Focus", "10:00"]).title.as_deref(),
            Some("Focus")
        );
        assert_eq!(
            run_opts(&["-tFocus", "10:00"]).title.as_deref(),
            Some("Focus")
        );
        assert_eq!(run_opts(&["10:00", "--title", "  "]).title, None);
    }

    #[test]
    fn stopwatch_args() {
        assert_eq!(run_opts(&["stopwatch"]).mode, Mode::Stopwatch);
        assert_eq!(run_opts(&["--up"]).mode, Mode::Stopwatch);
        assert_eq!(run_opts(&["-u", "-t", "Run"]).mode, Mode::Stopwatch);
        assert!(cli(&["--up", "5m"]).is_err());
        assert!(cli(&["stopwatch", "5m"]).is_err());
    }

    #[test]
    fn help_version_and_errors() {
        assert_eq!(cli(&["-h"]), Ok(Cli::Help));
        assert_eq!(cli(&["5m", "--version"]), Ok(Cli::Version));
        assert!(cli(&[]).is_err());
        assert!(cli(&["5m", "6m"]).is_err());
        assert!(cli(&["--frobnicate", "5m"]).is_err());
        assert!(cli(&["-t"]).is_err());
        assert!(cli(&["soon"]).is_err());
        // After --, a dash-looking argument is a duration, and a bad one.
        assert!(cli(&["--", "-5m"]).is_err());
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
}
