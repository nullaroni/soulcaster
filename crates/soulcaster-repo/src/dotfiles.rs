//! The Dotfiles Repo step: bring the repo to the approved commit, link its files, and, with its own
//! approval, run its install script. A later ticket wires this in after the password helper is gone,
//! between `ThirdParty` and `GitValues`, with the script after everything. Every call reads the
//! approval again, so nothing here can run on what a Config says about itself.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use soulcaster_core::approval::{ApprovalStore, Approvals, CommitSha, DotfilesApproval};
use soulcaster_core::command::{CommandEnd, CommandOutput, CommandRunner, OutputStream};
use soulcaster_core::lint::{FindingKind, fold, refused_links};
use soulcaster_core::plan::DotfilesPlan;

use crate::address::DotfilesSource;
use crate::copies::SUPPORT;
use crate::safe_fs::{PathTrouble, Placed, SafeRoot};
use crate::text::{escaped_lines, is_hidden};
use crate::tools::{GitCred, Tools};

/// The most of a change that gets shown in a preview.
const MAX_PREVIEW: usize = 256 * 1024;

/// A marker that this runs in the User Stage, after the password helper is gone. The wiring ticket
/// is the one place that should write this call.
#[derive(Clone, Copy, Debug)]
pub struct UserStage(());

impl UserStage {
    pub fn password_is_gone() -> Self {
        Self(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DotfilesError {
    PreviewFailed,
    PreviewCutOff,
    NotApproved,
    GitMissing,
    /// The clone location holds something else, or is reached through a symlink.
    LocationBusy,
    /// A repo at the clone location came from somewhere else.
    DifferentRepo,
    /// This Mac has its own changes, so Soulcaster won't update it.
    ChangesOnThisMac,
    Offline,
    /// GitHub no longer has the approved commit, or won't say.
    CommitGone,
    /// The repo on disk is past what was approved, so it needs a new preview.
    PastApproved {
        head: CommitSha,
    },
    /// Git landed somewhere other than the approved commit.
    CommitMismatch {
        wanted: CommitSha,
        found: CommitSha,
    },
}

impl fmt::Display for DotfilesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotApproved => "This Dotfiles Repo isn't approved on this Mac at this version.",
            Self::GitMissing => "This needs Command Line Tools, which aren't on this Mac yet.",
            Self::LocationBusy => {
                "Something else is already where the Dotfiles Repo goes, so it was left alone."
            }
            Self::DifferentRepo => "A different repo is already where the Dotfiles Repo goes.",
            Self::PreviewFailed => {
                "Soulcaster couldn't work out what changed in the Dotfiles Repo, so nothing moved."
            }
            Self::PreviewCutOff => {
                "These updates are too long to show in full, so they can't be approved here."
            }
            Self::ChangesOnThisMac => {
                "This Mac has its own changes, so Soulcaster won't update the Dotfiles Repo. Update it yourself in Terminal."
            }
            Self::Offline => "Soulcaster couldn't reach the Dotfiles Repo. Try again.",
            Self::CommitGone => {
                "The version you approved isn't there any more. Treat this as a repo you've never seen."
            }
            Self::PastApproved { .. } => {
                "The Dotfiles Repo on this Mac has moved past what you approved. Review the updates."
            }
            Self::CommitMismatch { .. } => {
                "The Dotfiles Repo didn't match the version you approved, so it wasn't kept."
            }
        })
    }
}

impl std::error::Error for DotfilesError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkRefusal {
    /// Lint refuses this location.
    Lint(FindingKind),
    /// Not a location under `~/` that could be right on any Machine.
    BadLocation,
    SourceMissing,
    /// The path, or a symlink in the repo, leads outside the repo or into `.git`.
    SourceLeavesRepo,
    /// One of this repo's links, for another file, is already there.
    AnotherLinkHere,
    /// A symlink above the location would carry the write somewhere else.
    SymlinkInPath {
        at: String,
    },
    NotAFolder {
        at: String,
    },
    /// The path changed between the check and the write.
    PathChanged,
    /// The files on this Mac aren't the approved version, whatever version it says it's at.
    RepoChanged,
    Io,
}

impl fmt::Display for LinkRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RepoChanged => {
                f.write_str("The files on this Mac aren't the version you approved")
            }
            Self::Lint(FindingKind::LinkProtected) => {
                f.write_str("This folder holds your data, so a link can't replace it")
            }
            Self::Lint(FindingKind::LinkOwnFolder) => {
                f.write_str("Would write into Soulcaster's own folder")
            }
            Self::Lint(FindingKind::LinkClone) => {
                f.write_str("Would write into or over the Dotfiles Repo's own folder")
            }
            Self::Lint(FindingKind::LinkDuplicate) => {
                f.write_str("Same location as another link on this Mac")
            }
            Self::Lint(_) => f.write_str("Overlaps another link"),
            Self::BadLocation => f.write_str("Not a location inside your home folder"),
            Self::SourceMissing => f.write_str("That file isn't in the Dotfiles Repo"),
            Self::SourceLeavesRepo => f.write_str("Leads outside the Dotfiles Repo"),
            Self::AnotherLinkHere => f.write_str("Another link from this repo is already there"),
            Self::SymlinkInPath { at } => {
                write!(
                    f,
                    "~/{} is a link, so this would land somewhere else",
                    crate::text::escaped(at)
                )
            }
            Self::NotAFolder { at } => {
                write!(f, "~/{} isn't a folder", crate::text::escaped(at))
            }
            Self::PathChanged => f.write_str("Something changed this location while linking"),
            Self::Io => f.write_str("This Mac wouldn't allow the link"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked { backup: Option<PathBuf> },
    AlreadyLinked,
    Refused(LinkRefusal),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRow {
    pub target: String,
    pub source: String,
    pub outcome: LinkOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkReport {
    pub commit: CommitSha,
    pub rows: Vec<LinkRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptOutcome {
    /// It ran, once, for this commit. A failure doesn't stop the Run.
    Ran {
        success: bool,
    },
    AlreadyRan,
    NotApproved,
    /// The repo on disk isn't what was approved. `dotfiles.script-held` is the words for this.
    HeldForReview,
    CouldNotRun,
}

/// What moving to a newer commit would change. Only [`DotfilesStep::preview_move`] makes one, and
/// the only way to move takes one, so nothing moves past an approval without being shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovePreview {
    from: CommitSha,
    to: CommitSha,
    text: String,
    complete: bool,
}

impl MovePreview {
    /// The linked files' changes. Printable text, with no terminal control.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn to(&self) -> &CommitSha {
        &self.to
    }

    /// Whether this is all of it. What wasn't shown can't be approved.
    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

/// Where a Dotfiles Repo stands against its approval. Nothing here moves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStanding {
    NotApproved,
    /// A transferred repo, or a different repo in the file.
    RepoMoved,
    /// A link was added or changed at the same commit.
    LinksChanged,
    Current,
    /// The file names a script the approval doesn't cover.
    ScriptHeld,
    /// Newer on GitHub. A stranger's starts off, your own starts on, and neither moves until asked.
    Behind,
    /// The approved commit isn't in the new history: a repo you've never seen.
    HistoryRewritten,
}

/// Compares GitHub's newest commit with what was approved. `moved_to` is where GitHub says the repo
/// lives now, when that's somewhere else.
pub fn standing(
    approvals: &Approvals,
    plan: &DotfilesPlan,
    head: &CommitSha,
    is_ancestor: bool,
    moved_to: Option<&str>,
) -> MoveStanding {
    let Some(held) = &approvals.dotfiles else {
        return MoveStanding::NotApproved;
    };
    let key = DotfilesSource::parse(&plan.repo).map(|source| source.key());
    if moved_to.is_some() || key.as_deref() != Some(held.repo.as_str()) {
        return MoveStanding::RepoMoved;
    }
    if held.links != plan.links {
        return MoveStanding::LinksChanged;
    }
    if *head == held.commit {
        let script_held = plan.install_script.as_ref().is_some_and(|path| {
            !approvals
                .script
                .as_ref()
                .is_some_and(|s| s.path == *path && s.commit == *head)
        });
        return if script_held {
            MoveStanding::ScriptHeld
        } else {
            MoveStanding::Current
        };
    }
    if is_ancestor {
        MoveStanding::Behind
    } else {
        MoveStanding::HistoryRewritten
    }
}

/// The parts of a path, refused if any of it could climb out, reach `.git`, or read as something
/// else. The schema refuses all of this already. This is for a plan that didn't come through it.
fn safe_parts(path: &str) -> Option<Vec<&str>> {
    if path.is_empty() || path.chars().any(is_hidden) {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    let bad =
        |part: &&str| part.is_empty() || *part == "." || *part == ".." || fold(part) == ".git";
    (!parts.iter().any(bad)).then_some(parts)
}

fn location_parts(target: &str) -> Option<Vec<&str>> {
    safe_parts(target.strip_prefix("~/")?)
}

fn verify_source(root: &Path, source: &str) -> Result<PathBuf, LinkRefusal> {
    if source.starts_with('-') {
        return Err(LinkRefusal::SourceLeavesRepo);
    }
    safe_parts(source).ok_or(LinkRefusal::SourceLeavesRepo)?;
    let literal = root.join(source);
    let canonical = fs::canonicalize(&literal).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => LinkRefusal::SourceMissing,
        _ => LinkRefusal::Io,
    })?;
    let inside = canonical
        .strip_prefix(root)
        .map_err(|_| LinkRefusal::SourceLeavesRepo)?;
    if inside
        .components()
        .any(|part| fold(&part.as_os_str().to_string_lossy()) == ".git")
    {
        return Err(LinkRefusal::SourceLeavesRepo);
    }
    Ok(literal)
}

fn refusal_of(trouble: PathTrouble) -> LinkRefusal {
    match trouble {
        PathTrouble::SymlinkInPath { at } => LinkRefusal::SymlinkInPath { at },
        PathTrouble::NotAFolder { at } => LinkRefusal::NotAFolder { at },
        PathTrouble::Changed => LinkRefusal::PathChanged,
        PathTrouble::Io => LinkRefusal::Io,
    }
}

pub struct DotfilesStep<'a> {
    pub runner: &'a dyn CommandRunner,
    pub tools: &'a Tools,
    pub store: &'a dyn ApprovalStore,
    /// Where this Machine keeps the repo, written with `~/`.
    pub clone_location: &'a str,
    /// Names this Run's backup folder. One name per Run keeps its backups together.
    pub stamp: &'a str,
}

impl DotfilesStep<'_> {
    fn approvals(&self) -> Approvals {
        // A record that can't be read approves nothing.
        self.store.read().unwrap_or_default()
    }

    /// The approval for exactly this repo and these links, if there is one.
    fn approval(&self, plan: &DotfilesPlan) -> Option<DotfilesApproval> {
        let source = DotfilesSource::parse(&plan.repo)?;
        self.approvals()
            .dotfiles
            .filter(|held| held.repo == source.key() && held.links == plan.links)
    }

    fn cred_for(source: &DotfilesSource) -> GitCred {
        if source.github().is_some() {
            GitCred::GitHub
        } else {
            GitCred::None
        }
    }

    fn git(
        &self,
        cred: GitCred,
        cwd: &Path,
        args: &[&str],
    ) -> Result<CommandOutput, DotfilesError> {
        self.git_with(cred, cwd, args, &[])
    }

    /// With a clone, git is told where its folder and its files are, so `core.worktree` in the
    /// clone's own config can't send a check somewhere else.
    fn git_with(
        &self,
        cred: GitCred,
        cwd: &Path,
        args: &[&str],
        env: &[(&str, String)],
    ) -> Result<CommandOutput, DotfilesError> {
        let mut command = self
            .tools
            .git(cred, Some(cwd))
            .ok_or(DotfilesError::GitMissing)?
            .args(args.iter().copied());
        let git_dir = cwd.join(".git");
        if fs::symlink_metadata(&git_dir).is_ok_and(|meta| meta.is_dir()) {
            command = command
                .env("GIT_DIR", git_dir.to_string_lossy())
                .env("GIT_WORK_TREE", cwd.to_string_lossy());
        }
        for (key, value) in env {
            command = command.env(*key, value.clone());
        }
        self.runner
            .run(&command, &mut |_, _| {})
            .map_err(|_| DotfilesError::GitMissing)
    }

    fn head(&self, clone: &Path) -> Result<CommitSha, DotfilesError> {
        let output = self.git(
            GitCred::None,
            clone,
            &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"],
        )?;
        CommitSha::parse(output.stdout_text().trim()).ok_or(DotfilesError::LocationBusy)
    }

    /// Whether the files on disk are exactly `commit`'s. Nothing the clone holds is trusted for
    /// this: a fresh index is built from the commit, git hashes every file against it, and only
    /// then says what differs. So index flags, forged file times, a commit-graph and
    /// `core.worktree` can't hide a change. `ignored_in` also counts ignored files under those
    /// paths, because a linked folder carries them along.
    fn matches_commit(
        &self,
        clone: &Path,
        commit: &CommitSha,
        ignored_in: &[&str],
    ) -> Result<bool, DotfilesError> {
        let git_dir = clone.join(".git");
        if !fs::symlink_metadata(&git_dir).is_ok_and(|meta| meta.is_dir()) {
            return Ok(false);
        }
        let index = git_dir.join("soulcaster-check-index");
        let _ = fs::remove_file(&index);
        let env = [("GIT_INDEX_FILE", index.to_string_lossy().into_owned())];
        let result = (|| {
            let built =
                self.git_with(GitCred::None, clone, &["read-tree", commit.as_str()], &env)?;
            if !built.is_success() {
                return Ok(false);
            }
            let _ = self.git_with(
                GitCred::None,
                clone,
                &["update-index", "-q", "--refresh"],
                &env,
            )?;
            let status = self.git_with(
                GitCred::None,
                clone,
                &["status", "--porcelain=v1", "--untracked-files=all"],
                &env,
            )?;
            if !status.is_success() || !status.stdout.is_empty() {
                return Ok(false);
            }
            if ignored_in.is_empty() {
                return Ok(true);
            }
            let mut args = vec![
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--ignored=matching",
                "--",
            ];
            args.extend(ignored_in);
            let ignored = self.git_with(GitCred::None, clone, &args, &env)?;
            Ok(ignored.is_success() && ignored.stdout.is_empty())
        })();
        let _ = fs::remove_file(&index);
        result
    }

    fn fetch(
        &self,
        source: &DotfilesSource,
        clone: &Path,
        commit: &CommitSha,
    ) -> Result<(), DotfilesError> {
        let output = self.git(
            Self::cred_for(source),
            clone,
            &[
                "fetch",
                "--depth=1",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-auto-gc",
                "--",
                "origin",
                commit.as_str(),
            ],
        )?;
        if output.is_success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(
            if stderr.contains("not our ref") || stderr.contains("couldn't find remote ref") {
                DotfilesError::CommitGone
            } else {
                DotfilesError::Offline
            },
        )
    }

    fn checkout(&self, clone: &Path, commit: &CommitSha) -> Result<(), DotfilesError> {
        let output = self.git(
            GitCred::None,
            clone,
            &["checkout", "--detach", "--quiet", commit.as_str()],
        )?;
        if !output.is_success() {
            return Err(DotfilesError::CommitGone);
        }
        let found = self.head(clone)?;
        if found != *commit {
            return Err(DotfilesError::CommitMismatch {
                wanted: commit.clone(),
                found,
            });
        }
        Ok(())
    }

    /// Where the clone goes, without following a symlink to get there.
    fn location(&self) -> Result<(SafeRoot, Vec<String>), DotfilesError> {
        let root = SafeRoot::open(&self.tools.home).map_err(|_| DotfilesError::LocationBusy)?;
        let parts = location_parts(self.clone_location)
            .ok_or(DotfilesError::LocationBusy)?
            .into_iter()
            .map(str::to_string)
            .collect();
        Ok((root, parts))
    }

    /// The existing clone, once it's shown to be a plain repo of this source.
    fn check_existing(&self, clone: &Path, source: &DotfilesSource) -> Result<(), DotfilesError> {
        // A `.git` file can point anywhere, and a `.git` symlink can too.
        let is_real_folder =
            fs::symlink_metadata(clone.join(".git")).is_ok_and(|meta| meta.is_dir());
        if !is_real_folder {
            return Err(DotfilesError::LocationBusy);
        }
        let output = self.git(
            GitCred::None,
            clone,
            &[
                "config",
                "--file",
                ".git/config",
                "--get",
                "remote.origin.url",
            ],
        )?;
        if output.stdout_text().trim() != source.clone_url() {
            return Err(DotfilesError::DifferentRepo);
        }
        Ok(())
    }

    /// Brings the repo to the approved commit if it isn't on this Mac yet. A repo already here
    /// stays as it is: past the approved commit it needs a new preview, and this reports that.
    pub fn ensure_clone(
        &self,
        plan: &DotfilesPlan,
        _stage: UserStage,
    ) -> Result<PathBuf, DotfilesError> {
        let held = self.approval(plan).ok_or(DotfilesError::NotApproved)?;
        let source = DotfilesSource::parse(&plan.repo).ok_or(DotfilesError::NotApproved)?;
        let (root, parts) = self.location()?;
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        root.make_folder(&refs)
            .map_err(|_| DotfilesError::LocationBusy)?;
        let clone = root.path().join(parts.join("/"));
        let empty = fs::read_dir(&clone)
            .map_err(|_| DotfilesError::LocationBusy)?
            .next()
            .is_none();
        if !empty {
            self.check_existing(&clone, &source)?;
            let head = self.head(&clone)?;
            return if head == held.commit {
                Ok(clone)
            } else {
                Err(DotfilesError::PastApproved { head })
            };
        }
        let made = self.fresh_clone(&clone, &source, &held.commit);
        if made.is_err() {
            // Nothing half-made is left for a later step to trust.
            let _ = fs::remove_dir_all(&clone);
        }
        made.map(|()| clone)
    }

    fn fresh_clone(
        &self,
        clone: &Path,
        source: &DotfilesSource,
        commit: &CommitSha,
    ) -> Result<(), DotfilesError> {
        let path = clone.to_string_lossy();
        let setup = [
            vec!["init", "--quiet", "--template=", "--", path.as_ref()],
            vec!["remote", "add", "--", "origin", &source.clone_url()],
        ]
        .map(|args| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>());
        for args in setup {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let output = self.git(GitCred::None, clone, &args)?;
            if !output.is_success() {
                return Err(DotfilesError::LocationBusy);
            }
        }
        self.fetch(source, clone, commit)?;
        self.checkout(clone, commit)
    }

    /// Links every approved location, refusing per link and carrying on with the rest.
    pub fn apply_links(
        &self,
        plan: &DotfilesPlan,
        stage: UserStage,
    ) -> Result<LinkReport, DotfilesError> {
        let clone = self.ensure_clone(plan, stage)?;
        let held = self.approval(plan).ok_or(DotfilesError::NotApproved)?;
        let root = SafeRoot::open(&self.tools.home).map_err(|_| DotfilesError::LocationBusy)?;
        // The same rule a move uses. A file that was edited here isn't what was approved.
        let sources: Vec<&str> = plan.links.values().map(String::as_str).collect();
        let clean = self
            .matches_commit(&clone, &held.commit, &sources)
            .unwrap_or(false);
        let clone = fs::canonicalize(&clone).map_err(|_| DotfilesError::LocationBusy)?;
        let refused = refused_links(plan.links.keys().map(String::as_str), self.clone_location);
        let mut rows = Vec::with_capacity(plan.links.len());
        for (target, source) in &plan.links {
            let outcome = if clean {
                self.apply_one(&root, &clone, target, source, refused.get(target))
            } else {
                LinkOutcome::Refused(LinkRefusal::RepoChanged)
            };
            rows.push(LinkRow {
                target: target.clone(),
                source: source.clone(),
                outcome,
            });
        }
        Ok(LinkReport {
            commit: held.commit,
            rows,
        })
    }

    fn apply_one(
        &self,
        root: &SafeRoot,
        clone: &Path,
        target: &str,
        source: &str,
        refused: Option<&FindingKind>,
    ) -> LinkOutcome {
        let Some(parts) = location_parts(target) else {
            return LinkOutcome::Refused(LinkRefusal::BadLocation);
        };
        if let Some(kind) = refused {
            return LinkOutcome::Refused(LinkRefusal::Lint(*kind));
        }
        let link_text = match verify_source(clone, source) {
            Ok(path) => path,
            Err(why) => return LinkOutcome::Refused(why),
        };
        let mut backup_parts: Vec<&str> = SUPPORT.to_vec();
        backup_parts.push("backups");
        backup_parts.push(self.stamp);
        backup_parts.extend(parts.iter().copied());
        match root.place_link(&parts, &link_text, clone, &backup_parts, &mut || {}) {
            Ok(Placed::Linked { backup }) => LinkOutcome::Linked { backup },
            Ok(Placed::AlreadyLinked) => LinkOutcome::AlreadyLinked,
            Ok(Placed::AnotherLink) => LinkOutcome::Refused(LinkRefusal::AnotherLinkHere),
            Err(trouble) => LinkOutcome::Refused(refusal_of(trouble)),
        }
    }

    /// Runs the install script, once, and only with its own approval of this repo and commit. The
    /// repo on disk has to be exactly the approved commit, with no changes.
    pub fn run_script(
        &self,
        plan: &DotfilesPlan,
        stage: UserStage,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> ScriptOutcome {
        let approvals = self.approvals();
        let (Some(held), Some(path)) = (self.approval(plan), plan.install_script.as_deref()) else {
            return ScriptOutcome::NotApproved;
        };
        let Some(script) = approvals
            .script
            .as_ref()
            .filter(|s| s.repo == held.repo && s.commit == held.commit && s.path == path)
        else {
            return ScriptOutcome::NotApproved;
        };
        if script.ran {
            return ScriptOutcome::AlreadyRan;
        }
        if path.starts_with('-') || safe_parts(path).is_none() {
            return ScriptOutcome::NotApproved;
        }
        let clone = match self.ensure_clone(plan, stage) {
            Ok(clone) => clone,
            Err(DotfilesError::PastApproved { .. }) => return ScriptOutcome::HeldForReview,
            Err(_) => return ScriptOutcome::CouldNotRun,
        };
        match self.matches_commit(&clone, &held.commit, &[path]) {
            Ok(true) => {}
            Ok(false) => return ScriptOutcome::HeldForReview,
            Err(_) => return ScriptOutcome::CouldNotRun,
        }
        let Ok(canonical_clone) = fs::canonicalize(&clone) else {
            return ScriptOutcome::CouldNotRun;
        };
        let literal = canonical_clone.join(path);
        let is_plain_file = fs::symlink_metadata(&literal).is_ok_and(|meta| meta.is_file());
        let inside = fs::canonicalize(&literal).is_ok_and(|c| c == literal);
        if !is_plain_file || !inside {
            return ScriptOutcome::HeldForReview;
        }
        // Once per commit, so it's on record before it runs. If that can't be stored, it doesn't
        // run. This reads again so a revocation since the top of this function holds, and writes
        // what it read, so nothing older goes back over the record.
        let Ok(mut record) = self.store.read() else {
            return ScriptOutcome::CouldNotRun;
        };
        let Some(current) = record
            .script
            .as_mut()
            .filter(|s| s.repo == held.repo && s.commit == held.commit && s.path == path)
        else {
            return ScriptOutcome::NotApproved;
        };
        if current.ran {
            return ScriptOutcome::AlreadyRan;
        }
        current.ran = true;
        if self.store.write(&record).is_err() {
            return ScriptOutcome::CouldNotRun;
        }
        let command = self.tools.script(&literal, &canonical_clone);
        let Ok(output) = self.runner.run(&command, on_line) else {
            // It never started, so it's still owed. If this write fails, it stays marked as run,
            // which is the safe side of "once".
            if let Ok(mut record) = self.store.read()
                && let Some(current) = record.script.as_mut()
            {
                current.ran = false;
                let _ = self.store.write(&record);
            }
            return ScriptOutcome::CouldNotRun;
        };
        ScriptOutcome::Ran {
            success: output.is_success(),
        }
    }

    /// What changed in the linked files between the approved commit and a newer one, as text. It
    /// downloads the newer commit's objects and moves nothing.
    pub fn preview_move(
        &self,
        plan: &DotfilesPlan,
        newer: &CommitSha,
    ) -> Result<MovePreview, DotfilesError> {
        let held = self.approval(plan).ok_or(DotfilesError::NotApproved)?;
        let source = DotfilesSource::parse(&plan.repo).ok_or(DotfilesError::NotApproved)?;
        let clone = self.existing_clone()?;
        self.check_existing(&clone, &source)?;
        self.fetch(&source, &clone, newer)?;
        let mut args: Vec<&str> = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            held.commit.as_str(),
            newer.as_str(),
            "--",
        ];
        for path in plan.links.values() {
            args.push(path);
        }
        if let Some(script) = &plan.install_script {
            args.push(script);
        }
        let output = self.git(GitCred::None, &clone, &args)?;
        // 1 means there are differences. Anything else, a stall included, isn't a preview.
        if !matches!(output.end, CommandEnd::Exited(0 | 1)) {
            return Err(DotfilesError::PreviewFailed);
        }
        let mut text = output.stdout;
        let complete = text.len() <= MAX_PREVIEW;
        text.truncate(MAX_PREVIEW);
        Ok(MovePreview {
            from: held.commit,
            to: newer.clone(),
            text: escaped_lines(&String::from_utf8_lossy(&text)),
            complete,
        })
    }

    /// Moves to the commit a preview showed, only with no changes on this Mac, then records the new
    /// approval. The script's approval doesn't carry over: it was for the old commit. A failed
    /// record leaves the repo past what's approved, which asks for a new preview: the safe side.
    pub fn move_after_review(
        &self,
        plan: &DotfilesPlan,
        preview: &MovePreview,
    ) -> Result<(), DotfilesError> {
        if !preview.complete {
            return Err(DotfilesError::PreviewCutOff);
        }
        let held = self.approval(plan).ok_or(DotfilesError::NotApproved)?;
        if held.commit != preview.from {
            return Err(DotfilesError::NotApproved);
        }
        let source = DotfilesSource::parse(&plan.repo).ok_or(DotfilesError::NotApproved)?;
        let clone = self.existing_clone()?;
        self.check_existing(&clone, &source)?;
        if !self.matches_commit(&clone, &held.commit, &[])? {
            return Err(DotfilesError::ChangesOnThisMac);
        }
        self.fetch(&source, &clone, &preview.to)?;
        self.checkout(&clone, &preview.to)?;
        // Read again and change only this approval, so a revocation since then holds.
        let mut approvals = self.approvals();
        if let Some(record) = approvals
            .dotfiles
            .as_mut()
            .filter(|record| record.repo == held.repo && record.commit == preview.from)
        {
            record.commit = preview.to.clone();
            approvals.script = None;
            let _ = self.store.write(&approvals);
        }
        Ok(())
    }

    fn existing_clone(&self) -> Result<PathBuf, DotfilesError> {
        let (root, parts) = self.location()?;
        Ok(root.path().join(parts.join("/")))
    }
}
