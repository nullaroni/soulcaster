//! Approvals live in the Machine Record. The review writes them before a Run starts, and the Run
//! reads them. Neither a Config nor anything from a repo can reach this.

use std::path::PathBuf;

use soulcaster_core::approval::{ApprovalStore, ApprovalStoreError, Approvals};

use crate::record::MachineRecord;
use crate::run::lock_run;

/// The Machine Record as an [`ApprovalStore`]. A write takes the Run lock, so it can't land in
/// the middle of a Run and be overwritten by the Run's own save.
pub struct RecordApprovals {
    home: PathBuf,
}

impl RecordApprovals {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}

impl ApprovalStore for RecordApprovals {
    fn read(&self) -> Result<Approvals, ApprovalStoreError> {
        MachineRecord::load(&self.home)
            .map(|record| record.approvals)
            .map_err(|error| ApprovalStoreError(error.to_string()))
    }

    fn write(&self, approvals: &Approvals) -> Result<(), ApprovalStoreError> {
        let _held = lock_run(&self.home).map_err(|error| ApprovalStoreError(error.to_string()))?;
        let mut record = MachineRecord::load(&self.home)
            .map_err(|error| ApprovalStoreError(error.to_string()))?;
        record.approvals = approvals.clone();
        record
            .save(&self.home)
            .map_err(|error| ApprovalStoreError(error.to_string()))
    }
}

/// The Run's own copy, for a step inside the Run. The Run saves it back afterwards.
pub(crate) struct HeldApprovals(pub(crate) std::sync::Mutex<Approvals>);

impl ApprovalStore for HeldApprovals {
    fn read(&self) -> Result<Approvals, ApprovalStoreError> {
        self.0
            .lock()
            .map(|held| held.clone())
            .map_err(|_| ApprovalStoreError("the Run's approvals are unavailable".to_string()))
    }

    fn write(&self, approvals: &Approvals) -> Result<(), ApprovalStoreError> {
        let mut held = self
            .0
            .lock()
            .map_err(|_| ApprovalStoreError("the Run's approvals are unavailable".to_string()))?;
        *held = approvals.clone();
        Ok(())
    }
}
