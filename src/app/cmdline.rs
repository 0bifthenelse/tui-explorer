//! Command-line assistance: live suggestions, Tab completion of command
//! names and paths, and Up/Down history.

use std::path::{Path, PathBuf};

use crate::app::state::AppState;
use crate::input::command::{COMMANDS, PATH_COMMANDS};

/// One suggestion row: text to insert and a description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub hint: String,
    pub desc: String,
}

/// The command word and the argument being typed, if any.
fn split_input(input: &str) -> (&str, Option<&str>) {
    let trimmed = input.trim_start();
    match trimmed.find(char::is_whitespace) {
        Some(i) => (&trimmed[..i], Some(trimmed[i..].trim_start())),
        None => (trimmed, None),
    }
}

/// Suggestions for the dropdown above the command line.
pub fn suggestions(state: &AppState) -> Vec<Suggestion> {
    let input = state.command_input.as_str();
    let (head, arg) = split_input(input);
    match arg {
        None => {
            let prefix = head.to_ascii_lowercase();
            COMMANDS
                .iter()
                .filter(|(name, _, _)| name.starts_with(&prefix))
                .map(|(name, hint, desc)| Suggestion {
                    label: name.to_string(),
                    hint: hint.to_string(),
                    desc: desc.to_string(),
                })
                .collect()
        }
        Some(_) if PATH_COMMANDS.contains(&head) => state
            .completions
            .iter()
            .map(|c| Suggestion {
                label: c.clone(),
                hint: String::new(),
                desc: String::new(),
            })
            .collect(),
        Some(_) => COMMANDS
            .iter()
            .filter(|(name, _, _)| *name == head)
            .map(|(name, hint, desc)| Suggestion {
                label: name.to_string(),
                hint: hint.to_string(),
                desc: desc.to_string(),
            })
            .collect(),
    }
}

fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for item in &items[1..] {
        let chars: Vec<char> = item.chars().collect();
        let n = prefix
            .iter()
            .zip(chars.iter())
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(n);
    }
    prefix.into_iter().collect()
}

/// Splits a path argument into (directory to list, typed prefix, text
/// before the prefix as typed).
pub fn path_parts(state: &AppState, arg: &str) -> (PathBuf, String, String) {
    let (dir_text, prefix) = match arg.rfind('/') {
        Some(i) => (&arg[..=i], &arg[i + 1..]),
        None => ("", arg),
    };
    let dir = if dir_text.is_empty() {
        state.browser.cwd.clone()
    } else {
        crate::app::reduce::resolve_user_path(state, dir_text)
    };
    (dir, prefix.to_string(), dir_text.to_string())
}

/// Tab: complete the command word locally, or request path candidates.
pub fn complete(state: &mut AppState) {
    let input = state.command_input.clone();
    let (head, arg) = split_input(&input);
    match arg {
        None => {
            let names: Vec<String> = COMMANDS
                .iter()
                .filter(|(name, _, _)| name.starts_with(head))
                .map(|(name, _, _)| name.to_string())
                .collect();
            match names.len() {
                0 => {}
                1 => state.command_input = format!("{} ", names[0]),
                _ => {
                    let prefix = common_prefix(&names);
                    if prefix.len() > head.len() {
                        state.command_input = prefix;
                    } else {
                        // Cycle through candidates on repeated Tab.
                        let idx = state
                            .completion_index
                            .map(|i| (i + 1) % names.len())
                            .unwrap_or(0);
                        state.completion_index = Some(idx);
                        state.command_input = names[idx].clone();
                        state.completion_cycle = true;
                    }
                }
            }
        }
        Some(arg) if PATH_COMMANDS.contains(&head) => {
            let candidates = path_candidates(state, arg);
            let (_, prefix, typed_dir) = path_parts(state, arg);
            match candidates.len() {
                0 => {}
                1 => {
                    state.command_input =
                        format!("{head} {typed_dir}{}", quote_if_needed(&candidates[0]));
                }
                _ => {
                    let common = common_prefix(&candidates);
                    if common.chars().count() > prefix.chars().count() {
                        state.command_input = format!("{head} {typed_dir}{common}");
                    } else {
                        let idx = state
                            .completion_index
                            .map(|i| (i + 1) % candidates.len())
                            .unwrap_or(0);
                        state.completion_index = Some(idx);
                        state.command_input =
                            format!("{head} {typed_dir}{}", quote_if_needed(&candidates[idx]));
                        state.completion_cycle = true;
                    }
                }
            }
            state.completions = candidates;
        }
        Some(_) => {}
    }
}

fn quote_if_needed(name: &str) -> String {
    if name.contains(' ') {
        format!("\"{name}\"")
    } else {
        name.to_string()
    }
}

/// Candidate entries (folders end with `/`) for a path argument, from the
/// listings the state already holds (current folder and Miller parent).
pub fn path_candidates(state: &AppState, arg: &str) -> Vec<String> {
    let (dir, prefix, _) = path_parts(state, arg);
    let lower = prefix.to_lowercase();
    let mut out: Vec<String> = listing_for(state, &dir)
        .into_iter()
        .filter(|(name, _)| {
            name.to_lowercase().starts_with(&lower)
                && (!name.starts_with('.') || prefix.starts_with('.'))
        })
        .map(|(name, is_dir)| if is_dir { format!("{name}/") } else { name })
        .collect();
    out.sort_by(|a, b| {
        b.ends_with('/')
            .cmp(&a.ends_with('/'))
            .then_with(|| crate::browser::natural_cmp(a, b))
    });
    out.truncate(200);
    out
}

fn listing_for(state: &AppState, dir: &Path) -> Vec<(String, bool)> {
    if dir == state.browser.cwd {
        return state
            .browser
            .entries
            .iter()
            .map(|e| (e.entry.display_name(), e.entry.is_dir_like()))
            .collect();
    }
    if let Some(entries) = state.side_listings.get(dir) {
        return entries
            .iter()
            .map(|e| (e.entry.display_name(), e.entry.is_dir_like()))
            .collect();
    }
    // Any other folder: read it directly. Completion is interactive and
    // read-only, so a synchronous directory listing is acceptable here.
    std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| {
                    let is_dir = std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir());
                    (e.file_name().to_string_lossy().into_owned(), is_dir)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Up/Down: walk the persisted command history.
pub fn history_step(state: &mut AppState, delta: isize) {
    let history = &state.settings.command_history;
    if history.is_empty() {
        return;
    }
    let len = history.len() as isize;
    let current = match state.history_cursor {
        Some(i) => i as isize,
        None => {
            state.command_draft = state.command_input.clone();
            len
        }
    };
    let next = (current + if delta < 0 { -1 } else { 1 }).clamp(0, len);
    if next == len {
        state.history_cursor = None;
        state.command_input = state.command_draft.clone();
    } else {
        state.history_cursor = Some(next as usize);
        state.command_input = history[next as usize].clone();
    }
}

/// Resets transient completion state after the input changed by typing.
pub fn reset(state: &mut AppState) {
    if !state.completion_cycle {
        state.completion_index = None;
    }
    state.completion_cycle = false;
    let input = state.command_input.clone();
    let (head, arg) = split_input(&input);
    state.completions = match arg {
        Some(arg) if PATH_COMMANDS.contains(&head) => path_candidates(state, arg),
        _ => Vec::new(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::builders::demo_state;

    #[test]
    fn suggestions_filter_by_prefix() {
        let mut s = demo_state(80, 24);
        s.command_input = "so".into();
        let sug = suggestions(&s);
        assert_eq!(sug.len(), 1);
        assert_eq!(sug[0].label, "sort");
        s.command_input = "c".into();
        let names: Vec<String> = suggestions(&s).into_iter().map(|x| x.label).collect();
        assert!(names.contains(&"cd".to_string()) && names.contains(&"create".to_string()));
    }

    #[test]
    fn tab_completes_unique_and_common_prefix() {
        let mut s = demo_state(80, 24);
        s.command_input = "bulk".into();
        complete(&mut s);
        assert_eq!(s.command_input, "bulkrename ");
        s.command_input = "unas".into();
        complete(&mut s);
        assert_eq!(s.command_input, "unassoc ");
    }

    #[test]
    fn history_walks_and_restores_draft() {
        let mut s = demo_state(80, 24);
        s.settings.command_history = vec!["cd /".into(), "mkdir x".into()];
        s.command_input = "draft".into();
        history_step(&mut s, -1);
        assert_eq!(s.command_input, "mkdir x");
        history_step(&mut s, -1);
        assert_eq!(s.command_input, "cd /");
        history_step(&mut s, 1);
        history_step(&mut s, 1);
        assert_eq!(s.command_input, "draft");
    }
}
