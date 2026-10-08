use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Copy {
        dest: String,
    },
    Move {
        dest: String,
    },
    Rename {
        name: String,
    },
    Delete,
    Tag {
        name: String,
    },
    Untag {
        name: String,
    },
    Tags,
    Open,
    OpenWith {
        program: String,
        args: Vec<String>,
    },
    Cd {
        path: String,
    },
    Mkdir {
        name: String,
    },
    Touch {
        name: String,
    },
    SelectAll,
    InvertSelection,
    Deselect,
    Filter {
        query: String,
    },
    ClearFilter,
    Sort {
        field: String,
    },
    Refresh,
    Quit,
    Help,
    /// Highlight and jump to matches (`n` / `N`).
    Search {
        query: String,
    },
    /// Recursive name search below the current folder.
    Find {
        pattern: String,
    },
    /// Recursive content search below the current folder.
    Grep {
        text: String,
    },
    /// Create a file, or a folder when the name ends with `/`; nested
    /// paths (`a/b/c.txt`) create their parents.
    Create {
        name: String,
    },
    Trash,
    Undo,
    Chmod {
        mode: String,
    },
    Symlink {
        name: String,
    },
    BulkRename,
    Du,
    /// Raw shell line with `%f %s %d` macros.
    Shell {
        command: String,
    },
    Tab {
        arg: Option<String>,
    },
    Mark {
        key: char,
    },
    View {
        mode: String,
    },
    Set {
        key: String,
        value: Option<String>,
    },
    BookmarkUrl {
        url: String,
        title: Option<String>,
    },
    Url {
        url: String,
    },
    Links,
    Assoc {
        ext: Option<String>,
        command: Option<String>,
    },
    Unassoc {
        ext: String,
    },
    Play,
    Pause,
    Next,
    Prev,
    Queue,
    Sub {
        path: String,
    },
}

/// Command catalog: (name, argument hint, description). Drives the
/// command-line suggestions, Tab completion and the help screen.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("cd", "<path>", "change folder (~, relative, file://)"),
    ("copy", "<dest>", "copy selection to a folder (alias cp)"),
    ("move", "<dest>", "move selection to a folder (alias mv)"),
    ("rename", "<name>", "rename the focused entry"),
    ("bulkrename", "", "rename the selection in $EDITOR"),
    ("create", "<name>", "new file; trailing / makes a folder"),
    ("mkdir", "<name>", "create a folder"),
    ("touch", "<name>", "create a file / bump its time"),
    ("delete", "", "delete forever (confirmed, alias rm)"),
    ("trash", "", "move selection to the trash"),
    ("undo", "", "undo the last move/rename/trash/copy"),
    ("symlink", "<name>", "symlink to the focused entry"),
    ("chmod", "<mode|+x>", "change permissions (644, u+x, -w)"),
    ("search", "<text>", "highlight matches, n / N jump"),
    ("filter", "<text>", "show only matching names"),
    ("clearfilter", "", "show every name again"),
    ("find", "<pattern>", "search names recursively"),
    ("grep", "<text>", "search file contents recursively"),
    ("sort", "<key>[-desc]", "name size modified type extension"),
    ("view", "<layout>", "list, grid or columns"),
    (
        "set",
        "<key> <value>",
        "animations hidden ascii icons video subs",
    ),
    ("du", "", "measure folder sizes"),
    ("shell", "<command>", "run a shell line (%f %s %d)"),
    ("tab", "[new|close|next|prev|N]", "manage tabs"),
    ("mark", "<key>", "mark this folder (jump with `key)"),
    ("bookmark-url", "<url> [title]", "save a web link"),
    ("links", "", "open the web links hub"),
    ("url", "<url>", "open a web URL or stream it"),
    ("open", "", "open the focused entry"),
    (
        "open-with",
        "<cmd> [args]",
        "open with a program (alias ow)",
    ),
    ("assoc", "[ext] [cmd]", "show or set a remembered opener"),
    ("unassoc", "<ext>", "forget a remembered opener"),
    ("tag", "<name>", "tag the selection"),
    ("untag", "<name>", "untag the selection"),
    ("tags", "", "tag picker"),
    ("selectall", "", "select every entry"),
    ("invert", "", "invert the selection"),
    ("deselect", "", "clear the selection"),
    ("play", "", "resume or reopen the player"),
    ("pause", "", "pause playback"),
    ("next", "", "next track"),
    ("prev", "", "previous track"),
    ("queue", "", "append the selection to the queue"),
    ("sub", "<path>", "load a subtitle file"),
    ("refresh", "", "reload the folder (alias reload)"),
    ("help", "", "keys and commands"),
    ("quit", "", "quit (alias q)"),
];

/// Commands whose argument is a filesystem path (Tab completes paths).
pub const PATH_COMMANDS: &[&str] = &["cd", "copy", "cp", "move", "mv", "sub", "create"];

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

/// Text after the command word, untouched (quotes preserved).
fn raw_rest(trimmed: &str) -> &str {
    let after = trimmed
        .find(char::is_whitespace)
        .map(|i| &trimmed[i..])
        .unwrap_or("");
    after.trim()
}

fn rest_text(cmd: &'static str, rest: &[String]) -> Result<String, ParseError> {
    if rest.is_empty() {
        Err(ParseError::MissingArgument(cmd))
    } else {
        Ok(rest.join(" "))
    }
}

pub fn parse(input: &str) -> Result<Command, ParseError> {
    let trimmed = input.trim_start_matches(':').trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    let head_word = trimmed.split_whitespace().next().unwrap_or("");
    // Raw-tail commands are parsed before tokenizing so quotes survive.
    match head_word {
        "shell" | "!" => {
            let command = raw_rest(trimmed);
            if command.is_empty() {
                return Err(ParseError::MissingArgument("shell"));
            }
            return Ok(Command::Shell {
                command: command.to_string(),
            });
        }
        "assoc" => {
            let tail = raw_rest(trimmed);
            if tail.is_empty() {
                return Ok(Command::Assoc {
                    ext: None,
                    command: None,
                });
            }
            let (ext, cmd) = match tail.split_once(char::is_whitespace) {
                Some((e, c)) => (e, Some(c.trim().to_string()).filter(|c| !c.is_empty())),
                None => (tail, None),
            };
            return Ok(Command::Assoc {
                ext: Some(ext.trim_start_matches('.').to_ascii_lowercase()),
                command: cmd,
            });
        }
        _ => {}
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
        "filter" => Command::Filter {
            query: rest_text("filter", rest)?,
        },
        "search" | "s" => Command::Search {
            query: rest_text("search", rest)?,
        },
        "find" | "locate" => Command::Find {
            pattern: rest_text("find", rest)?,
        },
        "grep" => Command::Grep {
            text: rest_text("grep", rest)?,
        },
        "create" | "new" => Command::Create {
            name: one_arg("create", rest)?,
        },
        "trash" => {
            no_args("trash", rest)?;
            Command::Trash
        }
        "undo" => {
            no_args("undo", rest)?;
            Command::Undo
        }
        "chmod" => Command::Chmod {
            mode: one_arg("chmod", rest)?,
        },
        "symlink" | "ln" => Command::Symlink {
            name: one_arg("symlink", rest)?,
        },
        "bulkrename" => {
            no_args("bulkrename", rest)?;
            Command::BulkRename
        }
        "du" => {
            no_args("du", rest)?;
            Command::Du
        }
        "tab" => Command::Tab {
            arg: match rest.len() {
                0 => None,
                1 => Some(rest[0].clone()),
                _ => return Err(ParseError::TooManyArguments("tab")),
            },
        },
        "mark" => {
            let key = one_arg("mark", rest)?;
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Command::Mark { key: c },
                _ => return Err(ParseError::MissingArgument("mark")),
            }
        }
        "view" | "layout" => Command::View {
            mode: one_arg("view", rest)?,
        },
        "set" => match rest {
            [] => return Err(ParseError::MissingArgument("set")),
            [key] => Command::Set {
                key: key.clone(),
                value: None,
            },
            [key, value] => Command::Set {
                key: key.clone(),
                value: Some(value.clone()),
            },
            _ => return Err(ParseError::TooManyArguments("set")),
        },
        "bookmark-url" | "link" => match rest {
            [] => return Err(ParseError::MissingArgument("bookmark-url")),
            [url] => Command::BookmarkUrl {
                url: url.clone(),
                title: None,
            },
            [url, title @ ..] => Command::BookmarkUrl {
                url: url.clone(),
                title: Some(title.join(" ")),
            },
        },
        "url" | "open-url" | "browse" => Command::Url {
            url: one_arg("url", rest)?,
        },
        "links" => {
            no_args("links", rest)?;
            Command::Links
        }
        "unassoc" => Command::Unassoc {
            ext: one_arg("unassoc", rest)?
                .trim_start_matches('.')
                .to_ascii_lowercase(),
        },
        "play" => {
            no_args("play", rest)?;
            Command::Play
        }
        "pause" => {
            no_args("pause", rest)?;
            Command::Pause
        }
        "next" => {
            no_args("next", rest)?;
            Command::Next
        }
        "prev" | "previous" => {
            no_args("prev", rest)?;
            Command::Prev
        }
        "queue" | "enqueue" => {
            no_args("queue", rest)?;
            Command::Queue
        }
        "sub" | "subtitle" => Command::Sub {
            path: one_arg("sub", rest)?,
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
            Ok(Command::Search {
                query: "report 2026".into()
            })
        );
        assert_eq!(
            parse("filter my notes"),
            Ok(Command::Filter {
                query: "my notes".into()
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
    fn ranger_style_commands() {
        assert_eq!(
            parse("shell echo \"%f\" | wc -c"),
            Ok(Command::Shell {
                command: "echo \"%f\" | wc -c".into()
            })
        );
        assert_eq!(parse("shell"), Err(ParseError::MissingArgument("shell")));
        assert_eq!(
            parse("create notes/today.md"),
            Ok(Command::Create {
                name: "notes/today.md".into()
            })
        );
        assert_eq!(parse("new x/"), Ok(Command::Create { name: "x/".into() }));
        assert_eq!(parse("mark a"), Ok(Command::Mark { key: 'a' }));
        assert_eq!(
            parse("set animations off"),
            Ok(Command::Set {
                key: "animations".into(),
                value: Some("off".into())
            })
        );
        assert_eq!(
            parse("assoc .PDF zathura --fork"),
            Ok(Command::Assoc {
                ext: Some("pdf".into()),
                command: Some("zathura --fork".into())
            })
        );
        assert_eq!(
            parse("bookmark-url https://docs.rs Rust docs"),
            Ok(Command::BookmarkUrl {
                url: "https://docs.rs".into(),
                title: Some("Rust docs".into())
            })
        );
        assert_eq!(
            parse("find *.rs"),
            Ok(Command::Find {
                pattern: "*.rs".into()
            })
        );
        assert_eq!(
            parse("tab new"),
            Ok(Command::Tab {
                arg: Some("new".into())
            })
        );
        assert_eq!(parse("chmod +x"), Ok(Command::Chmod { mode: "+x".into() }));
    }

    #[test]
    fn catalog_names_all_parse_or_need_args() {
        for (name, args, _) in COMMANDS {
            let result = parse(name);
            if args.starts_with('<') {
                assert!(
                    matches!(result, Err(ParseError::MissingArgument(_))),
                    "{name}: {result:?}"
                );
            } else {
                assert!(result.is_ok(), "{name}: {result:?}");
            }
        }
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
