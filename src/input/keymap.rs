use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::action::{Action, ConflictDecision};
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
        Mode::Password(_) => match key.code {
            KeyCode::Enter => Some(Action::PasswordSubmit),
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Backspace => Some(Action::PasswordBackspace),
            KeyCode::Char(c) => Some(Action::PasswordChar(c)),
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
        KeyCode::Char('h') | KeyCode::Left => Some(Action::MoveLeft),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::MoveRight),
        KeyCode::Backspace => Some(Action::OpenParent),
        // Opening happens only through Enter, `e`, or a double left click.
        KeyCode::Enter | KeyCode::Char('e') => Some(Action::OpenFocused),
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
        KeyCode::Char('X') => Some(Action::EncryptToggle),
        KeyCode::Char('b') => Some(Action::ToggleSidebar),
        KeyCode::Char('p') => Some(Action::TogglePreview),
        KeyCode::Char('B') => Some(Action::ToggleBookmark),
        KeyCode::Char('T') => Some(Action::OpenTagPicker),
        KeyCode::Char(':') => Some(Action::EnterCommand),
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char('q') => Some(Action::Quit),
        _ => None,
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
            Some(Action::MoveLeft)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('l')), &s),
            Some(Action::MoveRight)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Backspace), &s),
            Some(Action::OpenParent)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('e')), &s),
            Some(Action::OpenFocused)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('X')), &s),
            Some(Action::EncryptToggle)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('b')), &s),
            Some(Action::ToggleSidebar)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('p')), &s),
            Some(Action::TogglePreview)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('B')), &s),
            Some(Action::ToggleBookmark)
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
}
