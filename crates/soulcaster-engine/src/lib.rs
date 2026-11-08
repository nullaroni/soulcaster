//! The Run. Give it a plan, a command runner, a home directory, an event sink and a way to ask
//! for the password, and it brings that Machine in line with the plan.
//!
//! The password moves only through [`soulcaster_core::ask::Asks`] and, for one admin step at a
//! time, a private socket that answers the signed helper. It is dropped when the Admin Stage ends,
//! and `sudo`'s own timestamp with it. Nothing in the event stream, the Machine Record or an error
//! can hold it.

mod act;
mod admin;
mod process;
mod record;
mod run;

pub use admin::{AdminGate, AdminWindow, Budget, PeerRule, SocketGate, Validation};
pub use process::ProcessRunner;
pub use record::{
    Approvals, DotfilesApproval, InProgress, ItemOutcome, LastApplied, MachineRecord, QueueItem,
    SCHEMA_VERSION, TapApproval, machine_record_path,
};
pub use run::{CancelFlag, Clock, EngineError, RunInput, RunReport, SystemClock, perform};
