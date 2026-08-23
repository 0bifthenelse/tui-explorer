//! File and folder encryption using the maintained `age` crate's passphrase
//! encryption API. No cryptographic primitives are implemented here and no
//! external encryption programs are shelled out to.
//!
//! Naming convention:
//! * a regular file `report.txt` encrypts to `report.txt.age`
//! * a directory `photos` is first serialized into a portable tar stream and
//!   then encrypted to `photos.tar.age`
//!
//! Safety properties:
//! * output is written to a temporary file in the destination filesystem,
//!   the age stream is finalized, the writer flushed, and only then the
//!   temporary file is atomically renamed into place
//! * existing destinations are never overwritten (creation is exclusive)
//! * sources are never deleted automatically
//! * temporary files are removed after cancellation or failure
//! * folders are archived with relative paths only; on extraction, entries
//!   with absolute paths, `..` components, or any path escaping the
//!   destination are rejected
//! * symlinks are archived as symlinks (never followed); on extraction,
//!   links whose targets are absolute or contain `..` are skipped

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use age::secrecy::SecretString;

pub const ENCRYPTED_EXTENSION: &str = "age";
pub const ARCHIVE_EXTENSION: &str = "tar.age";

const CHUNK: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("wrong password or corrupted data")]
    DecryptionFailed,
    #[error("unsupported encrypted input: {0}")]
    UnsupportedInput(PathBuf),
    #[error("unsafe archive entry rejected: {0}")]
    UnsafeEntry(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("input/output error: {0}")]
    Io(#[from] io::Error),
    #[error("encryption error: {0}")]
    Encrypt(#[from] age::EncryptError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoKind {
    Encrypt,
    Decrypt,
}

#[derive(Clone, Debug)]
pub struct CryptoOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: CryptoKind,
}

/// Progress callback: (current source, sources done, total sources).
pub type Progress<'a> = dyn FnMut(&Path, usize, usize) + Send + 'a;

pub fn is_encrypted_name(name: &str) -> bool {
    name.ends_with(".age")
}

pub fn is_encrypted_archive(name: &str) -> bool {
    name.ends_with(".tar.age")
}

/// Destination path for encrypting `source`.
pub fn encrypted_destination(source: &Path, is_dir: bool) -> PathBuf {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new_name = if is_dir {
        format!("{name}.tar.age")
    } else {
        format!("{name}.age")
    };
    source.with_file_name(new_name)
}

/// Destination path for decrypting `source` (must end in `.age`).
pub fn decrypted_destination(source: &Path) -> Result<PathBuf, CryptoError> {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if is_encrypted_archive(&name) {
        Ok(source.with_file_name(name.trim_end_matches(".tar.age")))
    } else if is_encrypted_name(&name) {
        Ok(source.with_file_name(name.trim_end_matches(".age")))
    } else {
        Err(CryptoError::UnsupportedInput(source.to_path_buf()))
    }
}

struct TempOutput {
    temp: PathBuf,
    final_path: PathBuf,
    writer: Option<BufWriter<File>>,
    done: bool,
}

impl TempOutput {
    /// Create `<dest>.part-<pid>` exclusively; refuse when the final
    /// destination already exists so nothing is ever silently overwritten.
    fn create(final_path: &Path) -> Result<Self, CryptoError> {
        if final_path.exists() {
            return Err(CryptoError::DestinationExists(final_path.to_path_buf()));
        }
        let name = final_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "out".to_string());
        let temp = final_path.with_file_name(format!(".{name}.part-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        Ok(TempOutput {
            temp,
            final_path: final_path.to_path_buf(),
            writer: Some(BufWriter::new(file)),
            done: false,
        })
    }

    /// Flush, validate non-empty output and atomically rename into place.
    fn finalize(mut self) -> Result<PathBuf, CryptoError> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("temp output missing writer"))?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        let meta = std::fs::metadata(&self.temp)?;
        if meta.len() == 0 {
            std::fs::remove_file(&self.temp).ok();
            return Err(CryptoError::Io(io::Error::new(
                io::ErrorKind::WriteZero,
                "encryption produced no output",
            )));
        }
        // create_new race guard: never clobber a destination that appeared
        // while we were working.
        if self.final_path.exists() {
            std::fs::remove_file(&self.temp).ok();
            return Err(CryptoError::DestinationExists(self.final_path.clone()));
        }
        std::fs::rename(&self.temp, &self.final_path)?;
        self.done = true;
        Ok(self.final_path.clone())
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        if !self.done {
            std::fs::remove_file(&self.temp).ok();
        }
    }
}

fn copy_with_cancel(
    reader: &mut impl Read,
    writer: &mut impl Write,
    cancel: &Arc<AtomicBool>,
) -> Result<u64, CryptoError> {
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CryptoError::Cancelled);
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
    Ok(total)
}

fn encrypt_stream(
    reader: &mut impl Read,
    dest: &Path,
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, CryptoError> {
    let mut out = TempOutput::create(dest)?;
    let encryptor = age::Encryptor::with_user_passphrase(password.clone());
    let writer = out
        .writer
        .take()
        .ok_or_else(|| io::Error::other("temp output missing writer"))?;
    let mut age_writer = encryptor.wrap_output(writer)?;
    if let Err(error) = copy_with_cancel(reader, &mut age_writer, cancel) {
        drop(age_writer);
        match std::fs::remove_file(&out.temp) {
            Ok(()) => out.done = true,
            Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => out.done = true,
            Err(cleanup) => return Err(CryptoError::Io(cleanup)),
        }
        return Err(error);
    }
    // Finalize the age stream, then recover the underlying writer.
    out.writer = Some(age_writer.finish()?);
    out.finalize()
}

/// Encrypt a single regular file.
pub fn encrypt_file(
    source: &Path,
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, CryptoError> {
    let dest = encrypted_destination(source, false);
    let mut input = BufReader::new(File::open(source)?);
    encrypt_stream(&mut input, &dest, password, cancel)
}

/// Archive a directory into a tar stream (relative paths, symlinks kept as
/// links, empty directories preserved) and encrypt that stream.
pub fn encrypt_directory(
    source: &Path,
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, CryptoError> {
    let dest = encrypted_destination(source, true);
    let base = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let (reader, writer) = io::pipe()?;
    let src = source.to_path_buf();
    let cancel_thread = cancel.clone();
    let handle = std::thread::spawn(move || -> io::Result<()> {
        let mut builder = tar::Builder::new(writer);
        builder.follow_symlinks(false);
        append_dir(&mut builder, &src, Path::new(&base), &cancel_thread)?;
        builder.finish()
    });
    let mut reader = BufReader::new(reader);
    let result = encrypt_stream(&mut reader, &dest, password, cancel);
    let join = handle.join();
    result?;
    match join {
        Ok(Ok(())) => Ok(dest),
        Ok(Err(e)) => {
            std::fs::remove_file(&dest).ok();
            Err(CryptoError::Io(e))
        }
        Err(_) => {
            std::fs::remove_file(&dest).ok();
            Err(CryptoError::Cancelled)
        }
    }
}

fn append_dir(
    builder: &mut tar::Builder<impl Write>,
    disk: &Path,
    archive: &Path,
    cancel: &Arc<AtomicBool>,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    builder.append_dir(archive, disk)?;
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(disk)? {
        children.push(entry?.path());
    }
    children.sort();
    for child in children {
        let name = child.file_name().unwrap_or_default();
        let archive_path = archive.join(name);
        let meta = std::fs::symlink_metadata(&child)?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            // Stored as a symlink entry; never followed.
            let target = std::fs::read_link(&child)?;
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(meta.permissions().mode_bits());
            header.set_mtime(
                meta.modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            header.set_cksum();
            builder.append_link(&mut header, &archive_path, &target)?;
        } else if ft.is_dir() {
            append_dir(builder, &child, &archive_path, cancel)?;
        } else if ft.is_file() {
            builder.append_path_with_name(&child, &archive_path)?;
        }
        // sockets, pipes and devices are skipped: they cannot be restored.
    }
    Ok(())
}

trait ModeBits {
    fn mode_bits(&self) -> u32;
}

impl ModeBits for std::fs::Permissions {
    fn mode_bits(&self) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.mode() & 0o7777
        }
        #[cfg(not(unix))]
        {
            0o644
        }
    }
}

/// Open an age passphrase-encrypted input as a plaintext reader.
/// Wrong passwords and corrupt inputs both surface as DecryptionFailed
/// without ever exposing the password.
fn passphrase_reader(
    input: File,
    password: &SecretString,
) -> Result<age::stream::StreamReader<BufReader<File>>, CryptoError> {
    let identity = age::scrypt::Identity::new(password.clone());
    let decryptor = age::Decryptor::new_buffered(BufReader::new(input))
        .map_err(|_| CryptoError::DecryptionFailed)?;
    decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| CryptoError::DecryptionFailed)
}

/// Decrypt a `.age` file back into a regular file.
pub fn decrypt_file(
    source: &Path,
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, CryptoError> {
    let dest = decrypted_destination(source)?;
    let input = File::open(source)?;
    let mut reader = passphrase_reader(input, password)?;
    let mut out = TempOutput::create(&dest)?;
    let copy_result = {
        let writer = out
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("temp output missing writer"))?;
        copy_with_cancel(&mut reader, writer, cancel)
    };
    copy_result?;
    out.finalize()
}

/// Decrypt a `.tar.age` archive and restore the directory tree safely.
pub fn decrypt_directory(
    source: &Path,
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, CryptoError> {
    let dest = decrypted_destination(source)?;
    if dest.exists() {
        return Err(CryptoError::DestinationExists(dest));
    }
    let input = File::open(source)?;
    let reader = passphrase_reader(input, password)?;
    let mut archive = tar::Archive::new(reader);
    let temp_root = source.with_file_name(format!(
        ".{}.part-{}",
        dest.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "extract".to_string()),
        std::process::id()
    ));
    std::fs::create_dir(&temp_root)?;
    let result = extract_safely(&mut archive, &temp_root, cancel);
    match result {
        Ok(()) => {
            // The archive contains a single top-level directory; move it out.
            let mut entries = std::fs::read_dir(&temp_root)?;
            let only = entries.next().transpose()?;
            if entries.next().is_some() {
                std::fs::remove_dir_all(&temp_root).ok();
                return Err(CryptoError::UnsafeEntry(
                    "archive holds more than one top-level entry".to_string(),
                ));
            }
            let Some(only) = only else {
                std::fs::remove_dir_all(&temp_root).ok();
                return Err(CryptoError::UnsafeEntry("archive is empty".to_string()));
            };
            std::fs::rename(only.path(), &dest)?;
            std::fs::remove_dir_all(&temp_root).ok();
            Ok(dest)
        }
        Err(e) => {
            std::fs::remove_dir_all(&temp_root).ok();
            Err(e)
        }
    }
}

fn safe_join(base: &Path, entry_path: &Path) -> Result<PathBuf, CryptoError> {
    let mut out = base.to_path_buf();
    for component in entry_path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => {
                return Err(CryptoError::UnsafeEntry(entry_path.display().to_string()));
            }
        }
    }
    Ok(out)
}

fn extract_safely<R: Read>(
    archive: &mut tar::Archive<R>,
    dest_root: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), CryptoError> {
    for entry in archive.entries()? {
        if cancel.load(Ordering::Relaxed) {
            return Err(CryptoError::Cancelled);
        }
        let mut entry = entry?;
        let entry_path = entry.path()?.into_owned();
        let target = safe_join(dest_root, &entry_path)?;
        // The joined path must stay inside the destination root.
        if !target.starts_with(dest_root) {
            return Err(CryptoError::UnsafeEntry(entry_path.display().to_string()));
        }
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&target)?;
            }
            tar::EntryType::Regular => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                entry.unpack(&target)?;
            }
            tar::EntryType::Symlink => {
                let link_target = entry
                    .link_name()?
                    .ok_or_else(|| CryptoError::UnsafeEntry(entry_path.display().to_string()))?;
                // Links with absolute targets or `..` are skipped, never followed.
                let unsafe_link = link_target.is_absolute()
                    || link_target
                        .components()
                        .any(|c| matches!(c, Component::ParentDir));
                if unsafe_link {
                    continue;
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(&link_target, &target)?;
            }
            _ => {
                // Devices, fifos and hardlinks are not restored.
                continue;
            }
        }
    }
    Ok(())
}

/// Run one job (encrypt or decrypt) over every source.
pub fn run_job(
    kind: CryptoKind,
    sources: &[PathBuf],
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
    progress: &mut Progress<'_>,
) -> (Vec<CryptoOutcome>, Vec<(PathBuf, CryptoError)>) {
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        progress(source, idx, sources.len());
        let result = match (kind, source.is_dir()) {
            (CryptoKind::Encrypt, true) => encrypt_directory(source, password, cancel),
            (CryptoKind::Encrypt, false) => encrypt_file(source, password, cancel),
            (CryptoKind::Decrypt, _) => {
                let name = source
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if is_encrypted_archive(&name) {
                    decrypt_directory(source, password, cancel)
                } else {
                    decrypt_file(source, password, cancel)
                }
            }
        };
        match result {
            Ok(destination) => done.push(CryptoOutcome {
                source: source.clone(),
                destination,
                kind,
            }),
            Err(e) => {
                failed.push((source.clone(), e));
                if matches!(failed.last().map(|(_, e)| e), Some(CryptoError::Cancelled)) {
                    break;
                }
            }
        }
    }
    (done, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(pass: &str) -> SecretString {
        SecretString::from(pass.to_string())
    }

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tui-explorer-crypto-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn file_roundtrip_preserves_bytes() {
        let dir = fixture();
        let src = dir.join("notes.txt");
        std::fs::write(&src, b"hello secret world\n".repeat(100)).unwrap();
        let enc = encrypt_file(&src, &secret("pw"), &no_cancel()).unwrap();
        assert_eq!(enc, dir.join("notes.txt.age"));
        assert!(enc.exists());
        let dec = decrypt_file(&enc, &secret("pw"), &no_cancel()).unwrap_err();
        // destination (original) still exists -> refused, never overwritten
        assert!(matches!(dec, CryptoError::DestinationExists(_)));
        let original = std::fs::read(&src).unwrap();
        std::fs::remove_file(&src).unwrap();
        let dec = decrypt_file(&enc, &secret("pw"), &no_cancel()).unwrap();
        assert_eq!(std::fs::read(&dec).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_password_fails_without_touching_source() {
        let dir = fixture();
        let src = dir.join("data.bin");
        let bytes: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &bytes).unwrap();
        let enc = encrypt_file(&src, &secret("right"), &no_cancel()).unwrap();
        std::fs::remove_file(&src).unwrap();
        let err = decrypt_file(&enc, &secret("wrong"), &no_cancel()).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
        // no partial output left behind
        assert!(!dir.join("data.bin").exists());
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains(".part-"))
                .count(),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_roundtrip_preserves_tree_and_empty_dirs() {
        let dir = fixture();
        let tree = dir.join("proj");
        std::fs::create_dir_all(tree.join("src/nested")).unwrap();
        std::fs::create_dir_all(tree.join("empty-dir")).unwrap();
        std::fs::write(tree.join("src/main.rs"), b"fn main() {}\n").unwrap();
        std::fs::write(tree.join("src/nested/deep.bin"), vec![7u8; 4096]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("src/main.rs", tree.join("link-ok")).unwrap();
        let enc = encrypt_directory(&tree, &secret("pw"), &no_cancel()).unwrap();
        assert_eq!(enc, dir.join("proj.tar.age"));
        std::fs::remove_dir_all(&tree).unwrap();
        let restored = decrypt_directory(&enc, &secret("pw"), &no_cancel()).unwrap();
        assert_eq!(restored, tree);
        assert_eq!(
            std::fs::read(tree.join("src/main.rs")).unwrap(),
            b"fn main() {}\n"
        );
        assert_eq!(
            std::fs::read(tree.join("src/nested/deep.bin")).unwrap(),
            vec![7u8; 4096]
        );
        assert!(tree.join("empty-dir").is_dir());
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(tree.join("link-ok")).unwrap(),
            PathBuf::from("src/main.rs")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malicious_archive_paths_are_rejected() {
        let dir = fixture();
        let tar_path = dir.join("evil.tar");
        // Craft a raw tar by hand: the tar crate rightly refuses to write
        // `..` paths, so we build the header bytes ourselves.
        let data = b"pwned";
        let mut header = [0u8; 512];
        header[..13].copy_from_slice(b"../escape.txt");
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(b"00000000005\0");
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        let sum: u32 = header.iter().map(|b| *b as u32).sum();
        let chk = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(chk.as_bytes());
        let mut raw = Vec::new();
        raw.extend_from_slice(&header);
        raw.extend_from_slice(data);
        raw.resize(512 + 512, 0);
        raw.resize(512 + 512 + 1024, 0);
        std::fs::write(&tar_path, &raw).unwrap();
        let mut archive = tar::Archive::new(File::open(&tar_path).unwrap());
        let dest = dir.join("dest");
        std::fs::create_dir(&dest).unwrap();
        let err = extract_safely(&mut archive, &dest, &no_cancel()).unwrap_err();
        assert!(matches!(err, CryptoError::UnsafeEntry(_)));
        assert!(!dir.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_destination_is_never_overwritten() {
        let dir = fixture();
        let src = dir.join("a.txt");
        std::fs::write(&src, b"one").unwrap();
        let enc = encrypt_file(&src, &secret("pw"), &no_cancel()).unwrap();
        std::fs::write(&enc, b"tampered").unwrap();
        let err = encrypt_file(&src, &secret("pw"), &no_cancel()).unwrap_err();
        assert!(matches!(err, CryptoError::DestinationExists(_)));
        assert_eq!(std::fs::read(&enc).unwrap(), b"tampered");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancellation_leaves_no_artifacts() {
        let dir = fixture();
        let src = dir.join("big.bin");
        std::fs::write(&src, vec![3u8; 5 * 1024 * 1024]).unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let err = encrypt_file(&src, &secret("pw"), &cancel).unwrap_err();
        assert!(matches!(err, CryptoError::Cancelled));
        assert!(!dir.join("big.bin.age").exists());
        assert_eq!(std::fs::read(&src).unwrap().len(), 5 * 1024 * 1024);
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".part-"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
