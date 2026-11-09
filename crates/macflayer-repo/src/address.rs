//! Types for every name that could reach a command line or a request path. Each one is ASCII, can't
//! start with `-`, and can't climb out of where it belongs, so a hostile value is refused here and
//! never has to be escaped later.

use std::fmt;

use macflayer_core::approval::CommitSha;
use macflayer_core::lint::fold;

/// Why a pasted address was refused. The paste stays on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressError {
    Empty,
    NotHttps,
    Credentials,
    NotGitHub { host: String },
    HiddenCharacters,
    Malformed,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Paste owner/repo, or a link to a repo or to one file."),
            Self::NotHttps => f.write_str("Only https:// links to GitHub can be read."),
            Self::Credentials => f.write_str(
                "This link has a sign-in in it, so MacFlayer won't read it. Paste it without one.",
            ),
            Self::NotGitHub { host } => write!(
                f,
                "This link goes to {}, and only github.com can be read.",
                crate::text::escaped(host)
            ),
            Self::HiddenCharacters => f.write_str(
                "This has characters in it that change how it reads, so MacFlayer won't read it.",
            ),
            Self::Malformed => {
                f.write_str("That isn't owner/repo, or a link to a repo or to one file.")
            }
        }
    }
}

impl std::error::Error for AddressError {}

fn is_owner(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=39).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

fn is_repo_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=100).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && bytes[0] != b'-'
        && text != "."
        && text != ".."
        && !text.eq_ignore_ascii_case(".git")
        && !text.to_ascii_lowercase().ends_with(".git")
}

/// `owner/name` on github.com.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoId {
    owner: String,
    name: String,
}

impl RepoId {
    pub fn new(owner: &str, name: &str) -> Result<Self, AddressError> {
        if is_owner(owner) && is_repo_name(name) {
            Ok(Self {
                owner: owner.to_string(),
                name: name.to_string(),
            })
        } else {
            Err(AddressError::Malformed)
        }
    }

    /// `owner/name`, as a Config's `dotfiles.repo` or GitHub's `full_name` spells it.
    pub fn parse(text: &str) -> Option<Self> {
        let (owner, name) = text.split_once('/')?;
        Self::new(owner, name).ok()
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// The identity an approval is tied to. GitHub compares names without case.
    pub fn key(&self) -> String {
        format!("github.com/{}", self.full_name()).to_ascii_lowercase()
    }

    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}.git", self.full_name())
    }
}

/// A branch, tag or commit named in a link. It only ever gets resolved to a full commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRef(String);

impl GitRef {
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let ok = (1..=200).contains(&bytes.len())
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            && bytes[0] != b'-'
            && bytes[0] != b'.'
            && !text.contains("..")
            && !text.ends_with(".lock")
            && !text.ends_with('.');
        ok.then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The commit this names outright, when it is a full commit ID.
    pub fn as_commit(&self) -> Option<CommitSha> {
        CommitSha::parse(&self.0)
    }
}

/// One path segment that can go in a request or on a command line as it is.
fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 128
        && segment.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+' | b'@' | b'~')
        })
        && !segment.starts_with('-')
        && segment != "."
        && segment != ".."
        && fold(segment) != ".git"
}

/// Whether a path inside a repo can be asked for by name.
pub fn is_safe_repo_path(path: &str) -> bool {
    path.len() <= 512 && path.split('/').all(is_safe_segment)
}

/// A path to a Config inside a repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPath(String);

impl ConfigPath {
    pub fn parse(text: &str) -> Option<Self> {
        (is_safe_repo_path(text) && text.ends_with(".json")).then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a paste points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportTarget {
    Repo(RepoId),
    File {
        repo: RepoId,
        git_ref: GitRef,
        path: ConfigPath,
    },
}

impl ImportTarget {
    pub fn parse(text: &str) -> Result<Self, AddressError> {
        let text = text.trim_matches([' ', '\n', '\r', '\t']);
        if text.is_empty() {
            return Err(AddressError::Empty);
        }
        if !text.is_ascii() || text.chars().any(|c| c.is_ascii_control() || c == ' ') {
            return Err(AddressError::HiddenCharacters);
        }
        let lower = text.to_ascii_lowercase();
        if lower.split_once("://").is_some_and(|(s, _)| s != "https") || lower.starts_with("git@") {
            return Err(AddressError::NotHttps);
        }
        if text.contains('@') {
            return Err(AddressError::Credentials);
        }
        if text.contains("::") {
            return Err(AddressError::NotHttps);
        }
        let url = lower
            .strip_prefix("https://")
            .map(|_| &text["https://".len()..]);
        let looks_like_host = text
            .split('/')
            .next()
            .is_some_and(|first| first.contains('.') && text.split('/').count() > 2);
        let rest = match url {
            Some(rest) => {
                let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
                check_host(host)?;
                path
            }
            None if looks_like_host => {
                let (host, path) = text.split_once('/').unwrap_or((text, ""));
                check_host(host)?;
                path
            }
            None => text,
        };
        if rest.contains(['?', '#', '%', '\\', ':']) {
            return Err(AddressError::Malformed);
        }
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let segments: Vec<&str> = rest.split('/').collect();
        match segments.as_slice() {
            [owner, name] => {
                let name = name.strip_suffix(".git").unwrap_or(name);
                Ok(Self::Repo(RepoId::new(owner, name)?))
            }
            [owner, name, "blob", git_ref, path @ ..] if !path.is_empty() => Ok(Self::File {
                repo: RepoId::new(owner, name)?,
                git_ref: GitRef::parse(git_ref).ok_or(AddressError::Malformed)?,
                path: ConfigPath::parse(&path.join("/")).ok_or(AddressError::Malformed)?,
            }),
            _ => Err(AddressError::Malformed),
        }
    }
}

fn check_host(host: &str) -> Result<(), AddressError> {
    if host.eq_ignore_ascii_case("github.com") {
        Ok(())
    } else {
        Err(AddressError::NotGitHub {
            host: host.to_ascii_lowercase(),
        })
    }
}

/// Where a Config's `dotfiles.repo` lives. Only exactly `github.com` counts as GitHub, so a sign-in
/// is never offered to a host that merely starts like it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DotfilesSource {
    GitHub(RepoId),
    Other { url: String, host: String },
}

impl DotfilesSource {
    /// `None` for anything that isn't a repo address, including a longer path on GitHub, which
    /// would otherwise read as belonging to its first segment.
    pub fn parse(repo: &str) -> Option<Self> {
        if !repo.is_ascii() || repo.chars().any(|c| c.is_ascii_control() || c == ' ') {
            return None;
        }
        if repo.contains(['@', '?', '#', '%', '\\']) || repo.contains("..") {
            return None;
        }
        let lower = repo.to_ascii_lowercase();
        let Some(rest) = lower
            .strip_prefix("https://")
            .map(|_| &repo["https://".len()..])
        else {
            if repo.contains("://") {
                return None;
            }
            return RepoId::parse(repo).map(Self::GitHub);
        };
        let (host, path) = rest.split_once('/')?;
        let host = host.to_ascii_lowercase();
        let host_ok = host.starts_with(|c: char| c.is_ascii_alphanumeric())
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
        if !host_ok || path.is_empty() {
            return None;
        }
        if host == "github.com" {
            let path = path.strip_suffix('/').unwrap_or(path);
            let path = path.strip_suffix(".git").unwrap_or(path);
            let (owner, name) = path.split_once('/')?;
            return RepoId::new(owner, name).ok().map(Self::GitHub);
        }
        if !path.split('/').all(is_safe_segment) {
            return None;
        }
        Some(Self::Other {
            url: format!("https://{host}/{path}"),
            host,
        })
    }

    pub fn github(&self) -> Option<&RepoId> {
        match self {
            Self::GitHub(id) => Some(id),
            Self::Other { .. } => None,
        }
    }

    pub fn key(&self) -> String {
        match self {
            Self::GitHub(id) => id.key(),
            Self::Other { url, .. } => url.clone(),
        }
    }

    pub fn clone_url(&self) -> String {
        match self {
            Self::GitHub(id) => id.clone_url(),
            Self::Other { url, .. } => url.clone(),
        }
    }

    pub fn host(&self) -> &str {
        match self {
            Self::GitHub(_) => "github.com",
            Self::Other { host, .. } => host,
        }
    }
}
