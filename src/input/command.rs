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
    Cd { path: String },
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
            if !rest.is_empty() {
                return Err(ParseError::TooManyArguments("tags"));
            }
            Command::Tags
        }
        "open" => {
            if !rest.is_empty() {
                return Err(ParseError::TooManyArguments("open"));
            }
            Command::Open
        }
        "cd" => Command::Cd {
            path: one_arg("cd", rest)?,
        },
        "quit" | "q" => {
            if !rest.is_empty() {
                return Err(ParseError::TooManyArguments("quit"));
            }
            Command::Quit
        }
        "help" => {
            if !rest.is_empty() {
                return Err(ParseError::TooManyArguments("help"));
            }
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
