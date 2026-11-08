//! The Configs kept on this Mac: your own saves, which wait here when GitHub can't take them, and
//! copies of someone else's. Each is written whole, private to this user, and without following a
//! symlink. Where an Import came from sits beside it, never in the file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use soulcaster_core::approval::CommitSha;
use soulcaster_core::config::Config;

use crate::import::{Fetched, Source};
use crate::safe_fs::{PathTrouble, SafeRoot};

pub(crate) const SUPPORT: &[&str] = &["Library", "Application Support", "Soulcaster"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyError {
    Write,
    /// A different Config is already kept under this name's file. Both names make the same slug.
    NameTaken,
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Write => "Soulcaster couldn't write to its folder on this Mac.",
            Self::NameTaken => {
                "Another Config is kept under the same file name. Choose a different name."
            }
        })
    }
}

impl std::error::Error for CopyError {}

impl From<PathTrouble> for CopyError {
    fn from(_: PathTrouble) -> Self {
        Self::Write
    }
}

/// A file name from a Config's name: plain lowercase words and nothing that could be a path or a
/// flag. Different names can land on the same slug, and a save then finds the other file there.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(64);
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "config".to_string()
    } else {
        trimmed.to_string()
    }
}

fn write_in_support(
    home: &Path,
    folder: &str,
    file: &str,
    bytes: &[u8],
) -> Result<PathBuf, CopyError> {
    let root = SafeRoot::open(home)?;
    let mut parts: Vec<&str> = SUPPORT.to_vec();
    parts.push(folder);
    parts.push(file);
    root.write_file(&parts, bytes, 0o600)?;
    Ok(root.path().join(parts.join("/")))
}

/// Your own Config, on this Mac, before anything is sent. `text` is already the canonical form.
pub fn keep_own(home: &Path, name: &str, text: &str) -> Result<PathBuf, CopyError> {
    let file = format!("{}.json", slug(name));
    let mut parts: Vec<&str> = SUPPORT.to_vec();
    parts.push("configs");
    parts.push(&file);
    let there = SafeRoot::open(home)?.path().join(parts.join("/"));
    // Only your own earlier save of this same Config can be replaced. A file that can't be read as
    // one might be newer than this version understands, and it isn't this save's to lose.
    if let Ok(bytes) = std::fs::read(&there)
        && !Config::parse(&bytes).is_ok_and(|kept| kept.name == name)
    {
        return Err(CopyError::NameTaken);
    }
    write_in_support(home, "configs", &file, text.as_bytes())
}

/// The commit a copy was read at, beside the copy. No approval is recorded, and the file itself
/// doesn't say where it came from.
#[derive(Serialize)]
struct CopySource<'a> {
    repo: String,
    path: &'a str,
    commit: &'a CommitSha,
}

/// Someone else's Config, kept without running it. Its own folder and a name built from the source
/// mean a stranger's file name can't overwrite your Config or another copy.
pub fn keep_import(home: &Path, fetched: &Fetched) -> Result<PathBuf, CopyError> {
    let Source { repo, path, commit } = &fetched.source;
    let stem = format!(
        "{}-{}-{}-{}",
        slug(repo.owner()),
        slug(repo.name()),
        commit.short(),
        slug(path.as_str().trim_end_matches(".json"))
    );
    let text = fetched.config.to_canonical_json();
    let kept = write_in_support(home, "imports", &format!("{stem}.json"), text.as_bytes())?;
    let source = serde_json::to_vec_pretty(&CopySource {
        repo: repo.full_name(),
        path: path.as_str(),
        commit,
    })
    .map_err(|_| CopyError::Write)?;
    write_in_support(home, "imports", &format!("{stem}.source.json"), &source)?;
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_is_plain_whatever_the_name_is() {
        assert_eq!(slug("My Mac"), "my-mac");
        assert_eq!(slug("../../evil"), "evil");
        assert_eq!(slug("-rf"), "rf");
        assert_eq!(slug(".git"), "git");
        assert_eq!(slug("日本語"), "config");
        assert_eq!(slug("a\nb"), "a-b");
    }

    #[test]
    fn a_copy_lands_only_in_soulcasters_folder_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let kept = keep_own(dir.path(), "../../evil", "{}\n").unwrap();
        let home = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            kept,
            home.join("Library/Application Support/Soulcaster/configs/evil.json")
        );
        assert_eq!(
            std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
