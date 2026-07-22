use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::action::{Action, ConflictDecision, MouseKind};
use crate::app::state::AppState;
use crate::app::state::Mode;

pub fn map_key(key: KeyEvent, state: &AppState) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    match &state.mode {
        Mode::Command => match key.code {
            KeyCode::Enter => Some(Action::CommandSubmit),
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Backspace => Some(Action::CommandBackspace),
            KeyCode::Char(c) => Some(Action::CommandChar(c)),
            _ => None,
        },
        Mode::Confirm(_) => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Some(Action::Confirm),
            KeyCode::Char('n') | KeyCode::Esc => Some(Action::Reject),
            _ => None,
        },
        Mode::Conflict(_) => match key.code {
            KeyCode::Char('c') | KeyCode::Esc => {
                Some(Action::ConflictChoice(ConflictDecision::Cancel))
            }
            KeyCode::Char('s') => Some(Action::ConflictChoice(ConflictDecision::Skip)),
            KeyCode::Char('r') => Some(Action::ConflictChoice(ConflictDecision::Replace)),
            _ => None,
        },
        Mode::TagPicker(picker) => {
            if picker.input.is_some() {
                return match key.code {
                    KeyCode::Enter => Some(Action::PickerSubmitNew),
                    KeyCode::Esc => Some(Action::PickerCancelInput),
                    KeyCode::Backspace => Some(Action::PickerBackspace),
                    KeyCode::Char(c) => Some(Action::PickerChar(c)),
                    _ => None,
                };
            }
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(Action::PickerMove(1)),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::PickerMove(-1)),
                KeyCode::Enter | KeyCode::Char(' ') => Some(Action::PickerToggle),
                KeyCode::Char('n') => Some(Action::PickerNew),
                KeyCode::Char('d') => Some(Action::PickerDelete),
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::Cancel),
                _ => None,
            }
        }
        Mode::ContextMenu(_) => match key.code {
            KeyCode::Char('j') | KeyCode::Down => Some(Action::ContextMove(1)),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::ContextMove(-1)),
            KeyCode::Enter => Some(Action::ContextChoose),
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Cancel),
            _ => None,
        },
        Mode::Help => match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => Some(Action::Cancel),
            _ => None,
        },
        Mode::Browser => map_browser_key(key),
    }
}

fn map_browser_key(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Char('h') | KeyCode::Left => Some(Action::OpenParent),
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => Some(Action::OpenFocused),
        KeyCode::Char('g') => Some(Action::KeyG),
        KeyCode::Char('G') => Some(Action::GotoLast),
        KeyCode::Char('u') if ctrl => Some(Action::HalfPageUp),
        KeyCode::Char('d') if ctrl => Some(Action::HalfPageDown),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        KeyCode::Char(' ') => Some(Action::ToggleSelect),
        KeyCode::Char('v') => Some(Action::ToggleVisual),
        KeyCode::Char('.') => Some(Action::ToggleHidden),
        KeyCode::Char('t') => Some(Action::QuickTag),
        KeyCode::Char('T') => Some(Action::OpenTagPicker),
        KeyCode::Char(':') => Some(Action::EnterCommand),
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char('q') => Some(Action::Quit),
        _ => None,
    }
}

pub struct ClickTracker {
    last: Option<(Instant, u16, u16)>,
    threshold: Duration,
}

impl ClickTracker {
    pub fn new() -> Self {
        ClickTracker {
            last: None,
            threshold: Duration::from_millis(500),
        }
    }

    pub fn register(&mut self, now: Instant, x: u16, y: u16) -> MouseKind {
        if let Some((when, lx, ly)) = self.last {
            if lx == x && ly == y && now.duration_since(when) <= self.threshold {
                self.last = None;
                return MouseKind::DoubleLeft;
            }
        }
        self.last = Some((now, x, y));
        MouseKind::Left
    }
}

impl Default for ClickTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn state() -> AppState {
        AppState::new(
            std::path::PathBuf::from("/d"),
            std::path::PathBuf::from("/home/u"),
        )
    }

    #[test]
    fn browser_keys() {
        let s = state();
        assert!(matches!(
            map_key(key(KeyCode::Char('j')), &s),
            Some(Action::MoveDown)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('k')), &s),
            Some(Action::MoveUp)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('h')), &s),
            Some(Action::OpenParent)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Enter), &s),
            Some(Action::OpenFocused)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('g')), &s),
            Some(Action::KeyG)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('G')), &s),
            Some(Action::GotoLast)
        ));
        assert!(matches!(
            map_key(key(KeyCode::PageUp), &s),
            Some(Action::PageUp)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char(' ')), &s),
            Some(Action::ToggleSelect)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('.')), &s),
            Some(Action::ToggleHidden)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char(':')), &s),
            Some(Action::EnterCommand)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('q')), &s),
            Some(Action::Quit)
        ));
    }

    #[test]
    fn ctrl_navigation() {
        let s = state();
        let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(matches!(map_key(ctrl_u, &s), Some(Action::HalfPageUp)));
        assert!(matches!(map_key(ctrl_d, &s), Some(Action::HalfPageDown)));
    }

    #[test]
    fn click_tracker_detects_double() {
        let mut tracker = ClickTracker::new();
        let t0 = Instant::now();
        assert_eq!(tracker.register(t0, 5, 5), MouseKind::Left);
        assert_eq!(
            tracker.register(t0 + Duration::from_millis(200), 5, 5),
            MouseKind::DoubleLeft
        );
        assert_eq!(
            tracker.register(t0 + Duration::from_millis(300), 5, 5),
            MouseKind::Left
        );
        assert_eq!(
            tracker.register(t0 + Duration::from_millis(900), 5, 5),
            MouseKind::Left
        );
        assert_eq!(
            tracker.register(t0 + Duration::from_millis(950), 6, 5),
            MouseKind::Left
        );
    }
}
