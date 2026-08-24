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
    pub fn spawn(
        path: &Path,
        geometry_cells: (u16, u16),
        size_cells: (u16, u16),
        cell_pixels: (u16, u16),
        session: u64,
    ) -> Result<Self, String> {
        let socket_path = private_socket_path(session)?;
        prepare_socket_dir(&socket_path)?;
        let _ = std::fs::remove_file(&socket_path);

        // vo-kitty unit contract (mpv manual): cols/rows are the surface
        // size in CELLS, width/height the available area in PIXELS, and
        // left/top the image origin in CELLS (1 = first column/row).
        let width_px = u32::from(size_cells.0) * u32::from(cell_pixels.0);
        let height_px = u32::from(size_cells.1) * u32::from(cell_pixels.1);
        let left_cells = u32::from(geometry_cells.0).saturating_add(1);
        let top_cells = u32::from(geometry_cells.1).saturating_add(1);

        let mut command = Command::new("mpv");
        command
            .arg("--no-config")
            .arg("--terminal=no")
            .arg("--really-quiet")
            .arg("--idle=yes")
            .arg("--force-window=no")
            .arg("--profile=sw-fast")
            .arg("--input-default-bindings=no")
            .arg("--vo=kitty")
            .arg("--vo-kitty-alt-screen=no")
            .arg("--vo-kitty-config-clear=no")
            .arg("--vo-kitty-use-shm=no")
            .arg(format!("--vo-kitty-cols={}", size_cells.0))
            .arg(format!("--vo-kitty-rows={}", size_cells.1))
            .arg(format!("--vo-kitty-left={left_cells}"))
            .arg(format!("--vo-kitty-top={top_cells}"))
            .arg(format!("--vo-kitty-width={width_px}"))
            .arg(format!("--vo-kitty-height={height_px}"))
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .arg(path)
            .stdin(Stdio::null())
            // The Kitty video output paints through stdout; discarding it
            // would leave audio-only playback. stderr stays piped for
            // bounded diagnostics, stdin is unused.
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped());

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
