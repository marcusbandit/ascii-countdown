//! The add and edit screen: what kind of clock, its name, its time and
//! whether it starts now, over a live preview of the clock as it will look.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use crate::display::{self, Panel, Tone};
use crate::duration;
use crate::manager::{self, Key};
use crate::screen::{truncate, Grid, Ink, Rect, Style};
use crate::timer::{Mode, Timer};

/// Longest name the form takes, in characters.
const NAME_MAX: usize = 40;
/// The smallest screen the form works on.
const MIN_W: usize = 40;
const MIN_H: usize = 12;
/// Rows the preview needs: a title, the digits at their smallest, a status.
const PREVIEW_MIN: usize = 9;
/// The times the arrow keys step through on the time field.
const PRESETS: [&str; 7] = ["1m", "5m", "10m", "15m", "25m", "45m", "1h"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Stopwatch,
    Countdown,
    Rounds,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Stopwatch, Kind::Countdown, Kind::Rounds];

    fn word(self) -> &'static str {
        match self {
            Kind::Stopwatch => "stopwatch",
            Kind::Countdown => "countdown",
            Kind::Rounds => "rounds",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Kind,
    Name,
    Time,
    Start,
}

#[derive(Debug, PartialEq)]
pub enum FormEvent {
    /// Still open.
    Open,
    Cancel,
    Save {
        title: Option<String>,
        mode: Mode,
        /// False to add the clock paused, to start later.
        start: bool,
    },
}

#[derive(Debug)]
pub struct Form {
    /// The clock being edited, or `None` for a new one.
    pub target: Option<usize>,
    kind: Kind,
    name: String,
    time: String,
    start: bool,
    focus: Field,
    error: Option<String>,
}

impl Form {
    pub fn new_clock() -> Form {
        Form {
            target: None,
            kind: Kind::Stopwatch,
            name: String::new(),
            time: String::new(),
            start: true,
            focus: Field::Name,
            error: None,
        }
    }

    pub fn edit(index: usize, t: &Timer) -> Form {
        let (kind, time) = match t.mode {
            Mode::Countdown(d) => (Kind::Countdown, duration::compact(d)),
            Mode::Stopwatch => (Kind::Stopwatch, String::new()),
            Mode::Rounds => (Kind::Rounds, String::new()),
        };
        Form {
            target: Some(index),
            kind,
            name: t.title.clone().unwrap_or_default(),
            time,
            start: true,
            focus: Field::Name,
            error: None,
        }
    }

    /// The fields in order. Only a countdown has a time, and only a new clock
    /// can wait to start.
    fn fields(&self) -> Vec<Field> {
        let mut f = vec![Field::Kind, Field::Name];
        if self.kind == Kind::Countdown {
            f.push(Field::Time);
        }
        if self.target.is_none() {
            f.push(Field::Start);
        }
        f
    }

    fn step(&mut self, by: isize) {
        let fields = self.fields();
        let at = fields.iter().position(|&f| f == self.focus).unwrap_or(0) as isize;
        let n = fields.len() as isize;
        self.focus = fields[((at + by) % n + n) as usize % fields.len()];
    }

    fn text(&mut self) -> Option<&mut String> {
        match self.focus {
            Field::Name => Some(&mut self.name),
            Field::Time => Some(&mut self.time),
            Field::Kind | Field::Start => None,
        }
    }

    /// Moves the time to the next preset up (`by` 1) or down (`by` -1). A time
    /// typed in by hand goes to the nearest preset that way.
    fn preset(&mut self, by: isize) {
        let secs = |p: &str| duration::parse(p).map(|d| d.as_secs()).unwrap_or(0);
        let all: Vec<u64> = PRESETS.iter().map(|p| secs(p)).collect();
        let now = duration::parse(&self.time).ok().map(|d| d.as_secs());
        let i = match (now, by > 0) {
            (Some(t), true) => all.iter().position(|&p| p > t).unwrap_or(all.len() - 1),
            (Some(t), false) => all.iter().rposition(|&p| p < t).unwrap_or(0),
            (None, true) => 0,
            (None, false) => all.len() - 1,
        };
        self.time = PRESETS[i].to_string();
    }

    pub fn key(&mut self, key: KeyEvent) -> FormEvent {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let back = matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
        match (key.code, self.focus) {
            (KeyCode::Esc, _) => return FormEvent::Cancel,
            (KeyCode::Enter, _) => return self.submit(),
            (KeyCode::Tab | KeyCode::Down, _) => self.step(1),
            (KeyCode::BackTab | KeyCode::Up, _) => self.step(-1),
            (KeyCode::Left | KeyCode::Right | KeyCode::Char(' ' | 'h' | 'l'), Field::Kind) => {
                let i = Kind::ALL.iter().position(|&k| k == self.kind).unwrap_or(0);
                let n = Kind::ALL.len();
                self.kind = Kind::ALL[if back { (i + n - 1) % n } else { (i + 1) % n }];
            }
            (KeyCode::Left | KeyCode::Right | KeyCode::Char(' ' | 'h' | 'l'), Field::Start) => {
                self.start = !self.start;
            }
            (KeyCode::Left, Field::Time) => self.preset(-1),
            (KeyCode::Right, Field::Time) => self.preset(1),
            (KeyCode::Char('u'), _) if ctrl => {
                if let Some(t) = self.text() {
                    t.clear();
                }
            }
            (KeyCode::Backspace, _) => {
                if let Some(t) = self.text() {
                    t.pop();
                }
            }
            (KeyCode::Char(c), _) if !ctrl => {
                let is_name = self.focus == Field::Name;
                if let Some(t) = self.text() {
                    if !is_name || t.chars().count() < NAME_MAX {
                        t.push(c);
                    }
                }
            }
            _ => return FormEvent::Open,
        }
        self.error = None;
        FormEvent::Open
    }

    fn submit(&mut self) -> FormEvent {
        let mode = match self.kind {
            Kind::Stopwatch => Mode::Stopwatch,
            Kind::Rounds => Mode::Rounds,
            Kind::Countdown => match duration::parse(&self.time) {
                Ok(d) => Mode::Countdown(d),
                Err(e) => {
                    self.error = Some(e);
                    self.focus = Field::Time;
                    return FormEvent::Open;
                }
            },
        };
        let name = self.name.trim();
        FormEvent::Save {
            title: (!name.is_empty()).then(|| name.to_string()),
            mode,
            start: self.start,
        }
    }

    /// The keys that work in the form right now, for the bar along the bottom.
    pub fn keys(&self) -> Vec<Key> {
        let enter = match (self.target, self.start) {
            (Some(_), _) => "save",
            (None, true) => "start",
            (None, false) => "add",
        };
        let mut keys = vec![("enter", enter), ("tab", "next")];
        match self.focus {
            Field::Kind => keys.push(("←→", "type")),
            Field::Time => keys.push(("←→", "presets")),
            Field::Start => keys.push(("←→", "now or later")),
            Field::Name => {}
        }
        keys.push(("esc", "close"));
        keys
    }

    /// True when a `cols` x `rows` screen has room for the whole form.
    pub fn fits(cols: usize, rows: usize) -> bool {
        cols >= MIN_W && rows >= MIN_H
    }

    /// The clock as it will look once saved.
    fn preview(&self) -> Panel {
        let face = match self.kind {
            Kind::Stopwatch | Kind::Rounds => duration::stopwatch_face(Default::default()),
            Kind::Countdown => match duration::parse(&self.time) {
                Ok(d) => duration::countdown_face(d.as_secs(), d.as_secs() >= 3600),
                Err(_) => "--:--".into(),
            },
        };
        let waits = self.target.is_none() && !self.start;
        let mut status: Vec<&str> = Vec::new();
        if waits {
            status.push("PAUSED");
        }
        if self.kind == Kind::Rounds {
            status.push("ROUND 1");
        }
        let name = self.name.trim();
        Panel {
            title: (!name.is_empty()).then(|| name.to_string()),
            face,
            tone: if waits { Tone::Paused } else { Tone::Normal },
            status: status.join("   "),
            flash: false,
        }
    }

    /// Draws the form over the whole of `grid`: the preview on top, the
    /// fields under it. The key bar on the last row is the caller's.
    pub fn draw(&self, grid: &mut Grid, color: bool) {
        let (cols, rows) = (grid.cols, grid.rows);
        if !Form::fits(cols, rows) {
            return;
        }
        let accent = Style::accent(color);
        let (x0, w) = manager::column(cols);
        let right = x0 + w;
        let heading = if self.target.is_some() {
            "Edit clock"
        } else {
            "New clock"
        };
        grid.put(x0 + 1, 1, heading, accent, right);

        // The preview gets what is left above room for every field, so it stays
        // put as fields come and go: five field rows, a free row between
        // fields when the screen is tall, an error's two with a free row each
        // side, and the key bar.
        let gap = usize::from(rows >= 28);
        let y0 = rows - 10 - 3 * gap;
        let preview = Rect {
            x: 0,
            y: 3,
            w: cols,
            h: y0.saturating_sub(4),
        };
        if preview.h >= PREVIEW_MIN {
            display::compose_in(grid, preview, &[self.preview()], color, None);
        }

        // Labels line up with the heading and the key bar; the pointer for the
        // focused one sits in the margin to their left.
        let label_x = x0 + 1;
        let value_x = label_x + 7;
        let end = right - 1;
        // The focused field gets a pointer as well as the accent, so focus
        // shows without colour too.
        let label = |grid: &mut Grid, y: usize, text: &str, field: Field| {
            let style = if self.focus == field {
                grid.put(label_x - 2, y, "›", accent, end);
                accent
            } else {
                Style::default()
            };
            grid.put(label_x, y, text, style, end);
        };
        // A text field: an underlined slot, so an empty one still shows where
        // typing goes, with a block cursor after the text when focused.
        let text_row = |grid: &mut Grid, y: usize, value: &str, field: Field| {
            let slot = Style {
                underline: true,
                ..Style::default()
            };
            grid.put(value_x, y, &" ".repeat(end - value_x), slot, end);
            let shown = truncate(value, end.saturating_sub(value_x + 1));
            let after = grid.put(value_x, y, &shown, slot, end);
            if self.focus == field {
                grid.put(after, y, " ", Style::default().reversed(), end);
            }
        };
        // Choices in a row: the chosen one swapped, in the accent while its
        // field has focus. Padded when they all fit that way.
        let chips = |grid: &mut Grid, y: usize, words: &[&str], chosen: usize, field: Field| {
            let bare: usize = words.iter().map(|w| w.width() + 1).sum();
            let pad = if value_x + bare + 2 * words.len() <= end {
                " "
            } else {
                ""
            };
            let mut x = value_x;
            for (i, word) in words.iter().enumerate() {
                let chip = format!("{pad}{word}{pad}");
                if x + chip.width() > end {
                    break;
                }
                let style = match (i == chosen, self.focus == field) {
                    (false, _) => Style::default(),
                    (true, true) => Style {
                        reverse: true,
                        ..accent
                    },
                    (true, false) => Style::default().reversed(),
                };
                x = grid.put(x, y, &chip, style, end) + 1;
            }
        };

        let kinds: Vec<&str> = Kind::ALL.iter().map(|k| k.word()).collect();
        let kind_at = Kind::ALL.iter().position(|&k| k == self.kind).unwrap_or(0);
        // The fields follow each other, with no gap where a hidden one was.
        let mut y = y0;
        label(grid, y, "Type", Field::Kind);
        chips(grid, y, &kinds, kind_at, Field::Kind);
        y += 1 + gap;

        label(grid, y, "Name", Field::Name);
        text_row(grid, y, &self.name, Field::Name);
        y += 1 + gap;

        if self.kind == Kind::Countdown {
            label(grid, y, "Time", Field::Time);
            text_row(grid, y, &self.time, Field::Time);
            let at = PRESETS
                .iter()
                .position(|p| *p == self.time.trim())
                .unwrap_or(usize::MAX);
            chips(grid, y + 1, &PRESETS, at, Field::Time);
            y += 2 + gap;
        }

        if self.target.is_none() {
            let later = usize::from(!self.start);
            label(grid, y, "Start", Field::Start);
            chips(grid, y, &["now", "later"], later, Field::Start);
            y += 1;
        }
        let err_y = y + 1;

        if let Some(e) = &self.error {
            let style = if color {
                Style::fg(Ink::Red).bold()
            } else {
                Style::default().bold()
            };
            // Two rows at most: a longer error ends its second row with an ellipsis.
            let width = end - label_x;
            let mut lines = wrap(e, width);
            if lines.len() > 2 {
                lines[1] = format!("{}…", truncate(&lines[1..].join(" "), width - 1));
            }
            for (i, line) in lines.iter().take(2).enumerate() {
                grid.put(label_x, err_y + i, line, style, end);
            }
        }
    }
}

/// Splits `text` into lines of at most `width` columns, at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if l.width() + 1 + word.width() <= width => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(truncate(word, width)),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::ClockSpec;
    use std::time::Duration;

    fn press(f: &mut Form, code: KeyCode) -> FormEvent {
        f.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(f: &mut Form, text: &str) {
        for c in text.chars() {
            press(f, KeyCode::Char(c));
        }
    }

    fn timer(mode: Mode) -> Timer {
        Timer::new(ClockSpec { mode, title: None }, false)
    }

    /// From the name field, on to the type and over to a countdown.
    fn countdown(f: &mut Form) {
        press(f, KeyCode::BackTab);
        press(f, KeyCode::Right);
        assert_eq!(f.kind, Kind::Countdown);
        press(f, KeyCode::Tab);
        press(f, KeyCode::Tab);
        assert_eq!(f.focus, Field::Time);
    }

    #[test]
    fn a_new_clock_starts_as_a_running_stopwatch() {
        let mut f = Form::new_clock();
        assert_eq!(f.focus, Field::Name);
        assert_eq!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                title: None,
                mode: Mode::Stopwatch,
                start: true,
            }
        );
    }

    #[test]
    fn a_new_countdown_needs_a_time() {
        let mut f = Form::new_clock();
        typed(&mut f, "Tea");
        countdown(&mut f);
        assert_eq!(press(&mut f, KeyCode::Enter), FormEvent::Open);
        assert!(f.error.is_some());
        typed(&mut f, "4m");
        assert!(f.error.is_none(), "typing clears the error");
        assert_eq!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                title: Some("Tea".into()),
                mode: Mode::Countdown(Duration::from_secs(240)),
                start: true,
            }
        );
    }

    #[test]
    fn the_arrows_step_through_the_presets() {
        let mut f = Form::new_clock();
        countdown(&mut f);
        press(&mut f, KeyCode::Right);
        assert_eq!(f.time, "1m");
        press(&mut f, KeyCode::Right);
        press(&mut f, KeyCode::Right);
        assert_eq!(f.time, "10m");
        press(&mut f, KeyCode::Left);
        assert_eq!(f.time, "5m");
        // A typed time goes to the nearest preset that way.
        f.time = "20m".into();
        press(&mut f, KeyCode::Right);
        assert_eq!(f.time, "25m");
        f.time = "20m".into();
        press(&mut f, KeyCode::Left);
        assert_eq!(f.time, "15m");
        // h and m are part of a time, so they type rather than move.
        f.time.clear();
        typed(&mut f, "1h");
        assert_eq!(f.time, "1h");
    }

    #[test]
    fn the_type_cycles_and_rounds_need_no_time() {
        let mut f = Form::new_clock();
        press(&mut f, KeyCode::BackTab);
        press(&mut f, KeyCode::Left);
        assert_eq!(f.kind, Kind::Rounds);
        assert!(!f.fields().contains(&Field::Time));
        assert!(matches!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                mode: Mode::Rounds,
                ..
            }
        ));
    }

    #[test]
    fn a_clock_can_wait_to_start() {
        let mut f = Form::new_clock();
        press(&mut f, KeyCode::Tab);
        assert_eq!(f.focus, Field::Start);
        assert_eq!(f.keys()[0], ("enter", "start"));
        press(&mut f, KeyCode::Char(' '));
        assert_eq!(f.keys()[0], ("enter", "add"));
        assert_eq!(f.preview().tone, Tone::Paused);
        assert_eq!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                title: None,
                mode: Mode::Stopwatch,
                start: false,
            }
        );
    }

    #[test]
    fn letters_type_into_the_name_even_h_and_l() {
        let mut f = Form::new_clock();
        typed(&mut f, "hello");
        assert_eq!(f.name, "hello");
        press(&mut f, KeyCode::Backspace);
        assert_eq!(f.name, "hell");
        f.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(f.name, "");
        typed(&mut f, &"x".repeat(60));
        assert_eq!(f.name.chars().count(), NAME_MAX);
    }

    #[test]
    fn editing_starts_from_the_clock_and_has_no_start_field() {
        let mut t = timer(Mode::Countdown(Duration::from_secs(5400)));
        t.title = Some("Bake".into());
        let mut f = Form::edit(3, &t);
        assert_eq!((f.name.as_str(), f.time.as_str()), ("Bake", "1h30m"));
        assert_eq!(f.target, Some(3));
        assert!(!f.fields().contains(&Field::Start));
        assert_eq!(f.keys()[0], ("enter", "save"));
        assert_eq!(Form::edit(0, &timer(Mode::Rounds)).kind, Kind::Rounds);
        assert_eq!(press(&mut f, KeyCode::Esc), FormEvent::Cancel);
    }

    #[test]
    fn the_preview_shows_the_clock_as_it_will_look() {
        let mut f = Form::new_clock();
        typed(&mut f, "Tea");
        assert_eq!(f.preview().face, "00:00.0");
        assert_eq!(f.preview().title.as_deref(), Some("Tea"));
        countdown(&mut f);
        assert_eq!(f.preview().face, "--:--");
        typed(&mut f, "4m");
        assert_eq!(f.preview().face, "04:00");
        let mut g = Grid::new(100, 30);
        f.draw(&mut g, true);
        assert!(g.find("New clock").is_some());
        assert!(g.find("Tea").is_some());
        assert!(g.find("█").is_some(), "the preview is in big digits");
        assert!(g.find("› Time").is_some(), "focus shows as a pointer");
        assert!(g.find(" 25m ").is_some());
        // A short screen drops the preview but keeps every field.
        let mut g = Grid::new(60, 14);
        f.draw(&mut g, true);
        assert!(g.find("█").is_none());
        assert!(g.find("Start").is_some());
        // A screen too small for it is left alone rather than overrun.
        assert!(!Form::fits(MIN_W - 1, 20));
        let mut tiny = Grid::new(MIN_W - 1, 20);
        f.draw(&mut tiny, true);
        assert!(tiny.find("New").is_none());
    }

    #[test]
    fn the_narrowest_form_shows_every_type_and_ends_a_long_error_plainly() {
        let mut f = Form::new_clock();
        let mut g = Grid::new(MIN_W, MIN_H);
        f.draw(&mut g, true);
        assert!(g.find("stopwatch countdown rounds").is_some());
        countdown(&mut f);
        typed(&mut f, "abcdefghijklmnopqrstuvwxyz");
        press(&mut f, KeyCode::Enter);
        let mut g = Grid::new(MIN_W, MIN_H);
        f.draw(&mut g, true);
        assert!(g.find("…").is_some());
        assert_eq!(wrap("日本 語", 4), vec!["日本", "語"]);
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
    }
}
