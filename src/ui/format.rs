use crate::filesystem::EntryKind;

pub fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max == 1 {
        return "~".to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('~');
    out
}

pub fn pad_right(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count >= width {
        return truncate(text, width);
    }
    let mut out = text.to_string();
    out.push_str(&" ".repeat(width - count));
    out
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
        assert_eq!(truncate("hello world", 8), "hello w~");
        assert_eq!(truncate("hi", 1), "~");
        assert_eq!(truncate("hi", 0), "");
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
    fn times() {
        assert_eq!(format_time(0), "unknown");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
    }
}
