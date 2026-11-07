//! What a person approved on this Machine. A Config has no field that can reach any of this: the
//! only writer is a Start Run from the review, through [`ApprovalStore`], which the wiring ticket
//! binds to the Machine Record.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// A full commit ID. A branch, a tag or a short ID can move, so none of them can be approved.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitSha(String);

impl CommitSha {
    pub fn parse(text: &str) -> Option<Self> {
        let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
        (text.len() == 40 && text.chars().all(hex)).then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The seven characters people recognize, for showing. Never for identifying.
    pub fn short(&self) -> &str {
        &self.0[..7]
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for CommitSha {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        Self::parse(&text).ok_or_else(|| "not a full commit ID".to_string())
    }
}

impl From<CommitSha> for String {
    fn from(sha: CommitSha) -> String {
        sha.0
    }
}

/// The Dotfiles Repo, at one commit, with one list of links. A different repo, a newer commit or
/// one more link is a different approval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DotfilesApproval {
    pub repo: String,
    pub commit: CommitSha,
    pub links: BTreeMap<String, String>,
}

/// The install script, separately from the repo it lives in. It runs once per commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScriptApproval {
    pub repo: String,
    pub commit: CommitSha,
    pub path: String,
    #[serde(default)]
    pub ran: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Approvals {
    /// Full names such as `owner/tap/name`. Approving one package doesn't cover the rest of its tap.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub packages: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dotfiles: Option<DotfilesApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptApproval>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalStoreError(pub String);

impl fmt::Display for ApprovalStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "couldn't read or write approvals: {}", self.0)
    }
}

impl std::error::Error for ApprovalStoreError {}

/// Where this Machine keeps approvals. A failed read means nothing is approved.
pub trait ApprovalStore: Send + Sync {
    fn read(&self) -> Result<Approvals, ApprovalStoreError>;
    fn write(&self, approvals: &Approvals) -> Result<(), ApprovalStoreError>;
}
