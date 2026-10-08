//! Color math and whole-buffer effects: interpolation for animated state
//! transitions, gradients, the modal scrim, and the 256-color fallback pass
//! for terminals without truecolor.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::icons::IconKind;
use crate::ui::palette::*;

/// RGB components of a color; non-RGB colors map to a neutral grey so
/// interpolation never panics on terminal palette colors.
pub fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        Color::Indexed(i) => indexed_to_rgb(i),
        _ => (128, 128, 128),
    }
}

/// Linear interpolation between two colors; `t` is clamped to `0..=1`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let lerp = |x: u8, y: u8| -> u8 {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
}

/// Two-stop accent gradient sampled at `t` (orange to amber).
pub fn accent_gradient(t: f32) -> Color {
    mix(ACCENT, ACCENT_AMBER, t)
}

/// Category hue for an icon kind (icons, badges, grid art).
pub fn hue_for(kind: IconKind) -> Color {
    use IconKind::*;
    match kind {
        Folder | FolderOpen | FolderHidden => HUE_DIR,
        Symlink => HUE_LINK,
        Executable | Shell => HUE_EXEC,
        SourceFile | Rust | TypeScript | JavaScript | C | Cpp | Python => HUE_CODE,
        Html | Css => HUE_WEB,
        Json | Toml | Yaml | Config | CargoToml | CargoLock | PackageJson | Lockfile | Makefile
        | Docker | Database => HUE_DATA,
        Markdown | Pdf | Text => HUE_DOC,
        WebLink => HUE_WEB,
        Subtitle => HUE_VIDEO,
        Image => HUE_IMAGE,
        Audio => HUE_AUDIO,
        Video => HUE_VIDEO,
        Archive => HUE_ARCHIVE,
        Git => HUE_SPECIAL,
        Socket | Pipe | Device => HUE_SPECIAL,
        Unknown => TEXT_SECONDARY,
    }
}

/// Blends every cell of `area` toward `toward` by `amount` (0 = untouched,
/// 1 = solid). Used for the modal scrim and drop shadows: text stays
/// legible but recedes, which reads as depth in a terminal.
pub fn dim_area(buf: &mut Buffer, area: Rect, toward: Color, amount: f32) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            let fg = if cell.fg == Color::Reset {
                TEXT_SECONDARY
            } else {
                cell.fg
            };
            let bg = if cell.bg == Color::Reset {
                SURFACE_0
            } else {
                cell.bg
            };
            cell.fg = mix(fg, toward, amount);
            cell.bg = mix(bg, toward, amount);
        }
    }
}

/// Paints a horizontal background gradient across `area` (left `from`,
/// right `to`), keeping glyphs and foreground colors.
pub fn gradient_bg(buf: &mut Buffer, area: Rect, from: Color, to: Color) {
    let area = area.intersection(buf.area);
    let width = area.width.max(1) as f32;
    for x in area.left()..area.right() {
        let t = (x - area.left()) as f32 / (width - 1.0).max(1.0);
        let color = mix(from, to, t);
        for y in area.top()..area.bottom() {
            buf[(x, y)].bg = color;
        }
    }
}

/// True when the environment advertises 24-bit color support.
pub fn truecolor_supported(get_env: &dyn Fn(&str) -> Option<String>) -> bool {
    if let Some(value) = get_env("COLORTERM") {
        let value = value.to_ascii_lowercase();
        if value.contains("truecolor") || value.contains("24bit") {
            return true;
        }
    }
    matches!(
        get_env("TERM_PROGRAM").as_deref(),
        Some("iTerm.app" | "WezTerm" | "vscode" | "ghostty")
    ) || get_env("TERM").is_some_and(|t| t.contains("kitty") || t.contains("direct"))
}

/// Maps every RGB cell color to the nearest xterm-256 palette index. Run as
/// the final pass on terminals without truecolor so gradients degrade to
/// stable steps instead of whatever the terminal guesses.
pub fn downsample_256(buf: &mut Buffer) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if let Color::Rgb(r, g, b) = cell.fg {
                cell.fg = Color::Indexed(nearest_256(r, g, b));
            }
            if let Color::Rgb(r, g, b) = cell.bg {
                cell.bg = Color::Indexed(nearest_256(r, g, b));
            }
        }
    }
}

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn cube_index(v: u8) -> usize {
    CUBE.iter()
        .enumerate()
        .min_by_key(|(_, c)| (i32::from(**c) - i32::from(v)).abs())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Nearest xterm-256 index (6x6x6 cube or the 24-step grey ramp).
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (cube_index(r), cube_index(g), cube_index(b));
    let cube = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let grey_i = ((avg.saturating_sub(8)) / 10).min(23) as u8;
    let grey_v = 8 + 10 * grey_i;
    let dist = |c: (u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(c.0, r) + d(c.1, g) + d(c.2, b)
    };
    if dist((grey_v, grey_v, grey_v)) < dist(cube) {
        232 + grey_i
    } else {
        cube_idx as u8
    }
}

fn indexed_to_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        16..=231 => {
            let i = i - 16;
            (
                CUBE[(i / 36) as usize],
                CUBE[((i / 6) % 6) as usize],
                CUBE[(i % 6) as usize],
            )
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
        _ => (128, 128, 128),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_endpoints_and_midpoint() {
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(200, 100, 50);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), Color::Rgb(100, 50, 25));
        assert_eq!(mix(a, b, f32::NAN), a);
    }

    #[test]
    fn nearest_256_hits_exact_cube_and_grey() {
        assert_eq!(nearest_256(255, 0, 0), 196);
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(128, 128, 128), 244);
    }

    #[test]
    fn dim_area_blends_toward_target() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf[(0, 0)].bg = Color::Rgb(100, 100, 100);
        buf[(0, 0)].fg = Color::Rgb(200, 200, 200);
        dim_area(&mut buf, Rect::new(0, 0, 1, 1), Color::Rgb(0, 0, 0), 0.5);
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(50, 50, 50));
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(100, 100, 100));
    }

    #[test]
    fn truecolor_detection() {
        let env = |k: &str| (k == "COLORTERM").then(|| "truecolor".to_string());
        assert!(truecolor_supported(&env));
        let none = |_: &str| None;
        assert!(!truecolor_supported(&none));
    }
}
