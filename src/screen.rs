//! Owns the terminal while the clocks run and puts it back afterwards.
//!
//! A frame is composed into a grid of cells first, one panel per clock, and
//! then written out row by row. Every cell is written on every draw, so
//! nothing from the last frame survives and no clear is needed between frames.

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

/// One clock's worth of state. Two equal panels draw the same pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub title: Option<String>,
    pub face: String,
    pub tone: Tone,
    pub status: &'static str,
    pub flash: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Title,
    Face,
    Status,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
    dim: bool,
    reverse: bool,
}

/// `'\0'` marks the right half of a wide character drawn in the cell before.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    ch: char,
    style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

pub struct Grid {
    cols: usize,
    cells: Vec<Cell>,
}

impl Grid {
    fn new(cols: usize, rows: usize) -> Grid {
        let blank = Cell {
            ch: ' ',
            style: Style::default(),
        };
        Grid {
            cols,
            cells: vec![blank; cols * rows],
        }
    }

    fn cell(&self, x: usize, y: usize) -> Cell {
        self.cells[y * self.cols + x]
    }

    fn fill(&mut self, r: Rect, style: Style) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                self.cells[y * self.cols + x] = Cell { ch: ' ', style };
            }
        }
    }

    /// Writes `text` from column `x`, stopping before column `end`.
    fn put(&mut self, x: usize, y: usize, text: &str, style: Style, end: usize) {
        let mut x = x;
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
    }

    /// The characters of row `y`, for tests.
    #[cfg(test)]
    fn row_text(&self, y: usize) -> String {
        (0..self.cols)
            .map(|x| self.cell(x, y).ch)
            .filter(|&c| c != '\0')
            .collect()
    }
}

/// Rows kept free above and below the digits for the title and the status.
const BAND: usize = 2;

/// Columns kept free on each side of a panel, when it is wide enough to spare
/// them. Panels next to each other always keep one, so their digits never touch.
fn side(w: usize, multi: bool) -> usize {
    match (w, multi) {
        (40.., _) => 2,
        (_, true) => 1,
        _ => 0,
    }
}

/// A size for the digits: columns and rows per font pixel, and whether the
/// title and status bands fit too.
type Scale = (usize, usize, bool);

/// Every scale worth trying for a terminal `rows` tall, best first. Square
/// pixels with room for the title beat everything; a squashed one-by-one pixel
/// is the last resort before dropping the title, and then before plain text.
fn scales(rows: usize) -> Vec<Scale> {
    let mut v = Vec::new();
    for bands in [true, false] {
        for s in (1..=rows / font::ROWS).rev() {
            v.push((2 * s, s, bands));
        }
        v.push((1, 1, bands));
    }
    v
}

fn fits(face: &str, r: Rect, (sx, sy, bands): Scale, multi: bool) -> bool {
    let band_rows = if bands { 2 * BAND } else { 0 };
    let inner = r.w.saturating_sub(2 * side(r.w, multi));
    font::pixel_width(face) * sx <= inner && font::ROWS * sy + band_rows <= r.h
}

/// Splits the screen into `n` panels, `per_row` to a row. A short last row
/// is centered.
fn panel_rects(n: usize, per_row: usize, cols: usize, rows: usize) -> Vec<Rect> {
    let grid_rows = n.div_ceil(per_row);
    let w = cols / per_row;
    (0..n)
        .map(|i| {
            let (row, col) = (i / per_row, i % per_row);
            let in_row = per_row.min(n - row * per_row);
            let y = row * rows / grid_rows;
            Rect {
                x: (cols - in_row * w) / 2 + col * w,
                y,
                w,
                h: (row + 1) * rows / grid_rows - y,
            }
        })
        .collect()
}

/// Picks how many panels go in a row and one scale shared by all of them, so
/// the digits are as big as they can be while every clock fits. `None` means
/// the screen is too small for big digits at all.
fn plan(panels: &[Panel], cols: usize, rows: usize) -> (Vec<Rect>, Option<Scale>) {
    let n = panels.len();
    let options = scales(rows);
    let mut best: Option<(usize, usize)> = None;
    for per_row in 1..=n {
        let rects = panel_rects(n, per_row, cols, rows);
        let fit = options.iter().position(|&s| {
            panels
                .iter()
                .zip(&rects)
                .all(|(p, &r)| fits(&p.face, r, s, n > 1))
        });
        // On a tie the layout with more panels to a row wins: side by side
        // suits a wide terminal.
        if let Some(rank) = fit {
            if best.is_none_or(|(b, _)| rank <= b) {
                best = Some((rank, per_row));
            }
        }
    }
    match best {
        Some((rank, per_row)) => (panel_rects(n, per_row, cols, rows), Some(options[rank])),
        None => {
            // Plain text needs up to three rows a panel.
            let per_row = (1..=n).find(|&c| n.div_ceil(c) * 3 <= rows).unwrap_or(n);
            (panel_rects(n, per_row, cols, rows), None)
        }
    }
}

/// Lays out every panel on a `cols` x `rows` grid.
pub fn compose(panels: &[Panel], cols: usize, rows: usize, color: bool) -> Grid {
    let mut grid = Grid::new(cols, rows);
    if panels.is_empty() || cols == 0 || rows == 0 {
        return grid;
    }
    let (rects, scale) = plan(panels, cols, rows);
    for (p, r) in panels.iter().zip(rects) {
        draw_panel(&mut grid, p, r, scale, color);
    }
    grid
}

fn draw_panel(grid: &mut Grid, p: &Panel, r: Rect, scale: Option<Scale>, color: bool) {
    grid.fill(r, base_style(p, color));
    if r.w == 0 || r.h == 0 {
        return;
    }
    let title = p
        .title
        .as_deref()
        .map(|t| truncate(t, r.w))
        .unwrap_or_default();
    let status = truncate(p.status, r.w);

    let mut block: Vec<Option<(Part, String)>> = Vec::new();
    match scale {
        Some((sx, sy, bands)) => {
            let face = font::render(&p.face, sx, sy).into_iter();
            // Both bands are always there, so the digits stay put when a
            // status appears or a title is missing.
            if bands {
                block.push(Some((Part::Title, title)));
                block.push(None);
            }
            block.extend(face.map(|l| Some((Part::Face, l))));
            if bands {
                block.push(None);
                block.push(Some((Part::Status, status)));
            }
        }
        None => {
            // Too small for big digits: plain text, the time first.
            if !title.is_empty() && r.h >= 3 {
                block.push(Some((Part::Title, title)));
            }
            block.push(Some((Part::Face, truncate(&p.face, r.w))));
            if !status.is_empty() && r.h > block.len() {
                block.push(Some((Part::Status, status)));
            }
        }
    }
    block.truncate(r.h);

    let top = r.y + (r.h - block.len()) / 2;
    for (i, line) in block.into_iter().enumerate() {
        if let Some((part, text)) = line {
            let x = r.x + (r.w - text.width().min(r.w)) / 2;
            grid.put(x, top + i, &text, part_style(p, part, color), r.x + r.w);
        }
    }
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

/// The style of a whole panel: plain, or inverted while it flashes.
fn base_style(p: &Panel, color: bool) -> Style {
    match (p.flash, color) {
        (false, _) => Style::default(),
        (true, true) => Style {
            fg: Some(Color::Black),
            bg: Some(Color::Red),
            ..Style::default()
        },
        (true, false) => Style {
            reverse: true,
            ..Style::default()
        },
    }
}

fn part_style(p: &Panel, part: Part, color: bool) -> Style {
    let mut s = base_style(p, color);
    if part != Part::Face {
        s.bold = true;
    }
    if p.flash || part == Part::Title {
        return s;
    }
    match (p.tone, color) {
        (Tone::Normal, _) => {}
        (Tone::Paused, true) => s.fg = Some(Color::DarkGrey),
        (Tone::Warn, true) => s.fg = Some(Color::Red),
        (Tone::Paused, false) => s.dim = true,
        (Tone::Warn, false) => s.bold = true,
    }
    s
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
    Ok(())
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

    pub fn draw(&mut self, panels: &[Panel], (cols, rows): (u16, u16)) -> io::Result<()> {
        let (cols, rows) = (cols as usize, rows as usize);
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        let grid = compose(panels, cols, rows, self.color);
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

    fn panel(face: &str, title: Option<&str>) -> Panel {
        Panel {
            title: title.map(str::to_string),
            face: face.to_string(),
            tone: Tone::Normal,
            status: "",
            flash: false,
        }
    }

    /// Rows holding digit pixels, between columns `x0` and `x1`.
    fn ink_rows(g: &Grid, x0: usize, x1: usize) -> Vec<usize> {
        (0..g.cells.len() / g.cols)
            .filter(|&y| (x0..x1).any(|x| g.cell(x, y).ch == '█'))
            .collect()
    }

    fn ink_cols(g: &Grid) -> Vec<usize> {
        (0..g.cols)
            .filter(|&x| (0..g.cells.len() / g.cols).any(|y| g.cell(x, y).ch == '█'))
            .collect()
    }

    fn find(g: &Grid, text: &str) -> Option<(usize, usize)> {
        (0..g.cells.len() / g.cols).find_map(|y| g.row_text(y).find(text).map(|x| (x, y)))
    }

    #[test]
    fn single_clock_is_centered_at_the_old_size() {
        let g = compose(&[panel("25:00", None)], 80, 24, true);
        let rows = ink_rows(&g, 0, 80);
        // 76 usable columns fit a scale of 2: 10 rows of digits.
        assert_eq!(rows.len(), 10);
        let (above, below) = (rows[0], 24 - 1 - rows[rows.len() - 1]);
        assert!(above.abs_diff(below) <= 1);
        let cols = ink_cols(&g);
        let (left, right) = (cols[0], 80 - 1 - cols[cols.len() - 1]);
        assert!(left.abs_diff(right) <= 1);
    }

    #[test]
    fn a_single_clock_keeps_square_pixels_in_a_narrow_pane() {
        // 17 font pixels two columns wide fill 34 columns exactly.
        let g = compose(&[panel("25:00", None)], 34, 9, true);
        let c = ink_cols(&g);
        assert_eq!((c[0], c[c.len() - 1]), (0, 33));
    }

    #[test]
    fn title_sits_above_digits_and_status_below() {
        let mut p = panel("25:00", Some("Tea"));
        p.status = "PAUSED";
        let g = compose(&[p], 80, 24, true);
        let rows = ink_rows(&g, 0, 80);
        assert_eq!(find(&g, "Tea").unwrap().1, rows[0] - 2);
        assert_eq!(find(&g, "PAUSED").unwrap().1, rows[rows.len() - 1] + 2);
    }

    #[test]
    fn digits_do_not_move_when_status_changes() {
        let mut p = panel("25:00", None);
        let before = ink_rows(&compose(&[p.clone()], 80, 24, true), 0, 80);
        p.status = "PAUSED";
        assert_eq!(ink_rows(&compose(&[p], 80, 24, true), 0, 80), before);
    }

    #[test]
    fn tiny_terminal_falls_back_to_plain_text() {
        let g = compose(&[panel("25:00", Some("Tea"))], 12, 3, true);
        assert!(find(&g, "25:00").is_some());
        assert!(ink_cols(&g).is_empty());
        // Too small for anything: nothing panics and the time shows cut down.
        let g = compose(&[panel("25:00", Some("Tea"))], 3, 1, true);
        assert_eq!(g.row_text(0), "25:");
    }

    #[test]
    fn two_clocks_sit_side_by_side_on_a_wide_screen() {
        let panels = [
            panel("04:00", Some("1  Tea")),
            panel("10:00", Some("2  Pasta")),
        ];
        let g = compose(&panels, 100, 26, true);
        let (tx, ty) = find(&g, "1  Tea").unwrap();
        let (px, py) = find(&g, "2  Pasta").unwrap();
        assert_eq!(ty, py);
        assert!(tx < 50 && px >= 50);
        // Each half holds its own digits, at the same rows.
        assert_eq!(ink_rows(&g, 0, 50), ink_rows(&g, 50, 100));
        assert!(!ink_rows(&g, 0, 50).is_empty());
    }

    #[test]
    fn clocks_stack_on_a_tall_screen() {
        let panels = [panel("04:00", Some("A")), panel("10:00", Some("B"))];
        let g = compose(&panels, 40, 40, true);
        let (_, ay) = find(&g, "A").unwrap();
        let (_, by) = find(&g, "B").unwrap();
        assert!(ay < 20 && by >= 20);
    }

    #[test]
    fn a_short_last_row_is_centered() {
        let r = panel_rects(3, 2, 100, 30);
        assert_eq!(
            r[0],
            Rect {
                x: 0,
                y: 0,
                w: 50,
                h: 15
            }
        );
        assert_eq!(
            r[1],
            Rect {
                x: 50,
                y: 0,
                w: 50,
                h: 15
            }
        );
        assert_eq!(
            r[2],
            Rect {
                x: 25,
                y: 15,
                w: 50,
                h: 15
            }
        );
    }

    #[test]
    fn panels_share_one_scale() {
        // A stopwatch face is wider than a countdown; both get the same pixels.
        let panels = [panel("04:00", None), panel("00:00.0", None)];
        let g = compose(&panels, 120, 20, true);
        let left = ink_rows(&g, 0, 60);
        assert_eq!(left, ink_rows(&g, 60, 120));
    }

    #[test]
    fn only_the_finished_panel_flashes() {
        let mut done = panel("00:00", Some("1"));
        done.flash = true;
        let g = compose(&[done, panel("09:59", Some("2"))], 100, 26, true);
        assert_eq!(g.cell(0, 0).style.bg, Some(Color::Red));
        assert_eq!(g.cell(49, 25).style.bg, Some(Color::Red));
        assert_eq!(g.cell(50, 0).style.bg, None);
        assert_eq!(g.cell(99, 25).style.bg, None);
    }

    #[test]
    fn nine_clocks_fit_somewhere() {
        let panels: Vec<Panel> = (1..=9)
            .map(|i| panel("25:00", Some(&i.to_string())))
            .collect();
        for (cols, rows) in [(200, 60), (100, 26), (40, 12), (10, 3)] {
            let g = compose(&panels, cols, rows, true);
            assert_eq!(g.cells.len(), cols * rows);
        }
        let g = compose(&panels, 200, 60, true);
        assert!(!ink_cols(&g).is_empty());
    }

    #[test]
    fn wide_characters_take_two_cells_and_never_spill() {
        let mut g = Grid::new(5, 1);
        g.put(0, 0, "日本語", Style::default(), 5);
        assert_eq!(g.row_text(0), "日本 ");
        assert_eq!(truncate("日本語", 5), "日本");
        assert_eq!(truncate("hello", 3), "hel");
    }
}
