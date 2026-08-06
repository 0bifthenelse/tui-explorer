//! Preview pipeline for the right-hand metadata/preview panel.
//!
//! All expensive work (reading file contents, decoding and resizing images)
//! happens on worker threads; the render loop only consumes the finished
//! [`PreviewLoaded`] values. Results are keyed by path + mtime + size so the
//! cache is invalidated whenever file modification metadata changes. Only the
//! latest preview is retained, which bounds memory use to a single decoded
//! image.

use std::path::Path;

/// Maximum bytes read for a text preview.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
/// Maximum directory entries listed in a directory preview.
pub const MAX_DIR_ENTRIES: usize = 200;
/// Images larger than this (in bytes) are not decoded.
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Images with more pixels than this are not decoded (decompression bombs).
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

const UNSUPPORTED_MESSAGE: &str = "binary or unsupported document; no text preview";

pub fn is_supported_image(name: &str) -> bool {
    extension(name)
        .map(|ext| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

fn extension(name: &str) -> Option<&str> {
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty()).then_some(ext)
}

fn is_unsupported_document(name: &str) -> bool {
    extension(name)
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods"
            )
        })
        .unwrap_or(false)
}

/// Decoded preview produced off the render loop.
#[derive(Clone)]
pub enum PreviewLoaded {
    Text { lines: Vec<String>, truncated: bool },
    Image(image::DynamicImage),
    Directory(Vec<String>),
    Unavailable(String),
}

impl std::fmt::Debug for PreviewLoaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewLoaded::Text { lines, truncated } => f
                .debug_struct("Text")
                .field("lines", &lines.len())
                .field("truncated", truncated)
                .finish(),
            PreviewLoaded::Image(img) => write!(f, "Image({}x{})", img.width(), img.height()),
            PreviewLoaded::Directory(names) => {
                f.debug_tuple("Directory").field(&names.len()).finish()
            }
            PreviewLoaded::Unavailable(msg) => f.debug_tuple("Unavailable").field(msg).finish(),
        }
    }
}

/// Load preview content for `path`. Intended to run on a worker thread.
/// Never panics on corrupt, huge, inaccessible or unsupported inputs.
pub fn load(path: &Path, is_dir: bool, name: &str) -> PreviewLoaded {
    if is_dir {
        return load_directory(path);
    }
    if is_supported_image(name) {
        return load_image(path);
    }
    if is_unsupported_document(name) {
        return PreviewLoaded::Unavailable(UNSUPPORTED_MESSAGE.to_string());
    }
    load_text(path)
}

fn load_directory(path: &Path) -> PreviewLoaded {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names.truncate(MAX_DIR_ENTRIES);
            PreviewLoaded::Directory(names)
        }
        Err(e) => PreviewLoaded::Unavailable(format!("cannot read directory: {e}")),
    }
}

/// Signatures that must never be interpreted as terminal text, even when the
/// file's initial bytes happen to be valid ASCII (notably PDF and ZIP).
fn has_binary_signature(bytes: &[u8]) -> bool {
    const SIGNATURES: &[&[u8]] = &[
        b"%PDF-",                            // PDF
        b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", // OLE: legacy DOC/XLS/PPT
        b"PK\x03\x04",                       // ZIP: DOCX/XLSX/PPTX and archives
        b"PK\x05\x06",
        b"PK\x07\x08",
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff", // JPEG
        b"GIF87a",
        b"GIF89a",
        b"BM",       // BMP
        b"RIFF",     // WebP/WAV/AVI container
        b"\x1f\x8b", // gzip
        b"7z\xbc\xaf\x27\x1c",
        b"Rar!\x1a\x07",
        b"\x7fELF",
    ];
    SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature))
}

/// Decode a bounded sample only when it is safe terminal text. A sample cut
/// in the middle of its final UTF-8 scalar is accepted only when the file was
/// actually truncated; malformed UTF-8 anywhere else is rejected.
fn safe_text(bytes: &[u8], truncated: bool) -> Option<&str> {
    if has_binary_signature(bytes) {
        return None;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if truncated && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
    {
        return None;
    }
    Some(text)
}

/// Final render-boundary defense. Classification currently permits only tabs
/// and line endings; tabs are expanded and every other control is replaced so
/// future classification changes cannot place control data in a Ratatui cell.
fn sanitize_line(line: &str) -> String {
    let mut clean = String::with_capacity(line.len());
    for ch in line.chars() {
        match ch {
            '\t' => clean.push_str("    "),
            ch if ch.is_control() => clean.push('\u{fffd}'),
            ch => clean.push(ch),
        }
    }
    clean
}

fn load_text(path: &Path) -> PreviewLoaded {
    let read = (|| -> std::io::Result<(Vec<u8>, bool)> {
        let file = std::fs::File::open(path)?;
        let mut limited = file.take(MAX_TEXT_BYTES as u64 + 1);
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut limited, &mut buf)?;
        let truncated = buf.len() > MAX_TEXT_BYTES;
        buf.truncate(MAX_TEXT_BYTES);
        Ok((buf, truncated))
    })();
    match read {
        Ok((bytes, truncated)) => {
            let Some(text) = safe_text(&bytes, truncated) else {
                return PreviewLoaded::Unavailable(UNSUPPORTED_MESSAGE.to_string());
            };
            let lines = text.lines().map(sanitize_line).collect();
            PreviewLoaded::Text { lines, truncated }
        }
        Err(e) => PreviewLoaded::Unavailable(format!("cannot read file: {e}")),
    }
}

fn load_image(path: &Path) -> PreviewLoaded {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return PreviewLoaded::Unavailable(format!("cannot stat image: {e}")),
    };
    if size > MAX_IMAGE_BYTES {
        return PreviewLoaded::Unavailable(format!("image too large to preview ({} bytes)", size));
    }
    let mut reader = match image::ImageReader::open(path).map(|r| r.with_guessed_format()) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return PreviewLoaded::Unavailable(format!("cannot read image format: {e}")),
        Err(e) => return PreviewLoaded::Unavailable(format!("cannot open image: {e}")),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_PIXELS.isqrt() as u32);
    limits.max_image_height = Some(MAX_IMAGE_PIXELS.isqrt() as u32);
    reader.limits(limits);
    match reader.decode() {
        Ok(img) => {
            if (img.width() as u64) * (img.height() as u64) > MAX_IMAGE_PIXELS {
                return PreviewLoaded::Unavailable("image dimensions too large".to_string());
            }
            // Animated GIFs are intentionally shown as their first frame.
            PreviewLoaded::Image(img)
        }
        Err(e) => PreviewLoaded::Unavailable(format!("cannot decode image: {e}")),
    }
}

use std::io::Read as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tui-explorer-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn image_detection() {
        assert!(is_supported_image("a.PNG"));
        assert!(is_supported_image("b.jpeg"));
        assert!(is_supported_image("c.gif"));
        assert!(is_supported_image("d.webp"));
        assert!(is_supported_image("e.bmp"));
        assert!(!is_supported_image("f.txt"));
        assert!(!is_supported_image("noext"));
    }

    fn assert_unavailable(path: &Path) {
        match load(
            path,
            false,
            path.file_name().unwrap().to_string_lossy().as_ref(),
        ) {
            PreviewLoaded::Unavailable(message) => assert_eq!(message, UNSUPPORTED_MESSAGE),
            other => panic!("expected unavailable preview, got {other:?}"),
        }
    }

    #[test]
    fn text_preview_expands_tabs_and_contains_no_controls() {
        let dir = fixture();
        let text = dir.join("a.txt");
        std::fs::write(&text, "one\ttwo\nUnicode: café\n").unwrap();
        match load(&text, false, "a.txt") {
            PreviewLoaded::Text { lines, truncated } => {
                assert!(!truncated);
                assert_eq!(lines, vec!["one    two", "Unicode: café"]);
                assert!(lines.iter().all(|line| !line.chars().any(char::is_control)));
            }
            other => panic!("expected text preview, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn controls_invalid_utf8_and_binary_bytes_are_rejected() {
        let dir = fixture();
        for (name, bytes) in [
            ("nul.bin", b"text\0tail".as_slice()),
            ("esc.bin", b"text\x1b[31mred".as_slice()),
            ("del.bin", b"text\x7ftail".as_slice()),
            ("c1.txt", "text\u{009b}31mred".as_bytes()),
            ("invalid.bin", b"text\xfftail".as_slice()),
            ("arbitrary.bin", b"\x01\x9f\x92\x96\xfe".as_slice()),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert_unavailable(&path);
        }
        let missing = dir.join("missing.txt");
        assert!(matches!(
            load(&missing, false, "missing.txt"),
            PreviewLoaded::Unavailable(message) if message.starts_with("cannot read file:")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn document_and_archive_signatures_are_rejected() {
        let dir = fixture();
        for (name, bytes) in [
            ("report.pdf", b"%PDF-1.7\n1 0 obj\n".as_slice()),
            (
                "legacy.doc",
                b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1document".as_slice(),
            ),
            ("modern.docx", b"PK\x03\x04word/document.xml".as_slice()),
            ("archive.zip", b"PK\x05\x06empty archive".as_slice()),
            ("empty.pdf", b"".as_slice()),
            ("plain.docx", b"printable but still a document".as_slice()),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert_unavailable(&path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_preview_is_bounded_and_reports_truncation() {
        let dir = fixture();
        let text = dir.join("large.txt");
        std::fs::write(&text, "x".repeat(MAX_TEXT_BYTES + 100)).unwrap();
        match load(&text, false, "large.txt") {
            PreviewLoaded::Text { lines, truncated } => {
                assert!(truncated);
                assert_eq!(lines.iter().map(String::len).sum::<usize>(), MAX_TEXT_BYTES);
            }
            other => panic!("expected text preview, got {other:?}"),
        }

        let unicode = dir.join("unicode.txt");
        let mut bytes = vec![b'x'; MAX_TEXT_BYTES - 1];
        bytes.extend_from_slice("€".as_bytes());
        std::fs::write(&unicode, bytes).unwrap();
        assert!(matches!(
            load(&unicode, false, "unicode.txt"),
            PreviewLoaded::Text {
                truncated: true,
                ..
            }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_image_is_graceful() {
        let dir = fixture();
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not a real png").unwrap();
        assert!(matches!(
            load(&bad, false, "bad.png"),
            PreviewLoaded::Unavailable(_)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn png_roundtrip_decodes() {
        let dir = fixture();
        let png = dir.join("ok.png");
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([255, 0, 0]));
        image::DynamicImage::ImageRgb8(img).save(&png).unwrap();
        match load(&png, false, "ok.png") {
            PreviewLoaded::Image(img) => assert_eq!((img.width(), img.height()), (4, 4)),
            other => panic!("expected image, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_listing() {
        let dir = fixture();
        std::fs::write(dir.join("x"), b"").unwrap();
        match load(&dir, true, "dir") {
            PreviewLoaded::Directory(names) => assert_eq!(names, vec!["x"]),
            _ => panic!("expected directory preview"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
