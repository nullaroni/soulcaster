//! The engine's vocabulary from the Item lifecycle design: states, failure kinds, stages and the
//! typed event stream. The screens read only these, so a real engine replaces `sim.rs` without
//! touching them. A secret has no field here on purpose: asks aren't events.

pub type ItemId = String;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemState {
    Pending,
    Checking,
    Acting,
    Verifying,
    Satisfied,
    Unknown,
    /// Only `plan` and `check` report this; a Run acts on it instead. Drift has its own list.
    #[allow(dead_code)]
    Differs,
    NeedsYou,
    Failed,
    Blocked,
    Skipped,
}

impl ItemState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, ItemState::Pending | ItemState::Checking | ItemState::Acting | ItemState::Verifying)
    }
    pub fn is_working(self) -> bool {
        matches!(self, ItemState::Checking | ItemState::Acting | ItemState::Verifying)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Assurance {
    Proven,
    /// The value reads back and nothing more, so the screen says "Set", never "Working".
    Stored,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FailureKind {
    Network,
    Checksum,
    NeedsAdmin,
}

impl FailureKind {
    /// What would fix it, in plain words.
    pub fn fix(self) -> &'static str {
        match self {
            FailureKind::Network => "The download kept timing out after 3 tries. Try again when your connection is steady.",
            FailureKind::Checksum => "The download didn't match what this version of Soulcaster expects. Update Soulcaster. Don't retry on an untrusted network.",
            FailureKind::NeedsAdmin => "It asked for your password after Soulcaster had already discarded it.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Admin,
    User,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueueKind {
    /// macOS or a vendor needs the person; only they can do it.
    Person,
    /// A license nobody accepted before the Run.
    License,
    /// A third-party source nobody approved before the Run.
    Tap,
    /// Learned during the Run: it needed admin after the password was gone.
    NeedsAdmin,
}

#[derive(Clone, Debug)]
pub struct QueueEntry {
    pub id: String,
    pub item: ItemId,
    pub kind: QueueKind,
    pub title: String,
    pub detail: String,
    pub action: String,
    /// Finishing it starts a scoped Run with admin work, which asks for the password.
    pub needs_password: bool,
}

#[derive(Clone, Debug)]
pub enum Event {
    RunStarted { items: Vec<ItemId> },
    AdminHeld { why: String },
    AdminDiscarded,
    ItemChanged { item: ItemId, to: ItemState, assurance: Option<Assurance>, failure: Option<FailureKind>, blocked_by: Option<ItemId>, acted: bool },
    ItemProgress { item: ItemId, phase: String, fraction: f32 },
    ItemLog { item: ItemId, lines: Vec<String> },
    QueueOpened(QueueEntry),
    QueueClosed { id: String },
    RunWarning { text: String },
    RunFinished,
}

/// One event on the stream, with the sequence number and time the design requires.
#[derive(Clone, Debug)]
pub struct Timed {
    pub seq: u64,
    pub at: f64,
    pub event: Event,
}
