//! Where the Dotfiles Repo fits in a Run. The engine doesn't know how to clone or link: the front
//! end hands it a [`DotfilesHook`], which `macflayer-repo` provides. The engine decides when.

use macflayer_core::approval::ApprovalStore;
use macflayer_core::plan::DotfilesPlan;

/// What applying the Dotfiles Repo has to say. Nothing here is a value from the person's files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DotfilesResult {
    /// Anything that didn't happen as approved, in words for the person.
    pub warnings: Vec<String>,
}

/// Applies the approved Dotfiles Repo. The Run calls it once, in the User Stage, after
/// third-party Items and before git settings, and only after the password is gone. The approvals
/// it reads and writes are the Run's own, saved to the Machine Record when it returns.
pub trait DotfilesHook: Send + Sync {
    fn apply(&self, plan: &DotfilesPlan, approvals: &dyn ApprovalStore) -> DotfilesResult;
}
