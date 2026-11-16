//! Owns the terminal while the clock runs and puts it back afterwards.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};

use crossterm::style::{
    Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, ClearType};
use crossterm::{cursor, execute, queue};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::font;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Normal,
    Paused,
    Warn,
}

/// One frame's worth of state. Two equal views draw the same frame.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub title: Option<String>,
    pub face: String,
    pub tone: Tone,
    pub status: &'static str,
    pub flash: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Blank,
    Title,
    Face,
    Status,
}

/// One terminal row: the column the text starts at, what it is, and the text.
#[derive(Debug, PartialEq)]
pub struct Line {
    pub left: usize,
    pub part: Part,
    pub text: String,
}

/// Rows kept free above and below the digits for the title and the status.
const BAND: usize = 2;
/// Columns kept free on each side once the terminal is wide enough to spare them.
const SIDE: usize = 2;

/// Lays out a view on a `cols` x `rows` terminal, one `Line` per row.
pub fn layout(view: &View, cols: usize, rows: usize) -> Vec<Line> {
    let side = if cols >= 40 { SIDE } else { 0 };
    let inner = cols - 2 * side;

    let mut block: Vec<(Part, String)> = Vec::new();
    let title = view.title.as_deref().map(|t| truncate(t, cols));
    let status = truncate(view.status, cols);

    if let Some((sx, sy)) = font::fit(&view.face, inner, rows.saturating_sub(2 * BAND)) {
        // Title and status each get a band, and both bands are always there so
        // the digits stay put when a status appears or a title is missing.
        block.push((Part::Title, title.unwrap_or_default()));
        block.push((Part::Blank, String::new()));
        block.extend(
            font::render(&view.face, sx, sy)
                .into_iter()
                .map(|l| (Part::Face, l)),
        );
        block.push((Part::Blank, String::new()));
        block.push((Part::Status, status));
    } else if let Some((sx, sy)) = font::fit(&view.face, inner, rows) {
        block.extend(
            font::render(&view.face, sx, sy)
                .into_iter()
                .map(|l| (Part::Face, l)),
        );
    } else {
        // Too small for big digits: plain text, as much of it as fits.
        // The time comes first, then the status, then the title.
        if let Some(t) = title.filter(|t| !t.is_empty() && rows >= 3) {
            block.push((Part::Title, t));
        }
        block.push((Part::Face, truncate(&view.face, cols)));
        if !status.is_empty() && rows > block.len() {
            block.push((Part::Status, status));
        }
        block.truncate(rows);
    }

    let top = (rows - block.len()) / 2;
    let mut lines: Vec<Line> = (0..rows)
        .map(|_| Line {
            left: 0,
            part: Part::Blank,
            text: String::new(),
        })
        .collect();
    for (i, (part, text)) in block.into_iter().enumerate() {
        let width = text.width();
        lines[top + i] = Line {
            left: (cols - width) / 2,
            part,
            text,
        };
    }
    lines
}

/// Cuts `s` down to at most `cols` terminal columns.
fn truncate(s: &str, cols: usize) -> String {
    let mut used = 0;
    s.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= cols
        })
        .collect()
}

/// Where frames go: the controlling terminal, not stdout, so stdout stays free
/// for the stopwatch's result even when it is redirected.
type Out = BufWriter<File>;

fn open_tty() -> io::Result<Out> {
    let tty = OpenOptions::new().write(true).open("/dev/tty")?;
    Ok(BufWriter::with_capacity(64 * 1024, tty))
}

pub struct Screen {
    out: Out,
    color: bool,
}

impl Screen {
    /// Switches to the alternate screen in raw mode with the cursor hidden.
    pub fn enter() -> io::Result<Screen> {
        let mut out = open_tty()?;
        terminal::enable_raw_mode()?;
        if let Err(e) = execute!(
            out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(ClearType::All)
        ) {
            restore();
            return Err(e);
        }
        // https://no-color.org: any non-empty NO_COLOR turns colours off.
        let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
        Ok(Screen { out, color })
    }

    pub fn clear(&mut self) -> io::Result<()> {
        execute!(
            self.out,
            SetAttribute(Attribute::Reset),
            terminal::Clear(ClearType::All)
        )
    }

    pub fn bell(&mut self) -> io::Result<()> {
        execute!(self.out, Print('\x07'))
    }

    /// Draws every row in full, so nothing from the last frame survives and no
    /// clear is needed between frames.
    pub fn draw(&mut self, view: &View, (cols, rows): (u16, u16)) -> io::Result<()> {
        let (cols, rows) = (cols as usize, rows as usize);
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        let out = &mut self.out;
        queue!(out, terminal::BeginSynchronizedUpdate)?;
        for (y, line) in layout(view, cols, rows).iter().enumerate() {
            queue!(out, cursor::MoveTo(0, y as u16))?;
            base_style(out, view, self.color)?;
            queue!(out, Print(" ".repeat(line.left)))?;
            part_style(out, view, line.part, self.color)?;
            queue!(out, Print(&line.text))?;
            base_style(out, view, self.color)?;
            // Stop one short on the last row: writing its last cell can scroll
            // some terminals. The erase fills that cell with the background.
            let end = if y + 1 == rows { cols - 1 } else { cols };
            let used = line.left + line.text.width();
            queue!(out, Print(" ".repeat(end.saturating_sub(used))))?;
            if y + 1 == rows {
                queue!(out, terminal::Clear(ClearType::UntilNewLine))?;
            }
        }
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            terminal::EndSynchronizedUpdate
        )?;
        out.flush()
    }
}

/// The style of the whole screen: plain, or inverted while flashing.
fn base_style(out: &mut Out, view: &View, color: bool) -> io::Result<()> {
    queue!(out, SetAttribute(Attribute::Reset))?;
    match (view.flash, color) {
        (false, _) => Ok(()),
        (true, true) => queue!(
            out,
            SetBackgroundColor(Color::Red),
            SetForegroundColor(Color::Black)
        ),
        (true, false) => queue!(out, SetAttribute(Attribute::Reverse)),
    }
}

fn part_style(out: &mut Out, view: &View, part: Part, color: bool) -> io::Result<()> {
    if part == Part::Title {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    if view.flash || !matches!(part, Part::Face | Part::Status) {
        return Ok(());
    }
    if part == Part::Status {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    match (view.tone, color) {
        (Tone::Normal, _) => Ok(()),
        (Tone::Paused, true) => queue!(out, SetForegroundColor(Color::DarkGrey)),
        (Tone::Warn, true) => queue!(out, SetForegroundColor(Color::Red)),
        (Tone::Paused, false) => queue!(out, SetAttribute(Attribute::Dim)),
        (Tone::Warn, false) => queue!(out, SetAttribute(Attribute::Bold)),
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        restore();
    }
}

/// Leaves the alternate screen, shows the cursor and turns raw mode off.
/// Safe to call more than once.
pub fn restore() {
    if let Ok(mut out) = open_tty() {
        let _ = execute!(
            out,
            SetAttribute(Attribute::Reset),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
    }
    let _ = terminal::disable_raw_mode();
}

/// Restores the terminal before a panic message prints, so it lands on the
/// normal screen instead of vanishing with the alternate one.
pub fn restore_on_panic() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(face: &str, title: Option<&str>) -> View {
        View {
            title: title.map(str::to_string),
            face: face.to_string(),
            tone: Tone::Normal,
            status: "",
            flash: false,
        }
    }

    fn face_rows(lines: &[Line]) -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.part == Part::Face)
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn one_line_per_row_and_nothing_off_screen() {
        for (cols, rows) in [(80, 24), (200, 60), (41, 9), (20, 5), (10, 3), (3, 1)] {
            let lines = layout(
                &view("25:00", Some("A long title for the timer")),
                cols,
                rows,
            );
            assert_eq!(lines.len(), rows);
            for l in &lines {
                assert!(l.left + l.text.width() <= cols, "{cols}x{rows}: {l:?}");
            }
        }
    }

    #[test]
    fn digits_are_centered() {
        let lines = layout(&view("25:00", None), 80, 24);
        let rows = face_rows(&lines);
        // 80 - 4 columns of margin fit a scale of 2: 10 rows of digits.
        assert_eq!(rows.len(), 10);
        let above = rows[0];
        let below = 24 - 1 - rows[rows.len() - 1];
        assert!(above.abs_diff(below) <= 1);
        let l = &lines[rows[0]];
        assert!((80 - l.left - l.text.width()).abs_diff(l.left) <= 1);
    }

    #[test]
    fn title_sits_above_digits_and_status_below() {
        let mut v = view("25:00", Some("Tea"));
        v.status = "PAUSED";
        let lines = layout(&v, 80, 24);
        let rows = face_rows(&lines);
        assert_eq!(lines[rows[0] - 2].text, "Tea");
        assert_eq!(lines[rows[rows.len() - 1] + 2].text, "PAUSED");
    }

    #[test]
    fn digits_do_not_move_when_status_changes() {
        let mut v = view("25:00", None);
        let before = face_rows(&layout(&v, 80, 24));
        v.status = "PAUSED";
        assert_eq!(face_rows(&layout(&v, 80, 24)), before);
    }

    #[test]
    fn tiny_terminal_falls_back_to_plain_text() {
        let lines = layout(&view("25:00", Some("Tea")), 12, 3);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(texts.contains(&"25:00"), "{texts:?}");
        assert_eq!(lines.iter().filter(|l| l.part == Part::Face).count(), 1);
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("hello", 3), "hel");
        assert_eq!(truncate("日本語", 5), "日本");
        assert_eq!(truncate("ok", 10), "ok");
    }
}
