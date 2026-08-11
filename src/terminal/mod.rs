#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TtyFlags {
    pub raw_mode: bool,
    pub alternate_screen: bool,
    pub mouse_capture: bool,
    pub cursor_hidden: bool,
}

impl TtyFlags {
    pub fn active() -> Self {
        TtyFlags {
            raw_mode: true,
            alternate_screen: true,
            mouse_capture: true,
            cursor_hidden: true,
        }
    }

    pub fn restored() -> Self {
        TtyFlags::default()
    }
}

pub trait TtyDriver {
    fn apply(&mut self, flags: TtyFlags) -> std::io::Result<()>;
}

#[derive(Debug)]
pub struct TerminalSession<D: TtyDriver> {
    driver: D,
    flags: TtyFlags,
    closed: bool,
}

impl<D: TtyDriver> TerminalSession<D> {
    pub fn enter(mut driver: D) -> std::io::Result<Self> {
        let flags = TtyFlags::active();
        driver.apply(flags)?;
        Ok(TerminalSession {
            driver,
            flags,
            closed: false,
        })
    }

    pub fn flags(&self) -> TtyFlags {
        self.flags
    }

    pub fn suspend(&mut self) -> std::io::Result<()> {
        self.flags = TtyFlags::restored();
        self.driver.apply(self.flags)
    }

    pub fn resume(&mut self) -> std::io::Result<()> {
        self.flags = TtyFlags::active();
        self.driver.apply(self.flags)
    }

    pub fn restore(&mut self) {
        if self.closed {
            return;
        }
        self.flags = TtyFlags::restored();
        let _ = self.driver.apply(self.flags);
        self.closed = true;
    }
}

impl<D: TtyDriver> Drop for TerminalSession<D> {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(unix)]
pub mod crossterm_driver {
    use std::io::{self, Write};

    use crossterm::cursor::{Hide, Show};
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use crossterm::queue;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };

    use super::{TtyDriver, TtyFlags};

    pub struct CrosstermTty {
        out: std::io::Stdout,
    }

    impl CrosstermTty {
        pub fn new() -> Self {
            CrosstermTty {
                out: std::io::stdout(),
            }
        }
    }

    impl Default for CrosstermTty {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TtyDriver for CrosstermTty {
        fn apply(&mut self, flags: TtyFlags) -> io::Result<()> {
            if flags.raw_mode {
                enable_raw_mode()?;
            } else {
                disable_raw_mode()?;
            }
            if flags.alternate_screen {
                queue!(self.out, EnterAlternateScreen)?;
            } else {
                queue!(self.out, LeaveAlternateScreen)?;
            }
            if flags.mouse_capture {
                queue!(self.out, EnableMouseCapture)?;
            } else {
                queue!(self.out, DisableMouseCapture)?;
            }
            if flags.cursor_hidden {
                queue!(self.out, Hide)?;
            } else {
                queue!(self.out, Show)?;
            }
            self.out.flush()
        }
    }
}

pub fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut tty = crossterm_driver::CrosstermTty::new();
        let _ = tty.apply(TtyFlags::restored());
        original(info);
    }));
}

/// Tracks whether the next frame must be preceded by a full terminal clear.
/// Ratatui diffs against its own cell buffer, which goes stale whenever a
/// child process writes to the tty; a full clear is the only reliable
/// recovery. Efficient diffing is preserved for every other frame.
#[derive(Debug)]
pub struct RedrawGate {
    full: bool,
}

impl RedrawGate {
    pub fn new() -> Self {
        RedrawGate { full: true }
    }

    pub fn request_full(&mut self) {
        self.full = true;
    }

    pub fn take_full(&mut self) -> bool {
        std::mem::replace(&mut self.full, false)
    }
}

impl Default for RedrawGate {
    fn default() -> Self {
        RedrawGate::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct MockTty {
        applied: Rc<RefCell<Vec<TtyFlags>>>,
    }

    impl TtyDriver for MockTty {
        fn apply(&mut self, flags: TtyFlags) -> std::io::Result<()> {
            self.applied.borrow_mut().push(flags);
            Ok(())
        }
    }

    #[test]
    fn enter_and_drop_restores() {
        let applied = Rc::new(RefCell::new(Vec::new()));
        {
            let session = TerminalSession::enter(MockTty {
                applied: applied.clone(),
            })
            .unwrap();
            assert_eq!(session.flags(), TtyFlags::active());
        }
        let log = applied.borrow();
        assert_eq!(log.first(), Some(&TtyFlags::active()));
        assert_eq!(log.last(), Some(&TtyFlags::restored()));
    }

    #[test]
    fn suspend_resume_cycle() {
        let applied = Rc::new(RefCell::new(Vec::new()));
        let mut session = TerminalSession::enter(MockTty {
            applied: applied.clone(),
        })
        .unwrap();
        session.suspend().unwrap();
        assert_eq!(session.flags(), TtyFlags::restored());
        session.resume().unwrap();
        assert_eq!(session.flags(), TtyFlags::active());
        session.restore();
        session.restore();
        let log = applied.borrow();
        let restores = log.iter().filter(|f| **f == TtyFlags::restored()).count();
        assert_eq!(restores, 2);
    }
}
