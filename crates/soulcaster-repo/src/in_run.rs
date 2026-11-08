//! The Dotfiles Repo as part of a Run. The engine decides when, and this applies the links and
//! the install script exactly as approved, through [`DotfilesStep`].

use soulcaster_core::approval::ApprovalStore;
use soulcaster_core::command::CommandRunner;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_engine::{DotfilesHook, DotfilesResult};

use crate::dotfiles::{DotfilesStep, LinkOutcome, ScriptOutcome, UserStage};
use crate::text::escaped;
use crate::tools::Tools;

pub struct RunDotfiles<'a> {
    pub runner: &'a dyn CommandRunner,
    pub tools: &'a Tools,
    /// Where this Machine keeps the repo, written with `~/`.
    pub clone_location: &'a str,
    /// Names this Run's backup folder.
    pub stamp: &'a str,
}

impl DotfilesHook for RunDotfiles<'_> {
    fn apply(&self, plan: &DotfilesPlan, approvals: &dyn ApprovalStore) -> DotfilesResult {
        let step = DotfilesStep {
            runner: self.runner,
            tools: self.tools,
            store: approvals,
            clone_location: self.clone_location,
            stamp: self.stamp,
        };
        // The engine only calls this in the User Stage, after the password is gone.
        let stage = UserStage::password_is_gone();
        let mut warnings = Vec::new();
        match step.apply_links(plan, stage) {
            Ok(report) => {
                for row in report.rows {
                    if let LinkOutcome::Refused(why) = row.outcome {
                        warnings.push(format!("{} wasn't linked: {why}", escaped(&row.target)));
                    }
                }
            }
            Err(error) => {
                warnings.push(format!("The Dotfiles Repo wasn't applied: {error}"));
                return DotfilesResult { warnings };
            }
        }
        if plan.install_script.is_some() {
            match step.run_script(plan, stage, &mut |_, _| {}) {
                ScriptOutcome::Ran { success: true }
                | ScriptOutcome::AlreadyRan
                | ScriptOutcome::NotApproved => {}
                ScriptOutcome::Ran { success: false } => {
                    warnings.push("The Dotfiles Repo's install script ran and failed.".into());
                }
                ScriptOutcome::HeldForReview => warnings.push(
                    "The Dotfiles Repo's install script wasn't run, because the repo on this Mac isn't the approved version."
                        .into(),
                ),
                ScriptOutcome::CouldNotRun => {
                    warnings.push("The Dotfiles Repo's install script couldn't be started.".into());
                }
            }
        }
        DotfilesResult { warnings }
    }
}
