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

use screen::{Panel, Screen, Tone};

const HELP: &str = "\
tock: full screen countdown timer and stopwatch in big ASCII-art digits

Usage:
  tock [options] <clock>...

  Each clock is a duration to count down from, or 'stopwatch' to count up.
  Give several to run them side by side:

  tock 25m
  tock -t Tea 4m -t Pasta 10m stopwatch

Durations:
  25m  1h30m  90s  1h2m3s  10:00 (mm:ss)  1:30:00 (h:mm:ss)  45 (seconds)

Options:
  -t, --title <text>   label the clock that follows (or the last one)
  -u, --up             add a stopwatch, same as 'stopwatch'
      --no-bell        flash at zero without ringing the terminal bell
  -h, --help           print this help
  -V, --version        print the version

Keys:
  space   pause or resume every clock
  1-9     pause or resume one clock
  r       reset every clock
  q       quit (also Esc and Ctrl+C)
  any key quits once every countdown has finished

Stopwatches print the time they stopped at, one line each, in order.
Exit status: 0 when every countdown finished, 1 when one was quit early,
2 on bad arguments, 130 when interrupted.
";

/// The number keys pick a clock, so there can be nine at most.
const MAX_CLOCKS: usize = 9;

#[derive(Debug, PartialEq)]
enum Mode {
    Countdown(Duration),
    Stopwatch,
}

#[derive(Debug, PartialEq)]
struct ClockSpec {
    mode: Mode,
    title: Option<String>,
}

#[derive(Debug, PartialEq)]
struct Options {
    clocks: Vec<ClockSpec>,
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
    let mut clocks: Vec<ClockSpec> = Vec::new();
    // A title waits here for the clock that follows it.
    let mut title: Option<String> = None;
    let mut bell = true;
    let mut only_positional = false;

    let set_title = |title: &mut Option<String>, value: String| {
        if let Some(old) = title.replace(value.clone()) {
            return Err(format!("two titles in a row: '{old}' and '{value}'"));
        }
        Ok(())
    };
    let push = |clocks: &mut Vec<ClockSpec>, title: &mut Option<String>, mode: Mode| {
        let title = title.take().map(|t| t.trim().to_string());
        clocks.push(ClockSpec {
            mode,
            title: title.filter(|t| !t.is_empty()),
        });
    };

    while let Some(arg) = args.next() {
        if only_positional || !arg.starts_with('-') || arg == "-" {
            let mode = if arg == "stopwatch" {
                Mode::Stopwatch
            } else {
                Mode::Countdown(duration::parse(&arg)?)
            };
            push(&mut clocks, &mut title, mode);
            continue;
        }
        match arg.as_str() {
            "--" => only_positional = true,
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            "-u" | "--up" => push(&mut clocks, &mut title, Mode::Stopwatch),
            "--no-bell" => bell = false,
            "-t" | "--title" => {
                let value = args.next().ok_or(format!("{arg} needs a value"))?;
                set_title(&mut title, value)?;
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--title=") {
                    set_title(&mut title, value.to_string())?;
                } else if let Some(value) = arg.strip_prefix("-t").filter(|v| !v.is_empty()) {
                    set_title(&mut title, value.to_string())?;
                } else {
                    return Err(format!("unknown option '{arg}'"));
                }
            }
        }
    }

    // A title after the last clock names that clock, as in `tock 25m -t Focus`.
    if let Some(t) = title {
        match clocks.last_mut() {
            Some(last) if last.title.is_none() => {
                let t = t.trim().to_string();
                last.title = Some(t).filter(|t| !t.is_empty());
            }
            Some(_) => {
                return Err(format!(
                    "title '{t}' comes after the last clock; put it before the clock it names"
                ))
            }
            None => {}
        }
    }
    if clocks.is_empty() {
        return Err("give a duration such as 25m, or 'stopwatch'".into());
    }
    if clocks.len() > MAX_CLOCKS {
        return Err(format!(
            "{} clocks, but {MAX_CLOCKS} is the most tock runs at once",
            clocks.len()
        ));
    }
    Ok(Cli::Run(Options { clocks, bell }))
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

/// How long a panel flashes after its countdown reaches zero.
const FLASH_FOR: Duration = Duration::from_secs(4);
/// One flash on or off.
const FLASH_HALF: Duration = Duration::from_millis(250);
/// The bell rings at zero and twice more, this far apart.
const BELL_EVERY: Duration = Duration::from_secs(1);
const BELLS: u32 = 3;
/// Keys pressed this soon after the last countdown ends do not quit, so a key
/// that was already on its way does not close the screen before anyone sees it.
const DONE_GRACE: Duration = Duration::from_millis(600);
/// A countdown turns red for its last seconds.
const WARN_SECS: u64 = 10;
/// How often the loop wakes up to see whether a face changed.
const TICK: Duration = Duration::from_millis(40);

/// One clock while it runs.
struct Running<'a> {
    spec: &'a ClockSpec,
    clock: Clock,
    done_at: Option<Instant>,
    bells_rung: u32,
}

impl Running<'_> {
    fn is_countdown(&self) -> bool {
        matches!(self.spec.mode, Mode::Countdown(_))
    }

    /// Marks a countdown finished the moment it reaches zero.
    fn check_done(&mut self) {
        if let Mode::Countdown(total) = self.spec.mode {
            if self.done_at.is_none() && self.clock.elapsed() >= total {
                self.clock.stop_at(total);
                self.done_at = Some(Instant::now());
            }
        }
    }

    /// Bells due since the countdown ended that have not rung yet.
    fn bells_due(&mut self) -> u32 {
        let Some(t) = self.done_at else { return 0 };
        let since = t.elapsed();
        let mut due = 0;
        while self.bells_rung < BELLS && since >= BELL_EVERY * self.bells_rung {
            self.bells_rung += 1;
            due += 1;
        }
        due
    }

    fn toggle(&mut self) {
        if self.done_at.is_none() {
            self.clock.toggle();
        }
    }

    /// Back to the start. A finished countdown starts over and runs.
    fn reset(&mut self) {
        if self.done_at.take().is_some() {
            self.clock = Clock::started();
            self.bells_rung = 0;
        } else {
            self.clock.reset();
        }
    }
}

enum Outcome {
    /// Every countdown finished, or the user quit with none left running.
    Finished(Vec<Duration>),
    /// Quit while a countdown was still going.
    QuitEarly(Vec<Duration>),
    Interrupted,
}

fn run(opts: &Options, stop: &AtomicBool) -> io::Result<Outcome> {
    let mut screen = Screen::enter()?;
    let mut clocks: Vec<Running> = opts
        .clocks
        .iter()
        .map(|spec| Running {
            spec,
            clock: Clock::started(),
            done_at: None,
            bells_rung: 0,
        })
        .collect();
    let multi = clocks.len() > 1;
    let mut last: Option<(Vec<Panel>, (u16, u16))> = None;

    let stopwatch_times = |clocks: &[Running]| -> Vec<Duration> {
        clocks
            .iter()
            .filter(|c| !c.is_countdown())
            .map(|c| c.clock.elapsed())
            .collect()
    };

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(Outcome::Interrupted);
        }

        let mut bells = 0;
        for c in &mut clocks {
            c.check_done();
            bells += c.bells_due();
        }
        // Several clocks ending together ring once, not once each.
        if opts.bell && bells > 0 {
            screen.bell()?;
        }

        let panels: Vec<Panel> = clocks
            .iter()
            .enumerate()
            .map(|(i, c)| panel_for(c, multi.then_some(i + 1)))
            .collect();
        let size = crossterm::terminal::size()?;
        if last.as_ref() != Some(&(panels.clone(), size)) {
            screen.draw(&panels, size)?;
            last = Some((panels, size));
        }

        if !event::poll(TICK)? {
            continue;
        }
        let key = match event::read()? {
            Event::Resize(..) => {
                screen.clear()?;
                last = None;
                continue;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => key,
            _ => continue,
        };

        // Every clock is a finished countdown: any key quits.
        let all_done = clocks.iter().all(|c| c.done_at.is_some());
        if all_done {
            let latest = clocks.iter().filter_map(|c| c.done_at).max();
            if is_interrupt(&key) || latest.is_some_and(|t| t.elapsed() >= DONE_GRACE) {
                return Ok(Outcome::Finished(Vec::new()));
            }
            continue;
        }
        if is_interrupt(&key) {
            return Ok(Outcome::Interrupted);
        }
        match key.code {
            KeyCode::Char(' ') => {
                // Pause everything that runs; if nothing runs, resume all.
                let live = clocks.iter_mut().filter(|c| c.done_at.is_none());
                let live: Vec<&mut Running> = live.collect();
                let any_running = live.iter().any(|c| c.clock.running());
                for c in live {
                    if c.clock.running() == any_running {
                        c.toggle();
                    }
                }
            }
            KeyCode::Char(d @ '1'..='9') => {
                let i = d as usize - '1' as usize;
                if let Some(c) = clocks.get_mut(i) {
                    c.toggle();
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => clocks.iter_mut().for_each(Running::reset),
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                let times = stopwatch_times(&clocks);
                let unfinished = clocks
                    .iter()
                    .any(|c| c.is_countdown() && c.done_at.is_none());
                return Ok(if unfinished {
                    Outcome::QuitEarly(times)
                } else {
                    Outcome::Finished(times)
                });
            }
            _ => {}
        }
    }
}

fn is_interrupt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
}

/// Everything that decides what one clock's panel shows, so the screen is
/// redrawn only when something visible changed. `number` is the key that
/// picks this clock, shown when there is more than one.
fn panel_for(c: &Running, number: Option<usize>) -> Panel {
    let paused = !c.clock.running() && c.done_at.is_none();
    let (face, mut tone) = match c.spec.mode {
        Mode::Countdown(total) => {
            let left = duration::remaining_secs(total, c.clock.elapsed());
            let face = duration::countdown_face(left, total.as_secs() >= 3600);
            let tone = if left <= WARN_SECS {
                Tone::Warn
            } else {
                Tone::Normal
            };
            (face, tone)
        }
        Mode::Stopwatch => (duration::stopwatch_face(c.clock.elapsed()), Tone::Normal),
    };
    let mut status = "";
    if paused {
        tone = Tone::Paused;
        status = "PAUSED";
    }
    let mut flash = false;
    if let Some(t) = c.done_at {
        let since = t.elapsed();
        tone = Tone::Warn;
        status = "TIME'S UP";
        flash = since < FLASH_FOR && (since.as_millis() / FLASH_HALF.as_millis()).is_multiple_of(2);
    }
    let title = match (number, c.spec.title.as_deref()) {
        (Some(n), Some(t)) => Some(format!("{n}  {t}")),
        (Some(n), None) => Some(n.to_string()),
        (None, t) => t.map(str::to_string),
    };
    Panel {
        title,
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

    let print_times = |times: &[Duration]| {
        for t in times {
            println!("{}", duration::stopwatch_face(*t));
        }
    };

    // The screen is dropped, and the terminal restored, before anything prints.
    match run(&opts, &stop) {
        Ok(Outcome::Finished(times)) => {
            print_times(&times);
            ExitCode::SUCCESS
        }
        Ok(Outcome::QuitEarly(times)) => {
            print_times(&times);
            ExitCode::from(1)
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

    fn one(args: &[&str]) -> ClockSpec {
        let mut o = run_opts(args);
        assert_eq!(o.clocks.len(), 1, "{args:?}");
        o.clocks.remove(0)
    }

    fn countdown(secs: u64) -> Mode {
        Mode::Countdown(Duration::from_secs(secs))
    }

    #[test]
    fn countdown_args() {
        let o = run_opts(&["25m"]);
        assert_eq!(
            o.clocks,
            vec![ClockSpec {
                mode: countdown(1500),
                title: None
            }]
        );
        assert!(o.bell);

        let o = run_opts(&["-t", "Tea", "--no-bell", "3m"]);
        assert_eq!(o.clocks[0].title.as_deref(), Some("Tea"));
        assert!(!o.bell);
        assert_eq!(
            one(&["--title=Focus", "10:00"]).title.as_deref(),
            Some("Focus")
        );
        assert_eq!(one(&["-tFocus", "10:00"]).title.as_deref(), Some("Focus"));
        assert_eq!(
            one(&["10:00", "--title", "Focus"]).title.as_deref(),
            Some("Focus")
        );
        assert_eq!(one(&["10:00", "--title", "  "]).title, None);
    }

    #[test]
    fn stopwatch_args() {
        assert_eq!(one(&["stopwatch"]).mode, Mode::Stopwatch);
        assert_eq!(one(&["--up"]).mode, Mode::Stopwatch);
        let sw = one(&["-u", "-t", "Run"]);
        assert_eq!(
            (sw.mode, sw.title.as_deref()),
            (Mode::Stopwatch, Some("Run"))
        );
    }

    #[test]
    fn several_clocks_keep_order_and_titles() {
        let o = run_opts(&["-t", "Tea", "4m", "-t", "Pasta", "10m", "stopwatch"]);
        let got: Vec<(&Mode, Option<&str>)> = o
            .clocks
            .iter()
            .map(|c| (&c.mode, c.title.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (&countdown(240), Some("Tea")),
                (&countdown(600), Some("Pasta")),
                (&Mode::Stopwatch, None),
            ]
        );
        let o = run_opts(&["--up", "5m"]);
        assert_eq!(o.clocks[0].mode, Mode::Stopwatch);
        assert_eq!(o.clocks[1].mode, countdown(300));
        assert_eq!(run_opts(&["1m"; 9]).clocks.len(), 9);
    }

    #[test]
    fn help_version_and_errors() {
        assert_eq!(cli(&["-h"]), Ok(Cli::Help));
        assert_eq!(cli(&["5m", "--version"]), Ok(Cli::Version));
        assert!(cli(&[]).is_err());
        assert!(cli(&["--frobnicate", "5m"]).is_err());
        assert!(cli(&["-t"]).is_err());
        assert!(cli(&["-t", "Tea"]).is_err());
        assert!(cli(&["soon"]).is_err());
        assert!(cli(&["5m", "soon"]).is_err());
        assert!(cli(&["1m"; 10]).is_err());
        assert!(cli(&["-t", "A", "-t", "B", "5m"]).is_err());
        assert!(cli(&["-t", "A", "5m", "-t", "B"]).is_err());
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

    #[test]
    fn finished_countdown_resets_into_a_running_one() {
        let spec = ClockSpec {
            mode: Mode::Countdown(Duration::from_millis(1)),
            title: None,
        };
        let mut r = Running {
            spec: &spec,
            clock: Clock::started(),
            done_at: None,
            bells_rung: 0,
        };
        std::thread::sleep(Duration::from_millis(5));
        r.check_done();
        assert!(r.done_at.is_some());
        assert_eq!(r.bells_due(), 1);
        assert_eq!(r.bells_due(), 0);
        r.toggle();
        assert!(r.done_at.is_some(), "a finished clock ignores pause");
        r.reset();
        assert!(r.done_at.is_none());
        assert!(r.clock.running());
        assert_eq!(r.bells_rung, 0);
    }

    #[test]
    fn panel_numbers_only_with_several_clocks() {
        let spec = ClockSpec {
            mode: countdown(60),
            title: Some("Tea".into()),
        };
        let r = Running {
            spec: &spec,
            clock: Clock::started(),
            done_at: None,
            bells_rung: 0,
        };
        assert_eq!(panel_for(&r, None).title.as_deref(), Some("Tea"));
        assert_eq!(panel_for(&r, Some(2)).title.as_deref(), Some("2  Tea"));
        let spec = ClockSpec {
            mode: Mode::Stopwatch,
            title: None,
        };
        let r = Running {
            spec: &spec,
            clock: Clock::started(),
            done_at: None,
            bells_rung: 0,
        };
        assert_eq!(panel_for(&r, None).title, None);
        assert_eq!(panel_for(&r, Some(3)).title.as_deref(), Some("3"));
    }
}
