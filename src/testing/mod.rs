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

    fn exists(&self, path: &Path) -> bool {
        self.existing.contains(path)
    }
}

pub struct SyncHandler {
    pub fs: MemoryFileSystem,
    pub mutations: RecordingMutations,
    pub tags: TagStore,
    pub opened: Vec<PathBuf>,
    pub quit: bool,
    pub now: i64,
}

impl SyncHandler {
    pub fn new(fs: MemoryFileSystem) -> Self {
        let existing = fs.known_paths();
        SyncHandler {
            fs,
            mutations: RecordingMutations::new(existing),
            tags: TagStore::open_in_memory().expect("in-memory tag store"),
            opened: Vec::new(),
            quit: false,
            now: 1_700_000_000,
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
            Effect::OpenPath(path) => {
                self.opened.push(path);
                Vec::new()
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
