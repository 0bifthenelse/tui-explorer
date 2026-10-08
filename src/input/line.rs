//! A single-line text editor (cursor movement, word deletion, Home/End)
//! shared by inline rename, search, find and the address bar.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    pub text: String,
    /// Cursor position in chars (0..=len).
    pub cursor: usize,
}

/// One editing operation, produced from a key event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    WordLeft,
    WordRight,
    DeleteWordBack,
    ClearToStart,
    ClearToEnd,
    Set(String),
}

impl LineEdit {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        LineEdit { text, cursor }
    }

    pub fn with_cursor(text: impl Into<String>, cursor: usize) -> Self {
        let text = text.into();
        let cursor = cursor.min(text.chars().count());
        LineEdit { text, cursor }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Text before and after the cursor.
    pub fn split(&self) -> (&str, &str) {
        let at = self.byte_at(self.cursor);
        (&self.text[..at], &self.text[at..])
    }

    fn word_left(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && !chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && !chars[i].is_alphanumeric() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_alphanumeric() {
            i += 1;
        }
        i
    }

    pub fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Insert(c) => {
                let at = self.byte_at(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            Edit::Backspace => {
                if self.cursor > 0 {
                    let start = self.byte_at(self.cursor - 1);
                    let end = self.byte_at(self.cursor);
                    self.text.replace_range(start..end, "");
                    self.cursor -= 1;
                }
            }
            Edit::Delete => {
                if self.cursor < self.len() {
                    let start = self.byte_at(self.cursor);
                    let end = self.byte_at(self.cursor + 1);
                    self.text.replace_range(start..end, "");
                }
            }
            Edit::Left => self.cursor = self.cursor.saturating_sub(1),
            Edit::Right => self.cursor = (self.cursor + 1).min(self.len()),
            Edit::Home => self.cursor = 0,
            Edit::End => self.cursor = self.len(),
            Edit::WordLeft => self.cursor = self.word_left(),
            Edit::WordRight => self.cursor = self.word_right(),
            Edit::DeleteWordBack => {
                let target = self.word_left();
                let start = self.byte_at(target);
                let end = self.byte_at(self.cursor);
                self.text.replace_range(start..end, "");
                self.cursor = target;
            }
            Edit::ClearToStart => {
                let end = self.byte_at(self.cursor);
                self.text.replace_range(..end, "");
                self.cursor = 0;
            }
            Edit::ClearToEnd => {
                let start = self.byte_at(self.cursor);
                self.text.truncate(start);
            }
            Edit::Set(text) => *self = LineEdit::new(text),
        }
    }
}

/// Maps editing keys (readline-style) to an [`Edit`]; `None` for keys the
/// caller handles itself (Enter, Esc, Tab, Up/Down).
pub fn edit_for(key: &KeyEvent) -> Option<Edit> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    Some(match key.code {
        KeyCode::Char('a') if ctrl => Edit::Home,
        KeyCode::Char('e') if ctrl => Edit::End,
        KeyCode::Char('b') if ctrl => Edit::Left,
        KeyCode::Char('f') if ctrl => Edit::Right,
        KeyCode::Char('w') if ctrl => Edit::DeleteWordBack,
        KeyCode::Char('h') if ctrl => Edit::Backspace,
        KeyCode::Char('u') if ctrl => Edit::ClearToStart,
        KeyCode::Char('k') if ctrl => Edit::ClearToEnd,
        KeyCode::Char('d') if ctrl => Edit::Delete,
        KeyCode::Char('b') if alt => Edit::WordLeft,
        KeyCode::Char('f') if alt => Edit::WordRight,
        KeyCode::Char(c) if !ctrl && !alt => Edit::Insert(c),
        KeyCode::Backspace if ctrl || alt => Edit::DeleteWordBack,
        KeyCode::Backspace => Edit::Backspace,
        KeyCode::Delete => Edit::Delete,
        KeyCode::Left if ctrl => Edit::WordLeft,
        KeyCode::Right if ctrl => Edit::WordRight,
        KeyCode::Left => Edit::Left,
        KeyCode::Right => Edit::Right,
        KeyCode::Home => Edit::Home,
        KeyCode::End => Edit::End,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_move_delete() {
        let mut e = LineEdit::new("report.txt");
        e.apply(Edit::Home);
        e.apply(Edit::Insert('Q'));
        assert_eq!(e.text, "Qreport.txt");
        e.apply(Edit::End);
        e.apply(Edit::Backspace);
        assert_eq!(e.text, "Qreport.tx");
        e.apply(Edit::WordLeft);
        assert_eq!(e.split(), ("Qreport.", "tx"));
        e.apply(Edit::DeleteWordBack);
        assert_eq!(e.text, "tx");
        e.apply(Edit::ClearToEnd);
        assert!(e.is_empty());
    }

    #[test]
    fn multibyte_safe() {
        let mut e = LineEdit::new("日本");
        e.apply(Edit::Left);
        e.apply(Edit::Insert('x'));
        assert_eq!(e.text, "日x本");
        e.apply(Edit::Delete);
        assert_eq!(e.text, "日x");
        e.apply(Edit::Home);
        e.apply(Edit::Delete);
        assert_eq!(e.text, "x");
    }

    #[test]
    fn readline_keys() {
        let k = |code, m| KeyEvent::new(code, m);
        assert_eq!(
            edit_for(&k(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            Some(Edit::DeleteWordBack)
        );
        assert_eq!(
            edit_for(&k(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(Edit::Insert('x'))
        );
        assert_eq!(edit_for(&k(KeyCode::Enter, KeyModifiers::NONE)), None);
    }
}
