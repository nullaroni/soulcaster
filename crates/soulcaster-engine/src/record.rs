//! The Machine Record: last-applied values, approvals, Machine-local values and the queue.
//! It lives under the injected home, mode 0600, and a crash mid-write leaves the previous file.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use soulcaster_core::event::QueueKind;
use soulcaster_core::lifecycle::{Assurance, FailureKind, SkipReason};

use crate::run::EngineError;

pub const SCHEMA_VERSION: u32 = 1;

/// `{home}/Library/Application Support/Soulcaster/machine.json`
pub fn machine_record_path(home: &Path) -> PathBuf {
    support_dir(home).join("machine.json")
}

pub fn support_dir(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("Soulcaster")
}

/// What Drift will compare later. `prior_value` is whatever was there before the first write,
/// and later writes keep it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastApplied {
    pub value: Value,
    pub applied_at: u64,
    pub run_id: String,
    pub catalog_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos_version: Option<String>,
    pub assurance: Assurance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_value: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TapApproval {
    pub approved_at: u64,
}

/// Round-tripped so a later ticket can read it. This engine does not grant it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DotfilesApproval {
    pub repo: String,
    pub commit: String,
    #[serde(default)]
    pub script: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approvals {
    /// `owner/tap`. Granting one is a person's choice, stored here for the next Run.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub taps: BTreeMap<String, TapApproval>,
    /// `owner/tap/name`. Import records these; this engine does not grant them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub packages: BTreeMap<String, TapApproval>,
    /// The Dotfiles Repo approval is the repo and the commit it was previewed at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dotfiles: Option<DotfilesApproval>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub item: String,
    pub step_id: String,
    pub kind: QueueKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub opened_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_by: Option<String>,
    #[serde(default)]
    pub needs_password: bool,
}

impl QueueItem {
    pub fn is_open(&self) -> bool {
        self.finished_at.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemOutcome {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assurance: Option<Assurance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<SkipReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InProgress {
    pub run_id: String,
    pub item: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interruption {
    pub item: String,
    pub run_id: String,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedLicense {
    pub license: String,
    pub at: u64,
}

/// Preserved across saves. Drift, Adopt and Apply read `last_applied`; they are not decided here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineRecord {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub last_applied: BTreeMap<String, LastApplied>,
    #[serde(default, skip_serializing_if = "Approvals::is_empty")]
    pub approvals: Approvals,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local_values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<QueueItem>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unavailable: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_at_review: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<AcceptedLicense>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub learned_admin: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pulled_in: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follows: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone_path: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outcomes: BTreeMap<String, ItemOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<InProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interruptions: Vec<Interruption>,
}

impl Approvals {
    fn is_empty(&self) -> bool {
        self.taps.is_empty() && self.packages.is_empty() && self.dotfiles.is_none()
    }
}

impl Default for MachineRecord {
    fn default() -> Self {
        Self::empty()
    }
}

impl MachineRecord {
    pub fn empty() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            last_applied: BTreeMap::new(),
            approvals: Approvals {
                taps: BTreeMap::new(),
                packages: BTreeMap::new(),
                dotfiles: None,
            },
            local_values: BTreeMap::new(),
            queue: Vec::new(),
            unavailable: BTreeMap::new(),
            unknown_at_review: Vec::new(),
            accepted: Vec::new(),
            learned_admin: Vec::new(),
            pulled_in: BTreeMap::new(),
            follows: None,
            clone_path: None,
            outcomes: BTreeMap::new(),
            running: None,
            interruptions: Vec::new(),
        }
    }

    pub fn load(home: &Path) -> Result<Self, EngineError> {
        let path = machine_record_path(home);
        if !path.exists() {
            return Ok(Self::empty());
        }
        let bytes = fs::read(&path).map_err(|error| EngineError::RecordIo {
            path: path.clone(),
            detail: error.to_string(),
        })?;
        parse_record(&path, &bytes)
    }

    pub fn save(&self, home: &Path) -> Result<(), EngineError> {
        let path = machine_record_path(home);
        let dir = support_dir(home);
        fs::create_dir_all(&dir).map_err(|error| EngineError::RecordIo {
            path: dir.clone(),
            detail: error.to_string(),
        })?;
        let dir_mode = fs::Permissions::from_mode(0o700);
        fs::set_permissions(&dir, dir_mode).map_err(|error| EngineError::RecordIo {
            path: dir.clone(),
            detail: error.to_string(),
        })?;

        let bytes = serde_json::to_vec_pretty(self).map_err(|error| EngineError::RecordIo {
            path: path.clone(),
            detail: error.to_string(),
        })?;
        let tmp = dir.join(format!(".machine.json.{}.tmp", std::process::id()));
        let write_tmp = (|| -> Result<(), std::io::Error> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = write_tmp {
            let _ = fs::remove_file(&tmp);
            return Err(EngineError::RecordIo {
                path: path.clone(),
                detail: error.to_string(),
            });
        }
        fs::rename(&tmp, &path).map_err(|error| {
            let _ = fs::remove_file(&tmp);
            EngineError::RecordIo {
                path: path.clone(),
                detail: error.to_string(),
            }
        })?;
        // Rename is atomic on the same volume. Syncing the directory makes that rename durable
        // if the Machine loses power before the directory entry is written.
        if let Ok(dir_file) = File::open(&dir) {
            let _ = dir_file.sync_all();
        }
        Ok(())
    }

    pub fn open_queue_for<'a>(&'a self, item: &'a str) -> impl Iterator<Item = &'a QueueItem> + 'a {
        self.queue
            .iter()
            .filter(move |entry| entry.item == item && entry.is_open())
    }
}

fn parse_record(path: &Path, bytes: &[u8]) -> Result<MachineRecord, EngineError> {
    if bytes.is_empty() {
        return Err(corrupt(path, "the file is empty"));
    }
    // Approvals live in here, and serde_json would keep the last of a duplicate key.
    let value: Value =
        soulcaster_core::config::strict_parse(bytes).map_err(|error| corrupt(path, &error))?;
    let Some(found) = value.get("schemaVersion").and_then(Value::as_u64) else {
        return Err(corrupt(path, "missing schemaVersion"));
    };
    if found > u64::from(u32::MAX) || found != u64::from(SCHEMA_VERSION) {
        return Err(EngineError::UnsupportedRecord {
            path: path.to_path_buf(),
            found: u32::try_from(found).unwrap_or(u32::MAX),
        });
    }
    serde_json::from_value(value).map_err(|error| corrupt(path, &error.to_string()))
}

fn corrupt(path: &Path, detail: &str) -> EngineError {
    EngineError::CorruptRecord {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    }
}
