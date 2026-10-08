//! Desktop integration: the system clipboard (OSC 52 plus `wl-copy` /
//! `xclip` / `xsel`), detached program launches and `$EDITOR` round trips.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// OSC 52 "set clipboard" sequence: works over SSH and inside tmux (with
/// `set-clipboard on`) where no clipboard program can reach the desktop.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Puts `text` on the clipboard: OSC 52 always, plus the first desktop
/// clipboard tool available. Succeeds when either path worked.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut stdout = std::io::stdout();
    let osc_ok = stdout
        .write_all(osc52(text).as_bytes())
        .and_then(|()| stdout.flush())
        .is_ok();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    let mut tools: Vec<(&str, &[&str])> = Vec::new();
    if wayland {
        tools.push(("wl-copy", &[]));
    }
    if x11 {
        tools.push(("xclip", &["-selection", "clipboard"]));
        tools.push(("xsel", &["--clipboard", "--input"]));
    }
    if cfg!(target_os = "macos") {
        tools.push(("pbcopy", &[]));
    }
    for (program, args) in tools {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().is_ok_and(|s| s.success()) {
            return Ok(());
        }
    }
    if osc_ok {
        Ok(())
    } else {
        Err("no clipboard available".to_string())
    }
}

/// Starts `program args…` in its own process group with null stdio, so it
/// outlives neither the TUI's terminal nor blocks it. A reaper thread
/// collects the exit status.
pub fn spawn_detached(program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The user's editor command split into words (`$VISUAL`, `$EDITOR`, vi).
pub fn editor_command() -> Vec<String> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty())
        .and_then(|v| crate::input::command::split_words(&v).ok())
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| vec!["vi".to_string()])
}

/// Writes `lines` to a temporary file, lets the user edit it in their
/// editor (the terminal must already be released), and returns the
/// edited lines.
pub fn edit_lines(lines: &[String]) -> Result<Vec<String>, String> {
    let path = std::env::temp_dir().join(format!(
        "tui-explorer-bulkrename-{}.txt",
        std::process::id()
    ));
    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(&path, body).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    let editor = editor_command();
    let status = Command::new(&editor[0])
        .args(&editor[1..])
        .arg(&path)
        .status()
        .map_err(|e| format!("could not start {}: {e}", editor[0]));
    let result = match status {
        Ok(s) if s.success() => std::fs::read_to_string(&path)
            .map(|text| text.lines().map(str::to_string).collect())
            .map_err(|e| format!("could not read back {}: {e}", path.display())),
        Ok(s) => Err(format!("{} exited with {s}", editor[0])),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&path);
    result
}

/// Runs a shell in `cwd` with the terminal released: interactive when
/// `command` is `None`, otherwise `$SHELL -c command` followed by a
/// "press Enter" pause so the output can be read.
pub fn run_shell(command: Option<&str>, cwd: &Path) -> Result<(), String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    let mut cmd = Command::new(&shell);
    cmd.current_dir(cwd);
    if let Some(line) = command {
        cmd.arg("-c").arg(line);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("could not start {shell}: {e}"))?;
    if command.is_some() {
        let mut out = std::io::stdout();
        let _ = write!(
            out,
            "\n\x1b[38;2;255;125;39m▌\x1b[0m {} — press Enter to return ",
            if status.success() {
                "done".to_string()
            } else {
                format!("exited with {status}")
            }
        );
        let _ = out.flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn osc52_wraps_base64_payload() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }
}
