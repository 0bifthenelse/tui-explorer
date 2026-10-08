//! Terminal graphics capability probe.
//!
//! Asks the terminal for Kitty graphics support, sixel support (DA1) and
//! the cell size in pixels, then waits for the Device Status Report reply
//! that every terminal sends. Unlike a background reader thread, the probe
//! polls stdin from the calling thread with a hard deadline, so no reader
//! is ever left behind to steal keystrokes from the event loop.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub kitty: bool,
    pub sixel: bool,
    /// Cell size in pixels (width, height).
    pub cell: Option<(u16, u16)>,
    /// The terminal answered the status report (the reply is complete).
    pub answered: bool,
}

/// The query: Kitty graphics, DA1, cell size, then DSR as a terminator.
pub fn query(in_tmux: bool) -> String {
    let kitty = "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
    let rest = "\x1b[c\x1b[16t\x1b[5n";
    if in_tmux {
        // tmux forwards the graphics query only inside a passthrough.
        format!(
            "\x1bPtmux;{}\x1b\\{rest}",
            kitty.replace('\x1b', "\x1b\x1b")
        )
    } else {
        format!("{kitty}{rest}")
    }
}

/// Parses everything the terminal sent back.
pub fn parse(reply: &[u8]) -> Probe {
    let text = String::from_utf8_lossy(reply);
    let mut probe = Probe {
        kitty: text.contains("_Gi=31;OK"),
        ..Probe::default()
    };
    // CSI sequences: ESC [ params final.
    let mut rest: &str = &text;
    while let Some(start) = rest.find("\x1b[") {
        let body = &rest[start + 2..];
        let Some(end) = body.find(|c: char| c.is_ascii_alphabetic()) else {
            break;
        };
        let params = &body[..end];
        let fin = &body[end..=end];
        match fin {
            "c" if params.starts_with('?') => {
                probe.sixel = params[1..].split(';').any(|p| p == "4");
            }
            "t" => {
                let nums: Vec<u16> = params.split(';').filter_map(|p| p.parse().ok()).collect();
                if nums.len() == 3 && nums[0] == 6 && nums[1] > 0 && nums[2] > 0 {
                    probe.cell = Some((nums[2], nums[1]));
                }
            }
            "n" if params == "0" => probe.answered = true,
            _ => {}
        }
        rest = &body[end + 1..];
    }
    probe
}

/// Runs the probe against the controlling terminal. Raw mode is enabled
/// for the duration and restored afterwards.
#[cfg(unix)]
pub fn run(timeout: Duration) -> Probe {
    use std::io::Write;
    use std::os::unix::io::AsRawFd;
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    if unsafe { libc::isatty(fd) } != 1 {
        return Probe::default();
    }
    if crossterm::terminal::enable_raw_mode().is_err() {
        return Probe::default();
    }
    let in_tmux = std::env::var_os("TMUX").is_some();
    let mut out = std::io::stdout();
    let sent = out
        .write_all(query(in_tmux).as_bytes())
        .and_then(|()| out.flush())
        .is_ok();
    let mut reply: Vec<u8> = Vec::new();
    let deadline = Instant::now() + timeout;
    if sent {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as libc::c_int) };
            if ready <= 0 {
                break;
            }
            let mut buf = [0u8; 256];
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            reply.extend_from_slice(&buf[..n as usize]);
            if parse(&reply).answered {
                break;
            }
        }
    }
    let _ = crossterm::terminal::disable_raw_mode();
    parse(&reply)
}

#[cfg(not(unix))]
pub fn run(_timeout: Duration) -> Probe {
    Probe::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kitty_sixel_and_cell_size() {
        let reply = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;4;22c\x1b[6;20;10t\x1b[0n";
        let probe = parse(reply);
        assert!(probe.kitty && probe.sixel && probe.answered);
        assert_eq!(probe.cell, Some((10, 20)));
    }

    #[test]
    fn plain_terminal_answers_status_only() {
        let probe = parse(b"\x1b[?1;2c\x1b[0n");
        assert!(!probe.kitty && !probe.sixel && probe.answered);
        assert_eq!(probe.cell, None);
    }

    #[test]
    fn tmux_wraps_graphics_query() {
        let q = query(true);
        assert!(q.starts_with("\x1bPtmux;\x1b\x1b_G"));
        assert!(q.ends_with("\x1b[5n"));
    }
}
