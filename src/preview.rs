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

pub fn is_supported_image(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower
        .rsplit_once('.')
        .map(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext))
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
            if bytes.contains(&0) {
                return PreviewLoaded::Unavailable("binary file, no text preview".to_string());
            }
            let text = String::from_utf8_lossy(&bytes);
            let lines = text.lines().map(|l| l.to_string()).collect();
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

    #[test]
    fn text_and_binary_previews() {
        let dir = fixture();
        let text = dir.join("a.txt");
        std::fs::write(&text, "one\ntwo\n").unwrap();
        match load(&text, false, "a.txt") {
            PreviewLoaded::Text { lines, .. } => assert_eq!(lines, vec!["one", "two"]),
            _ => panic!("expected text preview"),
        }
        let bin = dir.join("b.bin");
        std::fs::write(&bin, [0u8, 1, 2]).unwrap();
        assert!(matches!(
            load(&bin, false, "b.bin"),
            PreviewLoaded::Unavailable(_)
        ));
        let missing = dir.join("missing.txt");
        assert!(matches!(
            load(&missing, false, "missing.txt"),
            PreviewLoaded::Unavailable(_)
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
