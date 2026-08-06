use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Copy { dest: String },
    Move { dest: String },
    Rename { name: String },
    Delete,
    Tag { name: String },
    Untag { name: String },
    Tags,
    Open,
    OpenWith { program: String, args: Vec<String> },
    Cd { path: String },
    Mkdir { name: String },
    Touch { name: String },
    SelectAll,
    InvertSelection,
    Deselect,
    Filter { query: String },
    ClearFilter,
    Sort { field: String },
    Refresh,
    Quit,
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    UnknownCommand(String),
    MissingArgument(&'static str),
    TooManyArguments(&'static str),
    UnterminatedQuote,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty command"),
            ParseError::UnknownCommand(c) => write!(f, "unknown command: {c}"),
            ParseError::MissingArgument(c) => write!(f, "missing argument for :{c}"),
            ParseError::TooManyArguments(c) => write!(f, "too many arguments for :{c}"),
            ParseError::UnterminatedQuote => write!(f, "unterminated quote"),
        }
    }
}

impl std::error::Error for ParseError {}

fn tokenize(input: &str) -> Result<Vec<String>, ParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_content = false;
    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
            continue;
        }
        if in_double {
            if c == '"' {
                in_double = false;
            } else if c == '\\' && chars.peek() == Some(&'"') {
                current.push('"');
                chars.next();
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                has_content = true;
            }
            '"' => {
                in_double = true;
                has_content = true;
            }
            c if c.is_whitespace() => {
                if has_content || !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    has_content = false;
                }
            }
            _ => {
                current.push(c);
                has_content = true;
            }
        }
    }
    if in_single || in_double {
        return Err(ParseError::UnterminatedQuote);
    }
    if has_content || !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn one_arg(cmd: &'static str, rest: &[String]) -> Result<String, ParseError> {
    match rest.len() {
        0 => Err(ParseError::MissingArgument(cmd)),
        1 => Ok(rest[0].clone()),
        _ => Err(ParseError::TooManyArguments(cmd)),
    }
}

fn no_args(cmd: &'static str, rest: &[String]) -> Result<(), ParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(ParseError::TooManyArguments(cmd))
    }
}

/// Splits a raw string into shell-like tokens, honoring single and double
/// quotes. Shared by [`parse`] (for `:open-with`) and the interactive
/// "open with" prompt, so both entry points quote the same way.
pub fn split_words(input: &str) -> Result<Vec<String>, ParseError> {
    tokenize(input)
}

pub fn parse(input: &str) -> Result<Command, ParseError> {
    let trimmed = input.trim_start_matches(':').trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    let tokens = tokenize(trimmed)?;
    let (head, rest) = tokens.split_first().ok_or(ParseError::Empty)?;
    let cmd = match head.as_str() {
        "copy" | "cp" => Command::Copy {
            dest: one_arg("copy", rest)?,
        },
        "move" | "mv" => Command::Move {
            dest: one_arg("move", rest)?,
        },
        "rename" => Command::Rename {
            name: one_arg("rename", rest)?,
        },
        "delete" | "rm" => {
            if !rest.is_empty() {
                return Err(ParseError::TooManyArguments("delete"));
            }
            Command::Delete
        }
        "tag" => Command::Tag {
            name: one_arg("tag", rest)?,
        },
        "untag" => Command::Untag {
            name: one_arg("untag", rest)?,
        },
        "tags" => {
            no_args("tags", rest)?;
            Command::Tags
        }
        "open" => {
            no_args("open", rest)?;
            Command::Open
        }
        "open-with" | "ow" => {
            let (program, args) = rest
                .split_first()
                .ok_or(ParseError::MissingArgument("open-with"))?;
            Command::OpenWith {
                program: program.clone(),
                args: args.to_vec(),
            }
        }
        "cd" => Command::Cd {
            path: one_arg("cd", rest)?,
        },
        "mkdir" => Command::Mkdir {
            name: one_arg("mkdir", rest)?,
        },
        "touch" => Command::Touch {
            name: one_arg("touch", rest)?,
        },
        "selectall" | "select-all" => {
            no_args("selectall", rest)?;
            Command::SelectAll
        }
        "invert" | "invertselection" => {
            no_args("invert", rest)?;
            Command::InvertSelection
        }
        "deselect" | "clearselection" => {
            no_args("deselect", rest)?;
            Command::Deselect
        }
        "filter" | "search" => Command::Filter {
            query: one_arg("filter", rest)?,
        },
        "clearfilter" | "clear-search" => {
            no_args("clearfilter", rest)?;
            Command::ClearFilter
        }
        "sort" => Command::Sort {
            field: one_arg("sort", rest)?,
        },
        "refresh" | "reload" => {
            no_args("refresh", rest)?;
            Command::Refresh
        }
        "quit" | "q" => {
            no_args("quit", rest)?;
            Command::Quit
        }
        "help" => {
            no_args("help", rest)?;
            Command::Help
        }
        other => return Err(ParseError::UnknownCommand(other.to_string())),
    };
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_commands() {
        assert_eq!(parse("delete"), Ok(Command::Delete));
        assert_eq!(parse(":tags"), Ok(Command::Tags));
        assert_eq!(parse("open"), Ok(Command::Open));
        assert_eq!(parse("quit"), Ok(Command::Quit));
        assert_eq!(parse("q"), Ok(Command::Quit));
        assert_eq!(parse("help"), Ok(Command::Help));
        assert_eq!(
            parse("search \"report 2026\""),
            Ok(Command::Filter {
                query: "report 2026".into()
            })
        );
        assert_eq!(parse("clearfilter"), Ok(Command::ClearFilter));
        assert_eq!(parse("reload"), Ok(Command::Refresh));
        assert_eq!(
            parse("sort modified"),
            Ok(Command::Sort {
                field: "modified".into()
            })
        );
    }

    #[test]
    fn paths_with_spaces() {
        assert_eq!(
            parse("copy /mnt/my files/backup"),
            Err(ParseError::TooManyArguments("copy"))
        );
        assert_eq!(
            parse("copy \"/mnt/my files/backup\""),
            Ok(Command::Copy {
                dest: "/mnt/my files/backup".into()
            })
        );
        assert_eq!(
            parse("move '/a b/c d'"),
            Ok(Command::Move {
                dest: "/a b/c d".into()
            })
        );
        assert_eq!(
            parse("cd \"~/My Documents\""),
            Ok(Command::Cd {
                path: "~/My Documents".into()
            })
        );
    }

    #[test]
    fn rename_and_tag() {
        assert_eq!(
            parse("rename new name.txt"),
            Err(ParseError::TooManyArguments("rename"))
        );
        assert_eq!(
            parse("rename \"new name.txt\""),
            Ok(Command::Rename {
                name: "new name.txt".into()
            })
        );
        assert_eq!(parse("tag fav"), Ok(Command::Tag { name: "fav".into() }));
        assert_eq!(
            parse("untag fav"),
            Ok(Command::Untag { name: "fav".into() })
        );
    }

    #[test]
    fn mkdir_touch() {
        assert_eq!(
            parse("mkdir new-folder"),
            Ok(Command::Mkdir {
                name: "new-folder".into()
            })
        );
        assert_eq!(parse("mkdir"), Err(ParseError::MissingArgument("mkdir")));
        assert_eq!(
            parse("mkdir a b"),
            Err(ParseError::TooManyArguments("mkdir"))
        );
        assert_eq!(
            parse("touch new-file.txt"),
            Ok(Command::Touch {
                name: "new-file.txt".into()
            })
        );
        assert_eq!(parse("touch"), Err(ParseError::MissingArgument("touch")));
    }

    #[test]
    fn selection_utilities() {
        assert_eq!(parse("selectall"), Ok(Command::SelectAll));
        assert_eq!(parse("select-all"), Ok(Command::SelectAll));
        assert_eq!(
            parse("selectall x"),
            Err(ParseError::TooManyArguments("selectall"))
        );
        assert_eq!(parse("invert"), Ok(Command::InvertSelection));
        assert_eq!(parse("invertselection"), Ok(Command::InvertSelection));
        assert_eq!(parse("deselect"), Ok(Command::Deselect));
        assert_eq!(parse("clearselection"), Ok(Command::Deselect));
    }

    #[test]
    fn open_with() {
        assert_eq!(
            parse("open-with mupdf"),
            Ok(Command::OpenWith {
                program: "mupdf".into(),
                args: vec![]
            })
        );
        assert_eq!(
            parse("ow mupdf -r 150"),
            Ok(Command::OpenWith {
                program: "mupdf".into(),
                args: vec!["-r".into(), "150".into()]
            })
        );
        assert_eq!(
            parse("open-with \"my app\" --flag"),
            Ok(Command::OpenWith {
                program: "my app".into(),
                args: vec!["--flag".into()]
            })
        );
        assert_eq!(
            parse("open-with"),
            Err(ParseError::MissingArgument("open-with"))
        );
    }

    #[test]
    fn split_words_quoting() {
        assert_eq!(
            split_words("mupdf -r 150").unwrap(),
            vec!["mupdf", "-r", "150"]
        );
        assert_eq!(
            split_words("'my app' --flag").unwrap(),
            vec!["my app", "--flag"]
        );
        assert_eq!(
            split_words("unterminated \""),
            Err(ParseError::UnterminatedQuote)
        );
    }

    #[test]
    fn errors() {
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert_eq!(parse(":"), Err(ParseError::Empty));
        assert_eq!(
            parse("bogus x"),
            Err(ParseError::UnknownCommand("bogus".into()))
        );
        assert_eq!(parse("copy"), Err(ParseError::MissingArgument("copy")));
        assert_eq!(parse("copy a b"), Err(ParseError::TooManyArguments("copy")));
        assert_eq!(parse("copy \"unclosed"), Err(ParseError::UnterminatedQuote));
        assert_eq!(
            parse("delete extra"),
            Err(ParseError::TooManyArguments("delete"))
        );
    }
}
