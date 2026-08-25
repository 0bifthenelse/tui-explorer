use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// One mpv child plus its persistent IPC connection. Owns the process handle,
/// a newline-framed JSON reader/writer over one nonblocking UnixStream, and
/// bounded stderr capture. Cleanup is idempotent and unlink-safe.
///
/// The Kitty video output writes every graphics frame to stdout, so the
/// child must inherit the terminal's stdout (see `spawn`).
pub struct MpvProcess {
    child: Child,
    socket_path: PathBuf,
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    stderr: Option<std::thread::JoinHandle<Vec<u8>>>,
    next_request_id: u64,
}

#[derive(Clone, Debug)]
pub struct PropertyChange {
    pub name: String,
    pub value: serde_json::Value,
}

/// A decoded line from mpv: either a reply to a request or an event.
/// Property-change events carry the observed property name and value;
/// other events keep only their name.
pub enum IpcMessage {
    Reply {
        request_id: u64,
        error: String,
    },
    Event {
        name: String,
        property: Option<PropertyChange>,
    },
    Other(serde_json::Value),
}

impl MpvProcess {
    /// Spawns mpv with the Kitty video output and a private IPC socket.
    /// `geometry_cells` is (left, top) of the reserved rectangle in cells;
    /// `size_cells` is its (width, height) in cells; `cell_pixels` is the
    /// font size in pixels used to convert cells to vo-kitty pixel values.
    /// Resume hints apply natively at load time: `--start=<secs>` seeks
    /// once the file is loaded (no IPC race) and `--pause` holds the
    /// first frame, so the supervised restart cycles (fullscreen/resize)
    /// land exactly where the previous session stopped.
    pub fn spawn(
        path: &Path,
        geometry_cells: (u16, u16),
        size_cells: (u16, u16),
        cell_pixels: (u16, u16),
        session: u64,
        resume: Option<(f64, bool)>,
    ) -> Result<Self, String> {
        let socket_path = open_private_socket(session)?;

        let mut command = Command::new("mpv");
        command
            .args(common_mpv_args(path, &socket_path))
            .args(resume_args(resume))
            .args(kitty_video_args(geometry_cells, size_cells, cell_pixels))
            .stdin(Stdio::null())
            // The Kitty video output paints through stdout; discarding it
            // would leave audio-only playback. stderr stays piped for
            // bounded diagnostics.
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped());
        finish_spawn(command, socket_path)
    }

    /// Spawns an audio-only mpv (`--vo=null --no-video --audio-display=no`)
    /// over the same IPC/stderr/shutdown machinery as `spawn`. Used by the
    /// supervisor for audio codecs symphonia cannot decode; no vo-kitty
    /// geometry is passed and nothing ever paints, so the child's stdout is
    /// discarded.
    pub fn spawn_audio(
        path: &Path,
        session: u64,
        resume: Option<(f64, bool)>,
    ) -> Result<Self, String> {
        let socket_path = open_private_socket(session)?;

        let mut command = Command::new("mpv");
        command
            .args(common_mpv_args(path, &socket_path))
            .args(AUDIO_ONLY_ARGS.iter().copied())
            .args(resume_args(resume))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        finish_spawn(command, socket_path)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends one minified JSON command with a fresh request id. Arguments
    /// keep their JSON types: mpv's IPC rejects strings where an argument
    /// is declared numeric (e.g. the observe_property observer id).
    pub fn send_command(
        &mut self,
        command: &[serde_json::Value],
        extra: Option<serde_json::Value>,
    ) -> Result<u64, String> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let mut message = serde_json::json!({
            "command": command,
            "request_id": request_id,
        });
        if let Some(extra) = extra {
            merge_object(&mut message, extra);
        }
        let mut line = serde_json::to_string(&message)
            .map_err(|error| format!("cannot encode IPC command: {error}"))?;
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .and_then(|_| self.stream.flush())
            .map_err(|error| format!("cannot write IPC command: {error}"))?;
        Ok(request_id)
    }

    /// True once the child has exited. `stderr_tail` must only be called
    /// after this returns true: the capture thread blocks on the pipe, which
    /// closes when the process dies.
    pub fn has_exited(&mut self) -> bool {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .unwrap_or(true)
    }

    /// Joins the bounded stderr capture and returns its trimmed tail. The
    /// capture handle is consumed; later calls (including from `shutdown`)
    /// see no capture at all.
    pub fn stderr_tail(&mut self) -> Option<String> {
        let buffer = self.stderr.take()?.join().unwrap_or_default();
        stderr_tail_string(buffer)
    }

    /// Reads any buffered IPC messages; returns None when no complete line
    /// is available yet. Never blocks.
    pub fn poll_message(&mut self) -> Result<Option<IpcMessage>, String> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err("mpv IPC closed".to_string()),
            Ok(_) => {
                let value: serde_json::Value = serde_json::from_str(line.trim())
                    .map_err(|error| format!("malformed IPC JSON: {error}"))?;
                Ok(Some(decode_message(value)))
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::Interrupted =>
            {
                Ok(None)
            }
            Err(error) => Err(format!("IPC read failed: {error}")),
        }
    }

    /// Graceful shutdown: quit command, wait up to one second, then kill.
    /// The socket file is always unlinked.
    pub fn shutdown(&mut self) {
        let _ = self.send_command(&[serde_json::Value::from("quit")], None);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if std::time::Instant::now() >= deadline => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::info!(pid = self.child.id(), "mpv shutdown complete");
    }
}

impl Drop for MpvProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn decode_message(value: serde_json::Value) -> IpcMessage {
    if value.get("event").is_some() && value.get("request_id").is_none() {
        let name = value["event"].as_str().unwrap_or_default().to_string();
        let property = if name == "property-change" {
            value
                .get("name")
                .and_then(|name| name.as_str())
                .map(|name| PropertyChange {
                    name: name.to_string(),
                    // Missing or null data stays Null; consumers decide how
                    // to degrade (e.g. duration unavailable).
                    value: value
                        .get("data")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })
        } else {
            None
        };
        return IpcMessage::Event { name, property };
    }
    if let Some(id) = value.get("request_id").and_then(|id| id.as_u64()) {
        let error = value
            .get("error")
            .and_then(|error| error.as_str())
            .unwrap_or_default()
            .to_string();
        return IpcMessage::Reply {
            request_id: id,
            error,
        };
    }
    IpcMessage::Other(value)
}

/// Trims the bounded stderr capture down to a reportable one-line tail.
/// Returns None when nothing was captured.
fn stderr_tail_string(buffer: Vec<u8>) -> Option<String> {
    let text = String::from_utf8_lossy(&buffer);
    let tail: String = text
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("; ");
    if tail.is_empty() {
        None
    } else {
        Some(tail.chars().take(300).collect())
    }
}

fn merge_object(target: &mut serde_json::Value, extra: serde_json::Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), extra.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// Creates the private IPC socket path for `session`, prepares its
/// directory, and clears any stale socket left by a previous run.
fn open_private_socket(session: u64) -> Result<PathBuf, String> {
    let socket_path = private_socket_path(session)?;
    prepare_socket_dir(&socket_path)?;
    let _ = std::fs::remove_file(&socket_path);
    Ok(socket_path)
}

/// `--start`/`--pause` flags for the resume hints; omitted entirely when
/// no resume is requested so first opens behave exactly as before.
fn resume_args(resume: Option<(f64, bool)>) -> Vec<String> {
    match resume {
        None => Vec::new(),
        Some((position, paused)) => {
            let mut args = Vec::new();
            if position.is_finite() && position > 0.05 {
                args.push(format!("--start={position:.3}"));
            }
            if paused {
                args.push("--pause".to_string());
            }
            args
        }
    }
}

/// Flags shared by every mpv invocation: clean environment, quiet terminal,
/// no window of its own, private IPC socket, target media last. Shared
/// verbatim between the video and audio-only profiles.
fn common_mpv_args(path: &Path, socket_path: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "--no-config",
        "--terminal=no",
        "--really-quiet",
        "--idle=yes",
        "--force-window=no",
        "--profile=sw-fast",
        "--input-default-bindings=no",
    ]
    .iter()
    .map(|flag| (*flag).to_string())
    .collect();
    args.push(format!("--input-ipc-server={}", socket_path.display()));
    args.push(path.display().to_string());
    args
}

/// Audio-only output profile: no video decoder and no surface claimed at
/// all (`--audio-display=no` keeps cover art from forcing a window).
const AUDIO_ONLY_ARGS: &[&str] = &["--vo=null", "--no-video", "--audio-display=no"];

/// vo-kitty geometry flags pinning the video output to the reserved
/// rectangle. Unit contract (mpv manual): cols/rows are the surface size
/// in CELLS, width/height the available area in PIXELS, and left/top the
/// image origin in CELLS (1 = first column/row).
fn kitty_video_args(
    geometry_cells: (u16, u16),
    size_cells: (u16, u16),
    cell_pixels: (u16, u16),
) -> Vec<String> {
    let width_px = u32::from(size_cells.0) * u32::from(cell_pixels.0);
    let height_px = u32::from(size_cells.1) * u32::from(cell_pixels.1);
    let left_cells = u32::from(geometry_cells.0).saturating_add(1);
    let top_cells = u32::from(geometry_cells.1).saturating_add(1);
    vec![
        "--vo=kitty".to_string(),
        "--vo-kitty-alt-screen=no".to_string(),
        "--vo-kitty-config-clear=no".to_string(),
        "--vo-kitty-use-shm=no".to_string(),
        format!("--vo-kitty-cols={}", size_cells.0),
        format!("--vo-kitty-rows={}", size_cells.1),
        format!("--vo-kitty-left={left_cells}"),
        format!("--vo-kitty-top={top_cells}"),
        format!("--vo-kitty-width={width_px}"),
        format!("--vo-kitty-height={height_px}"),
    ]
}

/// Spawns the prepared command with the pdeathsig guard, captures bounded
/// stderr, waits briefly for the IPC socket, and wires the nonblocking
/// stream pair. Shared tail of `spawn` and `spawn_audio`.
fn finish_spawn(mut command: Command, socket_path: PathBuf) -> Result<MpvProcess, String> {
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0);
            Ok(())
        });
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start mpv: {error}"))?;
    let stderr = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(stderr) = stderr {
            use std::io::Read;
            let _ = stderr.take(8192).read_to_end(&mut buffer);
        }
        buffer
    });

    // Wait briefly for the socket, then connect.
    let mut connected = None;
    for _ in 0..100 {
        if let Ok(stream) = UnixStream::connect(&socket_path) {
            connected = Some(stream);
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            let _ = std::fs::remove_file(&socket_path);
            let detail = stderr_tail_string(stderr_thread.join().unwrap_or_default());
            return Err(match detail {
                Some(tail) => format!("mpv exited during startup ({status}): {tail}"),
                None => format!("mpv exited during startup ({status})"),
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let stream = connected.ok_or_else(|| "mpv IPC socket never appeared".to_string())?;
    stream
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure IPC stream: {error}"))?;
    let clone = stream
        .try_clone()
        .map_err(|error| format!("cannot clone IPC stream: {error}"))?;

    Ok(MpvProcess {
        child,
        socket_path,
        stream,
        reader: BufReader::new(clone),
        stderr: Some(stderr_thread),
        next_request_id: 1,
    })
}

/// `$XDG_RUNTIME_DIR/tui-explorer/mpv-{pid}-{session}.sock`, or
/// `$XDG_CACHE_HOME/tui-explorer/run/...` when the runtime dir is absent.
fn private_socket_path(session: u64) -> Result<PathBuf, String> {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() && Path::new(&runtime).is_absolute() {
            return Ok(Path::new(&runtime)
                .join("tui-explorer")
                .join(format!("mpv-{}-{session}.sock", std::process::id())));
        }
    }
    let cache = std::env::var("XDG_CACHE_HOME").unwrap_or_default();
    if cache.is_empty() || !Path::new(&cache).is_absolute() {
        return Err("XDG_RUNTIME_DIR or XDG_CACHE_HOME must be an absolute path".to_string());
    }
    Ok(Path::new(&cache)
        .join("tui-explorer")
        .join("run")
        .join(format!("mpv-{}-{session}.sock", std::process::id())))
}

/// Creates the parent directory of `socket` with mode 0700, enforcing the
/// mode even when it already exists.
fn prepare_socket_dir(socket: &Path) -> Result<(), String> {
    let dir = socket
        .parent()
        .ok_or_else(|| "socket has no parent directory".to_string())?;
    std::fs::create_dir_all(dir).map_err(|error| format!("cannot create socket dir: {error}"))?;
    set_private_mode(dir)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_mode(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("cannot enforce socket dir mode: {error}"))
}

#[cfg(not(unix))]
fn set_private_mode(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_and_video_profiles_differ_only_in_output_flags() {
        let common = common_mpv_args(Path::new("/tmp/tone.opus"), Path::new("/tmp/sock"));
        // Media path stays the final argument; IPC socket and no --vo
        // output flag live in the shared base.
        assert_eq!(common.last().map(String::as_str), Some("/tmp/tone.opus"));
        assert!(
            common
                .iter()
                .any(|arg| arg.starts_with("--input-ipc-server=")),
            "shared base must carry the IPC socket flag"
        );
        assert!(
            !common.iter().any(|arg| arg.starts_with("--vo")),
            "shared base must not pick an output profile"
        );

        let video = kitty_video_args((2, 3), (40, 20), (10, 20));
        let audio: Vec<String> = AUDIO_ONLY_ARGS.iter().map(ToString::to_string).collect();

        // Video pins vo-kitty to the reserved rectangle (cells +1 origin,
        // pixel sizes from the font metrics).
        assert!(video.contains(&"--vo=kitty".to_string()));
        assert!(video.contains(&"--vo-kitty-cols=40".to_string()));
        assert!(video.contains(&"--vo-kitty-rows=20".to_string()));
        assert!(video.contains(&"--vo-kitty-left=3".to_string()));
        assert!(video.contains(&"--vo-kitty-top=4".to_string()));
        assert!(video.contains(&"--vo-kitty-width=400".to_string()));
        assert!(video.contains(&"--vo-kitty-height=400".to_string()));

        // Audio claims no video surface whatsoever.
        assert_eq!(audio, ["--vo=null", "--no-video", "--audio-display=no"]);
        for flag in &video {
            assert!(!audio.contains(flag), "audio profile must not carry {flag}");
        }
    }
}
