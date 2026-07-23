use std::path::PathBuf;

use crate::app::action::Action;
use crate::app::state::Password;
use crate::crypto::CryptoKind;
use crate::operations::OperationPlan;

#[derive(Debug)]
pub enum Effect {
    LoadDirectory(PathBuf),
    RunOperation(Box<OperationPlan>),
    RunRename(Box<OperationPlan>),
    OpenPath(PathBuf),
    /// Load preview content for the focused entry (worker thread).
    LoadPreview {
        key: (PathBuf, i64, u64),
        name: String,
        is_dir: bool,
    },
    /// Run encryption/decryption for one target (worker thread).
    Crypto {
        kind: CryptoKind,
        target: PathBuf,
        password: Password,
    },
    ToggleBookmark(PathBuf),
    TagAssign {
        name: String,
        paths: Vec<PathBuf>,
        create: bool,
    },
    TagUnassign {
        name: String,
        paths: Vec<PathBuf>,
    },
    TagCreate(String),
    TagDelete(String),
    TagMove {
        from: PathBuf,
        to: PathBuf,
    },
    Quit,
}

pub trait EffectHandler {
    fn handle(&mut self, effect: Effect) -> Vec<Action>;
}
