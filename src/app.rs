//! What tock is doing: the clocks, which view is up, and what each key does.
//! Nothing here touches the terminal, so all of it runs under `cargo test`.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::display::{self, Panel};
use crate::form::{Form, FormEvent};
use crate::manager;
use crate::screen::Grid;
use crate::timer::{ClockSpec, Timer};

/// The display view shows this many clocks at most.
pub const MAX_SHOWN: usize = 4;
/// The selection frame hides after this long without a key.
pub const FRAME_HIDES: Duration = Duration::from_secs(10);
/// Keys pressed this soon after the last countdown ends do not quit, so a key
/// that was already on its way does not close the screen before anyone sees it.
const DONE_GRACE: Duration = Duration::from_millis(600);
/// Bells due at the same moment, from clocks ending close together, ring one
/// after another this far apart so each one is heard.
const BELL_GAP: Duration = Duration::from_millis(300);
/// How long a note stays on the list.
const NOTE_FOR: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    List,
    Display,
}

#[derive(Debug, PartialEq)]
pub enum Exit {
    Quit,
    Interrupt,
}

pub struct App {
    pub timers: Vec<Timer>,
    pub view: View,
    /// The selected clock, an index into `timers`. Both views share it.
    pub sel: usize,
    /// When a key was last pressed on the display, for the selection frame.
    active_at: Option<Instant>,
    form: Option<Form>,
    note: Option<(String, Instant)>,
    /// The list's scroll position.
    list_top: usize,
    /// What the display drew last frame: the clocks in view and how many went
    /// in a row. The keys move by these.
    window: Vec<usize>,
    window_start: usize,
    per_row: usize,
    bell: bool,
    color: bool,
    bells_waiting: u32,
    next_bell: Instant,
    /// Started with clocks on the command line and never sent to the list:
    /// once every countdown is done, a stray key quits, as `tock 25m` always has.
    quick: bool,
    /// The screen size of the last frame, to know whether the form fits.
    size: (usize, usize),
}

impl App {
    pub fn new(clocks: Vec<ClockSpec>, bell: bool, color: bool) -> App {
        let view = if clocks.is_empty() {
            View::List
        } else {
            View::Display
        };
        let timers = clocks
            .into_iter()
            .enumerate()
            .map(|(i, spec)| Timer::new(spec, i < MAX_SHOWN))
            .collect();
        App {
            timers,
            view,
            sel: 0,
            active_at: None,
            form: None,
            note: None,
            list_top: 0,
            window: Vec::new(),
            window_start: 0,
            per_row: 1,
            bell,
            color,
            bells_waiting: 0,
            next_bell: Instant::now(),
            quick: view == View::Display,
            size: (80, 24),
        }
    }

    /// Indices of the clocks on display, in list order.
    pub fn shown(&self) -> Vec<usize> {
        (0..self.timers.len())
            .filter(|&i| self.timers[i].shown)
            .collect()
    }

    fn all_done(&self) -> bool {
        !self.timers.is_empty() && self.timers.iter().all(Timer::is_done)
    }

    fn note(&mut self, text: &str) {
        self.note = Some((text.to_string(), Instant::now()));
    }

    /// Puts clock `i` on display if there is room. When there is not, says so
    /// on the list and returns false.
    fn try_show(&mut self, i: usize) -> bool {
        if self.timers[i].shown {
            return true;
        }
        if self.shown().len() >= MAX_SHOWN {
            self.note("Four clocks are on display; take one off with v first");
            return false;
        }
        self.timers[i].shown = true;
        true
    }

    fn open_form(&mut self, form: Form) {
        if Form::fits(self.size.0, self.size.1) {
            self.form = Some(form);
        } else {
            self.note("The window is too small for the form");
        }
    }

    /// Advances the clocks. Returns true when the bell should ring now.
    pub fn tick(&mut self) -> bool {
        for t in &mut self.timers {
            t.check_done();
            self.bells_waiting += t.bells_due();
        }
        if self
            .note
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= NOTE_FOR)
        {
            self.note = None;
        }
        if self.bells_waiting > 0 && Instant::now() >= self.next_bell {
            self.bells_waiting -= 1;
            self.next_bell = Instant::now() + BELL_GAP;
            return self.bell;
        }
        false
    }

    /// True while the selection frame should show on the display.
    pub fn frame_visible(&self) -> bool {
        self.active_at.is_some_and(|t| t.elapsed() < FRAME_HIDES)
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<Exit> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c' | 'C')) {
            return Some(if self.all_done() {
                Exit::Quit
            } else {
                Exit::Interrupt
            });
        }
        if let Some(form) = &mut self.form {
            match form.key(key) {
                FormEvent::Open => {}
                FormEvent::Cancel => self.form = None,
                FormEvent::Save { title, mode } => {
                    let target = form.target;
                    self.form = None;
                    self.save(target, ClockSpec { mode, title });
                }
            }
            if self.view == View::Display {
                self.active_at = Some(Instant::now());
            }
            return None;
        }
        // Chords such as Ctrl+X or Alt+R are never a tock key, so a stray one
        // from a terminal multiplexer cannot cancel a clock.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match self.view {
            View::List => self.list_key(key),
            View::Display => self.display_key(key),
        }
    }

    fn save(&mut self, target: Option<usize>, spec: ClockSpec) {
        match target.and_then(|i| self.timers.get_mut(i)) {
            Some(t) => {
                t.title = spec.title;
                if t.mode != spec.mode {
                    t.mode = spec.mode;
                    t.restart();
                }
            }
            None => {
                let shown = self.shown().len() < MAX_SHOWN;
                self.timers.push(Timer::new(spec, shown));
                self.sel = self.timers.len() - 1;
                if !shown {
                    // Show where it went, rather than leave the display unchanged.
                    self.view = View::List;
                    self.quick = false;
                    self.note("Four clocks are on display; this one waits in the list");
                }
            }
        }
    }

    /// Keys both views share. Returns true when the key was one of them.
    fn common_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('a') => self.open_form(Form::new_clock()),
            KeyCode::Char('e') => {
                if let Some(t) = self.timers.get(self.sel) {
                    let form = Form::edit(self.sel, t);
                    self.open_form(form);
                }
            }
            KeyCode::Char(' ' | 'p') => {
                if let Some(t) = self.timers.get_mut(self.sel) {
                    t.toggle();
                }
            }
            KeyCode::Char('r') => {
                if let Some(t) = self.timers.get_mut(self.sel) {
                    t.reset();
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => self.cancel(),
            KeyCode::Char('P') => {
                // Pause everything that runs; if nothing runs, resume all.
                let any_running = self.timers.iter().any(|t| !t.is_paused() && !t.is_done());
                for t in &mut self.timers {
                    if any_running {
                        t.pause();
                    } else {
                        t.resume();
                    }
                }
            }
            KeyCode::Char('R') => self.timers.iter_mut().for_each(Timer::reset),
            _ => return false,
        }
        true
    }

    /// Removes the selected clock.
    fn cancel(&mut self) {
        if self.sel >= self.timers.len() {
            return;
        }
        self.timers.remove(self.sel);
        if self.view == View::Display {
            // Stay on a clock that is on display: the next one, else the one before.
            let shown = self.shown();
            self.sel = shown
                .iter()
                .copied()
                .find(|&i| i >= self.sel)
                .or(shown.last().copied())
                .unwrap_or(0);
            if shown.is_empty() {
                self.view = View::List;
            }
        }
        self.sel = self.sel.min(self.timers.len().saturating_sub(1));
    }

    fn list_key(&mut self, key: KeyEvent) -> Option<Exit> {
        if self.common_key(key.code) {
            return None;
        }
        let last = self.timers.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('q' | 'Q') => return Some(Exit::Quit),
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.sel = (self.sel + 1).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.sel = 0,
            KeyCode::End | KeyCode::Char('G') => self.sel = last,
            KeyCode::Char('v') if self.sel < self.timers.len() => {
                if self.timers[self.sel].shown {
                    self.timers[self.sel].shown = false;
                } else {
                    self.try_show(self.sel);
                }
            }
            // try_show notes why when there is no room.
            KeyCode::Enter if self.sel < self.timers.len() && self.try_show(self.sel) => {
                self.view = View::Display;
                self.active_at = Some(Instant::now());
            }
            _ => {}
        }
        None
    }

    fn display_key(&mut self, key: KeyEvent) -> Option<Exit> {
        self.active_at = Some(Instant::now());
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.step(-1),
            KeyCode::Right | KeyCode::Char('l') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step_row(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step_row(1),
            KeyCode::Esc => {
                self.view = View::List;
                self.quick = false;
            }
            KeyCode::Char('q' | 'Q') => return Some(Exit::Quit),
            _ => return self.display_action(key),
        }
        None
    }

    fn display_action(&mut self, key: KeyEvent) -> Option<Exit> {
        // Started as `tock 25m` and every countdown is done: the keys that
        // still mean something keep working and any other key quits.
        if self.quick && self.all_done() {
            let keeps = matches!(
                key.code,
                KeyCode::Char('r' | 'R' | 'e' | 'x' | 'a') | KeyCode::Delete | KeyCode::Enter
            );
            if !keeps {
                let latest = self.timers.iter().filter_map(|t| t.done_at).max();
                if latest.is_some_and(|t| t.elapsed() >= DONE_GRACE) {
                    return Some(Exit::Quit);
                }
                return None;
            }
        }
        self.common_key(key.code);
        None
    }

    /// Moves along the clocks on display, in order.
    fn step(&mut self, by: isize) {
        let shown = self.shown();
        if let Some(pos) = shown.iter().position(|&i| i == self.sel) {
            let to = (pos as isize + by).clamp(0, shown.len() as isize - 1) as usize;
            self.sel = shown[to];
        }
    }

    /// Moves a row up or down in the layout the display drew last.
    fn step_row(&mut self, by: isize) {
        let Some(pos) = self.window.iter().position(|&i| i == self.sel) else {
            return;
        };
        let per_row = self.per_row.max(1);
        let n = self.window.len();
        let target = pos as isize + by * per_row as isize;
        let to = if target >= 0 && (target as usize) < n {
            target as usize
        } else if by > 0 && pos / per_row < (n - 1) / per_row {
            // Down into a short last row: land on its last clock.
            n - 1
        } else {
            return;
        };
        self.sel = self.window[to];
    }

    /// Lays out the current view on a `cols` x `rows` screen.
    pub fn compose(&mut self, cols: usize, rows: usize) -> Grid {
        self.size = (cols, rows);
        let mut grid = Grid::new(cols, rows);
        if self.view == View::Display && self.shown().is_empty() {
            self.view = View::List;
        }
        match self.view {
            View::List => {
                let note = self.note.as_ref().map(|(n, _)| n.as_str());
                self.list_top = manager::compose(
                    &mut grid,
                    &self.timers,
                    self.sel,
                    self.list_top,
                    note,
                    self.color,
                );
            }
            View::Display => self.compose_display(&mut grid),
        }
        if let Some(form) = &self.form {
            form.draw(&mut grid, self.color);
        }
        grid
    }

    fn compose_display(&mut self, grid: &mut Grid) {
        let shown = self.shown();
        if !shown.contains(&self.sel) {
            self.sel = shown[0];
        }
        let pos = shown.iter().position(|&i| i == self.sel).unwrap_or(0);
        let panels: Vec<Panel> = shown.iter().map(|&i| self.timers[i].panel()).collect();

        // As many as fit in big digits, in a window that keeps the selection.
        let window_for = |k: usize, start: usize| {
            let start = if pos < start {
                pos
            } else if pos >= start + k {
                pos + 1 - k
            } else {
                start
            };
            start.min(shown.len() - k)
        };
        let mut k = shown.len();
        while k > 1 {
            let s = window_for(k, self.window_start);
            if display::fits_big(&panels[s..s + k], grid.cols, grid.rows) {
                break;
            }
            k -= 1;
        }
        let start = window_for(k, self.window_start);
        self.window_start = start;
        self.window = shown[start..start + k].to_vec();
        let selected = self.frame_visible().then_some(pos - start);
        self.per_row = display::compose(grid, &panels[start..start + k], self.color, selected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::Mode;

    fn press(app: &mut App, code: KeyCode) -> Option<Exit> {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn spec(title: &str, secs: u64) -> ClockSpec {
        ClockSpec {
            mode: Mode::Countdown(Duration::from_secs(secs)),
            title: Some(title.into()),
        }
    }

    fn app(n: usize) -> App {
        let clocks = (0..n).map(|i| spec(&format!("C{i}"), 600)).collect();
        App::new(clocks, false, true)
    }

    #[test]
    fn no_clocks_opens_the_list_and_some_open_the_display() {
        assert_eq!(app(0).view, View::List);
        let a = app(6);
        assert_eq!(a.view, View::Display);
        assert_eq!(a.shown(), vec![0, 1, 2, 3], "four go on display");
    }

    #[test]
    fn adding_a_clock_from_the_list() {
        let mut a = app(0);
        press(&mut a, KeyCode::Char('a'));
        typed(&mut a, "Tea");
        press(&mut a, KeyCode::Tab);
        press(&mut a, KeyCode::Tab);
        typed(&mut a, "4m");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.timers.len(), 1);
        assert_eq!(a.timers[0].title.as_deref(), Some("Tea"));
        assert!(a.timers[0].shown);
        // Enter shows it.
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.view, View::Display);
        // Esc goes back.
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.view, View::List);
    }

    #[test]
    fn renaming_keeps_the_time_and_a_new_time_restarts() {
        let mut a = app(1);
        std::thread::sleep(Duration::from_millis(20));
        press(&mut a, KeyCode::Char('e'));
        for _ in 0..2 {
            press(&mut a, KeyCode::Backspace);
        }
        typed(&mut a, "Pasta");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.timers[0].title.as_deref(), Some("Pasta"));
        assert!(a.timers[0].clock.elapsed() >= Duration::from_millis(20));

        press(&mut a, KeyCode::Char('e'));
        press(&mut a, KeyCode::Tab);
        press(&mut a, KeyCode::Tab);
        a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        typed(&mut a, "2m");
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.timers[0].mode, Mode::Countdown(Duration::from_secs(120)));
        assert!(a.timers[0].clock.elapsed() < Duration::from_millis(20));
    }

    #[test]
    fn only_four_go_on_display() {
        let mut a = app(5);
        a.view = View::List;
        a.sel = 4;
        press(&mut a, KeyCode::Char('v'));
        assert!(!a.timers[4].shown);
        assert!(a.note.is_some());
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.view, View::List, "enter refuses a fifth clock");
        a.sel = 0;
        press(&mut a, KeyCode::Char('v'));
        a.sel = 4;
        press(&mut a, KeyCode::Char('v'));
        assert_eq!(a.shown(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn hjkl_and_arrows_move_around_a_two_by_two_grid() {
        let mut a = app(4);
        a.compose(120, 40);
        assert_eq!(a.per_row, 2);
        assert_eq!(a.window, vec![0, 1, 2, 3]);
        press(&mut a, KeyCode::Char('l'));
        assert_eq!(a.sel, 1);
        press(&mut a, KeyCode::Char('j'));
        assert_eq!(a.sel, 3);
        press(&mut a, KeyCode::Left);
        assert_eq!(a.sel, 2);
        press(&mut a, KeyCode::Up);
        assert_eq!(a.sel, 0);
        press(&mut a, KeyCode::Char('k'));
        assert_eq!(a.sel, 0, "nothing above the top row");
        press(&mut a, KeyCode::Char('h'));
        assert_eq!(a.sel, 0);
    }

    #[test]
    fn down_into_a_short_last_row() {
        let mut a = app(3);
        // Too short to stack three, too narrow for three in a row: two and one.
        a.compose(80, 24);
        assert_eq!(a.per_row, 2);
        a.sel = 1;
        press(&mut a, KeyCode::Char('j'));
        assert_eq!(a.sel, 2);
    }

    #[test]
    fn hotkeys_act_on_the_selected_clock() {
        let mut a = app(3);
        a.compose(120, 40);
        press(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Char(' '));
        assert!(a.timers[1].is_paused());
        assert!(!a.timers[0].is_paused());
        press(&mut a, KeyCode::Char('P'));
        assert!(a.timers.iter().all(Timer::is_paused), "P pauses the rest");
        press(&mut a, KeyCode::Char('P'));
        assert!(
            a.timers.iter().all(|t| !t.is_paused()),
            "and then resumes all"
        );
        press(&mut a, KeyCode::Char('x'));
        assert_eq!(a.timers.len(), 2);
        assert_eq!(
            a.timers[a.sel].title.as_deref(),
            Some("C2"),
            "selection moves on"
        );
        press(&mut a, KeyCode::Char('x'));
        press(&mut a, KeyCode::Char('x'));
        assert!(a.timers.is_empty());
        assert_eq!(a.view, View::List, "nothing left to display");
    }

    #[test]
    fn the_frame_shows_after_a_key_and_hides_after_ten_seconds() {
        let mut a = app(2);
        assert!(!a.frame_visible(), "hidden until a key");
        let g = a.compose(100, 26);
        assert!(g.find("╭").is_none());
        press(&mut a, KeyCode::Char('l'));
        assert!(a.frame_visible());
        let g = a.compose(100, 26);
        assert_eq!(g.cell(50, 0).ch, '╭');
        a.active_at = Some(Instant::now() - FRAME_HIDES);
        assert!(!a.frame_visible());
        assert!(a.compose(100, 26).find("╭").is_none());
    }

    #[test]
    fn a_small_screen_shows_fewer_and_follows_the_selection() {
        let mut a = app(4);
        a.compose(40, 12);
        assert!(a.window.len() < 4, "four do not fit in 40x12");
        let k = a.window.len();
        for _ in 0..3 {
            press(&mut a, KeyCode::Char('l'));
            a.compose(40, 12);
        }
        assert_eq!(a.sel, 3);
        assert!(a.window.contains(&3));
        assert_eq!(a.window.len(), k);
    }

    #[test]
    fn finished_countdowns_quit_on_a_stray_key_but_not_on_r() {
        let mut a = App::new(
            vec![ClockSpec {
                mode: Mode::Countdown(Duration::from_millis(1)),
                title: None,
            }],
            false,
            true,
        );
        std::thread::sleep(Duration::from_millis(5));
        a.tick();
        assert!(a.all_done());
        assert_eq!(press(&mut a, KeyCode::Char('z')), None, "grace period");
        a.timers[0].done_at = Some(Instant::now() - DONE_GRACE);
        a.quick = false;
        assert_eq!(
            press(&mut a, KeyCode::Char(' ')),
            None,
            "a list session never quits on a stray key"
        );
        a.quick = true;
        a.timers[0].done_at = Some(Instant::now() - DONE_GRACE);
        assert_eq!(press(&mut a, KeyCode::Char('r')), None);
        assert!(!a.timers[0].is_done(), "r starts it over");
    }

    #[test]
    fn esc_in_display_returns_to_the_list_and_q_quits() {
        let mut a = app(1);
        assert_eq!(press(&mut a, KeyCode::Esc), None);
        assert_eq!(a.view, View::List);
        assert_eq!(press(&mut a, KeyCode::Char('q')), Some(Exit::Quit));
        let mut a = app(1);
        assert_eq!(
            a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Exit::Interrupt)
        );
    }

    #[test]
    fn chords_do_nothing() {
        let mut a = app(2);
        a.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        a.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
        assert_eq!(a.timers.len(), 2);
    }

    #[test]
    fn a_fifth_clock_from_the_display_goes_to_the_list() {
        let mut a = app(4);
        press(&mut a, KeyCode::Char('a'));
        press(&mut a, KeyCode::Tab);
        press(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.timers.len(), 5);
        assert_eq!(a.view, View::List);
        assert_eq!(a.sel, 4);
        assert!(a.note.is_some());
    }

    #[test]
    fn the_form_waits_for_room() {
        let mut a = app(0);
        a.compose(20, 6);
        press(&mut a, KeyCode::Char('a'));
        assert!(a.form.is_none());
        assert!(a.note.is_some());
        a.compose(80, 24);
        press(&mut a, KeyCode::Char('a'));
        assert!(a.form.is_some());
    }

    #[test]
    fn a_form_on_the_display_adds_to_the_display() {
        let mut a = app(1);
        press(&mut a, KeyCode::Char('a'));
        typed(&mut a, "Run");
        press(&mut a, KeyCode::Tab);
        press(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.view, View::Display);
        assert_eq!(a.shown(), vec![0, 1]);
        assert_eq!(a.sel, 1);
        assert_eq!(a.timers[1].mode, Mode::Stopwatch);
    }
}
