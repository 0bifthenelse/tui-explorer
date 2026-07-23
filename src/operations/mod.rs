use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::filesystem::MutationBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Move,
    Delete,
    Encrypt,
    Decrypt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    Ask,
    Skip,
    Replace,
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
        OperationKind::Copy | OperationKind::Move => {
            let dir = plan.dest_dir.as_ref().ok_or(OpError::MissingDestination)?;
            let name = source.file_name().ok_or(OpError::MissingDestination)?;
            Ok(dir.join(name))
        }
        OperationKind::Delete => Ok(source.to_path_buf()),
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
        OperationKind::Copy | OperationKind::Move => {
            let dest_dir = plan.dest_dir.as_ref().ok_or(OpError::MissingDestination)?;
            for src in &plan.sources {
                let dst = destination_for(plan, src)?;
                if dst == *src {
                    return Err(OpError::SamePath(src.clone()));
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
        OperationKind::Delete => Ok(()),
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
    pub moves: Vec<(PathBuf, PathBuf)>,
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
        .collect()
}

pub fn run_operation(
    plan: &OperationPlan,
    mutations: &dyn MutationBackend,
    mut progress: impl FnMut(PathBuf, usize, usize),
) -> OperationReport {
    let total = plan.sources.len();
    let mut report = OperationReport::default();
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
            OperationKind::Copy | OperationKind::Move => {
                let dst = match destination_for(plan, src) {
                    Ok(d) => d,
                    Err(e) => {
                        report.results.push(OpEntryResult {
                            source: src.clone(),
                            outcome: OpOutcome::Failed(e.to_string()),
                        });
                        continue;
                    }
                };
                if dst != *src && mutations.exists(&dst) {
                    match plan.policy {
                        ConflictPolicy::Skip => {
                            report.results.push(OpEntryResult {
                                source: src.clone(),
                                outcome: OpOutcome::Skipped,
                            });
                            continue;
                        }
                        ConflictPolicy::Ask => {
                            report.results.push(OpEntryResult {
                                source: src.clone(),
                                outcome: OpOutcome::Skipped,
                            });
                            continue;
                        }
                        ConflictPolicy::Replace => {}
                    }
                }
                let result = if plan.kind == OperationKind::Copy {
                    mutations.copy_entry(src, &dst, plan.policy == ConflictPolicy::Replace)
                } else {
                    mutations.move_entry(src, &dst, plan.policy == ConflictPolicy::Replace)
                };
                match result {
                    Ok(()) => {
                        if plan.kind == OperationKind::Move {
                            report.moves.push((src.clone(), dst));
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

#[cfg(test)]
mod tests {
    use super::*;

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
