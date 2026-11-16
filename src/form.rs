//! The add and edit dialog: a name, countdown or stopwatch, and a time.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::duration;
use crate::manager::Key;
use crate::screen::{truncate, Grid, Ink, Rect, Style};
use crate::timer::{Mode, Timer};

/// Longest name the form takes, in characters.
const NAME_MAX: usize = 40;
/// The dialog's size: its rows, and the fewest columns it still works in.
const HEIGHT: usize = 8;
const MIN_W: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Countdown,
    Stopwatch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Name,
    Kind,
    Time,
}

#[derive(Debug, PartialEq)]
pub enum FormEvent {
    /// Still open.
    Open,
    Cancel,
    Save {
        title: Option<String>,
        mode: Mode,
    },
}

#[derive(Debug)]
pub struct Form {
    /// The clock being edited, or `None` for a new one.
    pub target: Option<usize>,
    kind: Kind,
    name: String,
    time: String,
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
            focus: Field::Name,
            error: None,
        }
    }

    pub fn edit(index: usize, t: &Timer) -> Form {
        let (kind, time) = match t.mode {
            Mode::Countdown(d) => (Kind::Countdown, duration::compact(d)),
            Mode::Stopwatch => (Kind::Stopwatch, String::new()),
        };
        Form {
            target: Some(index),
            kind,
            name: t.title.clone().unwrap_or_default(),
            time,
            focus: Field::Name,
            error: None,
        }
    }

    fn fields(&self) -> Vec<Field> {
        match self.kind {
            Kind::Countdown => vec![Field::Name, Field::Kind, Field::Time],
            Kind::Stopwatch => vec![Field::Name, Field::Kind],
        }
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
            Field::Kind => None,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> FormEvent {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return FormEvent::Cancel,
            KeyCode::Enter => return self.submit(),
            KeyCode::Tab | KeyCode::Down => self.step(1),
            KeyCode::BackTab | KeyCode::Up => self.step(-1),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ' | 'h' | 'l')
                if self.focus == Field::Kind =>
            {
                self.kind = match self.kind {
                    Kind::Countdown => Kind::Stopwatch,
                    Kind::Stopwatch => Kind::Countdown,
                };
            }
            KeyCode::Char('u') if ctrl => {
                if let Some(t) = self.text() {
                    t.clear();
                }
            }
            KeyCode::Backspace => {
                if let Some(t) = self.text() {
                    t.pop();
                }
            }
            KeyCode::Char(c) if !ctrl => {
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
        }
    }

    /// The keys that work in the dialog, for the bar along the bottom.
    pub fn keys(&self) -> Vec<Key> {
        let mut keys = vec![
            (
                "enter",
                if self.target.is_some() {
                    "save"
                } else {
                    "start"
                },
            ),
            ("tab", "next"),
        ];
        if self.focus == Field::Kind {
            keys.push(("space", "switch"));
        }
        keys.push(("esc", "close"));
        keys
    }

    /// True when a `cols` x `rows` screen has room for the whole dialog.
    pub fn fits(cols: usize, rows: usize) -> bool {
        cols >= MIN_W && rows >= HEIGHT
    }

    /// Draws the dialog centered on `grid`.
    pub fn draw(&self, grid: &mut Grid, color: bool) {
        if !Form::fits(grid.cols, grid.rows) {
            return;
        }
        let w = 50.min(grid.cols);
        let h = HEIGHT;
        let r = Rect {
            x: (grid.cols - w) / 2,
            y: (grid.rows - h) / 2,
            w,
            h,
        };
        let accent = Style::accent(color);
        grid.fill(r, Style::default());
        grid.frame(r, accent);
        let heading = if self.target.is_some() {
            " Edit clock "
        } else {
            " New clock "
        };
        grid.put(r.x + 2, r.y, heading, accent, r.x + w - 1);

        let (x, end) = (r.x + 4, r.x + w - 3);
        let value_x = x + 6;
        // The focused field gets a pointer as well as the accent, so focus
        // shows without colour too.
        let label = |grid: &mut Grid, y: usize, text: &str, field: Field| {
            let style = if self.focus == field {
                grid.put(x - 2, y, "›", accent, end);
                accent
            } else {
                Style::default()
            };
            grid.put(x, y, text, style, end);
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

        let y = r.y + 2;
        label(grid, y, "Name", Field::Name);
        text_row(grid, y, &self.name, Field::Name);

        // The chosen type is swapped; colour only adds the focus.
        label(grid, y + 1, "Type", Field::Kind);
        let chip = |kind: Kind| match (self.kind == kind, self.focus == Field::Kind) {
            (false, _) => Style::default(),
            (true, true) => Style {
                reverse: true,
                ..accent
            },
            (true, false) => Style::default().reversed(),
        };
        let after = grid.put(value_x, y + 1, " stopwatch ", chip(Kind::Stopwatch), end);
        grid.put(after + 1, y + 1, " countdown ", chip(Kind::Countdown), end);

        if self.kind == Kind::Countdown {
            label(grid, y + 2, "Time", Field::Time);
            text_row(grid, y + 2, &self.time, Field::Time);
        }

        if let Some(e) = &self.error {
            let style = if color {
                Style::fg(Ink::Red).bold()
            } else {
                Style::default().bold()
            };
            for (i, line) in wrap(e, end - x).iter().take(2).enumerate() {
                grid.put(x, y + 3 + i, line, style, end);
            }
        }
    }
}

/// Splits `text` into lines of at most `width` columns, at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if l.chars().count() + 1 + word.chars().count() <= width => {
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

    #[test]
    fn a_new_clock_starts_as_a_stopwatch() {
        let mut f = Form::new_clock();
        assert_eq!(f.kind, Kind::Stopwatch);
        assert_eq!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                title: None,
                mode: Mode::Stopwatch
            }
        );
    }

    #[test]
    fn a_new_countdown_needs_a_time() {
        let mut f = Form::new_clock();
        typed(&mut f, "Tea");
        press(&mut f, KeyCode::Tab);
        press(&mut f, KeyCode::Char('l'));
        assert_eq!(f.kind, Kind::Countdown);
        assert_eq!(press(&mut f, KeyCode::Enter), FormEvent::Open);
        assert!(f.error.is_some());
        assert_eq!(f.focus, Field::Time);
        typed(&mut f, "4m");
        assert!(f.error.is_none(), "typing clears the error");
        assert_eq!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                title: Some("Tea".into()),
                mode: Mode::Countdown(Duration::from_secs(240))
            }
        );
    }

    #[test]
    fn switching_to_a_stopwatch_drops_the_time_field() {
        let mut f = Form::new_clock();
        press(&mut f, KeyCode::Tab);
        assert_eq!(f.focus, Field::Kind);
        press(&mut f, KeyCode::Char('l'));
        press(&mut f, KeyCode::Tab);
        assert_eq!(f.focus, Field::Time);
        press(&mut f, KeyCode::BackTab);
        press(&mut f, KeyCode::Char(' '));
        assert_eq!(f.kind, Kind::Stopwatch);
        press(&mut f, KeyCode::Tab);
        assert_eq!(f.focus, Field::Name, "no time field to land on");
        assert_eq!(
            press(&mut f, KeyCode::Enter),
            FormEvent::Save {
                title: None,
                mode: Mode::Stopwatch
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
    fn editing_starts_from_the_clock() {
        let t = Timer::new(
            ClockSpec {
                mode: Mode::Countdown(Duration::from_secs(5400)),
                title: Some("Bake".into()),
            },
            true,
        );
        let mut f = Form::edit(3, &t);
        assert_eq!((f.name.as_str(), f.time.as_str()), ("Bake", "1h30m"));
        assert_eq!(f.target, Some(3));
        assert_eq!(press(&mut f, KeyCode::Esc), FormEvent::Cancel);
    }

    #[test]
    fn the_dialog_draws_inside_the_screen() {
        let mut f = Form::new_clock();
        typed(&mut f, "Tea");
        let mut g = Grid::new(80, 24);
        f.draw(&mut g, true);
        assert!(g.find("New clock").is_some());
        assert!(g.find("Tea").is_some());
        assert!(g.find("countdown").is_some());
        assert!(g.find("› Name").is_some(), "focus shows as a pointer");
        // A screen too small for it is left alone rather than overrun.
        assert!(!Form::fits(19, 10));
        let mut tiny = Grid::new(19, 10);
        f.draw(&mut tiny, true);
        assert!(tiny.find("New").is_none());
    }

    #[test]
    fn a_long_error_wraps_instead_of_being_cut() {
        let mut f = Form::edit(
            0,
            &Timer::new(
                ClockSpec {
                    mode: Mode::Countdown(Duration::from_secs(60)),
                    title: None,
                },
                false,
            ),
        );
        press(&mut f, KeyCode::Tab);
        press(&mut f, KeyCode::Tab);
        typed(&mut f, "x");
        press(&mut f, KeyCode::Enter);
        let e = f.error.clone().unwrap();
        let mut g = Grid::new(80, 24);
        f.draw(&mut g, true);
        let last = e.split(' ').next_back().unwrap();
        assert!(g.find(last).is_some(), "the end of {e:?} is on screen");
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
    }

    #[test]
    fn the_keys_name_what_enter_does() {
        let f = Form::new_clock();
        assert_eq!(f.keys()[0], ("enter", "start"));
        let t = Timer::new(
            ClockSpec {
                mode: Mode::Stopwatch,
                title: None,
            },
            false,
        );
        assert_eq!(Form::edit(0, &t).keys()[0], ("enter", "save"));
    }
}
