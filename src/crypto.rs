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
//! * under `SourceDisposition::Keep` sources are never removed; under
//!   `RemoveAfterSuccess` a source is deleted only after its finalized
//!   destination is re-verified, and any failure leaves both copies intact
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

/// Whether a successful operation removes the source it consumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDisposition {
    /// Leave the source in place (non-destructive mode).
    Keep,
    /// Remove the source only after the produced output is finalized.
    RemoveAfterSuccess,
}

/// Re-check a produced output, then delete the source it consumed.
/// Nothing is deleted before the destination is confirmed final.
fn remove_consumed_source(
    source: &Path,
    destination: &Path,
    restores_tree: bool,
    was_dir: bool,
) -> Result<(), CryptoError> {
    let finalized = match std::fs::metadata(destination) {
        Ok(meta) if restores_tree => meta.is_dir(),
        Ok(meta) => meta.is_file() && meta.len() > 0,
        Err(_) => false,
    };
    if !finalized {
        return Err(CryptoError::Io(io::Error::other(format!(
            "output {} is missing or empty, source kept",
            destination.display()
        ))));
    }
    let removal = if was_dir {
        std::fs::remove_dir_all(source)
    } else {
        std::fs::remove_file(source)
    };
    removal.map_err(|e| {
        CryptoError::Io(io::Error::new(
            e.kind(),
            format!("cannot remove source {}: {e}", source.display()),
        ))
    })
}

/// Runs one job (encrypt or decrypt) over every source.
pub fn run_job(
    kind: CryptoKind,
    sources: &[PathBuf],
    password: &SecretString,
    cancel: &Arc<AtomicBool>,
    disposition: SourceDisposition,
    progress: &mut Progress<'_>,
) -> (Vec<CryptoOutcome>, Vec<(PathBuf, CryptoError)>) {
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        progress(source, idx, sources.len());
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let was_dir = source.is_dir();
        // Only a `.tar.age` decrypt restores a tree; every other job writes a file.
        let restores_tree = kind == CryptoKind::Decrypt && is_encrypted_archive(&name);
        let result = match (kind, was_dir) {
            (CryptoKind::Encrypt, true) => encrypt_directory(source, password, cancel),
            (CryptoKind::Encrypt, false) => encrypt_file(source, password, cancel),
            (CryptoKind::Decrypt, _) if is_encrypted_archive(&name) => {
                decrypt_directory(source, password, cancel)
            }
            (CryptoKind::Decrypt, _) => decrypt_file(source, password, cancel),
        };
        match result {
            Ok(destination) => {
                let removal = match disposition {
                    SourceDisposition::Keep => Ok(()),
                    SourceDisposition::RemoveAfterSuccess => {
                        remove_consumed_source(source, &destination, restores_tree, was_dir)
                    }
                };
                match removal {
                    Ok(()) => done.push(CryptoOutcome {
                        source: source.clone(),
                        destination,
                        kind,
                    }),
                    Err(e) => failed.push((source.clone(), e)),
                }
            }
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
        crate::filesystem::sandbox::fixture("crypto")
    }

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn run(
        kind: CryptoKind,
        sources: &[PathBuf],
        password: &SecretString,
        cancel: &Arc<AtomicBool>,
        disposition: SourceDisposition,
    ) -> (Vec<CryptoOutcome>, Vec<(PathBuf, CryptoError)>) {
        run_job(
            kind,
            sources,
            password,
            cancel,
            disposition,
            &mut |_, _, _| {},
        )
    }

    fn part_leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".part-"))
            .collect();
        names.sort();
        names
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

    #[test]
    fn keep_disposition_never_removes_sources() {
        let dir = fixture();
        let keep = dir.join("keep.txt");
        std::fs::write(&keep, b"kept bytes\n").unwrap();
        let drop = dir.join("drop.txt");
        std::fs::write(&drop, b"removed bytes\n").unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let sources = vec![keep.clone(), drop.clone()];
        let (done, failed) = run(
            CryptoKind::Encrypt,
            &sources,
            &password,
            &cancel,
            SourceDisposition::Keep,
        );
        assert!(failed.is_empty());
        assert_eq!(done.len(), 2);
        assert!(keep.exists());
        assert!(drop.exists());
        assert!(std::fs::metadata(dir.join("keep.txt.age")).unwrap().len() > 0);
        assert!(std::fs::metadata(dir.join("drop.txt.age")).unwrap().len() > 0);
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypt_then_remove_deletes_source_only_after_output() {
        let dir = fixture();
        let src = dir.join("notes.txt");
        let plaintext = b"hello secret world\n".repeat(200);
        std::fs::write(&src, &plaintext).unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let (done, failed) = run(
            CryptoKind::Encrypt,
            std::slice::from_ref(&src),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(failed.is_empty());
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].source, src);
        let enc = dir.join("notes.txt.age");
        assert_eq!(done[0].destination, enc);
        assert!(std::fs::metadata(&enc).unwrap().len() > 0);
        assert!(!src.exists());
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_disposition_removes_tree_after_archive() {
        let dir = fixture();
        let tree = dir.join("proj");
        std::fs::create_dir_all(tree.join("src/nested")).unwrap();
        std::fs::create_dir_all(tree.join("empty-dir")).unwrap();
        std::fs::write(tree.join("src/main.rs"), b"fn main() {}\n").unwrap();
        let deep = vec![9u8; 2048];
        std::fs::write(tree.join("src/nested/deep.bin"), &deep).unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let (done, failed) = run(
            CryptoKind::Encrypt,
            std::slice::from_ref(&tree),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(failed.is_empty());
        assert_eq!(done.len(), 1);
        let archive = dir.join("proj.tar.age");
        assert_eq!(done[0].destination, archive);
        assert!(std::fs::metadata(&archive).unwrap().len() > 0);
        assert!(!tree.exists());
        assert!(part_leftovers(&dir).is_empty());

        let (restored, failed) = run(
            CryptoKind::Decrypt,
            std::slice::from_ref(&archive),
            &password,
            &cancel,
            SourceDisposition::Keep,
        );
        assert!(failed.is_empty());
        assert_eq!(restored[0].destination, tree);
        assert_eq!(
            std::fs::read(tree.join("src/main.rs")).unwrap(),
            b"fn main() {}\n"
        );
        assert_eq!(
            std::fs::read(tree.join("src/nested/deep.bin")).unwrap(),
            deep
        );
        assert!(tree.join("empty-dir").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_encryption_keeps_plaintext() {
        let dir = fixture();
        let src = dir.join("a.txt");
        let plaintext = b"one\n".repeat(64);
        std::fs::write(&src, &plaintext).unwrap();
        let enc = dir.join("a.txt.age");
        let pre_existing = b"tampered".repeat(8);
        std::fs::write(&enc, &pre_existing).unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let (done, failed) = run(
            CryptoKind::Encrypt,
            std::slice::from_ref(&src),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(done.is_empty());
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, src);
        assert!(matches!(failed[0].1, CryptoError::DestinationExists(_)));
        assert_eq!(std::fs::read(&src).unwrap(), plaintext);
        assert_eq!(std::fs::read(&enc).unwrap(), pre_existing);
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_password_keeps_encrypted_source() {
        let dir = fixture();
        let src = dir.join("data.bin");
        let plaintext: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &plaintext).unwrap();
        let enc = encrypt_file(&src, &secret("right"), &no_cancel()).unwrap();
        let ciphertext = std::fs::read(&enc).unwrap();
        std::fs::remove_file(&src).unwrap();
        let wrong = secret("wrong");
        let cancel = no_cancel();
        let (done, failed) = run(
            CryptoKind::Decrypt,
            std::slice::from_ref(&enc),
            &wrong,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(done.is_empty());
        assert_eq!(failed.len(), 1);
        assert!(matches!(failed[0].1, CryptoError::DecryptionFailed));
        assert!(enc.exists());
        assert_eq!(std::fs::read(&enc).unwrap(), ciphertext);
        assert!(!dir.join("data.bin").exists());
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancellation_keeps_source_under_remove_disposition() {
        let dir = fixture();
        let src = dir.join("big.bin");
        let payload = vec![3u8; 5 * 1024 * 1024];
        std::fs::write(&src, &payload).unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let password = secret("pw");
        let (done, failed) = run(
            CryptoKind::Encrypt,
            std::slice::from_ref(&src),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(done.is_empty());
        assert_eq!(failed.len(), 1);
        assert!(matches!(failed[0].1, CryptoError::Cancelled));
        assert_eq!(std::fs::read(&src).unwrap(), payload);
        assert!(!dir.join("big.bin.age").exists());
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decrypt_removes_encrypted_source_after_restore() {
        let dir = fixture();
        let src = dir.join("report.txt");
        let plaintext = b"quarterly numbers\n".repeat(300);
        std::fs::write(&src, &plaintext).unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let (done, failed) = run(
            CryptoKind::Encrypt,
            std::slice::from_ref(&src),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(failed.is_empty());
        let enc = done[0].destination.clone();
        assert!(!src.exists());

        let (restored, failed) = run(
            CryptoKind::Decrypt,
            std::slice::from_ref(&enc),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(failed.is_empty());
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].source, enc);
        assert_eq!(restored[0].destination, src);
        assert_eq!(std::fs::read(&src).unwrap(), plaintext);
        assert!(!enc.exists());
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decrypt_archive_removes_archive_after_tree_restore() {
        let dir = fixture();
        let tree = dir.join("bundle");
        std::fs::create_dir_all(tree.join("keep")).unwrap();
        std::fs::write(tree.join("keep/data.bin"), vec![5u8; 1024]).unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let (done, failed) = run(
            CryptoKind::Encrypt,
            std::slice::from_ref(&tree),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(failed.is_empty());
        let archive = done[0].destination.clone();
        assert!(!tree.exists());

        let (restored, failed) = run(
            CryptoKind::Decrypt,
            std::slice::from_ref(&archive),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(failed.is_empty());
        assert_eq!(restored[0].destination, tree);
        assert!(tree.is_dir());
        assert_eq!(
            std::fs::read(tree.join("keep/data.bin")).unwrap(),
            vec![5u8; 1024]
        );
        assert!(!archive.exists());
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decrypt_archive_keeps_archive_on_cancellation() {
        let dir = fixture();
        let tree = dir.join("bundle");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("keep.bin"), vec![6u8; 4096]).unwrap();
        let password = secret("pw");
        let cancel = no_cancel();
        let archive = encrypt_directory(&tree, &password, &cancel).unwrap();
        let ciphertext = std::fs::read(&archive).unwrap();
        // The extraction target must be free, otherwise the destination
        // check fires before cancellation is ever observed.
        std::fs::remove_dir_all(&tree).unwrap();

        let stopped = Arc::new(AtomicBool::new(true));
        let (done, failed) = run(
            CryptoKind::Decrypt,
            std::slice::from_ref(&archive),
            &password,
            &stopped,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert!(done.is_empty(), "cancelled decrypt must not report success");
        assert!(
            matches!(failed[0].1, CryptoError::Cancelled),
            "actual error: {:?}",
            failed[0].1
        );
        // The encrypted archive is the only remaining copy and must survive.
        assert!(archive.exists());
        assert_eq!(std::fs::read(&archive).unwrap(), ciphertext);
        assert!(!tree.exists());
        assert!(part_leftovers(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_never_in_debug_or_error() {
        let dir = fixture();
        let src = dir.join("secret.txt");
        std::fs::write(&src, b"classified\n".repeat(50)).unwrap();
        let password_text = "correct horse battery staple 42";
        let password = secret(password_text);
        let cancel = no_cancel();
        let enc = encrypt_file(&src, &password, &cancel).unwrap();
        std::fs::remove_file(&src).unwrap();

        let err = decrypt_file(&enc, &secret("bad guess"), &cancel).unwrap_err();
        let rendered = format!("{err} {err:?}");
        assert!(!rendered.contains(password_text));
        assert!(rendered.contains("wrong password or corrupted data"));

        let (done, failed) = run(
            CryptoKind::Decrypt,
            std::slice::from_ref(&enc),
            &password,
            &cancel,
            SourceDisposition::RemoveAfterSuccess,
        );
        assert_eq!(done.len(), 1);
        let state = format!(
            "{:?} {:?} {:?} {:?} {:?}",
            SourceDisposition::Keep,
            SourceDisposition::RemoveAfterSuccess,
            done[0],
            failed,
            secret(password_text)
        );
        assert!(!state.contains(password_text));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
