use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::filesystem::MutationBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Move,
    Delete,
    /// Move to the freedesktop trash (undoable).
    Trash,
    /// Create symbolic links to the sources in `dest_dir`.
    Symlink,
    Encrypt,
    Decrypt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    Ask,
    Skip,
    Replace,
    /// Keep both: conflicting destinations get a free `name (2).ext`.
    KeepBoth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationPlan {
    pub kind: OperationKind,
    pub sources: Vec<PathBuf>,
    pub dest_dir: Option<PathBuf>,
    pub rename_to: Option<OsString>,
    pub policy: ConflictPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    NoSources,
    MissingDestination,
    RenameNeedsOneSource,
    RenameInvalidName,
    SamePath(PathBuf),
    IntoItself { src: PathBuf, dst: PathBuf },
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::NoSources => write!(f, "nothing selected"),
            OpError::MissingDestination => write!(f, "missing destination"),
            OpError::RenameNeedsOneSource => write!(f, "rename needs exactly one entry"),
            OpError::RenameInvalidName => write!(f, "invalid name for rename"),
            OpError::SamePath(p) => {
                write!(f, "source and destination are the same: {}", p.display())
            }
            OpError::IntoItself { src, dst } => write!(
                f,
                "cannot copy or move {} into itself ({})",
                src.display(),
                dst.display()
            ),
        }
    }
}

impl std::error::Error for OpError {}

pub fn destination_for(plan: &OperationPlan, source: &Path) -> Result<PathBuf, OpError> {
    match plan.kind {
        OperationKind::Copy | OperationKind::Move | OperationKind::Symlink => {
            let dir = plan.dest_dir.as_ref().ok_or(OpError::MissingDestination)?;
            // Symlinks may carry an explicit link name (`:symlink name`).
            if plan.kind == OperationKind::Symlink
                && let Some(name) = plan.rename_to.as_ref()
            {
                return Ok(dir.join(name));
            }
            let name = source.file_name().ok_or(OpError::MissingDestination)?;
            Ok(dir.join(name))
        }
        OperationKind::Delete | OperationKind::Trash => Ok(source.to_path_buf()),
        OperationKind::Encrypt | OperationKind::Decrypt => Ok(source.to_path_buf()),
    }
}

pub fn rename_target(plan: &OperationPlan, source: &Path) -> Result<PathBuf, OpError> {
    let name = plan.rename_to.as_ref().ok_or(OpError::RenameInvalidName)?;
    if name.is_empty() || name.to_string_lossy().contains('/') {
        return Err(OpError::RenameInvalidName);
    }
    let parent = source
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    Ok(parent.join(name))
}

pub fn validate(plan: &OperationPlan) -> Result<(), OpError> {
    if plan.sources.is_empty() {
        return Err(OpError::NoSources);
    }
    match plan.kind {
        OperationKind::Copy | OperationKind::Move | OperationKind::Symlink => {
            let dest_dir = plan.dest_dir.as_ref().ok_or(OpError::MissingDestination)?;
            for src in &plan.sources {
                let dst = destination_for(plan, src)?;
                // A keep-both copy into the source's own folder duplicates
                // it under a fresh name ("photo (2).png").
                let duplicate =
                    plan.kind == OperationKind::Copy && plan.policy == ConflictPolicy::KeepBoth;
                if dst == *src && !duplicate {
                    return Err(OpError::SamePath(src.clone()));
                }
                if dst == *src {
                    continue;
                }
                if dst.starts_with(src) {
                    return Err(OpError::IntoItself {
                        src: src.clone(),
                        dst: dst.clone(),
                    });
                }
                let _ = dest_dir;
            }
            Ok(())
        }
        OperationKind::Delete | OperationKind::Trash => Ok(()),
        OperationKind::Encrypt | OperationKind::Decrypt => Ok(()),
    }
}

pub fn validate_rename(plan: &OperationPlan) -> Result<PathBuf, OpError> {
    if plan.sources.len() != 1 {
        return Err(OpError::RenameNeedsOneSource);
    }
    let src = &plan.sources[0];
    let dst = rename_target(plan, src)?;
    if dst == *src {
        return Err(OpError::SamePath(src.clone()));
    }
    Ok(dst)
}

pub fn planned_destinations(plan: &OperationPlan) -> Vec<(PathBuf, PathBuf)> {
    plan.sources
        .iter()
        .filter_map(|src| {
            destination_for(plan, src)
                .ok()
                .map(|dst| (src.clone(), dst))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpOutcome {
    Done,
    Skipped,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpEntryResult {
    pub source: PathBuf,
    pub outcome: OpOutcome,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationReport {
    pub results: Vec<OpEntryResult>,
    /// (from, to) for every entry that changed location (moves, renames,
    /// trashing): tags follow these, and undo reverses them.
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// Paths created by the job (copies, symlinks): undo trashes them.
    pub created: Vec<PathBuf>,
    pub kind: Option<OperationKind>,
}

impl OperationReport {
    pub fn done_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.outcome == OpOutcome::Done)
            .count()
    }

    pub fn skipped_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.outcome == OpOutcome::Skipped)
            .count()
    }

    pub fn failed(&self) -> Vec<&OpEntryResult> {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, OpOutcome::Failed(_)))
            .collect()
    }
}

pub fn find_conflicts(
    plan: &OperationPlan,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<(PathBuf, PathBuf)> {
    planned_destinations(plan)
        .into_iter()
        .filter(|(src, dst)| dst != src && exists(dst))
        .filter(|_| plan.policy != ConflictPolicy::KeepBoth)
        .collect()
}

pub fn run_operation(
    plan: &OperationPlan,
    mutations: &dyn MutationBackend,
    mut progress: impl FnMut(PathBuf, usize, usize),
) -> OperationReport {
    let total = plan.sources.len();
    let mut report = OperationReport {
        kind: Some(plan.kind),
        ..OperationReport::default()
    };
    for (idx, src) in plan.sources.iter().enumerate() {
        progress(src.clone(), idx, total);
        let outcome = match plan.kind {
            OperationKind::Encrypt | OperationKind::Decrypt => {
                unreachable!("crypto jobs never run through run_operation")
            }
            OperationKind::Delete => match mutations.delete_entry(src, true) {
                Ok(()) => OpOutcome::Done,
                Err(e) => OpOutcome::Failed(e.to_string()),
            },
            OperationKind::Trash => match mutations.trash(src) {
                Ok(dest) => {
                    report.moves.push((src.clone(), dest));
                    OpOutcome::Done
                }
                Err(e) => OpOutcome::Failed(e.to_string()),
            },
            OperationKind::Copy | OperationKind::Move | OperationKind::Symlink => {
                let mut dst = match destination_for(plan, src) {
                    Ok(d) => d,
                    Err(e) => {
                        report.results.push(OpEntryResult {
                            source: src.clone(),
                            outcome: OpOutcome::Failed(e.to_string()),
                        });
                        continue;
                    }
                };
                let conflict =
                    mutations.exists(&dst) && (dst != *src || plan.kind == OperationKind::Copy);
                if conflict {
                    match plan.policy {
                        ConflictPolicy::Skip | ConflictPolicy::Ask => {
                            report.results.push(OpEntryResult {
                                source: src.clone(),
                                outcome: OpOutcome::Skipped,
                            });
                            continue;
                        }
                        ConflictPolicy::KeepBoth => {
                            let exists = |p: &Path| mutations.exists(p);
                            dst = crate::filesystem::unique_destination(&dst, &exists);
                        }
                        ConflictPolicy::Replace => {}
                    }
                }
                let replace = plan.policy == ConflictPolicy::Replace;
                let result = match plan.kind {
                    OperationKind::Copy => mutations.copy_entry(src, &dst, replace),
                    OperationKind::Symlink => {
                        if replace && mutations.exists(&dst) {
                            let _ = mutations.delete_entry(&dst, false);
                        }
                        mutations.symlink(src, &dst)
                    }
                    _ => mutations.move_entry(src, &dst, replace),
                };
                match result {
                    Ok(()) => {
                        if plan.kind == OperationKind::Move {
                            report.moves.push((src.clone(), dst));
                        } else {
                            report.created.push(dst);
                        }
                        OpOutcome::Done
                    }
                    Err(e) => OpOutcome::Failed(e.to_string()),
                }
            }
        };
        report.results.push(OpEntryResult {
            source: src.clone(),
            outcome,
        });
    }
    progress(
        plan.sources.last().cloned().unwrap_or_default(),
        total,
        total,
    );
    report
}

pub fn run_rename(
    plan: &OperationPlan,
    mutations: &dyn MutationBackend,
) -> Result<(PathBuf, PathBuf), String> {
    let src = plan.sources.first().ok_or("nothing selected")?;
    let dst = rename_target(plan, src).map_err(|e| e.to_string())?;
    if mutations.exists(&dst) && plan.policy != ConflictPolicy::Replace {
        return Err(format!("destination exists: {}", dst.display()));
    }
    mutations
        .move_entry(src, &dst, plan.policy == ConflictPolicy::Replace)
        .map_err(|e| e.to_string())?;
    Ok((src.clone(), dst))
}

/// Renames every (from, to) pair (bulk rename, undo). When one pair's
/// destination is another pair's source (swaps, shifted numbering) every
/// entry is staged through a temporary name first so nothing is clobbered.
/// A destination that already exists outside the set fails that pair.
pub fn run_moves(
    pairs: &[(PathBuf, PathBuf)],
    mutations: &dyn MutationBackend,
    kind: OperationKind,
) -> OperationReport {
    let sources: std::collections::BTreeSet<&PathBuf> = pairs.iter().map(|(f, _)| f).collect();
    let chained = pairs.iter().any(|(_, to)| sources.contains(to));
    let mut report = OperationReport {
        kind: Some(kind),
        ..Default::default()
    };
    let fail = |report: &mut OperationReport, source: &Path, err: String| {
        report.results.push(OpEntryResult {
            source: source.to_path_buf(),
            outcome: OpOutcome::Failed(err),
        });
    };
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
    for (i, (from, to)) in pairs.iter().enumerate() {
        if mutations.exists(to) && !sources.contains(to) {
            fail(
                &mut report,
                from,
                format!("{} already exists", to.display()),
            );
            continue;
        }
        if chained {
            let name = to
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let tmp = to.with_file_name(format!(".tui-explorer-rename-{i}-{name}"));
            match mutations.move_entry(from, &tmp, false) {
                Ok(()) => staged.push((from.clone(), tmp, to.clone())),
                Err(e) => fail(&mut report, from, e.to_string()),
            }
        } else {
            match mutations.move_entry(from, to, false) {
                Ok(()) => {
                    report.results.push(OpEntryResult {
                        source: from.clone(),
                        outcome: OpOutcome::Done,
                    });
                    report.moves.push((from.clone(), to.clone()));
                }
                Err(e) => fail(&mut report, from, e.to_string()),
            }
        }
    }
    for (from, tmp, to) in staged {
        match mutations.move_entry(&tmp, &to, false) {
            Ok(()) => {
                report.results.push(OpEntryResult {
                    source: from.clone(),
                    outcome: OpOutcome::Done,
                });
                report.moves.push((from, to));
            }
            Err(e) => {
                // Put the entry back under its old name when possible.
                let _ = mutations.move_entry(&tmp, &from, false);
                fail(&mut report, &from, e.to_string());
            }
        }
    }
    report
}

/// Reverses a journaled job: `moves` are (current, original) pairs and
/// `trash` lists paths the job created.
pub fn run_undo(
    moves: &[(PathBuf, PathBuf)],
    trash: &[PathBuf],
    mutations: &dyn MutationBackend,
) -> OperationReport {
    let mut report = run_moves(moves, mutations, OperationKind::Move);
    for path in trash {
        match mutations.trash(path) {
            Ok(dest) => {
                report.results.push(OpEntryResult {
                    source: path.clone(),
                    outcome: OpOutcome::Done,
                });
                report.moves.push((path.clone(), dest));
            }
            Err(e) => report.results.push(OpEntryResult {
                source: path.clone(),
                outcome: OpOutcome::Failed(e.to_string()),
            }),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_moves_swaps_through_temporary_names() {
        let log = crate::testing::RecordingMutations::new(
            ["/d/a", "/d/b"].iter().map(PathBuf::from).collect(),
        );
        let pairs = vec![
            (PathBuf::from("/d/a"), PathBuf::from("/d/b")),
            (PathBuf::from("/d/b"), PathBuf::from("/d/a")),
        ];
        let report = run_moves(&pairs, &log, OperationKind::Move);
        assert_eq!(report.done_count(), 2);
        assert_eq!(report.moves.len(), 2);
        assert!(log.exists(Path::new("/d/a")) && log.exists(Path::new("/d/b")));
    }

    #[test]
    fn run_moves_refuses_to_clobber() {
        let log = crate::testing::RecordingMutations::new(
            ["/d/a", "/d/c"].iter().map(PathBuf::from).collect(),
        );
        let pairs = vec![(PathBuf::from("/d/a"), PathBuf::from("/d/c"))];
        let report = run_moves(&pairs, &log, OperationKind::Move);
        assert_eq!(report.done_count(), 0);
        assert_eq!(report.failed().len(), 1);
    }

    fn plan(kind: OperationKind, sources: &[&str], dest: Option<&str>) -> OperationPlan {
        OperationPlan {
            kind,
            sources: sources.iter().map(PathBuf::from).collect(),
            dest_dir: dest.map(PathBuf::from),
            rename_to: None,
            policy: ConflictPolicy::Ask,
        }
    }

    #[test]
    fn rejects_same_path() {
        let p = plan(OperationKind::Copy, &["/a/f.txt"], Some("/a"));
        assert!(matches!(validate(&p), Err(OpError::SamePath(_))));
    }

    #[test]
    fn keep_both_copy_into_same_folder_duplicates() {
        let mut p = plan(OperationKind::Copy, &["/a/f.txt"], Some("/a"));
        p.policy = ConflictPolicy::KeepBoth;
        assert_eq!(validate(&p), Ok(()));
        let existing: std::collections::BTreeSet<PathBuf> =
            [PathBuf::from("/a/f.txt")].into_iter().collect();
        let m = crate::testing::RecordingMutations::new(existing);
        let report = run_operation(&p, &m, |_, _, _| {});
        assert_eq!(report.created, vec![PathBuf::from("/a/f (2).txt")]);
        assert_eq!(report.done_count(), 1);
    }

    #[test]
    fn trash_and_symlink_jobs_report_locations() {
        let p = plan(OperationKind::Trash, &["/a/x"], None);
        let m = crate::testing::RecordingMutations::default();
        let report = run_operation(&p, &m, |_, _, _| {});
        assert_eq!(
            report.moves,
            vec![(PathBuf::from("/a/x"), PathBuf::from("/trash/files/x"))]
        );
        let p = plan(OperationKind::Symlink, &["/a/x"], Some("/b"));
        let report = run_operation(&p, &m, |_, _, _| {});
        assert_eq!(report.created, vec![PathBuf::from("/b/x")]);
    }

    #[test]
    fn rejects_dir_into_itself() {
        let p = plan(OperationKind::Move, &["/a/data"], Some("/a/data/sub"));
        assert!(matches!(validate(&p), Err(OpError::IntoItself { .. })));
    }

    #[test]
    fn rejects_empty_sources() {
        let p = plan(OperationKind::Delete, &[], None);
        assert_eq!(validate(&p), Err(OpError::NoSources));
    }

    #[test]
    fn rename_target_rules() {
        let mut p = plan(OperationKind::Move, &["/a/old.txt"], None);
        p.rename_to = Some(OsString::from("new name.txt"));
        assert_eq!(
            rename_target(&p, Path::new("/a/old.txt")).unwrap(),
            PathBuf::from("/a/new name.txt")
        );
        p.rename_to = Some(OsString::from("bad/name"));
        assert!(rename_target(&p, Path::new("/a/old.txt")).is_err());
    }

    #[test]
    fn conflict_detection() {
        let p = plan(OperationKind::Copy, &["/a/x.txt", "/a/y.txt"], Some("/b"));
        let exists = |path: &Path| path == Path::new("/b/x.txt");
        let conflicts = find_conflicts(&p, &exists);
        assert_eq!(
            conflicts,
            vec![(PathBuf::from("/a/x.txt"), PathBuf::from("/b/x.txt"))]
        );
    }
}
