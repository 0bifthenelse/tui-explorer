use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::app::action::{Action, DirectorySnapshot};
use crate::app::effects::{Effect, EffectHandler};
use crate::app::state::AppState;
use crate::browser::EntryView;
use crate::filesystem::{DirEntry, FileSystem, MutationBackend, RecordedMutation};
use crate::operations::{
    ConflictPolicy, OpOutcome, OperationReport, find_conflicts, run_operation, run_rename,
};
use crate::tags::TagStore;

pub mod svg;

#[derive(Clone, Debug, Default)]
pub struct MemoryFileSystem {
    pub dirs: BTreeMap<PathBuf, Vec<DirEntry>>,
}

impl MemoryFileSystem {
    pub fn new() -> Self {
        MemoryFileSystem::default()
    }

    pub fn add_dir(&mut self, path: &Path) {
        let mut current = Some(path);
        while let Some(dir) = current {
            self.dirs.entry(dir.to_path_buf()).or_default();
            current = dir.parent();
        }
    }

    pub fn add_entry(&mut self, dir: &Path, entry: DirEntry) {
        if entry.kind.is_dir() {
            self.dirs.entry(entry.path.clone()).or_default();
        }
        self.dirs.entry(dir.to_path_buf()).or_default().push(entry);
    }

    pub fn known_paths(&self) -> BTreeSet<PathBuf> {
        let mut set: BTreeSet<PathBuf> = self.dirs.keys().cloned().collect();
        for entries in self.dirs.values() {
            for e in entries {
                set.insert(e.path.clone());
            }
        }
        set
    }
}

impl FileSystem for MemoryFileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        self.dirs.get(path).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no such directory: {}", path.display()),
            )
        })
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    fn exists(&self, path: &Path) -> bool {
        self.known_paths().contains(path)
    }
}

#[derive(Debug)]
pub struct RecordingMutations {
    pub log: Mutex<Vec<RecordedMutation>>,
    pub existing: BTreeSet<PathBuf>,
    pub fail_with: Option<String>,
}

impl RecordingMutations {
    pub fn new(existing: BTreeSet<PathBuf>) -> Self {
        RecordingMutations {
            log: Mutex::new(Vec::new()),
            existing,
            fail_with: None,
        }
    }

    pub fn recorded(&self) -> Vec<RecordedMutation> {
        self.log.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Default for RecordingMutations {
    fn default() -> Self {
        Self::new(BTreeSet::new())
    }
}

impl MutationBackend for RecordingMutations {
    fn copy_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        if let Ok(mut log) = self.log.lock() {
            log.push(RecordedMutation::Copy {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
                replace,
            });
        }
        if let Some(msg) = &self.fail_with {
            return Err(io::Error::other(msg.clone()));
        }
        Ok(())
    }

    fn move_entry(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        if let Ok(mut log) = self.log.lock() {
            log.push(RecordedMutation::Move {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
                replace,
            });
        }
        if let Some(msg) = &self.fail_with {
            return Err(io::Error::other(msg.clone()));
        }
        Ok(())
    }

    fn delete_entry(&self, path: &Path, recursive: bool) -> io::Result<()> {
        if let Ok(mut log) = self.log.lock() {
            log.push(RecordedMutation::Delete {
                path: path.to_path_buf(),
                recursive,
            });
        }
        if let Some(msg) = &self.fail_with {
            return Err(io::Error::other(msg.clone()));
        }
        Ok(())
    }

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        if let Ok(mut log) = self.log.lock() {
            log.push(RecordedMutation::CreateDir {
                path: path.to_path_buf(),
            });
        }
        if let Some(msg) = &self.fail_with {
            return Err(io::Error::other(msg.clone()));
        }
        Ok(())
    }

    fn create_file(&self, path: &Path) -> io::Result<()> {
        if let Ok(mut log) = self.log.lock() {
            log.push(RecordedMutation::CreateFile {
                path: path.to_path_buf(),
            });
        }
        if let Some(msg) = &self.fail_with {
            return Err(io::Error::other(msg.clone()));
        }
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.existing.contains(path)
    }
}

pub struct SyncHandler {
    pub fs: MemoryFileSystem,
    pub mutations: RecordingMutations,
    pub tags: TagStore,
    pub opened_with: Vec<(PathBuf, String, Vec<String>)>,
    pub started_media: Vec<(u64, PathBuf)>,
    pub media_commands: Vec<(u64, crate::media::MediaCommand)>,
    pub stopped_media: Vec<u64>,
    pub quit: bool,
    pub now: i64,
    pub bookmarks: Vec<PathBuf>,
    pub bookmark_store: crate::sidebar::MemoryBookmarks,
}

impl SyncHandler {
    pub fn new(fs: MemoryFileSystem) -> Self {
        let existing = fs.known_paths();
        SyncHandler {
            fs,
            mutations: RecordingMutations::new(existing),
            tags: TagStore::open_in_memory().expect("in-memory tag store"),
            opened_with: Vec::new(),
            quit: false,
            now: 1_700_000_000,
            started_media: Vec::new(),
            media_commands: Vec::new(),
            stopped_media: Vec::new(),
            bookmarks: Vec::new(),
            bookmark_store: crate::sidebar::MemoryBookmarks::default(),
        }
    }

    fn snapshot(&self, path: &Path) -> Result<DirectorySnapshot, String> {
        let raw = self.fs.read_dir(path).map_err(|e| e.to_string())?;
        let paths: Vec<PathBuf> = raw.iter().map(|e| e.path.clone()).collect();
        let tag_map = self
            .tags
            .tags_for_paths(&paths)
            .map_err(|e| e.to_string())?;
        let entries = raw
            .into_iter()
            .map(|entry| {
                let tags = tag_map.get(&entry.path).cloned().unwrap_or_default();
                EntryView { entry, tags }
            })
            .collect();
        let defs = self.tags.list_tags().map_err(|e| e.to_string())?;
        Ok(DirectorySnapshot {
            path: path.to_path_buf(),
            entries,
            defs,
        })
    }

    fn tag_applied(&self, message: String, last_tag: Option<String>) -> Vec<Action> {
        vec![Action::TagsApplied { message, last_tag }]
    }
}

impl EffectHandler for SyncHandler {
    fn handle(&mut self, effect: Effect) -> Vec<Action> {
        match effect {
            Effect::LoadDirectory(path) => vec![Action::DirectoryLoaded {
                result: self.snapshot(&path),
            }],
            Effect::RunOperation(plan) => {
                let exists = |p: &Path| self.mutations.exists(p);
                let conflicts = find_conflicts(&plan, &exists);
                if !conflicts.is_empty() && plan.policy == ConflictPolicy::Ask {
                    return vec![Action::ConflictsFound { plan, conflicts }];
                }
                let report = run_operation(&plan, &self.mutations, |_, _, _| {});
                vec![Action::OperationFinished { report }]
            }
            Effect::RunRename(plan) => match run_rename(&plan, &self.mutations) {
                Ok((from, to)) => {
                    let report = OperationReport {
                        results: vec![crate::operations::OpEntryResult {
                            source: from.clone(),
                            outcome: OpOutcome::Done,
                        }],
                        moves: vec![(from, to)],
                    };
                    vec![Action::OperationFinished { report }]
                }
                Err(err) => vec![Action::ErrorMessage(err)],
            },
            Effect::LoadPreview { key, name, is_dir } => {
                let result = crate::preview::load(&key.0, is_dir, &name);
                vec![Action::PreviewLoaded { key, result }]
            }
            Effect::Crypto {
                kind,
                target,
                password,
            } => {
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let secret = age::secrecy::SecretString::from(password.0.clone());
                let (done, failed) = crate::crypto::run_job(
                    kind,
                    std::slice::from_ref(&target),
                    &secret,
                    &cancel,
                    &mut |_, _, _| {},
                );
                vec![Action::CryptoFinished {
                    done,
                    failed: failed
                        .into_iter()
                        .map(|(p, e)| (p, e.to_string()))
                        .collect(),
                }]
            }
            Effect::ToggleBookmark(path) => {
                let mut bookmarks = self.bookmarks.clone();
                let added = self.bookmark_store.toggle(&mut bookmarks, &path);
                self.bookmarks = bookmarks.clone();
                vec![Action::BookmarksChanged {
                    bookmarks,
                    message: if added {
                        format!("bookmarked {}", path.display())
                    } else {
                        format!("removed bookmark {}", path.display())
                    },
                }]
            }
            Effect::OpenPathWith {
                path,
                program,
                args,
            } => {
                self.opened_with.push((path, program, args));
                Vec::new()
            }
            Effect::CreateEntry { path, is_dir } => {
                let result = if is_dir {
                    self.mutations.create_dir(&path)
                } else {
                    self.mutations.create_file(&path)
                };
                if let Err(e) = result {
                    return vec![Action::ErrorMessage(format!(
                        "could not create {}: {e}",
                        path.display()
                    ))];
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_os_string())
                    .unwrap_or_default();
                let kind = if is_dir {
                    crate::filesystem::EntryKind::Directory
                } else {
                    crate::filesystem::EntryKind::File
                };
                let hidden = name.to_string_lossy().starts_with('.');
                let entry = crate::filesystem::DirEntry {
                    name,
                    path: path.clone(),
                    kind,
                    size: 0,
                    mode: 0o644,
                    modified: self.now,
                    executable: false,
                    hidden,
                    device: None,
                    inode: None,
                };
                let parent = path.parent().map(Path::to_path_buf).unwrap_or(path);
                self.fs.add_entry(&parent, entry);
                vec![Action::DirectoryLoaded {
                    result: self.snapshot(&parent),
                }]
            }
            Effect::TagAssign {
                name,
                paths,
                create,
            } => {
                let result = if create {
                    self.tags.tag_paths(&paths, &name, self.now)
                } else {
                    match self.tags.find_tag(&name) {
                        Ok(Some(_)) => self.tags.tag_paths(&paths, &name, self.now),
                        Ok(None) => {
                            return vec![Action::ErrorMessage(format!("tag not found: {name}"))];
                        }
                        Err(e) => return vec![Action::ErrorMessage(e.to_string())],
                    }
                };
                match result {
                    Ok(count) => self.tag_applied(
                        format!(
                            "tagged {count} entr{} with [{name}]",
                            if count == 1 { "y" } else { "ies" }
                        ),
                        Some(name),
                    ),
                    Err(e) => vec![Action::ErrorMessage(e.to_string())],
                }
            }
            Effect::TagUnassign { name, paths } => match self.tags.untag_paths(&paths, &name) {
                Ok(count) => self.tag_applied(
                    format!(
                        "untagged {count} entr{} from [{name}]",
                        if count == 1 { "y" } else { "ies" }
                    ),
                    Some(name),
                ),
                Err(e) => vec![Action::ErrorMessage(e.to_string())],
            },
            Effect::TagCreate(name) => match self.tags.create_tag(&name, self.now) {
                Ok(_) => self.tag_applied(format!("created tag [{name}]"), Some(name)),
                Err(e) => vec![Action::ErrorMessage(e.to_string())],
            },
            Effect::TagDelete(name) => match self.tags.delete_tag(&name) {
                Ok(()) => self.tag_applied(format!("deleted tag [{name}]"), None),
                Err(e) => vec![Action::ErrorMessage(e.to_string())],
            },
            Effect::TagMove { from, to } => match self.tags.move_path(&from, &to, self.now) {
                Ok(_) => Vec::new(),
                Err(e) => vec![Action::ErrorMessage(e.to_string())],
            },
            Effect::StartMedia { session, path, .. } => {
                self.started_media.push((session, path));
                vec![Action::MediaBackendReady { session }]
            }
            Effect::MediaCommand { session, command } => {
                self.media_commands.push((session, command));
                Vec::new()
            }
            Effect::StopMedia { session } => {
                self.stopped_media.push(session);
                vec![Action::MediaStopped { session }]
            }
            Effect::Quit => {
                self.quit = true;
                Vec::new()
            }
        }
    }
}

pub fn drive(
    state: &mut AppState,
    handler: &mut SyncHandler,
    actions: impl IntoIterator<Item = Action>,
) {
    let mut queue: VecDeque<Action> = actions.into_iter().collect();
    let mut steps = 0usize;
    while let Some(action) = queue.pop_front() {
        steps += 1;
        if steps > 10_000 {
            panic!("event replay exceeded 10000 steps");
        }
        let effects = crate::app::reduce::reduce(state, action);
        for effect in effects {
            for follow in handler.handle(effect) {
                queue.push_back(follow);
            }
        }
    }
}

pub mod builders {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use crate::app::state::AppState;
    use crate::filesystem::{DirEntry, EntryKind};
    use crate::testing::MemoryFileSystem;

    pub const FIXED_TIME: i64 = 1_700_000_000;

    pub fn entry(
        dir: &Path,
        name: &str,
        kind: EntryKind,
        size: u64,
        mode: u32,
        modified: i64,
    ) -> DirEntry {
        let executable = kind == EntryKind::File && mode & 0o111 != 0;
        DirEntry {
            name: OsString::from(name),
            path: dir.join(name),
            kind,
            size,
            mode,
            modified,
            executable,
            hidden: name.starts_with('.'),
            device: None,
            inode: None,
        }
    }

    pub fn demo_root() -> PathBuf {
        PathBuf::from("/home/demo")
    }

    pub fn demo_fs() -> MemoryFileSystem {
        let root = demo_root();
        let mut fs = MemoryFileSystem::new();
        fs.add_dir(&root);
        fs.add_dir(&root.join("src"));
        fs.add_dir(&root.join("docs"));
        fs.add_dir(&root.join(".git"));
        let t = FIXED_TIME;
        let entries = [
            entry(&root, "src", EntryKind::Directory, 4096, 0o755, t - 400),
            entry(&root, "docs", EntryKind::Directory, 4096, 0o755, t - 9000),
            entry(&root, ".git", EntryKind::Directory, 4096, 0o755, t - 800),
            entry(&root, "Cargo.toml", EntryKind::File, 734, 0o644, t - 100),
            entry(&root, "Cargo.lock", EntryKind::File, 51_204, 0o644, t - 100),
            entry(&root, "package.json", EntryKind::File, 412, 0o644, t - 200),
            entry(&root, "main.rs", EntryKind::File, 2_048, 0o644, t - 300),
            entry(&root, "app.tsx", EntryKind::File, 5_120, 0o644, t - 300),
            entry(&root, "index.html", EntryKind::File, 1_024, 0o644, t - 700),
            entry(&root, "style.css", EntryKind::File, 2_560, 0o644, t - 700),
            entry(&root, "data.json", EntryKind::File, 8_192, 0o644, t - 500),
            entry(&root, "notes.md", EntryKind::File, 1_536, 0o644, t - 6000),
            entry(
                &root,
                "archive.tar.gz",
                EntryKind::File,
                1_048_576,
                0o644,
                t - 20_000,
            ),
            entry(
                &root,
                "song.mp3",
                EntryKind::File,
                5_242_880,
                0o644,
                t - 30_000,
            ),
            entry(
                &root,
                "photo.png",
                EntryKind::File,
                2_097_152,
                0o644,
                t - 40_000,
            ),
            entry(&root, "build.sh", EntryKind::File, 256, 0o755, t - 500),
            entry(&root, "deploy", EntryKind::File, 15_360, 0o755, t - 1000),
            entry(
                &root,
                "README link",
                EntryKind::Symlink { broken: false },
                9,
                0o777,
                t - 1000,
            ),
            entry(&root, ".hidden", EntryKind::File, 12, 0o600, t - 2000),
            entry(
                &root,
                "a very long file name that keeps going and going.txt",
                EntryKind::File,
                88,
                0o644,
                t - 3000,
            ),
        ];
        for e in entries {
            fs.add_entry(&root, e);
        }
        let src = root.join("src");
        fs.add_entry(
            &src,
            entry(&src, "main.rs", EntryKind::File, 1024, 0o644, t),
        );
        fs.add_entry(&src, entry(&src, "lib.rs", EntryKind::File, 2048, 0o644, t));
        fs
    }

    /// Demo filesystem plus one video entry, appended after the standard
    /// listing so existing hardcoded positions stay valid.
    pub fn demo_fs_with_video() -> MemoryFileSystem {
        let mut fs = demo_fs();
        let root = demo_root();
        fs.add_entry(
            &root,
            entry(
                &root,
                "clip.mkv",
                EntryKind::File,
                10_485_760,
                0o644,
                FIXED_TIME - 50_000,
            ),
        );
        fs
    }

    /// Larger deterministic filesystem used for the full-frame 1920x1080
    /// README screenshots: enough varied entries to fill a 240x60 grid.
    pub fn demo_fs_showcase() -> MemoryFileSystem {
        let root = demo_root();
        let mut fs = MemoryFileSystem::new();
        fs.add_dir(&root);
        fs.add_dir(&root.join("src"));
        let t = FIXED_TIME;
        let dirs = [
            "src", "docs", "assets", "media", "scripts", "tests", "backups", ".git",
        ];
        for (i, d) in dirs.iter().enumerate() {
            fs.add_entry(
                &root,
                entry(
                    &root,
                    d,
                    EntryKind::Directory,
                    4096,
                    0o755,
                    t - 400 * (i as i64 + 1),
                ),
            );
        }
        let files: &[(&str, u64, u32)] = &[
            ("Cargo.toml", 734, 0o644),
            ("Cargo.lock", 51_204, 0o644),
            ("Makefile", 312, 0o644),
            ("Dockerfile", 540, 0o644),
            ("package.json", 412, 0o644),
            ("README.md", 4_608, 0o644),
            ("CHANGELOG.md", 2_048, 0o644),
            ("main.rs", 2_048, 0o644),
            ("lib.rs", 8_192, 0o644),
            ("reduce.rs", 12_288, 0o644),
            ("state.rs", 9_216, 0o644),
            ("keymap.rs", 3_584, 0o644),
            ("app.tsx", 5_120, 0o644),
            ("index.ts", 1_024, 0o644),
            ("worker.js", 2_560, 0o644),
            ("main.c", 6_144, 0o644),
            ("vector.cpp", 4_096, 0o644),
            ("demo.py", 1_792, 0o644),
            ("build.sh", 256, 0o755),
            ("deploy.sh", 512, 0o755),
            ("index.html", 1_024, 0o644),
            ("style.css", 2_560, 0o644),
            ("data.json", 8_192, 0o644),
            ("config.toml", 640, 0o644),
            ("settings.yml", 896, 0o644),
            ("notes.md", 1_536, 0o644),
            ("archive.tar.gz", 1_048_576, 0o644),
            ("backup.zip", 2_097_152, 0o644),
            ("report.pdf", 524_288, 0o644),
            ("tags.sqlite3", 131_072, 0o644),
            ("photo.png", 2_097_152, 0o644),
            ("banner.jpg", 1_572_864, 0o644),
            ("icon.bmp", 262_144, 0o644),
            ("clip.gif", 786_432, 0o644),
            ("wallpaper.webp", 917_504, 0o644),
            ("song.mp3", 5_242_880, 0o644),
            ("voice.flac", 8_388_608, 0o644),
            ("screencast.mp4", 16_777_216, 0o644),
            ("talk.mkv", 33_554_432, 0o644),
            ("deploy", 15_360, 0o755),
            ("release.bin", 4_194_304, 0o755),
            ("report.txt", 12_288, 0o644),
            ("report.txt.age", 13_312, 0o600),
            ("photos.tar.age", 52_428_800, 0o600),
            ("quarterly figures 2023.csv", 20_480, 0o644),
            (
                "a very long file name that keeps going and going.txt",
                88,
                0o644,
            ),
            ("actions.rs", 7_680, 0o644),
            ("effects.rs", 3_072, 0o644),
            ("browser.rs", 11_264, 0o644),
            ("render.rs", 18_432, 0o644),
            ("icons.rs", 6_656, 0o644),
            ("tags.rs", 5_632, 0o644),
            ("crypto.rs", 4_608, 0o644),
            ("preview.rs", 3_328, 0o644),
            ("sidebar.rs", 4_096, 0o644),
            ("terminal.rs", 2_816, 0o644),
            ("config.rs", 1_792, 0o644),
            ("operations.rs", 9_728, 0o644),
            ("filesystem.rs", 5_120, 0o644),
            ("input.rs", 3_584, 0o644),
            ("util_test.rs", 2_048, 0o644),
            ("replay.rs", 6_144, 0o644),
            ("headless.rs", 4_608, 0o644),
            ("visual.rs", 3_072, 0o644),
            ("theme.css", 1_536, 0o644),
            ("reset.css", 640, 0o644),
            ("api.ts", 2_304, 0o644),
            ("router.ts", 1_920, 0o644),
            ("store.js", 2_816, 0o644),
            ("parse.py", 1_408, 0o644),
            ("train.py", 3_840, 0o644),
            ("render.c", 9_216, 0o644),
            ("matrix.cpp", 5_632, 0o644),
            ("linker.h", 1_280, 0o644),
            ("ci.yml", 1_152, 0o644),
            ("release.yml", 1_664, 0o644),
            ("lint.toml", 384, 0o644),
            ("schema.json", 12_288, 0o644),
            ("seed.json", 4_096, 0o644),
            ("guide.md", 7_168, 0o644),
            ("design.md", 5_632, 0o644),
            ("manual.pdf", 1_048_576, 0o644),
            ("invoice.pdf", 262_144, 0o644),
            ("sources.zip", 4_194_304, 0o644),
            ("assets.tar.gz", 8_388_608, 0o644),
            ("patch.tar", 2_097_152, 0o644),
            ("cover.png", 1_310_720, 0o644),
            ("avatar.png", 131_072, 0o644),
            ("screenshot.png", 524_288, 0o644),
            ("diagram.bmp", 393_216, 0o644),
            ("intro.gif", 1_572_864, 0o644),
            ("loop.webp", 655_360, 0o644),
            ("podcast.mp3", 12_582_912, 0o644),
            ("theme.flac", 6_291_456, 0o644),
            ("demo.mp4", 25_165_824, 0o644),
            ("clip.mkv", 10_485_760, 0o644),
            ("setup", 8_192, 0o755),
            ("bench", 24_576, 0o755),
            ("cache.db", 524_288, 0o644),
            ("history.db", 131_072, 0o644),
            ("draft.txt", 4_608, 0o644),
            ("todo.txt", 1_024, 0o644),
        ];
        for (i, (name, size, mode)) in files.iter().enumerate() {
            fs.add_entry(
                &root,
                entry(
                    &root,
                    name,
                    EntryKind::File,
                    *size,
                    *mode,
                    t - 100 * (i as i64 + 1),
                ),
            );
        }
        fs.add_entry(
            &root,
            entry(
                &root,
                "README link",
                EntryKind::Symlink { broken: false },
                9,
                0o777,
                t - 1000,
            ),
        );
        fs.add_entry(
            &root,
            entry(&root, ".hidden", EntryKind::File, 12, 0o600, t - 2000),
        );
        fs.add_entry(
            &root,
            entry(&root, ".env", EntryKind::File, 48, 0o600, t - 2100),
        );
        let src = root.join("src");
        for name in ["main.rs", "lib.rs", "mod.rs"] {
            fs.add_entry(&src, entry(&src, name, EntryKind::File, 1024, 0o644, t));
        }
        fs
    }

    pub fn demo_fs_with_non_utf8() -> MemoryFileSystem {
        use std::os::unix::ffi::OsStrExt;
        let mut fs = demo_fs();
        let root = demo_root();
        let raw = b"bad-\xff-name.bin";
        let name = std::ffi::OsStr::from_bytes(raw);
        fs.add_entry(
            &root,
            DirEntry {
                name: name.to_os_string(),
                path: root.join(name),
                kind: EntryKind::File,
                size: 64,
                mode: 0o644,
                modified: FIXED_TIME - 100,
                executable: false,
                hidden: false,
                device: None,
                inode: None,
            },
        );
        fs
    }

    pub fn demo_state(width: u16, height: u16) -> AppState {
        let root = demo_root();
        let mut state = AppState::new(root.clone(), root);
        state.width = width;
        state.height = height;
        state
    }
}
