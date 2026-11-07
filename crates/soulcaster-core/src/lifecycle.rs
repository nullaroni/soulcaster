//! Item states from the lifecycle design. A state carries what is only true of that state, so a
//! `satisfied` Item always has an assurance and a `blocked` one always names what it waits on.

use serde::{Deserialize, Serialize};

/// An entry's one key: Item ID after renames, setting ID, or `formula:<name>` / `cask:<name>`.
pub type ItemId = String;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ItemState {
    Pending,
    Checking,
    Acting,
    Verifying,
    Satisfied {
        assurance: Assurance,
    },
    /// No check failed and at least one can't answer. Never acted on, never Drift, never queued.
    Unknown,
    /// Only `plan` and `check` report this. A Run acts on it instead.
    Differs,
    NeedsYou {
        /// Set when a Run sent it here rather than a recipe step: a stall or a surprise admin need.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<FailureKind>,
    },
    Failed {
        kind: FailureKind,
    },
    Blocked {
        by: ItemId,
    },
    Skipped {
        reason: SkipReason,
    },
}

impl ItemState {
    pub fn is_terminal(&self) -> bool {
        !matches!(
            self,
            Self::Pending | Self::Checking | Self::Acting | Self::Verifying
        )
    }

    pub fn is_working(&self) -> bool {
        matches!(self, Self::Checking | Self::Acting | Self::Verifying)
    }

    /// The name used in counts and reports, the same spelling as on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Checking => "checking",
            Self::Acting => "acting",
            Self::Verifying => "verifying",
            Self::Satisfied { .. } => "satisfied",
            Self::Unknown => "unknown",
            Self::Differs => "differs",
            Self::NeedsYou { .. } => "needs-you",
            Self::Failed { .. } => "failed",
            Self::Blocked { .. } => "blocked",
            Self::Skipped { .. } => "skipped",
        }
    }

    /// The lifecycle's state diagram. A new Run starts every Item again at `pending`, so a
    /// terminal state may go back there and nowhere else.
    pub fn can_move_to(&self, to: &ItemState) -> bool {
        use ItemState as S;
        match self {
            S::Pending => matches!(to, S::Checking | S::Skipped { .. } | S::Blocked { .. }),
            S::Checking => matches!(
                to,
                S::Satisfied { .. } | S::Unknown | S::Differs | S::NeedsYou { .. } | S::Acting
            ),
            S::Acting => matches!(to, S::NeedsYou { .. } | S::Failed { .. } | S::Verifying),
            S::Verifying => matches!(to, S::Satisfied { .. } | S::Unknown | S::Failed { .. }),
            _ => matches!(to, S::Pending),
        }
    }
}

/// How far a `satisfied` can be trusted. `Stored` is shown as "Set", never "Working".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Assurance {
    Stored,
    Proven,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    Network,
    NotFound,
    Checksum,
    Signature,
    ToolExit,
    Verify,
    Stalled,
    NeedsAdmin,
    Interrupted,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    UnknownId,
    Retired,
    RenameCycle,
    /// Two spellings of one setting with different values.
    RenameCollision,
    /// A setting ID in `items`, or an Item ID in `settings`.
    WrongKind,
    BadValue,
    /// A lint finding on this entry, such as a secret-shaped value.
    Lint,
    MacosTooOld,
    ManagedByAdmin,
    BrokenHere,
    WritesThroughLink,
    MarkedByPerson,
}

impl SkipReason {
    /// A reason about this Machine, which the Config format records as Unavailable here: shown
    /// apart from Drift, and never offered for Adopt.
    pub fn is_unavailable_here(self) -> bool {
        matches!(
            self,
            Self::MacosTooOld
                | Self::ManagedByAdmin
                | Self::BrokenHere
                | Self::WritesThroughLink
                | Self::MarkedByPerson
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Admin,
    User,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn satisfied() -> ItemState {
        ItemState::Satisfied {
            assurance: Assurance::Proven,
        }
    }
    fn failed() -> ItemState {
        ItemState::Failed {
            kind: FailureKind::Verify,
        }
    }

    #[test]
    fn a_run_item_walks_the_diagram_to_satisfied() {
        let path = [
            ItemState::Pending,
            ItemState::Checking,
            ItemState::Acting,
            ItemState::Verifying,
            satisfied(),
        ];
        for pair in path.windows(2) {
            assert!(
                pair[0].can_move_to(&pair[1]),
                "{:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn nothing_acts_on_unknown_and_a_step_never_jumps_to_satisfied() {
        assert!(!ItemState::Unknown.can_move_to(&ItemState::Acting));
        // A step's exit code never counts as success. Only verifying can reach satisfied.
        assert!(!ItemState::Acting.can_move_to(&satisfied()));
        assert!(!ItemState::Verifying.can_move_to(&ItemState::Differs));
    }

    #[test]
    fn a_new_run_restarts_any_terminal_state_at_pending() {
        for state in [
            satisfied(),
            failed(),
            ItemState::Unknown,
            ItemState::Blocked { by: "x".into() },
        ] {
            assert!(state.is_terminal());
            assert!(state.can_move_to(&ItemState::Pending));
            assert!(!state.can_move_to(&ItemState::Checking));
        }
    }

    #[test]
    fn states_use_the_lifecycle_spelling_on_the_wire() {
        let json = serde_json::to_value(ItemState::NeedsYou {
            cause: Some(FailureKind::NeedsAdmin),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"state": "needs-you", "cause": "needs-admin"})
        );
        let json = serde_json::to_value(ItemState::Satisfied {
            assurance: Assurance::Stored,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"state": "satisfied", "assurance": "stored"})
        );
    }

    #[test]
    fn only_reasons_about_this_machine_count_as_unavailable_here() {
        assert!(SkipReason::WritesThroughLink.is_unavailable_here());
        assert!(!SkipReason::UnknownId.is_unavailable_here());
        assert!(!SkipReason::Retired.is_unavailable_here());
    }
}
