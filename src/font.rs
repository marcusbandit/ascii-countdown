//! A 3x5 pixel font for the clock face, scaled up to fill the terminal.

/// Height of every glyph in font pixels.
pub const ROWS: usize = 5;

/// Font pixels between two glyphs.
const GAP: usize = 1;

/// What a lit pixel is drawn with.
const INK: char = '█';

fn glyph(c: char) -> &'static [&'static str; ROWS] {
    match c {
        '0' => &["###", "# #", "# #", "# #", "###"],
        '1' => &["  #", "  #", "  #", "  #", "  #"],
        '2' => &["###", "  #", "###", "#  ", "###"],
        '3' => &["###", "  #", "###", "  #", "###"],
        '4' => &["# #", "# #", "###", "  #", "  #"],
        '5' => &["###", "#  ", "###", "  #", "###"],
        '6' => &["###", "#  ", "###", "# #", "###"],
        '7' => &["###", "  #", "  #", "  #", "  #"],
        '8' => &["###", "# #", "###", "# #", "###"],
        '9' => &["###", "# #", "###", "  #", "###"],
        ':' => &[" ", "#", " ", "#", " "],
        '.' => &[" ", " ", " ", " ", "#"],
        _ => &["   ", "   ", "   ", "   ", "   "],
    }
}

/// Width of `text` in font pixels, gaps included.
pub fn pixel_width(text: &str) -> usize {
    let glyphs: usize = text.chars().map(|c| glyph(c)[0].len()).sum();
    glyphs + GAP * text.chars().count().saturating_sub(1)
}

/// Picks the largest scale at which `text` fits in `cols` x `rows` cells.
///
/// Returns columns and rows per font pixel. Terminal cells are about twice as
/// tall as they are wide, so a pixel two columns wide and one row tall is
/// square. When even that does not fit, a squashed one-by-one pixel is tried.
pub fn fit(text: &str, cols: usize, rows: usize) -> Option<(usize, usize)> {
    let width = pixel_width(text);
    if width == 0 {
        return None;
    }
    let sy = (rows / ROWS).min(cols / (2 * width));
    if sy >= 1 {
        Some((2 * sy, sy))
    } else if width <= cols && ROWS <= rows {
        Some((1, 1))
    } else {
        None
    }
}

/// Renders `text` with each font pixel `sx` columns wide and `sy` rows tall.
pub fn render(text: &str, sx: usize, sy: usize) -> Vec<String> {
    let mut lines = Vec::with_capacity(ROWS * sy);
    for row in 0..ROWS {
        let mut line = String::new();
        for (i, c) in text.chars().enumerate() {
            if i > 0 {
                line.extend(std::iter::repeat_n(' ', GAP * sx));
            }
            for px in glyph(c)[row].chars() {
                let ch = if px == '#' { INK } else { ' ' };
                line.extend(std::iter::repeat_n(ch, sx));
            }
        }
        for _ in 0..sy {
            lines.push(line.clone());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        // 4 digits of 3, a colon of 1, 4 gaps.
        assert_eq!(pixel_width("25:00"), 17);
        assert_eq!(pixel_width("1:30:00"), 23);
        assert_eq!(pixel_width("01:23.4"), 23);
    }

    #[test]
    fn every_glyph_is_rectangular() {
        for c in "0123456789:.".chars() {
            let g = glyph(c);
            assert!(g.iter().all(|r| r.len() == g[0].len()), "{c} is ragged");
        }
    }

    #[test]
    fn fit_keeps_pixels_square_and_inside() {
        let (sx, sy) = fit("25:00", 200, 60).unwrap();
        assert_eq!(sx, 2 * sy);
        assert!(17 * sx <= 200 && 5 * sy <= 60);
        // Wider by one pixel would not fit any more.
        let bigger = sy + 1;
        assert!(17 * 2 * bigger > 200 || 5 * bigger > 60);
    }

    #[test]
    fn fit_falls_back_then_gives_up() {
        assert_eq!(fit("25:00", 34, 5), Some((2, 1)));
        assert_eq!(fit("25:00", 20, 5), Some((1, 1)));
        assert_eq!(fit("25:00", 16, 5), None);
        assert_eq!(fit("25:00", 80, 4), None);
    }

    #[test]
    fn render_scales() {
        let lines = render("1", 2, 1);
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "    ██");
        let lines = render("1:", 1, 2);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[2], "  █ █");
        assert!(lines.iter().all(|l| l.chars().count() == pixel_width("1:")));
    }
}
