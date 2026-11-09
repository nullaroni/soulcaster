//! Approvals live in the Machine Record. The review writes them before a Run starts, and the Run
//! reads them. Neither a Config nor anything from a repo can reach this.

use std::path::PathBuf;
use std::sync::Mutex;

use macflayer_core::approval::{ApprovalStore, ApprovalStoreError, Approvals};

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

/// The Run's own record, for a step inside the Run. A write is on disk before it returns, because
/// a step may act on it: the install script runs only once its record is stored.
pub(crate) struct HeldApprovals {
    pub(crate) record: Mutex<MachineRecord>,
    pub(crate) home: PathBuf,
}

fn unavailable() -> ApprovalStoreError {
    ApprovalStoreError("the Run's approvals are unavailable".to_string())
}

impl ApprovalStore for HeldApprovals {
    fn read(&self) -> Result<Approvals, ApprovalStoreError> {
        self.record
            .lock()
            .map(|record| record.approvals.clone())
            .map_err(|_| unavailable())
    }

    fn write(&self, approvals: &Approvals) -> Result<(), ApprovalStoreError> {
        let mut record = self.record.lock().map_err(|_| unavailable())?;
        let before = std::mem::replace(&mut record.approvals, approvals.clone());
        record.save(&self.home).map_err(|error| {
            record.approvals = before;
            ApprovalStoreError(error.to_string())
        })
    }
}
