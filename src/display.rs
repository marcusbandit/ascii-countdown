//! The display view: clocks in big digits, side by side, one panel each.

use unicode_width::UnicodeWidthStr;

use crate::font;
use crate::screen::{truncate, Grid, Ink, Rect, Style};

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

/// Rows kept free above and below the digits for the title and the status.
const BAND: usize = 2;

/// Columns kept free on each side of a panel, when it is wide enough to spare
/// them. Panels next to each other always keep one, so their digits never touch
/// and the selection frame has a column of its own.
fn side(w: usize, multi: bool) -> usize {
    match (w, multi) {
        (40.., _) => 2,
        (_, true) => 1,
        _ => 0,
    }
}

/// How good a layout is, smaller first: the scale's place in `scales`, the
/// gaps in the last row, and fewer panels to a row.
type Rank = (usize, usize, std::cmp::Reverse<usize>);

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
    // Several panels keep a row above and below for the selection frame.
    let frame_rows = if multi { 2 } else { 0 };
    let inner = r.w.saturating_sub(2 * side(r.w, multi));
    font::pixel_width(face) * sx <= inner && font::ROWS * sy + band_rows + frame_rows <= r.h
}

/// Splits the area into `n` panels, `per_row` to a row. A short last row is
/// centered.
fn panel_rects(n: usize, per_row: usize, area: Rect) -> Vec<Rect> {
    let grid_rows = n.div_ceil(per_row);
    let w = area.w / per_row;
    (0..n)
        .map(|i| {
            let (row, col) = (i / per_row, i % per_row);
            let in_row = per_row.min(n - row * per_row);
            let y = row * area.h / grid_rows;
            Rect {
                x: area.x + (area.w - in_row * w) / 2 + col * w,
                y: area.y + y,
                w,
                h: (row + 1) * area.h / grid_rows - y,
            }
        })
        .collect()
}

/// Picks how many panels go in a row and one scale shared by all of them, so
/// the digits are as big as they can be while every clock fits. `None` means
/// the area is too small for big digits at all.
fn plan(panels: &[Panel], area: Rect) -> (Vec<Rect>, Option<Scale>, usize) {
    let n = panels.len();
    let options = scales(area.h);
    // Best scale first; on a tie, the layout with no gaps in its last row,
    // then the one with more panels side by side, which suits a wide terminal.
    let mut best: Option<(Rank, usize)> = None;
    for per_row in 1..=n {
        let rects = panel_rects(n, per_row, area);
        let fit = options.iter().position(|&s| {
            panels
                .iter()
                .zip(&rects)
                .all(|(p, &r)| fits(&p.face, r, s, n > 1))
        });
        if let Some(rank) = fit {
            let gaps = n.div_ceil(per_row) * per_row - n;
            let key = (rank, gaps, std::cmp::Reverse(per_row));
            if best.as_ref().is_none_or(|(b, _)| key < *b) {
                best = Some((key, per_row));
            }
        }
    }
    match best {
        Some(((rank, _, _), per_row)) => {
            (panel_rects(n, per_row, area), Some(options[rank]), per_row)
        }
        None => {
            // Plain text needs up to three rows a panel.
            let per_row = (1..=n).find(|&c| n.div_ceil(c) * 3 <= area.h).unwrap_or(n);
            (panel_rects(n, per_row, area), None, per_row)
        }
    }
}

/// True when `panels` all fit in big digits on a `cols` x `rows` screen.
pub fn fits_big(panels: &[Panel], cols: usize, rows: usize) -> bool {
    let area = Rect {
        x: 0,
        y: 0,
        w: cols,
        h: rows,
    };
    plan(panels, area).1.is_some()
}

/// Draws every panel over the whole grid, with a frame around `selected`
/// when there is more than one. Returns how many panels went in a row.
pub fn compose(grid: &mut Grid, panels: &[Panel], color: bool, selected: Option<usize>) -> usize {
    let area = Rect {
        x: 0,
        y: 0,
        w: grid.cols,
        h: grid.rows,
    };
    if panels.is_empty() || area.w == 0 || area.h == 0 {
        return 1;
    }
    let (rects, scale, per_row) = plan(panels, area);
    let multi = panels.len() > 1;
    for (p, &r) in panels.iter().zip(&rects) {
        draw_panel(grid, p, r, scale, color, multi);
    }
    if let Some(i) = selected.filter(|_| multi) {
        let style = if color {
            Style::fg(Ink::Cyan).bold()
        } else {
            Style::default().bold()
        };
        if let Some(&r) = rects.get(i) {
            grid.frame(r, style);
        }
    }
    per_row
}

fn draw_panel(grid: &mut Grid, p: &Panel, r: Rect, scale: Option<Scale>, color: bool, multi: bool) {
    grid.fill(r, base_style(p, color));
    if r.w == 0 || r.h == 0 {
        return;
    }
    // Text stays clear of the column the selection frame uses.
    let text_w = r.w.saturating_sub(2 * side(r.w, multi));
    let title = p
        .title
        .as_deref()
        .map(|t| truncate(t, text_w))
        .unwrap_or_default();
    let status = truncate(p.status, text_w);

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

/// The style of a whole panel: plain, or inverted while it flashes.
fn base_style(p: &Panel, color: bool) -> Style {
    flash_style(p.flash, color)
}

pub fn flash_style(flash: bool, color: bool) -> Style {
    match (flash, color) {
        (false, _) => Style::default(),
        (true, true) => Style {
            fg: Some(Ink::Black),
            bg: Some(Ink::Red),
            ..Style::default()
        },
        (true, false) => Style {
            reverse: true,
            ..Style::default()
        },
    }
}

/// The colour a clock's time is drawn in.
pub fn tone_style(mut s: Style, tone: Tone, color: bool) -> Style {
    match (tone, color) {
        (Tone::Normal, _) => {}
        (Tone::Paused, true) => s.fg = Some(Ink::DarkGrey),
        (Tone::Warn, true) => s.fg = Some(Ink::Red),
        (Tone::Paused, false) => s.dim = true,
        (Tone::Warn, false) => s.bold = true,
    }
    s
}

fn part_style(p: &Panel, part: Part, color: bool) -> Style {
    let mut s = base_style(p, color);
    if part != Part::Face {
        s.bold = true;
    }
    if p.flash || part == Part::Title {
        return s;
    }
    tone_style(s, p.tone, color)
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

    fn draw(panels: &[Panel], cols: usize, rows: usize, selected: Option<usize>) -> Grid {
        let mut g = Grid::new(cols, rows);
        compose(&mut g, panels, true, selected);
        g
    }

    /// Rows holding digit pixels, between columns `x0` and `x1`.
    fn ink_rows(g: &Grid, x0: usize, x1: usize) -> Vec<usize> {
        (0..g.rows)
            .filter(|&y| (x0..x1).any(|x| g.cell(x, y).ch == '█'))
            .collect()
    }

    fn ink_cols(g: &Grid) -> Vec<usize> {
        (0..g.cols)
            .filter(|&x| (0..g.rows).any(|y| g.cell(x, y).ch == '█'))
            .collect()
    }

    #[test]
    fn single_clock_is_centered_at_the_old_size() {
        let g = draw(&[panel("25:00", None)], 80, 24, Some(0));
        let rows = ink_rows(&g, 0, 80);
        // 76 usable columns fit a scale of 2: 10 rows of digits.
        assert_eq!(rows.len(), 10);
        let (above, below) = (rows[0], 24 - 1 - rows[rows.len() - 1]);
        assert!(above.abs_diff(below) <= 1);
        let cols = ink_cols(&g);
        let (left, right) = (cols[0], 80 - 1 - cols[cols.len() - 1]);
        assert!(left.abs_diff(right) <= 1);
        // One clock never gets a selection frame.
        assert!(g.find("╭").is_none());
    }

    #[test]
    fn a_single_clock_keeps_square_pixels_in_a_narrow_pane() {
        // 17 font pixels two columns wide fill 34 columns exactly.
        let g = draw(&[panel("25:00", None)], 34, 9, None);
        let c = ink_cols(&g);
        assert_eq!((c[0], c[c.len() - 1]), (0, 33));
    }

    #[test]
    fn title_sits_above_digits_and_status_below() {
        let mut p = panel("25:00", Some("Tea"));
        p.status = "PAUSED";
        let g = draw(&[p], 80, 24, None);
        let rows = ink_rows(&g, 0, 80);
        assert_eq!(g.find("Tea").unwrap().1, rows[0] - 2);
        assert_eq!(g.find("PAUSED").unwrap().1, rows[rows.len() - 1] + 2);
    }

    #[test]
    fn digits_do_not_move_when_status_changes() {
        let mut p = panel("25:00", None);
        let before = ink_rows(&draw(&[p.clone()], 80, 24, None), 0, 80);
        p.status = "PAUSED";
        assert_eq!(ink_rows(&draw(&[p], 80, 24, None), 0, 80), before);
    }

    #[test]
    fn tiny_terminal_falls_back_to_plain_text() {
        let g = draw(&[panel("25:00", Some("Tea"))], 12, 3, None);
        assert!(g.find("25:00").is_some());
        assert!(ink_cols(&g).is_empty());
        let g = draw(&[panel("25:00", Some("Tea"))], 3, 1, None);
        assert_eq!(g.row_text(0), "25:");
        assert!(!fits_big(&[panel("25:00", None)], 12, 3));
        assert!(fits_big(&[panel("25:00", None)], 80, 24));
    }

    #[test]
    fn two_clocks_sit_side_by_side_on_a_wide_screen() {
        let panels = [panel("04:00", Some("Tea")), panel("10:00", Some("Pasta"))];
        let mut g = Grid::new(100, 26);
        let per_row = compose(&mut g, &panels, true, None);
        assert_eq!(per_row, 2);
        let (tx, ty) = g.find("Tea").unwrap();
        let (px, py) = g.find("Pasta").unwrap();
        assert_eq!(ty, py);
        assert!(tx < 50 && px >= 50);
        assert_eq!(ink_rows(&g, 0, 50), ink_rows(&g, 50, 100));
        assert!(!ink_rows(&g, 0, 50).is_empty());
    }

    #[test]
    fn clocks_stack_on_a_tall_screen() {
        let panels = [panel("04:00", Some("A")), panel("10:00", Some("B"))];
        let mut g = Grid::new(40, 40);
        assert_eq!(compose(&mut g, &panels, true, None), 1);
        assert!(g.find("A").unwrap().1 < 20 && g.find("B").unwrap().1 >= 20);
    }

    #[test]
    fn four_clocks_make_a_square() {
        let panels: Vec<Panel> = (0..4).map(|_| panel("25:00", None)).collect();
        let mut g = Grid::new(120, 40);
        assert_eq!(compose(&mut g, &panels, true, None), 2);
    }

    #[test]
    fn a_short_last_row_is_centered() {
        let area = Rect {
            x: 0,
            y: 0,
            w: 100,
            h: 30,
        };
        let r = panel_rects(3, 2, area);
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
        let panels = [panel("04:00", None), panel("00:00.0", None)];
        let g = draw(&panels, 120, 22, None);
        assert_eq!(ink_rows(&g, 0, 60), ink_rows(&g, 60, 120));
    }

    #[test]
    fn only_the_finished_panel_flashes() {
        let mut done = panel("00:00", None);
        done.flash = true;
        let g = draw(&[done, panel("09:59", None)], 100, 26, None);
        assert_eq!(g.cell(0, 0).style.bg, Some(Ink::Red));
        assert_eq!(g.cell(49, 25).style.bg, Some(Ink::Red));
        assert_eq!(g.cell(50, 0).style.bg, None);
        assert_eq!(g.cell(99, 25).style.bg, None);
    }

    #[test]
    fn the_selected_panel_gets_a_frame_clear_of_the_digits() {
        let panels = [panel("04:00", Some("Tea")), panel("10:00", Some("Pasta"))];
        let g = draw(&panels, 100, 26, Some(1));
        assert_eq!(g.cell(50, 0).ch, '╭');
        assert_eq!(g.cell(99, 25).ch, '╯');
        assert_eq!(g.cell(0, 0).ch, ' ');
        // The frame never covers a digit pixel.
        for y in 0..26 {
            assert_ne!(g.cell(50, y).ch, '█');
            assert_ne!(g.cell(99, y).ch, '█');
        }
        assert!(ink_rows(&g, 50, 100).iter().all(|&y| y > 0 && y < 25));
    }
}
