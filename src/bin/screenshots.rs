//! Deterministic screenshot generator for the README and visual tests.
//!
//! Renders real application states through the headless test backend:
//!
//! - Compact SVG frames (legacy visual-test artifacts) into `docs/screenshots/`.
//! - Native 1920x1080 PNG frames into `docs/screenshots/png/`. The buffer is
//!   rendered to a 240x60 grid SVG with an 8x18 cell (exactly 1920x1080) and
//!   rasterized with `rsvg-convert`; every output is validated against the
//!   required dimensions and any failure exits nonzero.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use tui_explorer::app::action::Action;
use tui_explorer::app::state::AppState;
use tui_explorer::preview::PreviewLoaded;
use tui_explorer::sidebar::MountInfo;
use tui_explorer::testing::builders::{FIXED_TIME, demo_fs, demo_fs_showcase, demo_state};
use tui_explorer::testing::svg::{SvgStyle, buffer_to_svg, buffer_to_svg_styled};
use tui_explorer::testing::{SyncHandler, drive};
use tui_explorer::ui;

/// Native README raster geometry: 240x60 cells at 8x18 px = 1920x1080.
const PNG_COLS: u16 = 240;
const PNG_ROWS: u16 = 60;
const PNG_WIDTH: u32 = 1920;
const PNG_HEIGHT: u32 = 1080;
const PNG_STYLE: SvgStyle = SvgStyle {
    cell_w: 8,
    cell_h: 18,
    font_size: 16,
};

#[derive(Debug)]
struct ShotError(String);

impl std::fmt::Display for ShotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ShotError {}

fn fail(message: impl Into<String>) -> ShotError {
    ShotError(message.into())
}

type ShotResult<T> = Result<T, ShotError>;

fn render_state(state: &mut AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    buffer_to_svg(terminal.backend().buffer())
}

fn render_buffer(state: &mut AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| ui::render(frame, state))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn base_state(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs());
    handler
        .tags
        .tag_paths(
            &[
                PathBuf::from("/home/demo/src"),
                PathBuf::from("/home/demo/main.rs"),
            ],
            "src",
            FIXED_TIME,
        )
        .expect("tag src");
    handler
        .tags
        .tag_paths(&[PathBuf::from("/home/demo/main.rs")], "fav", FIXED_TIME)
        .expect("tag fav");
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

/// Rich, fully populated state for the 1920x1080 README rasters: a larger
/// demo filesystem, several tags, mounts, and bookmarks so the frame is full.
fn showcase_state(width: u16, height: u16) -> (AppState, SyncHandler) {
    let mut state = demo_state(width, height);
    let mut handler = SyncHandler::new(demo_fs_showcase());
    let root = PathBuf::from("/home/demo");
    let tag = |handler: &mut SyncHandler, names: &[&str], tag: &str| {
        let paths: Vec<PathBuf> = names.iter().map(|n| root.join(n)).collect();
        handler
            .tags
            .tag_paths(&paths, tag, FIXED_TIME)
            .expect("tag showcase entry");
    };
    tag(
        &mut handler,
        &[
            "src",
            "main.rs",
            "lib.rs",
            "reduce.rs",
            "state.rs",
            "keymap.rs",
            "crypto.rs",
            "config.rs",
        ],
        "src",
    );
    tag(&mut handler, &["main.rs", "README.md"], "fav");
    tag(
        &mut handler,
        &["media", "photo.png", "banner.jpg", "wallpaper.webp"],
        "media",
    );
    tag(
        &mut handler,
        &["quarterly figures 2023.csv", "report.pdf"],
        "work",
    );
    state.mounts = vec![
        MountInfo {
            path: PathBuf::from("/"),
            fs: "ext4".to_string(),
            used: 180 * 1024 * 1024 * 1024,
            total: 512 * 1024 * 1024 * 1024,
        },
        MountInfo {
            path: PathBuf::from("/mnt/backup"),
            fs: "btrfs".to_string(),
            used: 600 * 1024 * 1024 * 1024,
            total: 2 * 1024 * 1024 * 1024 * 1024,
        },
    ];
    state.bookmarks = vec![
        root.join("src"),
        root.join("media"),
        PathBuf::from("/mnt/backup"),
    ];
    drive(&mut state, &mut handler, [Action::LoadInitial]);
    (state, handler)
}

/// Move the focus until `name` is focused (bounded so a missing entry fails
/// loudly instead of looping forever).
fn focus_entry(state: &mut AppState, handler: &mut SyncHandler, name: &str) -> ShotResult<()> {
    for _ in 0..200 {
        let focused = state
            .browser
            .focused()
            .map(|view| view.entry.name.to_string_lossy().into_owned());
        if focused.as_deref() == Some(name) {
            return Ok(());
        }
        drive(state, handler, [Action::MoveDown]);
    }
    Err(fail(format!("could not focus demo entry {name:?}")))
}

/// Demo text preview content injected through the real `PreviewLoaded`
/// reducer path (the demo filesystem is in-memory, so the on-disk loader
/// cannot read it; the state transition and rendering are the real ones).
fn demo_text_lines() -> Vec<String> {
    [
        "use std::path::PathBuf;",
        "",
        "use tui_explorer::app::state::AppState;",
        "use tui_explorer::terminal::TerminalGuard;",
        "",
        "fn main() -> std::io::Result<()> {",
        "    let start = std::env::args()",
        "        .nth(1)",
        "        .map(PathBuf::from)",
        "        .unwrap_or(std::env::current_dir()?);",
        "    let _guard = TerminalGuard::enter()?;",
        "    let mut state = AppState::new(start.clone(), start);",
        "    tui_explorer::app::run(&mut state)",
        "}",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Inject a text preview for the currently focused entry.
fn inject_text_preview(state: &mut AppState, handler: &mut SyncHandler) -> ShotResult<()> {
    let key = state
        .focused_preview_key()
        .ok_or_else(|| fail("no focused entry for preview shot"))?;
    drive(
        state,
        handler,
        [Action::PreviewLoaded {
            key,
            result: PreviewLoaded::Text {
                lines: demo_text_lines(),
                truncated: false,
            },
        }],
    );
    Ok(())
}

/// Deterministic synthetic demo image (a sunset gradient with a sun disc and
/// ridge lines) rendered through the real `PreviewLoaded::Image` pipeline.
fn demo_image() -> image::DynamicImage {
    let (w, h) = (640u32, 400u32);
    let mut img = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = y as f32 / h as f32;
            let (mut r, mut g, mut b) = (
                (26.0 + 200.0 * t) as u8,
                (24.0 + 120.0 * t) as u8,
                (70.0 + 40.0 * t) as u8,
            );
            // Sun disc with a soft edge.
            let (cx, cy, rad) = (w as f32 * 0.68, h as f32 * 0.42, 64.0f32);
            let dist = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if dist < rad {
                let glow = 1.0 - dist / rad;
                r = r.saturating_add((200.0 * glow) as u8);
                g = g.saturating_add((160.0 * glow) as u8);
                b = b.saturating_add((60.0 * glow) as u8);
            }
            // Two dark ridge lines.
            let ridge1 = h as f32 * 0.62 + 30.0 * (x as f32 * 0.011).sin();
            let ridge2 = h as f32 * 0.78 + 44.0 * (x as f32 * 0.007 + 2.0).sin();
            if y as f32 > ridge2 {
                (r, g, b) = (18, 18, 34);
            } else if y as f32 > ridge1 {
                (r, g, b) = (30, 26, 52);
            }
            img.put_pixel(x, y, image::Rgb([r, g, b]));
        }
    }
    image::DynamicImage::ImageRgb8(img)
}

fn state_main(width: u16, height: u16) -> ShotResult<AppState> {
    let (mut state, mut handler) = if width >= PNG_COLS {
        showcase_state(width, height)
    } else {
        base_state(width, height)
    };
    drive(
        &mut state,
        &mut handler,
        [
            Action::ToggleSelect,
            Action::ToggleSelect,
            Action::MoveDown,
            Action::MoveDown,
            Action::MoveDown,
        ],
    );
    if width >= PNG_COLS {
        focus_entry(&mut state, &mut handler, "crypto.rs")?;
        inject_text_preview(&mut state, &mut handler)?;
        // Keep the first grid rows (folders, selected tiles) in view; the
        // focused entry sits in the second tile row and stays visible.
        state.browser.scroll = 0;
    }
    Ok(state)
}

fn state_preview(width: u16, height: u16) -> ShotResult<AppState> {
    let (mut state, mut handler) = showcase_state(width, height);
    focus_entry(&mut state, &mut handler, "photo.png")?;
    let key = state
        .focused_preview_key()
        .ok_or_else(|| fail("no focused entry for preview shot"))?;
    drive(
        &mut state,
        &mut handler,
        [Action::PreviewLoaded {
            key,
            result: PreviewLoaded::Image(demo_image()),
        }],
    );
    Ok(state)
}

fn state_tags(width: u16, height: u16) -> ShotResult<AppState> {
    let (mut state, mut handler) = if width >= PNG_COLS {
        showcase_state(width, height)
    } else {
        base_state(width, height)
    };
    drive(&mut state, &mut handler, [Action::OpenTagPicker]);
    Ok(state)
}

fn state_command(width: u16, height: u16) -> ShotResult<AppState> {
    let (mut state, mut handler) = if width >= PNG_COLS {
        showcase_state(width, height)
    } else {
        base_state(width, height)
    };
    let mut actions = vec![Action::EnterCommand];
    for c in "copy \"/mnt/backup drive\"".chars() {
        actions.push(Action::CommandChar(c));
    }
    drive(&mut state, &mut handler, actions);
    Ok(state)
}

fn state_help(width: u16, height: u16) -> ShotResult<AppState> {
    let (mut state, mut handler) = if width >= PNG_COLS {
        showcase_state(width, height)
    } else {
        base_state(width, height)
    };
    drive(&mut state, &mut handler, [Action::ToggleHelp]);
    Ok(state)
}

fn state_compact(width: u16, height: u16) -> ShotResult<AppState> {
    let (state, _handler) = base_state(width, height);
    Ok(state)
}

/// Read the IHDR of a PNG file and require exact README dimensions.
fn verify_png(path: &Path) -> ShotResult<()> {
    let bytes =
        std::fs::read(path).map_err(|e| fail(format!("cannot read {}: {e}", path.display())))?;
    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != PNG_MAGIC || &bytes[12..16] != b"IHDR" {
        return Err(fail(format!("{} is not a valid PNG", path.display())));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width != PNG_WIDTH || height != PNG_HEIGHT {
        return Err(fail(format!(
            "{} has dimensions {width}x{height}, expected {PNG_WIDTH}x{PNG_HEIGHT}",
            path.display()
        )));
    }
    Ok(())
}

/// Rasterize a state at the README geometry into an exact 1920x1080 PNG.
fn write_png(out_dir: &Path, name: &str, state: &mut AppState) -> ShotResult<()> {
    let buffer = render_buffer(state, PNG_COLS, PNG_ROWS);
    let svg = buffer_to_svg_styled(&buffer, PNG_STYLE);
    debug_assert_eq!(u32::from(PNG_COLS) * PNG_STYLE.cell_w, PNG_WIDTH);
    debug_assert_eq!(u32::from(PNG_ROWS) * PNG_STYLE.cell_h, PNG_HEIGHT);

    let svg_path = out_dir.join(format!("{name}.svg.tmp"));
    std::fs::write(&svg_path, svg)
        .map_err(|e| fail(format!("cannot write {}: {e}", svg_path.display())))?;

    let png_path = out_dir.join(format!("{name}.png"));
    let output = std::process::Command::new("rsvg-convert")
        .arg("--format")
        .arg("png")
        .arg("--output")
        .arg(&png_path)
        .arg(&svg_path)
        .output()
        .map_err(|e| fail(format!("failed to spawn rsvg-convert: {e}")))?;
    let _ = std::fs::remove_file(&svg_path);
    if !output.status.success() {
        return Err(fail(format!(
            "rsvg-convert failed for {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if !png_path.exists() {
        return Err(fail(format!(
            "rsvg-convert did not create {}",
            png_path.display()
        )));
    }
    verify_png(&png_path)?;
    println!(
        "wrote {} ({}x{})",
        png_path.display(),
        PNG_WIDTH,
        PNG_HEIGHT
    );
    Ok(())
}

type StateFn = fn(u16, u16) -> ShotResult<AppState>;

fn run() -> ShotResult<()> {
    let out_dir = Path::new("docs/screenshots");
    std::fs::create_dir_all(out_dir)
        .map_err(|e| fail(format!("cannot create {}: {e}", out_dir.display())))?;

    // Legacy compact SVG frames (kept for visual-test workflows).
    let svg_shots: &[(&str, StateFn, u16, u16)] = &[
        ("main.svg", state_main, 120, 36),
        ("tags.svg", state_tags, 120, 36),
        ("command-mode.svg", state_command, 120, 36),
        ("compact.svg", state_compact, 60, 16),
        ("help.svg", state_help, 120, 36),
    ];
    for (name, build, w, h) in svg_shots {
        let mut state = build(*w, *h)?;
        let svg = render_state(&mut state, *w, *h);
        std::fs::write(out_dir.join(name), svg)
            .map_err(|e| fail(format!("cannot write {name}: {e}")))?;
        println!("wrote docs/screenshots/{name}");
    }

    // Native 1920x1080 README rasters.
    let png_dir = out_dir.join("png");
    std::fs::create_dir_all(&png_dir)
        .map_err(|e| fail(format!("cannot create {}: {e}", png_dir.display())))?;
    let png_shots: &[(&str, StateFn)] = &[
        ("overview-main", state_main),
        ("details-preview", state_preview),
        ("tag-picker", state_tags),
        ("command-mode", state_command),
        ("help-overlay", state_help),
    ];
    for (name, build) in png_shots {
        let mut state = build(PNG_COLS, PNG_ROWS)?;
        write_png(&png_dir, name, &mut state)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("screenshots: error: {err}");
            ExitCode::FAILURE
        }
    }
}
