//! The main view: every clock in a list, with keys to manage them.

use unicode_width::UnicodeWidthStr;

use crate::display::{flash_style, tone_style};
use crate::screen::{truncate, Grid, Ink, Rect, Style};
use crate::timer::Timer;

/// The keys shown along the bottom, as a toolbar.
const KEYS: [(&str, &str); 8] = [
    ("a", "add"),
    ("e", "edit"),
    ("space", "pause"),
    ("r", "reset"),
    ("x", "cancel"),
    ("v", "display"),
    ("enter", "show"),
    ("q", "quit"),
];

/// Draws the list. `top` is the first row scrolled into view; the return value
/// is the `top` that keeps `sel` visible, to pass back next frame.
pub fn compose(
    grid: &mut Grid,
    timers: &[Timer],
    sel: usize,
    top: usize,
    note: Option<&str>,
    color: bool,
) -> usize {
    let (cols, rows) = (grid.cols, grid.rows);
    if cols < 10 || rows < 4 {
        if let Some(t) = timers.get(sel) {
            grid.put(
                0,
                0,
                &format!("{} {}", t.label(), t.face()),
                Style::default(),
                cols,
            );
        }
        return top;
    }
    let accent = Style::accent(color);
    let dim = Style {
        dim: true,
        ..Style::default()
    };

    grid.put(2, 0, "tock", accent, cols);
    let count = match timers.len() {
        1 => "1 clock".to_string(),
        n => format!("{n} clocks"),
    };
    grid.put(cols.saturating_sub(count.width() + 2), 0, &count, dim, cols);

    // Rows 2 to rows-3 hold the list; the last two hold a note and the keys.
    let list = Rect {
        x: 0,
        y: 2,
        w: cols,
        h: rows.saturating_sub(4),
    };
    if timers.is_empty() {
        let text = "No clocks";
        grid.put((cols - text.width()) / 2, rows / 2 - 1, text, dim, cols);
    }
    let top = if sel < top {
        sel
    } else if list.h > 0 && sel >= top + list.h {
        sel + 1 - list.h
    } else {
        top
    };

    // Columns, from the right: status, time, then the name takes the rest.
    let status_w = 9;
    let time_w = 9;
    let status_x = cols.saturating_sub(status_w + 2);
    let time_x = status_x.saturating_sub(time_w + 2);
    let name_x = 6;

    for (row, (i, t)) in timers.iter().enumerate().skip(top).take(list.h).enumerate() {
        let y = list.y + row;
        let selected = i == sel;
        let mut base = flash_style(t.flashing(), color);
        if selected && !t.flashing() {
            base = if color {
                Style {
                    bg: Some(Ink::DarkGrey),
                    fg: Some(Ink::White),
                    ..Style::default()
                }
            } else {
                Style {
                    reverse: true,
                    ..Style::default()
                }
            };
        }
        grid.fill(
            Rect {
                x: 1,
                y,
                w: cols - 2,
                h: 1,
            },
            base,
        );
        if t.shown {
            let mark = if t.flashing() {
                base
            } else {
                Style {
                    bg: base.bg,
                    reverse: base.reverse,
                    ..accent
                }
            };
            grid.put(3, y, "●", mark, cols);
        }
        let name = truncate(&t.label(), time_x.saturating_sub(name_x + 1));
        let name_style = if t.title.is_some() {
            base.bold()
        } else {
            Style {
                dim: !selected,
                ..base
            }
        };
        grid.put(name_x, y, &name, name_style, time_x);

        let face = t.face();
        let face_style = if t.flashing() {
            base
        } else {
            tone_style(base, t.tone(), color)
        };
        grid.put(
            time_x + time_w - face.width().min(time_w),
            y,
            &face,
            face_style,
            status_x,
        );
        let status = match t.status() {
            "" => "running",
            "PAUSED" => "paused",
            _ => "time's up",
        };
        grid.put(status_x, y, status, face_style, cols - 1);
    }

    if let Some(n) = note {
        let style = if color {
            Style::fg(Ink::Yellow)
        } else {
            Style::default().bold()
        };
        grid.put(2, rows - 2, &truncate(n, cols - 4), style, cols);
    }
    toolbar(grid, rows - 1, &KEYS);
    top
}

/// Key chips in a row: the key reversed, then what it does.
pub fn toolbar(grid: &mut Grid, y: usize, keys: &[(&str, &str)]) {
    let key_style = Style::default().bold().reversed();
    let mut x = 2;
    for (k, what) in keys {
        let chip = format!(" {k} ");
        if x + chip.width() + 1 + what.width() > grid.cols.saturating_sub(1) {
            break;
        }
        x = grid.put(x, y, &chip, key_style, grid.cols);
        x = grid.put(x + 1, y, what, Style::default(), grid.cols) + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::{ClockSpec, Mode};
    use std::time::Duration;

    fn timer(title: Option<&str>, secs: u64, shown: bool) -> Timer {
        Timer::new(
            ClockSpec {
                mode: Mode::Countdown(Duration::from_secs(secs)),
                title: title.map(str::to_string),
            },
            shown,
        )
    }

    #[test]
    fn lists_every_clock_with_its_time() {
        let timers = vec![timer(Some("Tea"), 240, true), timer(None, 600, false)];
        let mut g = Grid::new(80, 24);
        compose(&mut g, &timers, 0, 0, None, true);
        let (_, tea_y) = g.find("Tea").unwrap();
        assert!(g.row_text(tea_y).contains("04:00"));
        assert!(g.row_text(tea_y).contains("●"));
        let (_, other_y) = g.find("10m").unwrap();
        assert!(!g.row_text(other_y).contains("●"));
        assert!(g.find("2 clocks").is_some());
        assert!(g.find("add").is_some());
        // The selected row is highlighted across its width.
        assert_eq!(g.cell(1, tea_y).style.bg, Some(Ink::DarkGrey));
        assert_eq!(g.cell(1, other_y).style.bg, None);
    }

    #[test]
    fn an_empty_list_says_so() {
        let mut g = Grid::new(80, 24);
        compose(&mut g, &[], 0, 0, None, true);
        assert!(g.find("No clocks").is_some());
    }

    #[test]
    fn scrolls_to_keep_the_selection_in_view() {
        let timers: Vec<Timer> = (0..30)
            .map(|i| timer(Some(&format!("T{i}")), 60, false))
            .collect();
        let mut g = Grid::new(60, 12);
        let top = compose(&mut g, &timers, 25, 0, None, true);
        assert!(top > 0);
        assert!(g.find("T25").is_some());
        assert!(g.find("T0 ").is_none());
    }

    #[test]
    fn notes_show_above_the_keys() {
        let mut g = Grid::new(80, 24);
        compose(&mut g, &[], 0, 0, Some("Four clocks are on display"), true);
        assert_eq!(g.find("Four clocks").unwrap().1, 22);
    }
}
