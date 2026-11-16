//! The main view: every clock in a list, with keys to manage them.

use std::borrow::Cow;

use unicode_width::UnicodeWidthStr;

use crate::display::{flash_style, tone_style, Tone};
use crate::screen::{truncate, Grid, Ink, Rect, Style};
use crate::timer::{Mode, State, Timer};

/// The list never grows wider than this, so a name and its time stay close.
const MAX_W: usize = 72;

/// A key and what it does, for the bar along the bottom.
pub type Key = (&'static str, &'static str);

/// Where the list sits: its left column and width on a `cols` wide screen.
pub fn column(cols: usize) -> (usize, usize) {
    let w = cols.saturating_sub(4).min(MAX_W);
    ((cols - w) / 2, w)
}

/// The keys that do something on the list right now, for the clock `t` the
/// selection is on.
pub fn keys(t: Option<&Timer>) -> Vec<Key> {
    let Some(t) = t else {
        return vec![("a", "add"), ("q", "quit")];
    };
    // Most used first: a narrow bar drops keys from the end, but keeps quit.
    let mut keys = vec![("a", "add"), ("enter", "full screen")];
    keys.extend(clock_keys(t));
    keys.extend([
        ("e", "edit"),
        ("v", if t.shown { "hide" } else { "show" }),
        ("r", "reset"),
        ("x", "cancel"),
        ("q", "quit"),
    ]);
    keys
}

/// The keys on the full screen view. While a clock is selected they are the
/// ones that act on it; once the selection hides, only the way back.
pub fn display_keys(t: Option<&Timer>, selected: bool, multi: bool) -> Vec<Key> {
    let mut keys = Vec::new();
    if let (Some(t), true) = (t, selected) {
        keys.extend(clock_keys(t));
        keys.extend([("e", "edit"), ("r", "reset"), ("x", "cancel")]);
        if multi {
            keys.push(("hjkl", "move"));
        }
        keys.push(("a", "add"));
    }
    keys.push(("esc", "list"));
    keys
}

/// The keys that act on clock `t` itself, which the list and the full screen
/// view share.
pub fn clock_keys(t: &Timer) -> Vec<Key> {
    let mut keys = Vec::new();
    // A clock whose time is up has nothing to pause; reset starts it over.
    match t.state() {
        State::Done => {}
        State::Paused => keys.push(("space", "start")),
        State::Running => keys.push(("space", "pause")),
    }
    match t.mode {
        // Minus is its mirror and stays off the bar to keep it short.
        Mode::Countdown(_) => keys.push(("+", "1 min")),
        Mode::Rounds => keys.push(("n", "next round")),
        Mode::Stopwatch => {}
    }
    keys
}

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
    let (x0, w) = column(cols);
    let right = x0 + w;
    // A row of air above the header once there is room for it.
    let head_y = usize::from(rows >= 12);

    grid.put(x0 + 1, head_y, "tock", accent, right);
    let shown = timers.iter().filter(|t| t.shown).count();
    if shown > 0 {
        let text = format!("{shown} shown");
        let x = right.saturating_sub(text.width() + 1);
        grid.put(x.saturating_sub(2), head_y, "●", accent, right);
        grid.put(x, head_y, &text, Style::default(), right);
    }

    // The list runs from under the header to above the note and the keys,
    // with a row of air above the note once there is room for it.
    let note_y = rows - 2 - usize::from(rows >= 12);
    let list = Rect {
        x: x0,
        y: head_y + 2,
        w,
        h: note_y.saturating_sub(head_y + 2),
    };
    if timers.is_empty() {
        let text = "No clocks";
        grid.put(
            (cols - text.width()) / 2,
            rows / 2 - 1,
            text,
            Style::default().bold(),
            cols,
        );
    }
    let top = if sel < top {
        sel
    } else if list.h > 0 && sel >= top + list.h {
        sel + 1 - list.h
    } else {
        top
    };

    // From the right: the status, the time, and the name takes the rest.
    let status_w = "paused, round 99".len();
    let status_end = right - 1;
    let time_end = status_end.saturating_sub(status_w + 2);
    let name_x = x0 + 4;

    for (row, (i, t)) in timers.iter().enumerate().skip(top).take(list.h).enumerate() {
        let y = list.y + row;
        let selected = i == sel;
        // The selection swaps the terminal's own colours, which reads in every
        // theme. On that row everything takes the plain swapped colours.
        let base = if t.flashing() {
            flash_style(true, color)
        } else if selected {
            Style::default().reversed()
        } else {
            Style::default()
        };
        let plain = t.flashing() || selected;
        grid.fill(Rect { y, h: 1, ..list }, base);
        if t.shown {
            grid.put(x0 + 2, y, "●", if plain { base } else { accent }, right);
        }
        let name = truncate(&t.label(), time_end.saturating_sub(name_x + 10));
        let name_style = if t.title.is_some() { base.bold() } else { base };
        grid.put(name_x, y, &name, name_style, time_end);

        let face = t.face();
        // On the swapped row colour would clash, so the last seconds and a
        // time that is up turn bold instead.
        let face_style = match (plain, t.tone()) {
            (true, Tone::Warn) => base.bold(),
            (true, _) => base,
            (false, tone) => tone_style(base, tone, color),
        };
        grid.put(
            time_end.saturating_sub(face.width()),
            y,
            &face,
            face_style,
            time_end,
        );
        // Only a rounds clock builds a string; the rest are fixed words.
        let status: Cow<str> = match (t.state(), t.mode) {
            (State::Done, _) => "time's up".into(),
            (State::Paused, Mode::Rounds) => format!("paused, round {}", t.round()).into(),
            (State::Paused, _) => "paused".into(),
            (State::Running, Mode::Rounds) => format!("round {}", t.round()).into(),
            (State::Running, _) => "".into(),
        };
        let status_style = match t.state() {
            State::Done if plain => base.bold(),
            State::Done if color => Style::fg(Ink::Red).bold(),
            State::Done => base.bold(),
            _ => base,
        };
        grid.put(
            status_end.saturating_sub(status.width()),
            y,
            &status,
            status_style,
            status_end,
        );
    }

    if let Some(n) = note {
        grid.put(
            x0 + 1,
            note_y,
            &truncate(n, w - 2),
            Style::default().bold(),
            right,
        );
    }
    top
}

/// The key bar: each key in the accent, then what it does. It starts at the
/// list's left edge and may run past its right one. Keys that do not fit are
/// dropped from the end, except the last, which is the way out.
pub fn toolbar(grid: &mut Grid, y: usize, keys: &[Key], color: bool) {
    let x = column(grid.cols).0 + 1;
    bar(grid, y, keys, color, Some(x));
}

/// The key bar centered on the screen, for the full screen view.
pub fn toolbar_centered(grid: &mut Grid, y: usize, keys: &[Key], color: bool) {
    bar(grid, y, keys, color, None);
}

fn bar(grid: &mut Grid, y: usize, keys: &[Key], color: bool, x: Option<usize>) {
    const GAP: usize = 3;
    let end = grid.cols.saturating_sub(2);
    let start = x.unwrap_or(2);
    grid.fill(
        Rect {
            x: 0,
            y,
            w: grid.cols,
            h: 1,
        },
        Style::default(),
    );
    let width = |k: &Key| k.0.width() + 1 + k.1.width();
    let total = |ks: &[Key]| ks.iter().map(width).sum::<usize>() + GAP * ks.len().saturating_sub(1);
    let mut keep: Vec<Key> = keys.to_vec();
    while keep.len() > 1 && start + total(&keep) > end {
        keep.remove(keep.len() - 2);
    }
    let mut x = x.unwrap_or_else(|| grid.cols.saturating_sub(total(&keep)) / 2);
    for (k, what) in keep {
        x = grid.put(x, y, k, Style::accent(color), end);
        x = grid.put(x + 1, y, what, Style::default(), end) + GAP;
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
        assert!(g.find("1 shown").is_some());
        // The selected row is swapped across its width, the others are not.
        let (x0, w) = column(80);
        assert!(g.cell(x0, tea_y).style.reverse);
        assert!(g.cell(x0 + w - 1, tea_y).style.reverse);
        assert!(!g.cell(x0, other_y).style.reverse);
    }

    #[test]
    fn a_paused_time_stays_readable_on_the_selected_row() {
        let mut timers = vec![
            timer(Some("Tea"), 240, true),
            timer(Some("Pasta"), 600, true),
        ];
        timers[0].pause();
        timers[1].pause();
        let mut g = Grid::new(80, 24);
        compose(&mut g, &timers, 0, 0, None, true);
        for name in ["Tea", "Pasta"] {
            let (_, y) = g.find(name).unwrap();
            let row = g.row_text(y);
            for word in ["04:00", "10:00", "paused"] {
                if let Some(x) = row.find(word) {
                    let st = g.cell(x, y).style;
                    let hidden = st.fg.is_some() && st.fg == st.bg;
                    assert!(!hidden, "{name}: {word} hides in its row");
                    // Faint text on the swapped row would fade into it.
                    assert!(!(st.reverse && st.dim), "{name}: {word} is faint");
                }
            }
        }
        // No theme colour stands in for faint text: bright black is the
        // background in some palettes.
        for y in 0..24 {
            for x in 0..80 {
                assert_ne!(g.cell(x, y).style.fg, Some(Ink::DarkGrey));
                assert_ne!(g.cell(x, y).style.bg, Some(Ink::DarkGrey));
            }
        }
    }

    #[test]
    fn running_needs_no_word_and_the_other_states_say_theirs() {
        let mut timers = vec![
            timer(Some("Tea"), 240, false),
            timer(Some("Egg"), 60, false),
        ];
        timers[1].pause();
        let mut g = Grid::new(80, 24);
        compose(&mut g, &timers, 0, 0, None, true);
        assert!(!g
            .find("Tea")
            .map(|(_, y)| g.row_text(y))
            .unwrap()
            .contains("running"));
        assert!(g.find("paused").is_some());
    }

    #[test]
    fn the_keys_follow_the_selected_clock() {
        assert_eq!(keys(None), vec![("a", "add"), ("q", "quit")]);
        let mut t = timer(None, 60, false);
        assert!(keys(Some(&t)).contains(&("space", "pause")));
        assert!(keys(Some(&t)).contains(&("v", "show")));
        t.pause();
        t.shown = true;
        assert!(keys(Some(&t)).contains(&("space", "start")));
        assert!(keys(Some(&t)).contains(&("+", "1 min")));
        assert!(keys(Some(&t)).contains(&("v", "hide")));
        let mut done = timer(None, 1, false);
        done.clock.stop_at(std::time::Duration::from_secs(2));
        done.check_done();
        assert!(done.is_done());
        assert!(!keys(Some(&done)).iter().any(|k| k.0 == "space"));
    }

    #[test]
    fn the_toolbar_drops_keys_that_do_not_fit() {
        let t = timer(None, 60, false);
        let mut g = Grid::new(40, 3);
        toolbar(&mut g, 2, &keys(Some(&t)), true);
        let row = g.row_text(2);
        assert!(row.contains("add"));
        assert!(row.contains("q quit"), "quit always stays: {row:?}");
        assert!(!row.contains("cancel"));
        assert!(row.trim_end().len() <= 38);
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
        compose(&mut g, &[], 0, 0, Some("Four clocks are shown"), true);
        assert_eq!(g.find("Four clocks").unwrap().1, 21);
    }

    #[test]
    fn a_tiny_list_still_fits_a_clock_and_never_underflows() {
        let mut timers = vec![timer(Some("Egg"), 1, false)];
        timers[0].clock.stop_at(std::time::Duration::from_secs(2));
        timers[0].check_done();
        for cols in 10..14 {
            let mut g = Grid::new(cols, 5);
            compose(&mut g, &timers, 0, 0, None, true);
        }
        let mut g = Grid::new(40, 5);
        compose(&mut g, &timers, 0, 0, None, true);
        assert!(g.find("Egg").is_some());
    }
}
