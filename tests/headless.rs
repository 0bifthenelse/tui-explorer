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
    let dir = std::env::temp_dir().join(format!(
        "tui-explorer-headless-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
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
    let dir = std::env::temp_dir().join(format!(
        "tui-explorer-headless-{}-preview-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("00-start.txt"),
        b"safe text preview\nsecond line\n",
    )
    .unwrap();
    std::fs::write(dir.join(target_name), bytes).unwrap();
    dir
}

/// Run the real binary in a pty of `cols`x`rows`, send `keys` after warm-up,
/// and return the final screen contents as seen by a vt100 terminal.
fn run_in_pty(cols: u16, rows: u16, dir: &Path, keys: &[&str], settle_ms: u64) -> String {
    let log = std::env::temp_dir().join(format!(
        "tui-explorer-pty-{}-{cols}x{rows}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
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
        .env("XDG_DATA_HOME", dir.join(".xdg")) // isolate the tag database
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn script pty");
    let mut stdin = child.stdin.take().expect("pty stdin");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    for key in keys {
        stdin.write_all(key.as_bytes()).expect("write key");
        stdin.flush().expect("flush key");
        std::thread::sleep(std::time::Duration::from_millis(350));
    }
    std::thread::sleep(std::time::Duration::from_millis(settle_ms));
    // Request one final full redraw and give the pty logger time to flush
    // before teardown so the captured frame is complete.
    stdin.write_all(b"\x0c").ok(); // Ctrl-L
    stdin.flush().ok();
    std::thread::sleep(std::time::Duration::from_millis(400));
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
    let path_line = screen.lines().find(|l| l.contains("Path:")).unwrap_or("");
    assert!(
        path_line.contains("docs"),
        "e entered docs, path bar: {path_line}\n{screen}"
    );
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
    let mut keys = vec!["G"]; // last entry (unicode file)
    keys.push("X");
    keys.push("s");
    keys.push("e");
    keys.push("c");
    keys.push("r");
    keys.push("e");
    keys.push("t");
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
