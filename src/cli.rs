//! Command line arguments.

use crate::duration;
use crate::timer::{ClockSpec, Mode};

pub const HELP: &str = "\
tock: full screen countdown timers and stopwatches in big ASCII-art digits

Usage:
  tock                          open the clock list, add clocks from there
  tock [options] <clock>...     start these clocks and show them

  Each clock is a duration to count down from, 'stopwatch' to count up, or
  'rounds' for a stopwatch that times each round and their average:

  tock 25m
  tock -t Tea 4m -t Pasta 10m stopwatch
  tock -t Laps rounds

Durations:
  25m  1h30m  90s  1h2m3s  10:00 (mm:ss)  1:30:00 (h:mm:ss)  45 (seconds)

Options:
  -t, --title <text>   name the clock that follows (or the last one)
  -u, --up             add a stopwatch, same as 'stopwatch'
      --no-bell        flash at zero without ringing the terminal bell
  -h, --help           print this help
  -V, --version        print the version

Clock list:
  j k, arrows    move
  enter          full screen, with the selected clock shown
  v              show or hide the selected clock; four fit at once
  a  e           add a clock, edit the selected one
  space  r  x    start or pause, reset, cancel the selected clock
  +  -           a minute more or less on a countdown, even one that ended
  n              next round, on a rounds clock
  P  R           pause or resume all, reset all
  q              quit

Display:
  h j k l, arrows   move between clocks; the frame hides after 10 s
  space  r  x  e    start or pause, reset, cancel, edit the selected clock
  +  -  n           a minute more or less, next round
                    the keys show along the bottom while a clock is selected
  P  R              pause or resume all, reset all
  esc               back to the clock list
  q                 quit; once every countdown is done, most keys quit

Stopwatches print the time they stopped at, one line each, in order.
Exit status: 0 when no countdown was left running, 1 when one was,
2 on bad arguments, 130 when interrupted.
";

#[derive(Debug, PartialEq)]
pub struct Options {
    pub clocks: Vec<ClockSpec>,
    pub bell: bool,
}

#[derive(Debug, PartialEq)]
pub enum Cli {
    Run(Options),
    Help,
    Version,
}

/// The command line starts at most this many, all on the display at once.
pub const MAX_CLOCKS: usize = crate::app::MAX_SHOWN;

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut args = args.into_iter();
    let mut clocks: Vec<ClockSpec> = Vec::new();
    // A title waits here for the clock that follows it.
    let mut title: Option<String> = None;
    let mut bell = true;
    let mut only_positional = false;

    // Titles are trimmed here, and a blank one counts as no title at all.
    let set_title = |title: &mut Option<String>, value: String| {
        let value = value.trim();
        if value.is_empty() {
            return Ok(());
        }
        if let Some(old) = title.replace(value.to_string()) {
            return Err(format!("two titles in a row: '{old}' and '{value}'"));
        }
        Ok(())
    };
    let push = |clocks: &mut Vec<ClockSpec>, title: &mut Option<String>, mode: Mode| {
        clocks.push(ClockSpec {
            mode,
            title: title.take(),
        });
    };

    while let Some(arg) = args.next() {
        if only_positional || !arg.starts_with('-') || arg == "-" {
            let mode = match arg.as_str() {
                "stopwatch" => Mode::Stopwatch,
                "rounds" => Mode::Rounds,
                _ => Mode::Countdown(duration::parse(&arg)?),
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
            Some(last) if last.title.is_none() => last.title = Some(t),
            Some(_) => {
                return Err(format!(
                    "title '{t}' comes after the last clock; put it before the clock it names"
                ))
            }
            None => {
                return Err(format!(
                    "title '{t}' names no clock; add a duration after it"
                ))
            }
        }
    }
    if clocks.len() > MAX_CLOCKS {
        return Err(format!(
            "{} clocks, but {MAX_CLOCKS} fit on the display; run tock on its own to keep more in a list",
            clocks.len()
        ));
    }
    Ok(Cli::Run(Options { clocks, bell }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
    fn no_arguments_opens_the_list() {
        let o = run_opts(&[]);
        assert!(o.clocks.is_empty());
        assert!(o.bell);
        assert!(run_opts(&["--no-bell"]).clocks.is_empty());
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
        assert_eq!(one(&["rounds"]).mode, Mode::Rounds);
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
        let got: Vec<(Mode, Option<&str>)> = o
            .clocks
            .iter()
            .map(|c| (c.mode, c.title.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (countdown(240), Some("Tea")),
                (countdown(600), Some("Pasta")),
                (Mode::Stopwatch, None),
            ]
        );
        assert_eq!(run_opts(&["1m"; 4]).clocks.len(), 4);
        assert!(cli(&["1m"; 5]).is_err());
    }

    #[test]
    fn help_version_and_errors() {
        assert_eq!(cli(&["-h"]), Ok(Cli::Help));
        assert_eq!(cli(&["5m", "--version"]), Ok(Cli::Version));
        assert!(cli(&["--frobnicate", "5m"]).is_err());
        assert!(cli(&["-t"]).is_err());
        assert!(cli(&["-t", "Tea"]).is_err());
        assert!(cli(&["soon"]).is_err());
        assert!(cli(&["5m", "soon"]).is_err());
        assert!(cli(&["-t", "A", "-t", "B", "5m"]).is_err());
        assert!(cli(&["-t", "A", "5m", "-t", "B"]).is_err());
        assert_eq!(
            one(&["--title=", "-t", "Tea", "5m"]).title.as_deref(),
            Some("Tea")
        );
        assert_eq!(
            one(&["-t", "A", "5m", "-t", "  "]).title.as_deref(),
            Some("A")
        );
        assert!(cli(&["--", "-5m"]).is_err());
    }
}
