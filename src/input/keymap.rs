use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::action::{Action, ConflictDecision};
use crate::app::reduce::ESCAPE_COLS;
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
        Mode::OpenWith(_) => match key.code {
            KeyCode::Enter => Some(Action::OpenWithSubmit),
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Backspace => Some(Action::OpenWithBackspace),
            KeyCode::Char(c) => Some(Action::OpenWithChar(c)),
            _ => None,
        },
        Mode::Bookmarks(nav) => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let searching = nav.searching;
            let query_empty = nav.query.is_empty();
            match key.code {
                KeyCode::Esc => Some(Action::Cancel),
                KeyCode::Enter => Some(Action::BookmarkSubmit),
                // Movement works in both modes, so `j`/`k` never become
                // query text and results can be walked without leaving search.
                KeyCode::Char('j') | KeyCode::Down => Some(Action::BookmarkVim(1)),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::BookmarkVim(-1)),
                KeyCode::Char('n') if ctrl => Some(Action::BookmarkVim(1)),
                KeyCode::Char('p') if ctrl => Some(Action::BookmarkVim(-1)),
                KeyCode::Char('d') if ctrl => Some(Action::BookmarkHalfPage(1)),
                KeyCode::Char('u') if ctrl => Some(Action::BookmarkHalfPage(-1)),
                KeyCode::Char('G') => Some(Action::BookmarkLast),
                KeyCode::Char('g') => Some(Action::BookmarkFirst),
                KeyCode::Char('/') if !searching => Some(Action::BookmarkSearchStart),
                KeyCode::Backspace if searching && !query_empty => Some(Action::BookmarkBackspace),
                // Typing while browsing opens the editor and seeds it: the
                // reducer turns on search on the first character.
                KeyCode::Char(c) if !ctrl => Some(Action::BookmarkChar(c)),
                _ => None,
            }
        }
        Mode::Escape(_) => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::Cancel),
                KeyCode::Enter => Some(Action::EscapeApply),
                KeyCode::Char('j') | KeyCode::Down => {
                    Some(Action::EscapeMove(ESCAPE_COLS as isize))
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    Some(Action::EscapeMove(-(ESCAPE_COLS as isize)))
                }
                KeyCode::Char('h') | KeyCode::Left => Some(Action::EscapeMove(-1)),
                KeyCode::Char('l') | KeyCode::Right => Some(Action::EscapeMove(1)),
                KeyCode::Char('d') if ctrl => Some(Action::EscapeHalfPage(1)),
                KeyCode::Char('u') if ctrl => Some(Action::EscapeHalfPage(-1)),
                KeyCode::Char('G') => Some(Action::EscapeLast),
                KeyCode::Char('g') => Some(Action::EscapeKeyG),
                KeyCode::Home => Some(Action::EscapeFirst),
                KeyCode::End => Some(Action::EscapeLast),
                KeyCode::Char('f') => Some(Action::EscapeClearFilter),
                KeyCode::Char('v') => Some(Action::EscapeClearSelection),
                _ => None,
            }
        }
        Mode::Media(_) => match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::MediaTogglePause),
            KeyCode::Left | KeyCode::Char('h') => Some(Action::MediaSeek(-15)),
            KeyCode::Right | KeyCode::Char('l') => Some(Action::MediaSeek(15)),
            KeyCode::Up | KeyCode::Char('+') => Some(Action::MediaVolume(5)),
            KeyCode::Down | KeyCode::Char('-') => Some(Action::MediaVolume(-5)),
            KeyCode::Char('s') => Some(Action::MediaStop),
            KeyCode::Char('f') => Some(Action::MediaToggleFullscreen),
            KeyCode::Char('n') => Some(Action::MediaNext),
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::MediaClose),
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
        KeyCode::F(5) => Some(Action::Refresh),
        // Opening happens only through Enter, `e`, or a double left click.
        KeyCode::Enter | KeyCode::Char('e') => Some(Action::OpenFocused),
        KeyCode::Char('r') => Some(Action::OpenWithPrompt),
        KeyCode::Char('g') => Some(Action::KeyG),
        KeyCode::Char('G') => Some(Action::GotoLast),
        KeyCode::Char('u') if ctrl => Some(Action::HalfPageUp),
        KeyCode::Char('d') if ctrl => Some(Action::HalfPageDown),
        KeyCode::Char('f') if ctrl => Some(Action::EnterFilter),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        KeyCode::Char(' ') => Some(Action::ToggleSelect),
        KeyCode::Char('v') => Some(Action::ToggleVisual),
        KeyCode::Char('.') => Some(Action::ToggleHidden),
        KeyCode::Char('t') => Some(Action::QuickTag),
        KeyCode::Char('X') => Some(Action::EncryptToggle),
        KeyCode::Char('b') if ctrl => Some(Action::ToggleBookmark),
        KeyCode::Char('b') => Some(Action::ToggleSidebar),
        KeyCode::Char('p') => Some(Action::TogglePreview),
        KeyCode::Char('B') => Some(Action::OpenBookmarks),
        KeyCode::Char('T') => Some(Action::OpenTagPicker),
        KeyCode::Char(':') => Some(Action::EnterCommand),
        KeyCode::Char('/') => Some(Action::EnterFilter),
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
            map_key(key(KeyCode::F(5)), &s),
            Some(Action::Refresh)
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
            map_key(key(KeyCode::Char('r')), &s),
            Some(Action::OpenWithPrompt)
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
            map_key(key(KeyCode::Char('/')), &s),
            Some(Action::EnterFilter)
        ));
        assert!(matches!(
            map_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL), &s),
            Some(Action::EnterFilter)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('B')), &s),
            Some(Action::OpenBookmarks)
        ));
        assert!(matches!(
            map_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL), &s),
            Some(Action::ToggleBookmark)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('b')), &s),
            Some(Action::ToggleSidebar)
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
    fn open_with_mode_keys() {
        let mut s = state();
        s.mode = Mode::OpenWith(Box::new(crate::app::state::OpenWithState {
            target: std::path::PathBuf::from("/d/f.txt"),
            input: String::new(),
        }));
        assert!(matches!(
            map_key(key(KeyCode::Char('m')), &s),
            Some(Action::OpenWithChar('m'))
        ));
        assert!(matches!(
            map_key(key(KeyCode::Backspace), &s),
            Some(Action::OpenWithBackspace)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Enter), &s),
            Some(Action::OpenWithSubmit)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Esc), &s),
            Some(Action::Cancel)
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
    fn media_keys_map_to_transport_actions() {
        let mut state = state();
        state.mode = Mode::Media(Box::new(crate::app::state::MediaState::preparing(
            1,
            std::path::PathBuf::from("/track.wav"),
            crate::media::MediaKind::Audio,
        )));
        assert!(matches!(
            map_key(key(KeyCode::Char(' ')), &state),
            Some(Action::MediaTogglePause)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Left), &state),
            Some(Action::MediaSeek(-15))
        ));
        assert!(matches!(
            map_key(key(KeyCode::Right), &state),
            Some(Action::MediaSeek(15))
        ));
        assert!(matches!(
            map_key(key(KeyCode::Up), &state),
            Some(Action::MediaVolume(5))
        ));
        assert!(matches!(
            map_key(key(KeyCode::Down), &state),
            Some(Action::MediaVolume(-5))
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('s')), &state),
            Some(Action::MediaStop)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Esc), &state),
            Some(Action::MediaClose)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('f')), &state),
            Some(Action::MediaToggleFullscreen)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('n')), &state),
            Some(Action::MediaNext)
        ));
    }
}
