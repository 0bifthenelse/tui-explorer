use ratatui::buffer::Buffer;
use ratatui::style::Color;

const CELL_W: u32 = 8;
const CELL_H: u32 = 16;
const FONT_SIZE: u32 = 14;
const BG: &str = "#1a1b26";
const FG: &str = "#c0caf5";

fn named_color(color: Color) -> Option<String> {
    let hex = match color {
        Color::Black => "#000000",
        Color::Red => "#f7768e",
        Color::Green => "#9ece6a",
        Color::Yellow => "#e0af68",
        Color::Blue => "#7aa2f7",
        Color::Magenta => "#bb9af7",
        Color::Cyan => "#7dcfff",
        Color::Gray => "#a9b1d6",
        Color::DarkGray => "#414868",
        Color::LightRed => "#ff899d",
        Color::LightGreen => "#9fe044",
        Color::LightYellow => "#faba4a",
        Color::LightBlue => "#8db0ff",
        Color::LightMagenta => "#c7a9ff",
        Color::LightCyan => "#a4daff",
        Color::White => "#ffffff",
        _ => return None,
    };
    Some(hex.to_string())
}

pub fn color_hex(color: Color) -> Option<String> {
    if let Some(named) = named_color(color) {
        return Some(named);
    }
    match color {
        Color::Reset => None,
        Color::Indexed(n) => Some(indexed_hex(n)),
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

fn indexed_hex(n: u8) -> String {
    const BASE: [&str; 16] = [
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ];
    match n {
        0..=15 => BASE[n as usize].to_string(),
        16..=231 => {
            let idx = n - 16;
            let r = idx / 36;
            let g = (idx % 36) / 6;
            let b = idx % 6;
            let scale = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            format!("#{:02x}{:02x}{:02x}", scale(r), scale(g), scale(b))
        }
        _ => {
            let level = 8 + (n - 232) * 10;
            format!("#{level:02x}{level:02x}{level:02x}")
        }
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct Run {
    text: String,
    fg: Option<String>,
    bg: Option<String>,
}

pub fn buffer_to_svg(buffer: &Buffer) -> String {
    let area = buffer.area;
    let width_px = u32::from(area.width) * CELL_W;
    let height_px = u32::from(area.height) * CELL_H;
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width_px}\" height=\"{height_px}\" viewBox=\"0 0 {width_px} {height_px}\">\n"
    ));
    out.push_str(&format!(
        "<rect width=\"{width_px}\" height=\"{height_px}\" fill=\"{BG}\"/>\n"
    ));
    let mut backgrounds = String::new();
    let mut texts = String::new();
    for y in 0..area.height {
        let mut runs: Vec<Run> = Vec::new();
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            let symbol = cell.symbol();
            let fg = color_hex(cell.fg);
            let bg = color_hex(cell.bg);
            match runs.last_mut() {
                Some(run) if run.fg == fg && run.bg == bg => run.text.push_str(symbol),
                _ => runs.push(Run {
                    text: symbol.to_string(),
                    fg,
                    bg,
                }),
            }
        }
        let mut cursor_x = 0u32;
        let mut spans = String::new();
        for run in &runs {
            let run_width = run.text.chars().count() as u32 * CELL_W;
            if let Some(bg) = &run.bg {
                backgrounds.push_str(&format!(
                    "<rect x=\"{cursor_x}\" y=\"{}\" width=\"{run_width}\" height=\"{CELL_H}\" fill=\"{bg}\"/>\n",
                    u32::from(y) * CELL_H
                ));
            }
            if !run.text.trim().is_empty() {
                let fill = run.fg.clone().unwrap_or_else(|| FG.to_string());
                spans.push_str(&format!(
                    "<tspan fill=\"{fill}\">{}</tspan>",
                    xml_escape(&run.text)
                ));
            }
            cursor_x += run_width;
        }
        if !spans.is_empty() {
            texts.push_str(&format!(
                "<text x=\"0\" y=\"{}\" font-family=\"monospace\" font-size=\"{FONT_SIZE}\" xml:space=\"preserve\">{spans}</text>\n",
                u32::from(y) * CELL_H + FONT_SIZE - 2
            ));
        }
    }
    out.push_str(&backgrounds);
    out.push_str(&texts);
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::{Terminal, text::Line, widgets::Paragraph};

    #[test]
    fn svg_is_deterministic_and_escaped() {
        let backend = TestBackend::new(10, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(Paragraph::new(Line::from("a<&>b")), f.area());
            })
            .unwrap();
        let first = buffer_to_svg(terminal.backend().buffer());
        let second = buffer_to_svg(terminal.backend().buffer());
        assert_eq!(first, second);
        assert!(first.contains("a&lt;&amp;&gt;b"));
        assert!(first.starts_with("<svg"));
        assert!(first.ends_with("</svg>\n"));
        assert!(!first.contains("202"));
    }
}
