//! Glyph sets. The default set uses Unicode box drawing, block elements and
//! geometric shapes that every mainstream monospace font ships (no Nerd
//! Font required). An ASCII set keeps the interface usable on bare
//! consoles; an optional Nerd Font set swaps file badges for icon glyphs.
//!
//! The active set is per thread: the UI renders on one thread, and tests
//! (one thread each) stay isolated from each other.

use std::cell::Cell;

use ratatui::symbols::border;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Charset {
    #[default]
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IconStyle {
    /// Short colored text badges (`rs`, `img`, ...). Works everywhere.
    #[default]
    Badges,
    /// Nerd Font codepoints; requires a patched font.
    Nerd,
}

pub struct Glyphs {
    pub rounded: border::Set,
    pub plain: border::Set,
    pub thick: border::Set,
    /// Accent rail drawn at the left edge of the focused row.
    pub bar: &'static str,
    pub check: &'static str,
    pub dot: &'static str,
    pub sep: &'static str,
    pub ellipsis: char,
    pub arrow_up: &'static str,
    pub arrow_down: &'static str,
    pub arrow_right: &'static str,
    pub home: &'static str,
    pub h_line: &'static str,
    pub h_heavy: &'static str,
    pub v_line: &'static str,
    pub thumb: &'static str,
    pub play: &'static str,
    pub pause: &'static str,
    pub stop: &'static str,
    pub prev: &'static str,
    pub next: &'static str,
    pub shuffle: &'static str,
    pub repeat: &'static str,
    pub volume: &'static str,
    pub mute: &'static str,
    pub subtitles: &'static str,
    pub fullscreen: &'static str,
    pub close: &'static str,
    pub search: &'static str,
    pub link: &'static str,
    pub star: &'static str,
    pub tag: &'static str,
    pub info: &'static str,
    pub warn: &'static str,
    /// Partial cells for horizontal meters, index 0 (empty) ..= 8 (full).
    pub eighths_h: [&'static str; 9],
    /// Partial cells for vertical bars, index 0 (empty) ..= 8 (full).
    pub eighths_v: [&'static str; 9],
    pub spinner: &'static [&'static str],
    pub pill_left: &'static str,
    pub pill_right: &'static str,
    pub scroll_thumb: &'static str,
    pub scroll_track: &'static str,
}

pub static UNICODE: Glyphs = Glyphs {
    rounded: border::ROUNDED,
    plain: border::PLAIN,
    thick: border::THICK,
    bar: "▌",
    check: "✓",
    dot: "•",
    sep: "›",
    ellipsis: '…',
    arrow_up: "▲",
    arrow_down: "▼",
    arrow_right: "▸",
    home: "⌂",
    h_line: "─",
    h_heavy: "━",
    v_line: "│",
    thumb: "●",
    play: "▶",
    pause: "‖",
    stop: "■",
    prev: "◀◀",
    next: "▶▶",
    shuffle: "⤨",
    repeat: "⟳",
    volume: "◢",
    mute: "◣",
    subtitles: "≡",
    fullscreen: "⛶",
    close: "✕",
    search: "⌕",
    link: "↗",
    star: "★",
    tag: "#",
    info: "●",
    warn: "▲",
    eighths_h: [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"],
    eighths_v: [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"],
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    pill_left: "▐",
    pill_right: "▌",
    scroll_thumb: "┃",
    scroll_track: "│",
};

const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

pub static ASCII: Glyphs = Glyphs {
    rounded: ASCII_BORDER,
    plain: ASCII_BORDER,
    thick: ASCII_BORDER,
    bar: "|",
    check: "*",
    dot: "*",
    sep: ">",
    ellipsis: '~',
    arrow_up: "^",
    arrow_down: "v",
    arrow_right: ">",
    home: "~",
    h_line: "-",
    h_heavy: "=",
    v_line: "|",
    thumb: "O",
    play: ">",
    pause: "||",
    stop: "#",
    prev: "<<",
    next: ">>",
    shuffle: "x",
    repeat: "@",
    volume: "v",
    mute: "m",
    subtitles: "cc",
    fullscreen: "[]",
    close: "x",
    search: "/",
    link: "@",
    star: "*",
    tag: "#",
    info: "i",
    warn: "!",
    eighths_h: [" ", " ", " ", "-", "-", "=", "=", "#", "#"],
    eighths_v: [" ", ".", ".", "-", "-", "=", "=", "#", "#"],
    spinner: &["|", "/", "-", "\\"],
    pill_left: " ",
    pill_right: " ",
    scroll_thumb: "#",
    scroll_track: "|",
};

thread_local! {
    static CHARSET: Cell<Charset> = const { Cell::new(Charset::Unicode) };
    static ICONS: Cell<IconStyle> = const { Cell::new(IconStyle::Badges) };
}

/// Selects the glyph set for this thread's renders.
pub fn set_charset(charset: Charset) {
    CHARSET.with(|c| c.set(charset));
}

pub fn charset() -> Charset {
    CHARSET.with(Cell::get)
}

pub fn set_icon_style(style: IconStyle) {
    ICONS.with(|c| c.set(style));
}

pub fn icon_style() -> IconStyle {
    if charset() == Charset::Ascii {
        return IconStyle::Badges;
    }
    ICONS.with(Cell::get)
}

/// The active glyph set.
pub fn g() -> &'static Glyphs {
    match charset() {
        Charset::Unicode => &UNICODE,
        Charset::Ascii => &ASCII,
    }
}

/// A horizontal meter of `width` cells filled to `ratio` with sub-cell
/// precision (eighths). Returns (filled glyphs, remaining width).
pub fn meter(width: usize, ratio: f64) -> (String, usize) {
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let eighths = (ratio * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let part = eighths % 8;
    let glyphs = g();
    let mut out = glyphs.eighths_h[8].repeat(full.min(width));
    let mut used = full.min(width);
    if part > 0 && used < width {
        out.push_str(glyphs.eighths_h[part]);
        used += 1;
    }
    (out, width - used)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_uses_eighths() {
        set_charset(Charset::Unicode);
        let (filled, rest) = meter(4, 0.5);
        assert_eq!(filled, "██");
        assert_eq!(rest, 2);
        let (filled, rest) = meter(4, 0.5 + 1.0 / 8.0);
        assert_eq!(filled, "██▌");
        assert_eq!(rest, 1);
        let (filled, rest) = meter(3, 2.0);
        assert_eq!(filled, "███");
        assert_eq!(rest, 0);
    }

    #[test]
    fn charset_is_thread_local() {
        set_charset(Charset::Ascii);
        assert_eq!(g().bar, "|");
        std::thread::spawn(|| assert_eq!(g().bar, "▌"))
            .join()
            .unwrap();
        set_charset(Charset::Unicode);
    }
}
