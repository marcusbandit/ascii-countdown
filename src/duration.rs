//! Parsing durations from the command line and formatting clock faces.

use std::time::Duration;

/// Longest countdown accepted: 99:59:59, the most two hour digits can show.
pub const MAX_SECS: u64 = 100 * 3600 - 1;

/// Parses `25m`, `1h30m`, `90s`, `10:00`, `1:30:00` or a bare number of seconds.
pub fn parse(input: &str) -> Result<Duration, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let secs = if s.contains(':') {
        parse_clock(s)?
    } else {
        parse_units(s)?
    };
    if secs == 0 {
        return Err(format!(
            "'{input}' is zero, give a duration longer than that"
        ));
    }
    if secs > MAX_SECS {
        return Err(format!("'{input}' is longer than the 99:59:59 maximum"));
    }
    Ok(Duration::from_secs(secs))
}

/// `MM:SS` or `H:MM:SS`.
fn parse_clock(s: &str) -> Result<u64, String> {
    let parts: Vec<&str> = s.split(':').collect();
    let nums = parts
        .iter()
        .map(|p| {
            number(p).ok_or_else(|| format!("'{s}' is not a clock time like 10:00 or 1:30:00"))
        })
        .collect::<Result<Vec<u64>, String>>()?;
    match nums[..] {
        [m, sec] if sec < 60 => Ok(m.saturating_mul(60).saturating_add(sec)),
        [h, m, sec] if m < 60 && sec < 60 => {
            Ok(h.saturating_mul(3600).saturating_add(m * 60 + sec))
        }
        [_, _] | [_, _, _] => Err(format!("'{s}' has minutes or seconds of 60 or more")),
        _ => Err(format!("'{s}' is not a clock time like 10:00 or 1:30:00")),
    }
}

/// `1h30m15s`, any subset of h, m and s in that order, or a bare number of seconds.
fn parse_units(s: &str) -> Result<u64, String> {
    if let Some(n) = number(s) {
        return Ok(n);
    }
    let bad = || format!("'{s}' is not a duration like 25m, 1h30m, 90s or 10:00");
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut last_rank = usize::MAX;
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let (rank, factor) = match c.to_ascii_lowercase() {
            'h' => (2, 3600),
            'm' => (1, 60),
            's' => (0, 1),
            _ => return Err(bad()),
        };
        // Units must come largest first and only once: 1h30m, not 30m1h or 5m5m.
        if rank >= last_rank {
            return Err(bad());
        }
        let n = number(&digits).ok_or_else(bad)?;
        total = total.saturating_add(n.saturating_mul(factor));
        digits.clear();
        last_rank = rank;
    }
    if !digits.is_empty() {
        return Err(bad());
    }
    Ok(total)
}

fn number(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// The shortest unit form of `d`, the way it would be typed: `25m`, `1h30m`, `90s`.
pub fn compact(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::new();
    if h > 0 {
        out += &format!("{h}h");
    }
    if m > 0 {
        out += &format!("{m}m");
    }
    if s > 0 || out.is_empty() {
        out += &format!("{s}s");
    }
    out
}

/// Seconds left on a countdown as shown: rounded up, so a fresh 25m timer reads
/// 25:00 and 00:00 appears only when the time is really up.
pub fn remaining_secs(total: Duration, elapsed: Duration) -> u64 {
    let left = total.saturating_sub(elapsed).as_millis() as u64;
    left.div_ceil(1000)
}

/// Clock face for a countdown. The hour field shows when the whole countdown is
/// an hour or longer, so the layout does not jump when it drops under an hour.
pub fn countdown_face(secs: u64, show_hours: bool) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if show_hours {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{:02}:{s:02}", secs / 60)
    }
}

/// Clock face for the stopwatch: `MM:SS.t`, then `H:MM:SS.t` past an hour.
pub fn stopwatch_face(elapsed: Duration) -> String {
    let tenths = elapsed.as_millis() as u64 / 100;
    let (secs, t) = (tenths / 10, tenths % 10);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{t}")
    } else {
        format!("{m:02}:{s:02}.{t}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: &str) -> u64 {
        parse(s).unwrap().as_secs()
    }

    #[test]
    fn unit_durations() {
        assert_eq!(secs("25m"), 1500);
        assert_eq!(secs("1h30m"), 5400);
        assert_eq!(secs("90s"), 90);
        assert_eq!(secs("1h2m3s"), 3723);
        assert_eq!(secs("2H"), 7200);
        assert_eq!(secs("45"), 45);
        assert_eq!(secs(" 5m "), 300);
    }

    #[test]
    fn clock_durations() {
        assert_eq!(secs("10:00"), 600);
        assert_eq!(secs("0:30"), 30);
        assert_eq!(secs("90:00"), 5400);
        assert_eq!(secs("1:30:00"), 5400);
        assert_eq!(secs("99:59:59"), MAX_SECS);
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "m",
            "5x",
            "5m5m",
            "30m1h",
            "1h30",
            "1.5h",
            "-5m",
            "10:",
            ":10",
            "1:60",
            "1:60:00",
            "1:2:3:4",
            "0",
            "0m",
            "0:00",
            "100h",
            "abc",
            "1234567890",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn remaining_rounds_up() {
        let total = Duration::from_secs(60);
        assert_eq!(remaining_secs(total, Duration::ZERO), 60);
        assert_eq!(remaining_secs(total, Duration::from_millis(1)), 60);
        assert_eq!(remaining_secs(total, Duration::from_millis(59_001)), 1);
        assert_eq!(remaining_secs(total, Duration::from_secs(60)), 0);
        assert_eq!(remaining_secs(total, Duration::from_secs(61)), 0);
    }

    #[test]
    fn compact_round_trips() {
        for (secs, text) in [
            (1500, "25m"),
            (5400, "1h30m"),
            (90, "1m30s"),
            (3723, "1h2m3s"),
            (7200, "2h"),
        ] {
            assert_eq!(compact(Duration::from_secs(secs)), text);
            assert_eq!(parse(text).unwrap().as_secs(), secs);
        }
    }

    #[test]
    fn faces() {
        assert_eq!(countdown_face(1500, false), "25:00");
        assert_eq!(countdown_face(5400, false), "90:00");
        assert_eq!(countdown_face(5400, true), "1:30:00");
        assert_eq!(countdown_face(59, true), "0:00:59");
        assert_eq!(stopwatch_face(Duration::from_millis(0)), "00:00.0");
        assert_eq!(stopwatch_face(Duration::from_millis(83_456)), "01:23.4");
        assert_eq!(stopwatch_face(Duration::from_secs(3723)), "1:02:03.0");
    }
}
