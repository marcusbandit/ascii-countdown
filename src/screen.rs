//! Owns the terminal while tock runs and puts it back afterwards.
//!
//! Every view composes a `Grid` of cells, and `Screen::draw` writes the whole
//! grid out row by row. Nothing from the last frame survives, so no clear is
//! needed between frames.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};

use crossterm::style::{
    Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, ClearType};
use crossterm::{cursor, execute, queue};
use unicode_width::UnicodeWidthChar;

pub use crossterm::style::Color as Ink;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub reverse: bool,
    pub underline: bool,
}

impl Style {
    pub fn fg(c: Color) -> Style {
        Style {
            fg: Some(c),
            ..Style::default()
        }
    }

    pub fn bold(self) -> Style {
        Style { bold: true, ..self }
    }

    pub fn reversed(self) -> Style {
        Style {
            reverse: true,
            ..self
        }
    }

    /// The one highlight colour: titles, the selection frame, focused fields.
    pub fn accent(color: bool) -> Style {
        if color {
            Style::fg(Color::Cyan).bold()
        } else {
            Style::default().bold()
        }
    }
}

/// `'\0'` marks the right half of a wide character drawn in the cell before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Clone, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<Cell>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        let blank = Cell {
            ch: ' ',
            style: Style::default(),
        };
        Grid {
            cols,
            rows,
            cells: vec![blank; cols * rows],
        }
    }

    pub fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.cols + x]
    }

    /// Blanks `r` in `style`, clipped to the grid.
    pub fn fill(&mut self, r: Rect, style: Style) {
        for y in r.y..(r.y + r.h).min(self.rows) {
            for x in r.x..(r.x + r.w).min(self.cols) {
                self.cells[y * self.cols + x] = Cell { ch: ' ', style };
            }
        }
    }

    /// Writes `text` from column `x`, stopping before column `end` and the
    /// grid's edge. Returns the column after the last character written.
    pub fn put(&mut self, x: usize, y: usize, text: &str, style: Style, end: usize) -> usize {
        let end = end.min(self.cols);
        let mut x = x;
        if y >= self.rows {
            return x;
        }
        for ch in text.chars() {
            let w = ch.width().unwrap_or(0);
            if w == 0 {
                continue;
            }
            if x + w > end {
                break;
            }
            self.cells[y * self.cols + x] = Cell { ch, style };
            if w == 2 {
                self.cells[y * self.cols + x + 1] = Cell { ch: '\0', style };
            }
            x += w;
        }
        x
    }

    /// Draws a rounded box along the edge of `r`, keeping each cell's background.
    pub fn frame(&mut self, r: Rect, style: Style) {
        if r.w < 2 || r.h < 2 || r.x + r.w > self.cols || r.y + r.h > self.rows {
            return;
        }
        let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
        let cols = self.cols;
        let mut set = |x: usize, y: usize, ch: char| {
            let cell = &mut self.cells[y * cols + x];
            cell.ch = ch;
            cell.style = Style {
                bg: cell.style.bg,
                ..style
            };
        };
        for x in r.x + 1..x1 {
            set(x, r.y, '─');
            set(x, y1, '─');
        }
        for y in r.y + 1..y1 {
            set(r.x, y, '│');
            set(x1, y, '│');
        }
        set(r.x, r.y, '╭');
        set(x1, r.y, '╮');
        set(r.x, y1, '╰');
        set(x1, y1, '╯');
    }

    /// The characters of row `y`, for tests.
    #[cfg(test)]
    pub fn row_text(&self, y: usize) -> String {
        (0..self.cols)
            .map(|x| self.cell(x, y).ch)
            .filter(|&c| c != '\0')
            .collect()
    }

    /// Finds `text` on screen as (column, row), for tests.
    #[cfg(test)]
    pub fn find(&self, text: &str) -> Option<(usize, usize)> {
        (0..self.rows).find_map(|y| {
            let row = self.row_text(y);
            row.find(text).map(|i| (row[..i].chars().count(), y))
        })
    }
}

/// Cuts `s` down to at most `cols` terminal columns.
pub fn truncate(s: &str, cols: usize) -> String {
    let mut used = 0;
    s.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= cols
        })
        .collect()
}

/// Where frames go: the controlling terminal, not stdout, so stdout stays free
/// for the stopwatch results even when it is redirected.
type Out = BufWriter<File>;

fn open_tty() -> io::Result<Out> {
    let tty = OpenOptions::new().write(true).open("/dev/tty")?;
    Ok(BufWriter::with_capacity(64 * 1024, tty))
}

fn apply(out: &mut Out, s: Style) -> io::Result<()> {
    queue!(out, SetAttribute(Attribute::Reset))?;
    if let Some(c) = s.fg {
        queue!(out, SetForegroundColor(c))?;
    }
    if let Some(c) = s.bg {
        queue!(out, SetBackgroundColor(c))?;
    }
    if s.bold {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    if s.dim {
        queue!(out, SetAttribute(Attribute::Dim))?;
    }
    if s.reverse {
        queue!(out, SetAttribute(Attribute::Reverse))?;
    }
    if s.underline {
        queue!(out, SetAttribute(Attribute::Underlined))?;
    }
    Ok(())
}

pub struct Screen {
    out: Out,
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
        Ok(Screen { out })
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

    pub fn draw(&mut self, grid: &Grid) -> io::Result<()> {
        let (cols, rows) = (grid.cols, grid.rows);
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        let out = &mut self.out;
        queue!(out, terminal::BeginSynchronizedUpdate)?;
        for y in 0..rows {
            queue!(out, cursor::MoveTo(0, y as u16))?;
            // Stop one short on the last row: writing its last cell can scroll
            // some terminals. The erase after fills that cell instead.
            let end = if y + 1 == rows { cols - 1 } else { cols };
            let mut style = None;
            let mut run = String::new();
            for x in 0..end {
                let mut cell = grid.cell(x, y);
                if cell.ch == '\0' {
                    continue;
                }
                // A wide character would spill into the column held back above.
                if x + cell.ch.width().unwrap_or(1) > end {
                    cell.ch = ' ';
                }
                if style != Some(cell.style) {
                    if let Some(s) = style {
                        apply(out, s)?;
                        queue!(out, Print(&run))?;
                        run.clear();
                    }
                    style = Some(cell.style);
                }
                run.push(cell.ch);
            }
            if let Some(s) = style {
                apply(out, s)?;
                queue!(out, Print(&run))?;
            }
            if y + 1 == rows {
                apply(out, grid.cell(cols - 1, y).style)?;
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

    #[test]
    fn wide_characters_take_two_cells_and_never_spill() {
        let mut g = Grid::new(5, 1);
        g.put(0, 0, "日本語", Style::default(), 5);
        assert_eq!(g.row_text(0), "日本 ");
        assert_eq!(truncate("日本語", 5), "日本");
        assert_eq!(truncate("hello", 3), "hel");
    }

    #[test]
    fn frame_draws_a_box_and_keeps_the_background() {
        let mut g = Grid::new(6, 4);
        let red = Style {
            bg: Some(Color::Red),
            ..Style::default()
        };
        g.fill(
            Rect {
                x: 0,
                y: 0,
                w: 6,
                h: 4,
            },
            red,
        );
        g.frame(
            Rect {
                x: 1,
                y: 0,
                w: 4,
                h: 3,
            },
            Style::fg(Color::Cyan),
        );
        assert_eq!(g.row_text(0), " ╭──╮ ");
        assert_eq!(g.row_text(1), " │  │ ");
        assert_eq!(g.row_text(2), " ╰──╯ ");
        assert_eq!(g.cell(1, 0).style.bg, Some(Color::Red));
        assert_eq!(g.cell(1, 0).style.fg, Some(Color::Cyan));
    }

    #[test]
    fn put_clips_at_the_grid_edge() {
        let mut g = Grid::new(4, 1);
        assert_eq!(g.put(2, 0, "abcdef", Style::default(), 99), 4);
        assert_eq!(g.row_text(0), "  ab");
    }
}
