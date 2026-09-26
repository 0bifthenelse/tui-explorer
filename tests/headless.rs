//! Headless visual verification of the real `tui-explorer` binary.
//!
//! These tests run the compiled application inside a pseudo-terminal at the
//! four release terminal sizes (160x48, 120x36, 90x28, 70x22), feed it real
//! keystrokes, and replay the raw escape stream through a `vt100` parser to
//! inspect exactly what a user would see. No display server, window manager
//! or human interaction is required.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tui_explorer::filesystem::sandbox;

fn binary() -> PathBuf {
    // Cargo builds the binary before integration tests run.
    let mut path = std::env::current_exe().expect("test exe path");
    path.pop(); // deps/
    path.pop(); // target profile dir
    path.push("tui-explorer");
    assert!(path.exists(), "binary missing at {}", path.display());
    path
}

fn fixture(tag: &str) -> PathBuf {
    // `sandbox::fixture` removes any stale directory and recreates it.
    let dir = sandbox::fixture(&format!("headless-{tag}"));
    std::fs::create_dir_all(dir.join("src/nested")).unwrap();
    std::fs::create_dir_all(dir.join("empty-dir")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        b"fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/nested/deep.bin"), vec![7u8; 4096]).unwrap();
    std::fs::write(dir.join("notes.txt"), b"hello world\nsecond line\n").unwrap();
    std::fs::write(dir.join("binary.dat"), [0u8, 159, 146, 150, 1]).unwrap();
    std::fs::write(dir.join(".hidden"), b"secret\n").unwrap();
    std::fs::write(
        dir.join("a very long file name that keeps going and going.txt"),
        b"long\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("unicode-\u{00e9}\u{00e8}\u{00ea}.txt"),
        b"unicode\n",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("notes.txt", dir.join("link-ok")).unwrap();
    // Real images in every supported format, plus a corrupt one.
    let rgb = image::RgbImage::from_pixel(8, 6, image::Rgb([200, 30, 30]));
    let dyn_img = image::DynamicImage::ImageRgb8(rgb);
    dyn_img.save(dir.join("photo.png")).unwrap();
    dyn_img
        .save_with_format(dir.join("photo.jpg"), image::ImageFormat::Jpeg)
        .unwrap();
    dyn_img
        .save_with_format(dir.join("anim.gif"), image::ImageFormat::Gif)
        .unwrap();
    dyn_img
        .save_with_format(dir.join("pic.webp"), image::ImageFormat::WebP)
        .unwrap();
    dyn_img
        .save_with_format(dir.join("pic.bmp"), image::ImageFormat::Bmp)
        .unwrap();
    std::fs::write(dir.join("corrupt.png"), b"not really a png").unwrap();
    dir
}

fn preview_fixture(tag: &str, target_name: &str, bytes: &[u8]) -> PathBuf {
    let dir = sandbox::fixture(&format!("headless-preview-{tag}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("00-start.txt"),
        b"safe text preview\nsecond line\n",
    )
    .unwrap();
    std::fs::write(dir.join(target_name), bytes).unwrap();
    dir
}

/// Blocks until `needle` appears in the pty log or `budget_ms` elapses.
/// Used instead of fixed sleeps so a loaded machine slows the test rather
/// than tearing the captured frame.
fn wait_for(log: &Path, needle: &[u8], budget_ms: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(budget_ms);
    while std::time::Instant::now() < deadline {
        let seen = std::fs::read(log)
            .ok()
            .is_some_and(|raw| raw.windows(needle.len()).any(|w| w == needle));
        if seen {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Blocks until the pty log has stopped growing for `quiet_ms`, or the
/// budget expires. A pty log that is still growing means a frame is still
/// being written, so reading it then would capture a torn screen.
fn wait_until_quiet(log: &Path, quiet_ms: u64, budget_ms: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(budget_ms);
    let mut last = log.metadata().map(|m| m.len()).unwrap_or(0);
    let mut stable_since = std::time::Instant::now();
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
        let now = log.metadata().map(|m| m.len()).unwrap_or(0);
        if now != last {
            last = now;
            stable_since = std::time::Instant::now();
            continue;
        }
        if stable_since.elapsed() >= std::time::Duration::from_millis(quiet_ms) {
            return;
        }
    }
}

/// Run the real binary in a pty of `cols`x`rows`, send `keys` after warm-up,
/// and return the final screen contents as seen by a vt100 terminal.
fn run_in_pty(cols: u16, rows: u16, dir: &Path, keys: &[&str], settle_ms: u64) -> String {
    let root = sandbox::default_root();
    sandbox::ensure(&root).expect("sandbox root");
    sandbox::ensure(&root.join("xdg/logs")).expect("sandbox log dir");
    // Concurrency-safe name: two tests can use the same geometry at the same
    // time, so the log is keyed by the (unique) fixture tag.
    let tag = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{cols}x{rows}"));
    let log = root.join(format!("xdg/logs/pty-{tag}.log"));
    // Every XDG base is redirected into the sandbox so the pty run cannot
    // read or write the real user directories. Each run gets its own XDG
    // subtree: tests run in parallel and must not contend on one SQLite tag
    // database.
    let (data, config, cache) = (
        root.join("xdg/data").join(&tag),
        root.join("xdg/config").join(&tag),
        root.join("xdg/cache").join(&tag),
    );
    for slot in [&data, &config, &cache] {
        sandbox::ensure(slot).expect("sandbox xdg dir");
    }
    let mut child = Command::new("script")
        .args([
            "-qfec",
            &format!(
                "stty cols {cols} rows {rows}; exec {} {}",
                binary().display(),
                dir.display()
            ),
            log.to_str().expect("log path utf8"),
        ])
        .env("TERM", "xterm-256color") // no graphics: exercises the fallback
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_CACHE_HOME", &cache)
        .env(sandbox::OVERRIDE_ENV, &root) // confine destructive operations
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn script pty");
    let mut stdin = child.stdin.take().expect("pty stdin");
    // The binary queries terminal capabilities at startup (ending with a
    // device status report request, `[5n`). Answer it exactly once so
    // detection completes deterministically instead of timing out.
    for _ in 0..500 {
        let query_seen = std::fs::read(&log)
            .ok()
            .is_some_and(|raw| raw.windows(3).any(|window| window == b"[5n"));
        if query_seen {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    stdin
        .write_all(b"\x1b[0n")
        .expect("terminal query response");
    stdin.flush().expect("flush terminal query response");
    // Deterministic readiness: wait until the first frame is on the log
    // rather than sleeping a fixed amount, so parallel tests cannot starve
    // each other into a torn capture.
    // Landmark drawn only by the application's first frame; the pty log's
    // own `script` header also contains the binary path, so it cannot be used.
    wait_for(&log, b"Press ? for help", 20_000);
    // A keystroke is only meaningful once the app has drawn its shell, so
    // wait for the ready landmark rather than for raw escape bytes.
    for key in keys {
        stdin.write_all(key.as_bytes()).expect("write key");
        stdin.flush().expect("flush key");
        // Each key must be fully processed before the next one lands, so the
        // redraw it caused has to settle first.
        wait_until_quiet(&log, 120, 15_000);
    }
    wait_until_quiet(&log, settle_ms.min(400) as u64, 15_000);
    // One final full repaint makes the captured frame complete rather than
    // whatever partial region the last interaction happened to touch.
    stdin.write_all(b"\x0c").ok(); // Ctrl-L
    stdin.flush().ok();
    // Ctrl-L repaints everything, but the preview worker may still deliver;
    // wait for the log to stop changing before reading it.
    wait_until_quiet(&log, 250, 15_000);
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let raw = std::fs::read(&log).expect("pty log");
    let _ = std::fs::remove_file(&log);
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(&raw);
    parser.screen().contents()
}

fn assert_layout_landmarks(screen: &str, cols: u16, context: &str) {
    assert!(screen.contains("tui-explorer"), "{context}: header missing");
    assert!(screen.contains("Path:"), "{context}: path bar missing");
    assert!(screen.contains("Press ? for help"), "{context}: help hint");
    assert!(
        screen.contains("Sort: name (asc)"),
        "{context}: grid header"
    );
    assert!(screen.contains("Open"), "{context}: legend open action");
    if cols < 100 {
        assert!(screen.contains("TIP"), "{context}: compact tip line");
    }
    if cols >= 100 {
        assert!(screen.contains("PLACES"), "{context}: sidebar at {cols}");
    }
}

#[test]
fn headless_all_target_sizes() {
    let dir = fixture("sizes");
    for (cols, rows) in [(160u16, 48u16), (120, 36), (90, 28), (70, 22)] {
        let screen = run_in_pty(cols, rows, &dir, &[], 800);
        assert_layout_landmarks(&screen, cols, &format!("{cols}x{rows}"));
        assert!(
            screen.contains("notes.txt"),
            "{cols}x{rows}: fixture file visible:\n{screen}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_keyboard_navigation_and_open() {
    let dir = fixture("nav");
    // The first entry (docs) is focused; `e` enters it within the app.
    let screen = run_in_pty(120, 36, &dir, &["e"], 500);
    // Any path row may be the live one when the frame scrolled, so every
    // row carrying a path bar is considered.
    let entered = screen
        .lines()
        .filter(|line| line.contains("Path:"))
        .any(|line| line.contains("docs"));
    assert!(entered, "e did not enter docs:\n{screen}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_problematic_file_previews_preserve_the_full_display() {
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 6, image::Rgb([200, 30, 30])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let cases: [(&str, &str, Vec<u8>, &str); 7] = [
        ("png", "10-target.png", png, "▀"),
        (
            "pdf",
            "10-target.pdf",
            b"%PDF-1.7\n1 0 obj\n".to_vec(),
            "binary or unsupported document",
        ),
        (
            "doc",
            "10-target.doc",
            b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1document".to_vec(),
            "binary or unsupported document",
        ),
        (
            "docx",
            "10-target.docx",
            b"PK\x03\x04word/document.xml".to_vec(),
            "binary or unsupported document",
        ),
        (
            "binary",
            "10-target.bin",
            b"prefix\x1b[2J\0\xfftail".to_vec(),
            "binary or unsupported document",
        ),
        (
            "corrupt-png",
            "10-target.png",
            b"not really a png".to_vec(),
            "cannot decode image",
        ),
        (
            "text",
            "10-target.txt",
            b"plain\ttext\nsecond line\n".to_vec(),
            "plain    text",
        ),
    ];

    for (tag, target_name, bytes, expected_preview) in cases {
        let dir = preview_fixture(tag, target_name, &bytes);
        // Move onto the target, back to text, then onto the target again. This
        // exercises stale worker-result rejection and repainting after content
        // type changes, not merely the initial selection.
        let screen = run_in_pty(160, 48, &dir, &["l", "h", "l"], 900);
        assert_layout_landmarks(&screen, 160, tag);
        assert!(
            screen.contains("BROWSER"),
            "{tag}: status missing:\n{screen}"
        );
        assert!(
            screen.contains("Preview"),
            "{tag}: preview title missing:\n{screen}"
        );
        assert!(
            screen.contains("Type:"),
            "{tag}: metadata missing:\n{screen}"
        );
        assert!(
            screen.contains(target_name),
            "{tag}: target not focused:\n{screen}"
        );
        assert!(
            screen.contains(expected_preview),
            "{tag}: expected preview {expected_preview:?}:\n{screen}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn headless_help_overlay() {
    let dir = fixture("help");
    let screen = run_in_pty(120, 36, &dir, &["?"], 400);
    assert!(screen.contains("HELP"), "help overlay:\n{screen}");
    assert!(screen.contains("encrypt"), "help documents X:\n{screen}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_password_dialog_masks_input() {
    let dir = fixture("pw");
    // Select first file entry (notes.txt is not first; use X on whatever is
    // focused after navigating into files) and type a password.
    // `X` now opens the destructive confirmation first, so the password
    // dialog is only reachable after explicitly accepting the warning.
    let keys = vec!["G", "X", "y", "s", "e", "c", "r", "e", "t"];
    let screen = run_in_pty(120, 36, &dir, &keys, 400);
    assert!(screen.contains("ENCRYPT"), "encrypt dialog:\n{screen}");
    assert!(screen.contains("new password:"), "prompt:\n{screen}");
    assert!(
        !screen.contains("secret"),
        "password never echoed to screen:\n{screen}"
    );
    assert!(screen.contains("***"), "masked input visible:\n{screen}");
    let _ = std::fs::remove_dir_all(&dir);
}
