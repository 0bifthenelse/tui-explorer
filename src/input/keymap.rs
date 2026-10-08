use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::action::{Action, ConflictDecision};
use crate::app::state::AppState;
use crate::app::state::Mode;
use crate::input::chords;
use crate::input::line;
use crate::settings::ViewMode;

pub fn map_key(key: KeyEvent, state: &AppState) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    match &state.mode {
        Mode::Command => match key.code {
            KeyCode::Enter => Some(Action::CommandSubmit),
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Backspace => Some(Action::CommandBackspace),
            KeyCode::Tab => Some(Action::CommandComplete),
            KeyCode::Up => Some(Action::CommandHistory(-1)),
            KeyCode::Down => Some(Action::CommandHistory(1)),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::CommandHistory(-1))
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::CommandHistory(1))
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::CommandChar(c))
            }
            _ => None,
        },
        Mode::Rename(_) => match key.code {
            KeyCode::Enter => Some(Action::PromptSubmit),
            KeyCode::Esc => Some(Action::Cancel),
            _ => line::edit_for(&key).map(Action::LineEdit),
        },
        Mode::Search(_) => match key.code {
            KeyCode::Enter => Some(Action::PromptSubmit),
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Down | KeyCode::Tab => Some(Action::SearchNext),
            KeyCode::Up | KeyCode::BackTab => Some(Action::SearchPrev),
            _ => line::edit_for(&key).map(Action::LineEdit),
        },
        Mode::QuickLook(_) => match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') => Some(Action::Cancel),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::QuickLookScroll(1)),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::QuickLookScroll(-1)),
            KeyCode::PageDown | KeyCode::Char(' ') => Some(Action::QuickLookScroll(20)),
            KeyCode::PageUp => Some(Action::QuickLookScroll(-20)),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::QuickLookScroll(10))
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::QuickLookScroll(-10))
            }
            KeyCode::Char('g') | KeyCode::Home => Some(Action::QuickLookScroll(-1_000_000)),
            KeyCode::Char('G') | KeyCode::End => Some(Action::QuickLookScroll(1_000_000)),
            KeyCode::Char('e') | KeyCode::Char('E') => Some(Action::EditFocused),
            KeyCode::Char('r') => Some(Action::OpenWithPrompt),
            _ => None,
        },
        Mode::Results(_) => match key.code {
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Enter => Some(Action::ResultsSubmit),
            KeyCode::Down | KeyCode::Tab => Some(Action::ResultsMove(1)),
            KeyCode::Up | KeyCode::BackTab => Some(Action::ResultsMove(-1)),
            KeyCode::PageDown => Some(Action::ResultsMove(10)),
            KeyCode::PageUp => Some(Action::ResultsMove(-10)),
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::ResultsMove(1))
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::ResultsMove(-1))
            }
            KeyCode::Backspace => Some(Action::ResultsBackspace),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::ResultsChar(c))
            }
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
            KeyCode::Char('k') | KeyCode::Char('b') => {
                Some(Action::ConflictChoice(ConflictDecision::KeepBoth))
            }
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
            KeyCode::Tab | KeyCode::Down => Some(Action::OpenWithCycle(1)),
            KeyCode::BackTab | KeyCode::Up => Some(Action::OpenWithCycle(-1)),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::OpenWithToggleRemember)
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::OpenWithChar(c))
            }
            _ => None,
        },
        Mode::Bookmarks(_) => match key.code {
            KeyCode::Esc => Some(Action::Cancel),
            KeyCode::Enter => Some(Action::BookmarkSubmit),
            KeyCode::Backspace => Some(Action::BookmarkBackspace),
            KeyCode::Tab => Some(Action::BookmarkSection(1)),
            KeyCode::BackTab => Some(Action::BookmarkSection(-1)),
            KeyCode::Delete => Some(Action::BookmarkDelete),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::BookmarkDelete)
            }
            KeyCode::Down => Some(Action::BookmarkMove(1)),
            KeyCode::Up => Some(Action::BookmarkMove(-1)),
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::BookmarkMove(1))
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::BookmarkMove(-1))
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::BookmarkChar(c))
            }
            _ => None,
        },
        Mode::Media(media) if media.sub_picker.is_some() => match key.code {
            KeyCode::Esc => Some(Action::SubPickerClose),
            KeyCode::Enter => Some(Action::SubPickerSubmit),
            KeyCode::Up | KeyCode::BackTab => Some(Action::SubPickerMove(-1)),
            KeyCode::Down | KeyCode::Tab => Some(Action::SubPickerMove(1)),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::SubPickerMove(-1))
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::SubPickerMove(1))
            }
            KeyCode::Backspace => Some(Action::SubPickerBackspace),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::SubPickerChar(c))
            }
            _ => None,
        },
        Mode::Media(_) => {
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => Some(Action::MediaTogglePause),
                KeyCode::Left if shift => Some(Action::MediaSeek(-60)),
                KeyCode::Right if shift => Some(Action::MediaSeek(60)),
                KeyCode::Left | KeyCode::Char('h') => Some(Action::MediaSeek(-15)),
                KeyCode::Right | KeyCode::Char('l') => Some(Action::MediaSeek(15)),
                KeyCode::Char('H') => Some(Action::MediaSeek(-60)),
                KeyCode::Char('L') => Some(Action::MediaSeek(60)),
                KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') => {
                    Some(Action::MediaVolume(5))
                }
                KeyCode::Down | KeyCode::Char('-') => Some(Action::MediaVolume(-5)),
                KeyCode::Char('m') => Some(Action::MediaMute),
                KeyCode::Char('s') => Some(Action::MediaStop),
                KeyCode::Char('f') => Some(Action::MediaToggleFullscreen),
                KeyCode::Char('n') => Some(Action::MediaNext),
                KeyCode::Char('p') => Some(Action::MediaPrev),
                KeyCode::Char('x') => Some(Action::MediaShuffle),
                KeyCode::Char('r') => Some(Action::MediaRepeat),
                KeyCode::Char(']') => Some(Action::MediaSpeed(1)),
                KeyCode::Char('[') => Some(Action::MediaSpeed(-1)),
                KeyCode::Backspace => Some(Action::MediaSpeed(0)),
                KeyCode::Char(c @ '0'..='9') => Some(Action::MediaSeekPercent(c as u8 - b'0')),
                KeyCode::Char('c') => Some(Action::MediaOpenSubs),
                KeyCode::Char('j') => Some(Action::MediaCycleSub),
                KeyCode::Char('v') => Some(Action::MediaToggleSubs),
                KeyCode::Char('z') => Some(Action::MediaSubDelay(-1)),
                KeyCode::Char('Z') => Some(Action::MediaSubDelay(1)),
                KeyCode::Char('a') => Some(Action::MediaCycleAudio),
                // Esc keeps music playing in the mini player; q stops.
                KeyCode::Esc => Some(Action::MediaMinimize),
                KeyCode::Char('q') => Some(Action::MediaClose),
                _ => None,
            }
        }
        Mode::Help => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let typing = !state.help_query.is_empty();
            match key.code {
                KeyCode::Esc => Some(Action::Cancel),
                KeyCode::Char('?') => Some(Action::ToggleHelp),
                KeyCode::Char('q') if !typing => Some(Action::ToggleHelp),
                KeyCode::Char('j') if !typing => Some(Action::HelpScroll(1)),
                KeyCode::Char('k') if !typing => Some(Action::HelpScroll(-1)),
                KeyCode::Down => Some(Action::HelpScroll(1)),
                KeyCode::Up => Some(Action::HelpScroll(-1)),
                KeyCode::Char('n') if ctrl => Some(Action::HelpScroll(1)),
                KeyCode::Char('p') if ctrl => Some(Action::HelpScroll(-1)),
                KeyCode::Char('d') if ctrl => Some(Action::HelpScroll(10)),
                KeyCode::Char('u') if ctrl => Some(Action::HelpScroll(-10)),
                KeyCode::PageDown => Some(Action::HelpScroll(20)),
                KeyCode::PageUp => Some(Action::HelpScroll(-20)),
                KeyCode::Home => Some(Action::HelpScroll(-1_000_000)),
                KeyCode::Backspace => Some(Action::HelpBackspace),
                KeyCode::Char(c) if !ctrl => Some(Action::HelpChar(c)),
                _ => None,
            }
        }
        Mode::Browser => map_browser_key(key, state),
    }
}

/// Browser keys resolve through the chord table (`input::chords`): a key
/// that starts a chord (or a count) becomes `ChordKey` and the reducer
/// keeps the pending sequence; complete single keys map directly.
fn map_browser_key(key: KeyEvent, state: &AppState) -> Option<Action> {
    let token = chords::token(&key)?;
    if token == "<Esc>" {
        return Some(Action::Cancel);
    }
    let pending = !state.pending_keys.is_empty() || state.pending_count.is_some();
    let is_count = state.pending_keys.is_empty()
        && token.len() == 1
        && token.chars().all(|c| c.is_ascii_digit())
        && (token != "0" || state.pending_count.is_some());
    if pending || is_count || chords::is_prefix(std::slice::from_ref(&token)) {
        return Some(Action::ChordKey(token));
    }
    // Grid tiles keep h/l and the arrows spatial.
    if state.view() == ViewMode::Grid {
        match token.as_str() {
            "h" | "<Left>" => return Some(Action::MoveLeft),
            "l" | "<Right>" => return Some(Action::MoveRight),
            _ => {}
        }
    }
    match chords::resolve(&[token], None) {
        chords::Resolution::Run(action) => Some(action),
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
        // List layout (default): h goes up a folder, l opens (ranger).
        assert!(matches!(
            map_key(key(KeyCode::Char('h')), &s),
            Some(Action::OpenParent)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('l')), &s),
            Some(Action::OpenFocused)
        ));
        let mut grid = state();
        grid.settings.view = ViewMode::Grid;
        assert!(matches!(
            map_key(key(KeyCode::Char('h')), &grid),
            Some(Action::MoveLeft)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Right), &grid),
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
        // `p` starts the paste chords (pp / po / pl).
        assert!(matches!(
            map_key(key(KeyCode::Char('p')), &s),
            Some(Action::ChordKey(ref t)) if t == "p"
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('/')), &s),
            Some(Action::EnterSearch)
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
            Some(Action::ChordKey(ref t)) if t == "g"
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
            suggestions: Vec::new(),
            suggestion: None,
            remember: true,
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
    fn chords_counts_and_pending_state() {
        let mut s = state();
        // A digit starts a count.
        assert!(matches!(
            map_key(key(KeyCode::Char('5')), &s),
            Some(Action::ChordKey(ref t)) if t == "5"
        ));
        // `0` alone is not a count.
        assert!(map_key(key(KeyCode::Char('0')), &s).is_none());
        // While a chord is pending every key feeds it.
        s.pending_keys = vec!["g".to_string()];
        assert!(matches!(
            map_key(key(KeyCode::Char('h')), &s),
            Some(Action::ChordKey(ref t)) if t == "h"
        ));
        // Esc always cancels.
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
            Some(Action::MediaMinimize)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('q')), &state),
            Some(Action::MediaClose)
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('7')), &state),
            Some(Action::MediaSeekPercent(7))
        ));
        assert!(matches!(
            map_key(key(KeyCode::Char('m')), &state),
            Some(Action::MediaMute)
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
