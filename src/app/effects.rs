use std::path::PathBuf;

use crate::app::action::Action;
use crate::operations::OperationPlan;

#[derive(Clone, Debug)]
pub enum Effect {
    LoadDirectory(PathBuf),
    RunOperation(Box<OperationPlan>),
    RunRename(Box<OperationPlan>),
    OpenPath(PathBuf),
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
