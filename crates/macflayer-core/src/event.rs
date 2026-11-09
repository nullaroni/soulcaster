//! The one event stream both front ends consume, from the lifecycle design's table. No variant has
//! a field that could hold a secret: asks go through [`crate::ask::Asks`].

use std::collections::BTreeMap;
use std::sync::mpsc;

use serde::{Deserialize, Serialize};

use crate::lifecycle::{ItemId, ItemState, Stage};

pub const EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub schema_version: u32,
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventKind {
    #[serde(rename = "run.started", rename_all = "camelCase")]
    RunStarted {
        run_id: String,
        scope: RunScope,
        config: ConfigRef,
        catalog_version: String,
        engine_version: String,
        macos_version: Option<String>,
        plan: Vec<PlanItem>,
    },
    #[serde(rename = "ask.state")]
    AskState { ask: AskKind, state: AskStatus },
    #[serde(rename = "admin.held")]
    AdminHeld { why: String },
    #[serde(rename = "admin.discarded")]
    AdminDiscarded { why: String },
    #[serde(rename = "item.state")]
    ItemState {
        item: ItemId,
        from: ItemState,
        to: ItemState,
    },
    #[serde(rename = "item.step", rename_all = "camelCase")]
    ItemStep {
        item: ItemId,
        index: usize,
        step_type: String,
        phase: StepPhase,
    },
    #[serde(rename = "item.progress", rename_all = "camelCase")]
    ItemProgress {
        item: ItemId,
        phase: String,
        bytes_done: u64,
        bytes_total: Option<u64>,
    },
    #[serde(rename = "item.log")]
    ItemLog {
        item: ItemId,
        stream: LogStream,
        lines: Vec<String>,
    },
    #[serde(rename = "queue.opened")]
    QueueOpened { entry: QueueEntry },
    #[serde(rename = "queue.closed")]
    QueueClosed { id: String },
    #[serde(rename = "run.warning")]
    RunWarning { text: String },
    #[serde(rename = "run.finished")]
    RunFinished {
        /// Items per terminal state, keyed by [`ItemState::name`].
        counts: BTreeMap<String, u32>,
        queue: Vec<QueueEntry>,
    },
}

impl EventKind {
    /// The two kinds the engine may merge or cap. Everything else is delivered and kept losslessly.
    pub fn may_merge(&self) -> bool {
        matches!(self, Self::ItemProgress { .. } | Self::ItemLog { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunScope {
    Full,
    /// A retry, or a finished queue entry: this Item, whatever it needs and whatever waits on it.
    Items {
        items: Vec<ItemId>,
    },
    /// An Apply: these and what they need, and nothing that depends on them.
    Only {
        items: Vec<ItemId>,
    },
}

/// Which Config a Run follows. The repo fields stay empty for a local file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// What a front end needs to draw an Item before any `item.state` for it arrives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub id: ItemId,
    pub name: String,
    pub stage: Stage,
    pub is_setting: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulled_in_by: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AskKind {
    Password,
    Github,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AskStatus {
    Waiting,
    Done,
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepPhase {
    Started,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogStream {
    Stdout,
    Stderr,
    /// The engine's own words, as opposed to a tool's output.
    Engine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueKind {
    /// macOS or a vendor needs the person, and only they can do it.
    Person,
    License,
    Tap,
    Conflict,
    /// It needed admin after the password was gone, or an admin step stalled.
    NeedsAdmin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub id: String,
    pub item: ItemId,
    pub kind: QueueKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Finishing it starts a scoped Run with admin work, which asks for the password.
    #[serde(default)]
    pub needs_password: bool,
}

/// Stamps a stream with its sequence numbers, so every producer numbers events the same way.
#[derive(Debug, Default)]
pub struct Stamper {
    seq: u64,
}

impl Stamper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Continues after `seq`, for a stream that picks up where another left off.
    pub fn after(seq: u64) -> Self {
        Self { seq }
    }

    pub fn stamp(&mut self, at_ms: u64, kind: EventKind) -> Event {
        self.seq += 1;
        Event {
            schema_version: EVENT_SCHEMA_VERSION,
            seq: self.seq,
            at_ms,
            kind,
        }
    }
}

/// Where the engine sends events. It must not block on a front end.
pub trait EventSink: Send {
    fn send(&mut self, event: Event);
}

impl EventSink for Vec<Event> {
    fn send(&mut self, event: Event) {
        self.push(event);
    }
}

impl EventSink for mpsc::Sender<Event> {
    fn send(&mut self, event: Event) {
        // A closed window means nobody is watching. The Run itself carries on.
        let _ = mpsc::Sender::send(self, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{Assurance, ItemState};

    #[test]
    fn events_use_the_lifecycle_names_on_the_wire_and_round_trip() {
        let event = Stamper::new().stamp(
            1_000,
            EventKind::ItemState {
                item: "ghostty".to_string(),
                from: ItemState::Verifying,
                to: ItemState::Satisfied {
                    assurance: Assurance::Proven,
                },
            },
        );
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "item.state");
        assert_eq!(json["seq"], 1);
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["to"]["state"], "satisfied");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), event);
    }

    #[test]
    fn only_progress_and_logs_may_be_merged() {
        let log = EventKind::ItemLog {
            item: "x".to_string(),
            stream: LogStream::Engine,
            lines: vec![],
        };
        assert!(log.may_merge());
        assert!(
            !EventKind::AdminDiscarded {
                why: "done".to_string()
            }
            .may_merge()
        );
        assert!(
            !EventKind::RunWarning {
                text: "x".to_string()
            }
            .may_merge()
        );
    }

    #[test]
    fn sequence_numbers_keep_climbing() {
        let mut stamper = Stamper::after(7);
        let kind = || EventKind::RunWarning {
            text: "x".to_string(),
        };
        assert_eq!(stamper.stamp(0, kind()).seq, 8);
        assert_eq!(stamper.stamp(0, kind()).seq, 9);
    }

    #[test]
    fn a_sink_is_a_vec_or_a_channel() {
        let mut stamper = Stamper::new();
        let mut events: Vec<Event> = Vec::new();
        EventSink::send(
            &mut events,
            stamper.stamp(
                0,
                EventKind::RunWarning {
                    text: "x".to_string(),
                },
            ),
        );
        assert_eq!(events.len(), 1);
        let (mut tx, rx) = mpsc::channel();
        EventSink::send(&mut tx, events[0].clone());
        assert_eq!(rx.recv().unwrap(), events[0]);
        drop(rx);
        EventSink::send(&mut tx, events[0].clone());
    }
}
