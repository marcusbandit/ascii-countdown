//! tock: full screen countdown timers and stopwatches in big ASCII-art digits.

mod app;
mod cli;
mod display;
mod duration;
mod font;
mod form;
mod manager;
mod screen;
mod timer;

use std::io;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

use app::{App, Exit};
use cli::{Cli, Options};
use screen::{Grid, Screen};

/// How often the loop wakes up to see whether the screen changed.
const TICK: Duration = Duration::from_millis(40);

fn run(app: &mut App, stop: &AtomicBool) -> io::Result<Exit> {
    let mut screen = Screen::enter()?;
    let mut last: Option<Grid> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(Exit::Interrupt);
        }
        if app.tick() {
            screen.bell()?;
        }
        let (cols, rows) = crossterm::terminal::size()?;
        let grid = app.compose(cols as usize, rows as usize);
        if last.as_ref() != Some(&grid) {
            screen.draw(&grid)?;
            last = Some(grid);
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
                if let Some(exit) = app.key(key) {
                    return Ok(exit);
                }
            }
            _ => {}
        }
    }
}

fn main() -> ExitCode {
    let Options { clocks, bell } = match cli::parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run(opts)) => opts,
        Ok(Cli::Help) => {
            print!("{}", cli::HELP);
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

    // https://no-color.org: any non-empty NO_COLOR turns colours off.
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let mut app = App::new(clocks, bell, color);

    // The screen is dropped, and the terminal restored, before anything prints.
    match run(&mut app, &stop) {
        Ok(Exit::Quit) => {
            for t in app.timers.iter().filter(|t| !t.is_countdown()) {
                println!("{}", duration::stopwatch_face(t.clock.elapsed()));
            }
            let unfinished = app.timers.iter().any(|t| t.is_countdown() && !t.is_done());
            ExitCode::from(if unfinished { 1 } else { 0 })
        }
        Ok(Exit::Interrupt) => ExitCode::from(130),
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
