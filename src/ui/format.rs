use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::filesystem::EntryKind;

/// Terminal display width of `text` (wide CJK glyphs count as two cells).
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Truncates `text` to at most `max` display cells, ending in the active
/// glyph set's ellipsis when anything was cut.
pub fn truncate(text: &str, max: usize) -> String {
    if display_width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let ellipsis = crate::ui::glyphs::g().ellipsis;
    if max == 1 {
        return ellipsis.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push(ellipsis);
    out
}

/// Truncates keeping the start and the end (`long_na…me.txt`) so file
/// extensions stay visible.
pub fn truncate_middle(text: &str, max: usize) -> String {
    if display_width(text) <= max || max < 6 {
        return truncate(text, max);
    }
    let ellipsis = crate::ui::glyphs::g().ellipsis;
    let tail_budget = (max - 1) / 3;
    let head_budget = max - 1 - tail_budget;
    let mut head = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > head_budget {
            break;
        }
        head.push(c);
        used += w;
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used_tail = 0usize;
    for c in text.chars().rev() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used_tail + w > tail_budget {
            break;
        }
        tail.push(c);
        used_tail += w;
    }
    tail.reverse();
    format!("{head}{ellipsis}{}", tail.into_iter().collect::<String>())
}

/// Pads (or truncates) `text` to exactly `width` display cells.
pub fn pad_right(text: &str, width: usize) -> String {
    let count = display_width(text);
    if count >= width {
        let cut = truncate(text, width);
        let cut_w = display_width(&cut);
        return format!("{cut}{}", " ".repeat(width.saturating_sub(cut_w)));
    }
    let mut out = text.to_string();
    out.push_str(&" ".repeat(width - count));
    out
}

/// Right-aligns `text` within `width` display cells.
pub fn pad_left(text: &str, width: usize) -> String {
    let count = display_width(text);
    if count >= width {
        return truncate(text, width);
    }
    format!("{}{text}", " ".repeat(width - count))
}

/// Centers `text` within `width` display cells.
pub fn center(text: &str, width: usize) -> String {
    let text = truncate(text, width);
    let count = display_width(&text);
    let left = (width - count) / 2;
    format!(
        "{}{text}{}",
        " ".repeat(left),
        " ".repeat(width - count - left)
    )
}

/// Human relative time ("now", "5m", "3h", "2d", "Oct 14", "2023").
pub fn relative_time(epoch: i64, now: i64) -> String {
    if epoch <= 0 {
        return "—".to_string();
    }
    let delta = now - epoch;
    if delta < 0 {
        return format_time(epoch)[..10].to_string();
    }
    match delta {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86_399 => format!("{}h ago", delta / 3600),
        86_400..=604_799 => format!("{}d ago", delta / 86_400),
        _ => {
            let (year, month, day) = civil_from_days(epoch.div_euclid(86_400));
            let (now_year, _, _) = civil_from_days(now.div_euclid(86_400));
            const MONTHS: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let name = MONTHS[(month as usize).saturating_sub(1).min(11)];
            if year == now_year {
                format!("{name} {day:02}")
            } else {
                format!("{name} {year}")
            }
        }
    }
}

/// `mm:ss`, or `h:mm:ss` past an hour.
pub fn format_clock(seconds: f64) -> String {
    let total = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    let mut value = size as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size}B")
    } else if value < 10.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}

pub fn format_mode(kind: &EntryKind, mode: u32) -> String {
    let type_char = match kind {
        EntryKind::Directory => 'd',
        EntryKind::Symlink { .. } => 'l',
        EntryKind::Socket => 's',
        EntryKind::Pipe => 'p',
        EntryKind::BlockDevice => 'b',
        EntryKind::CharDevice => 'c',
        _ => '-',
    };
    let mut out = String::with_capacity(10);
    out.push(type_char);
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    for (bit, c) in bits {
        out.push(if mode & bit != 0 { c } else { '-' });
    }
    out
}

pub fn format_time(epoch: i64) -> String {
    if epoch <= 0 {
        return "unknown".to_string();
    }
    let days = epoch.div_euclid(86400);
    let secs = epoch.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs / 3600;
    let minute = (secs % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn kind_label(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "directory",
        EntryKind::File => "file",
        EntryKind::Symlink { broken: true } => "symlink (broken)",
        EntryKind::Symlink { broken: false } => "symlink",
        EntryKind::Socket => "socket",
        EntryKind::Pipe => "pipe",
        EntryKind::BlockDevice => "block device",
        EntryKind::CharDevice => "char device",
        EntryKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_behaviour() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hi", 1), "…");
        assert_eq!(truncate("hi", 0), "");
        // Wide glyphs count two cells each.
        assert_eq!(truncate("日本語ファイル", 5), "日本…");
        assert_eq!(pad_right("日本", 6), "日本  ");
        assert_eq!(truncate_middle("a_very_long_name.txt", 12), "a_very_l…txt");
        assert_eq!(center("ab", 6), "  ab  ");
        assert_eq!(pad_left("7", 3), "  7");
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(2048), "2.0K");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0M");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0G");
    }

    #[test]
    fn modes() {
        assert_eq!(format_mode(&EntryKind::File, 0o644), "-rw-r--r--");
        assert_eq!(format_mode(&EntryKind::Directory, 0o755), "drwxr-xr-x");
        assert_eq!(
            format_mode(&EntryKind::Symlink { broken: false }, 0o777),
            "lrwxrwxrwx"
        );
    }

    #[test]
    fn relative_times() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now - 10, now), "just now");
        assert_eq!(relative_time(now - 300, now), "5m ago");
        assert_eq!(relative_time(now - 7200, now), "2h ago");
        assert_eq!(relative_time(now - 3 * 86_400, now), "3d ago");
        assert_eq!(relative_time(now - 40 * 86_400, now), "Oct 05");
        assert_eq!(relative_time(0, now), "—");
        assert_eq!(format_clock(65.0), "01:05");
        assert_eq!(format_clock(3725.0), "1:02:05");
    }

    #[test]
    fn times() {
        assert_eq!(format_time(0), "unknown");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
    }
}
