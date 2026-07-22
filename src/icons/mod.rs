use crate::filesystem::{DirEntry, EntryKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconSize {
    Compact,
    Small,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconVariant {
    Normal,
    Open,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    Folder,
    FolderOpen,
    FolderHidden,
    Symlink,
    Executable,
    SourceFile,
    Rust,
    TypeScript,
    JavaScript,
    C,
    Cpp,
    Python,
    Shell,
    Html,
    Css,
    Json,
    Toml,
    Yaml,
    Markdown,
    Image,
    Audio,
    Video,
    Archive,
    Pdf,
    Database,
    Git,
    CargoToml,
    CargoLock,
    PackageJson,
    Lockfile,
    Makefile,
    Docker,
    Config,
    Socket,
    Pipe,
    Device,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct IconDefinition {
    pub kind: IconKind,
    pub compact: &'static str,
    pub small: &'static str,
    pub large: &'static str,
}

const fn def(
    kind: IconKind,
    compact: &'static str,
    small: &'static str,
    large: &'static str,
) -> IconDefinition {
    IconDefinition {
        kind,
        compact,
        small,
        large,
    }
}

const ART_FOLDER: &str = "+--------+\n|        |\n| FOLDER |\n|        |\n+--------+";
const ART_FOLDER_OPEN: &str = "+--------+\n|        |\n| OPEN   |\n|        |\n+--------+";
const ART_FOLDER_HIDDEN: &str = "+--------+\n| . . .  |\n| HIDDEN |\n| . . .  |\n+--------+";
const ART_SYMLINK: &str = "+--------+\n|        |\n| LINK@> |\n|        |\n+--------+";
const ART_EXEC: &str = "+--------+\n|  >>>   |\n|  EXEC  |\n|  >>>   |\n+--------+";
const ART_SOURCE: &str = "+--------+\n| <      |\n|  SRC   |\n|    >   |\n+--------+";
const ART_RUST: &str = "+--------+\n|  rs    |\n|  RUST  |\n|  ()=>  |\n+--------+";
const ART_TS: &str = "+--------+\n|  ts    |\n|  TYP   |\n|  ESCR  |\n+--------+";
const ART_JS: &str = "+--------+\n|  js    |\n|  JAVA  |\n|  SCR   |\n+--------+";
const ART_C: &str = "+--------+\n|   c    |\n|  LANG  |\n|  *.h   |\n+--------+";
const ART_CPP: &str = "+--------+\n|  c++   |\n|  PLUS  |\n|  PLUS  |\n+--------+";
const ART_PY: &str = "+--------+\n|  py    |\n|  PYTH  |\n|  HON   |\n+--------+";
const ART_SH: &str = "+--------+\n|  $ sh  |\n|  SHELL |\n|  #!    |\n+--------+";
const ART_HTML: &str = "+--------+\n| <html> |\n|  MARK  |\n|  UP    |\n+--------+";
const ART_CSS: &str = "+--------+\n| {css}  |\n|  STYLE |\n|  #id   |\n+--------+";
const ART_JSON: &str = "+--------+\n| {\"k\":  |\n|  JSON  |\n|   1}   |\n+--------+";
const ART_TOML: &str = "+--------+\n| [toml] |\n|  k =   |\n|  \"v\"   |\n+--------+";
const ART_YAML: &str = "+--------+\n| - yaml |\n|  k:    |\n|   v    |\n+--------+";
const ART_MD: &str = "+--------+\n|  # md  |\n|  TEXT  |\n|  * *   |\n+--------+";
const ART_IMAGE: &str = "+--------+\n| .-^^-. |\n| IMAGE  |\n| pixels |\n+--------+";
const ART_AUDIO: &str = "+--------+\n|  ~~    |\n| AUDIO  |\n|  ~~    |\n+--------+";
const ART_VIDEO: &str = "+--------+\n| |>     |\n| VIDEO  |\n|  play  |\n+--------+";
const ART_ARCHIVE: &str = "+--------+\n| [====] |\n|  ZIP   |\n| [====] |\n+--------+";
const ART_PDF: &str = "+--------+\n|  PDF   |\n|  DOC   |\n|  A4    |\n+--------+";
const ART_DB: &str = "+--------+\n|  (==)  |\n|  DATA  |\n|  (==)  |\n+--------+";
const ART_GIT: &str = "+--------+\n|  o--o  |\n|  GIT   |\n|   \\    |\n+--------+";
const ART_CARGO_TOML: &str = "+--------+\n| [pkg]  |\n| CARGO  |\n|  TOML  |\n+--------+";
const ART_CARGO_LOCK: &str = "+--------+\n| [lock] |\n| CARGO  |\n|  LOCK  |\n+--------+";
const ART_PKG_JSON: &str = "+--------+\n| {name} |\n|  NODE  |\n|  PKG   |\n+--------+";
const ART_LOCKFILE: &str = "+--------+\n|  lock  |\n|  FILE  |\n|  [x]   |\n+--------+";
const ART_MAKE: &str = "+--------+\n|  make  |\n|  all:  |\n|  TAB   |\n+--------+";
const ART_DOCKER: &str = "+--------+\n| []==[] |\n| DOCKER |\n| ~~~~~  |\n+--------+";
const ART_CONFIG: &str = "+--------+\n|  cfg   |\n|  CONF  |\n|  = =   |\n+--------+";
const ART_SOCKET: &str = "+--------+\n|  o==o  |\n| SOCKET |\n|  o==o  |\n+--------+";
const ART_PIPE: &str = "+--------+\n|  ==    |\n|  PIPE  |\n|    ==  |\n+--------+";
const ART_DEVICE: &str = "+--------+\n|  dev   |\n| DEVICE |\n|  /dev  |\n+--------+";
const ART_UNKNOWN: &str = "+--------+\n|        |\n|   ??   |\n|        |\n+--------+";

static DEFINITIONS: &[IconDefinition] = &[
    def(IconKind::Folder, "+", "dir", ART_FOLDER),
    def(IconKind::FolderOpen, "-", "opn", ART_FOLDER_OPEN),
    def(IconKind::FolderHidden, ".", ".dr", ART_FOLDER_HIDDEN),
    def(IconKind::Symlink, "@", "lnk", ART_SYMLINK),
    def(IconKind::Executable, "*", "exe", ART_EXEC),
    def(IconKind::SourceFile, "#", "src", ART_SOURCE),
    def(IconKind::Rust, "r", "rs", ART_RUST),
    def(IconKind::TypeScript, "t", "ts", ART_TS),
    def(IconKind::JavaScript, "j", "js", ART_JS),
    def(IconKind::C, "c", "c", ART_C),
    def(IconKind::Cpp, "C", "c++", ART_CPP),
    def(IconKind::Python, "p", "py", ART_PY),
    def(IconKind::Shell, "s", "sh", ART_SH),
    def(IconKind::Html, "h", "htm", ART_HTML),
    def(IconKind::Css, "S", "css", ART_CSS),
    def(IconKind::Json, "{", "jsn", ART_JSON),
    def(IconKind::Toml, "T", "tml", ART_TOML),
    def(IconKind::Yaml, "y", "yml", ART_YAML),
    def(IconKind::Markdown, "m", "md", ART_MD),
    def(IconKind::Image, "i", "img", ART_IMAGE),
    def(IconKind::Audio, "a", "aud", ART_AUDIO),
    def(IconKind::Video, "v", "vid", ART_VIDEO),
    def(IconKind::Archive, "z", "zip", ART_ARCHIVE),
    def(IconKind::Pdf, "P", "pdf", ART_PDF),
    def(IconKind::Database, "d", "db", ART_DB),
    def(IconKind::Git, "g", "git", ART_GIT),
    def(IconKind::CargoToml, "R", "cgo", ART_CARGO_TOML),
    def(IconKind::CargoLock, "L", "clk", ART_CARGO_LOCK),
    def(IconKind::PackageJson, "n", "pkg", ART_PKG_JSON),
    def(IconKind::Lockfile, "l", "lck", ART_LOCKFILE),
    def(IconKind::Makefile, "M", "mk", ART_MAKE),
    def(IconKind::Docker, "D", "dkr", ART_DOCKER),
    def(IconKind::Config, "f", "cfg", ART_CONFIG),
    def(IconKind::Socket, "k", "soc", ART_SOCKET),
    def(IconKind::Pipe, "|", "pip", ART_PIPE),
    def(IconKind::Device, "b", "dev", ART_DEVICE),
    def(IconKind::Unknown, "?", "?", ART_UNKNOWN),
];

pub struct IconRegistry;

impl IconRegistry {
    pub fn new() -> Self {
        IconRegistry
    }

    pub fn get(&self, kind: IconKind) -> &'static IconDefinition {
        DEFINITIONS
            .iter()
            .find(|d| d.kind == kind)
            .unwrap_or(&DEFINITIONS[DEFINITIONS.len() - 1])
    }

    pub fn all(&self) -> &'static [IconDefinition] {
        DEFINITIONS
    }

    pub fn rendered_size(&self, kind: IconKind, size: IconSize) -> (u16, u16) {
        let d = self.get(kind);
        let text = match size {
            IconSize::Compact => d.compact,
            IconSize::Small => d.small,
            IconSize::Large => d.large,
        };
        let mut width = 0usize;
        let mut height = 0usize;
        for line in text.lines() {
            width = width.max(line.chars().count());
            height += 1;
        }
        (width as u16, height as u16)
    }

    pub fn glyph(&self, kind: IconKind, size: IconSize) -> &'static str {
        let d = self.get(kind);
        match size {
            IconSize::Compact => d.compact,
            IconSize::Small => d.small,
            IconSize::Large => d.large,
        }
    }
}

impl Default for IconRegistry {
    fn default() -> Self {
        Self::new()
    }
}

const COMPOUND_EXTENSIONS: &[(&str, IconKind)] = &[
    ("tar.gz", IconKind::Archive),
    ("tar.xz", IconKind::Archive),
    ("tar.bz2", IconKind::Archive),
    ("tar.zst", IconKind::Archive),
    ("d.ts", IconKind::TypeScript),
];

const EXTENSIONS: &[(&str, IconKind)] = &[
    ("rs", IconKind::Rust),
    ("ts", IconKind::TypeScript),
    ("tsx", IconKind::TypeScript),
    ("js", IconKind::JavaScript),
    ("jsx", IconKind::JavaScript),
    ("mjs", IconKind::JavaScript),
    ("cjs", IconKind::JavaScript),
    ("c", IconKind::C),
    ("h", IconKind::C),
    ("cpp", IconKind::Cpp),
    ("cc", IconKind::Cpp),
    ("cxx", IconKind::Cpp),
    ("hpp", IconKind::Cpp),
    ("hh", IconKind::Cpp),
    ("py", IconKind::Python),
    ("pyw", IconKind::Python),
    ("sh", IconKind::Shell),
    ("bash", IconKind::Shell),
    ("zsh", IconKind::Shell),
    ("html", IconKind::Html),
    ("htm", IconKind::Html),
    ("css", IconKind::Css),
    ("scss", IconKind::Css),
    ("sass", IconKind::Css),
    ("less", IconKind::Css),
    ("json", IconKind::Json),
    ("toml", IconKind::Toml),
    ("yaml", IconKind::Yaml),
    ("yml", IconKind::Yaml),
    ("md", IconKind::Markdown),
    ("markdown", IconKind::Markdown),
    ("txt", IconKind::Markdown),
    ("png", IconKind::Image),
    ("jpg", IconKind::Image),
    ("jpeg", IconKind::Image),
    ("gif", IconKind::Image),
    ("bmp", IconKind::Image),
    ("svg", IconKind::Image),
    ("webp", IconKind::Image),
    ("mp3", IconKind::Audio),
    ("flac", IconKind::Audio),
    ("ogg", IconKind::Audio),
    ("wav", IconKind::Audio),
    ("mp4", IconKind::Video),
    ("mkv", IconKind::Video),
    ("webm", IconKind::Video),
    ("avi", IconKind::Video),
    ("mov", IconKind::Video),
    ("zip", IconKind::Archive),
    ("tar", IconKind::Archive),
    ("gz", IconKind::Archive),
    ("xz", IconKind::Archive),
    ("bz2", IconKind::Archive),
    ("zst", IconKind::Archive),
    ("7z", IconKind::Archive),
    ("rar", IconKind::Archive),
    ("pdf", IconKind::Pdf),
    ("sqlite", IconKind::Database),
    ("sqlite3", IconKind::Database),
    ("db", IconKind::Database),
    ("sql", IconKind::Database),
    ("go", IconKind::SourceFile),
    ("java", IconKind::SourceFile),
    ("rb", IconKind::SourceFile),
    ("lua", IconKind::SourceFile),
    ("vim", IconKind::SourceFile),
    ("xml", IconKind::Config),
    ("ini", IconKind::Config),
    ("conf", IconKind::Config),
    ("cfg", IconKind::Config),
    ("env", IconKind::Config),
    ("desktop", IconKind::Config),
];

const EXACT_NAMES: &[(&str, IconKind)] = &[
    ("cargo.toml", IconKind::CargoToml),
    ("cargo.lock", IconKind::CargoLock),
    ("package.json", IconKind::PackageJson),
    ("package-lock.json", IconKind::Lockfile),
    ("yarn.lock", IconKind::Lockfile),
    ("pnpm-lock.yaml", IconKind::Lockfile),
    ("makefile", IconKind::Makefile),
    ("gnumakefile", IconKind::Makefile),
    ("dockerfile", IconKind::Docker),
    ("containerfile", IconKind::Docker),
    ("docker-compose.yml", IconKind::Docker),
    ("docker-compose.yaml", IconKind::Docker),
    (".gitignore", IconKind::Git),
    (".gitattributes", IconKind::Git),
    (".gitmodules", IconKind::Git),
    (".gitconfig", IconKind::Git),
    ("license", IconKind::Markdown),
    ("licence", IconKind::Markdown),
    ("copying", IconKind::Markdown),
    ("authors", IconKind::Markdown),
    ("contributors", IconKind::Markdown),
    ("changelog", IconKind::Markdown),
    ("changelog.md", IconKind::Markdown),
];

const SPECIAL_DIRS: &[(&str, IconKind)] = &[(".git", IconKind::Git)];

pub struct IconResolver {
    registry: IconRegistry,
}

impl IconResolver {
    pub fn new(registry: IconRegistry) -> Self {
        IconResolver { registry }
    }

    pub fn registry(&self) -> &IconRegistry {
        &self.registry
    }

    pub fn resolve(&self, entry: &DirEntry) -> IconKind {
        self.resolve_with(entry, IconVariant::Normal)
    }

    pub fn resolve_with(&self, entry: &DirEntry, variant: IconVariant) -> IconKind {
        match &entry.kind {
            EntryKind::Socket => return IconKind::Socket,
            EntryKind::Pipe => return IconKind::Pipe,
            EntryKind::BlockDevice | EntryKind::CharDevice => return IconKind::Device,
            EntryKind::Symlink { .. } => return IconKind::Symlink,
            EntryKind::Directory => {
                let lower = entry.name.to_string_lossy().to_lowercase();
                if let Some((_, kind)) = SPECIAL_DIRS.iter().find(|(n, _)| *n == lower) {
                    return *kind;
                }
                return match variant {
                    IconVariant::Open => IconKind::FolderOpen,
                    IconVariant::Hidden => IconKind::FolderHidden,
                    IconVariant::Normal => {
                        if entry.hidden {
                            IconKind::FolderHidden
                        } else {
                            IconKind::Folder
                        }
                    }
                };
            }
            _ => {}
        }
        let lower = entry.name.to_string_lossy().to_lowercase();
        if let Some((_, kind)) = EXACT_NAMES.iter().find(|(n, _)| *n == lower) {
            return *kind;
        }
        for (ext, kind) in COMPOUND_EXTENSIONS {
            if lower.ends_with(&format!(".{ext}")) {
                return *kind;
            }
        }
        if let Some((_, ext)) = lower.rsplit_once('.') {
            if !ext.is_empty() && lower.len() > ext.len() + 1 {
                if let Some((_, kind)) = EXTENSIONS.iter().find(|(e, _)| *e == ext) {
                    return *kind;
                }
            }
        }
        if entry.executable {
            return IconKind::Executable;
        }
        IconKind::Unknown
    }
}

impl Default for IconResolver {
    fn default() -> Self {
        Self::new(IconRegistry::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn entry(name: &str, kind: EntryKind) -> DirEntry {
        DirEntry {
            name: OsString::from(name),
            path: PathBuf::from(format!("/x/{name}")),
            kind,
            size: 0,
            mode: 0o644,
            modified: 0,
            executable: false,
            hidden: name.starts_with('.'),
            device: None,
            inode: None,
        }
    }

    #[test]
    fn resolution_table() {
        let resolver = IconResolver::default();
        let cases: &[(&str, EntryKind, IconKind)] = &[
            ("src", EntryKind::Directory, IconKind::Folder),
            (".git", EntryKind::Directory, IconKind::Git),
            (".hidden", EntryKind::Directory, IconKind::FolderHidden),
            (
                "link",
                EntryKind::Symlink { broken: false },
                IconKind::Symlink,
            ),
            ("sock", EntryKind::Socket, IconKind::Socket),
            ("fifo", EntryKind::Pipe, IconKind::Pipe),
            ("sda", EntryKind::BlockDevice, IconKind::Device),
            ("tty", EntryKind::CharDevice, IconKind::Device),
            ("main.rs", EntryKind::File, IconKind::Rust),
            ("app.ts", EntryKind::File, IconKind::TypeScript),
            ("app.tsx", EntryKind::File, IconKind::TypeScript),
            ("types.d.ts", EntryKind::File, IconKind::TypeScript),
            ("a.js", EntryKind::File, IconKind::JavaScript),
            ("a.jsx", EntryKind::File, IconKind::JavaScript),
            ("a.mjs", EntryKind::File, IconKind::JavaScript),
            ("a.cjs", EntryKind::File, IconKind::JavaScript),
            ("main.c", EntryKind::File, IconKind::C),
            ("main.h", EntryKind::File, IconKind::C),
            ("main.cpp", EntryKind::File, IconKind::Cpp),
            ("main.cc", EntryKind::File, IconKind::Cpp),
            ("main.hxx", EntryKind::File, IconKind::Unknown),
            ("main.hpp", EntryKind::File, IconKind::Cpp),
            ("main.hh", EntryKind::File, IconKind::Cpp),
            ("script.py", EntryKind::File, IconKind::Python),
            ("run.sh", EntryKind::File, IconKind::Shell),
            ("index.html", EntryKind::File, IconKind::Html),
            ("style.css", EntryKind::File, IconKind::Css),
            ("style.scss", EntryKind::File, IconKind::Css),
            ("data.json", EntryKind::File, IconKind::Json),
            ("cfg.toml", EntryKind::File, IconKind::Toml),
            ("cfg.yaml", EntryKind::File, IconKind::Yaml),
            ("cfg.yml", EntryKind::File, IconKind::Yaml),
            ("readme.md", EntryKind::File, IconKind::Markdown),
            ("notes.txt", EntryKind::File, IconKind::Markdown),
            ("pic.png", EntryKind::File, IconKind::Image),
            ("song.mp3", EntryKind::File, IconKind::Audio),
            ("film.mkv", EntryKind::File, IconKind::Video),
            ("pack.zip", EntryKind::File, IconKind::Archive),
            ("pack.tar.gz", EntryKind::File, IconKind::Archive),
            ("doc.pdf", EntryKind::File, IconKind::Pdf),
            ("tags.sqlite3", EntryKind::File, IconKind::Database),
            ("Cargo.toml", EntryKind::File, IconKind::CargoToml),
            ("Cargo.lock", EntryKind::File, IconKind::CargoLock),
            ("package.json", EntryKind::File, IconKind::PackageJson),
            ("package-lock.json", EntryKind::File, IconKind::Lockfile),
            ("Makefile", EntryKind::File, IconKind::Makefile),
            ("Dockerfile", EntryKind::File, IconKind::Docker),
            ("docker-compose.yml", EntryKind::File, IconKind::Docker),
            (".gitignore", EntryKind::File, IconKind::Git),
            ("settings.ini", EntryKind::File, IconKind::Config),
            ("mystery", EntryKind::File, IconKind::Unknown),
        ];
        for (name, kind, want) in cases {
            let got = resolver.resolve(&entry(name, kind.clone()));
            assert_eq!(got, *want, "name={name}");
        }
    }

    #[test]
    fn executable_beats_generic_but_not_known_extension() {
        let resolver = IconResolver::default();
        let mut e = entry("tool", EntryKind::File);
        e.executable = true;
        assert_eq!(resolver.resolve(&e), IconKind::Executable);
        let mut script = entry("build.sh", EntryKind::File);
        script.executable = true;
        assert_eq!(resolver.resolve(&script), IconKind::Shell);
    }

    #[test]
    fn sizes_are_ascii_and_consistent() {
        let registry = IconRegistry::new();
        for d in registry.all() {
            for size in [IconSize::Compact, IconSize::Small, IconSize::Large] {
                let glyph = registry.glyph(d.kind, size);
                assert!(glyph.is_ascii(), "{:?} {:?} not ascii", d.kind, size);
                let (w, h) = registry.rendered_size(d.kind, size);
                assert!(w > 0 && h > 0);
                match size {
                    IconSize::Compact => assert_eq!((w, h), (1, 1), "{:?}", d.kind),
                    IconSize::Small => assert_eq!(h, 1, "{:?}", d.kind),
                    IconSize::Large => assert!(h >= 3, "{:?}", d.kind),
                }
            }
        }
    }

    #[test]
    fn open_folder_variant() {
        let resolver = IconResolver::default();
        let e = entry("src", EntryKind::Directory);
        assert_eq!(
            resolver.resolve_with(&e, IconVariant::Open),
            IconKind::FolderOpen
        );
    }
}
