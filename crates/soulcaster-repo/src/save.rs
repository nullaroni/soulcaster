//! Saving a Config to the person's own GitHub repo. Nothing a Run does waits on this: a Run plans
//! from the Config it holds, and a save that can't reach GitHub leaves the change on this Mac.
//! Every file goes through [`Config::to_canonical_json`] and back through the strict parser first.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use soulcaster_core::approval::CommitSha;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{CommandOutput, CommandRunner};
use soulcaster_core::config::{Config, ConfigError, MAX_BYTES};
use soulcaster_core::lint::{DEFAULT_CLONE_LOCATION, Finding, FindingKind, lint};

use crate::address::RepoId;
use crate::copies::{CopyError, SUPPORT, keep_own, slug};
use crate::github::{FetchError, GitHub, Identity, RepoFacts, SignIn};
use crate::safe_fs::{PathTrouble, SafeRoot};
use crate::tools::{GitCred, Tools};

/// Where a person's Configs go when they have no Config Repo yet. It's created private.
pub const CONFIG_REPO_NAME: &str = "soulcaster-configs";

/// How many times a save goes back on top of whatever GitHub has now.
const TRIES: usize = 3;

#[derive(Clone, Debug)]
pub struct SaveRequest<'a> {
    pub config: &'a Config,
    pub repo_name: &'a str,
    /// The text this Mac last sent for this file, if any. A file on GitHub matching neither this
    /// nor the new text was changed elsewhere, so it comes back to the person.
    pub base: Option<&'a str>,
}

/// Why a change is still waiting on this Mac. Nothing is lost in any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitReason {
    Offline,
    SignIn,
    Denied,
    Busy,
}

impl WaitReason {
    /// The sync design's sentence for this.
    pub fn words(self) -> &'static str {
        match self {
            Self::Offline => {
                "Saved on this Mac, but not on GitHub yet. Your other Macs can't see this until it's sent."
            }
            Self::SignIn => {
                "GitHub needs you to sign in again before this can be sent. Nothing is lost."
            }
            Self::Denied => {
                "GitHub didn't accept this. You may not be allowed to change this Config, or its rules may not allow direct changes. Nothing is lost."
            }
            Self::Busy => {
                "Your Config keeps changing on GitHub, so this wasn't sent. Nothing is lost. Try again in a moment."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Sent {
        repo: RepoId,
        path: String,
        commit: CommitSha,
        public: bool,
        created_repo: bool,
    },
    /// GitHub already has exactly this, so nothing was written.
    AlreadyThere {
        repo: RepoId,
        path: String,
        public: bool,
    },
    Waiting {
        why: WaitReason,
        local: PathBuf,
    },
    /// Someone changed this file on GitHub. It wasn't replaced.
    ChangedElsewhere {
        path: String,
    },
}

/// Why a save was refused outright, before any command ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// Entries that must not leave this Mac. Each must be removed first.
    Blocked(Vec<Finding>),
    Invalid(ConfigError),
    LocalWrite(CopyError),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked(findings) => {
                let at: Vec<&str> = findings.iter().map(|f| f.at.as_str()).collect();
                write!(
                    f,
                    "These can't be saved to a Config until they're removed: {}",
                    crate::text::escaped(&at.join(", "))
                )
            }
            Self::Invalid(error) => write!(f, "{}", crate::text::escaped(&error.to_string())),
            Self::LocalWrite(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SaveError {}

fn must_stay_on_this_mac(finding: &Finding) -> bool {
    matches!(
        finding.kind,
        FindingKind::SecretValue
            | FindingKind::SecretKey
            | FindingKind::EmailValue
            | FindingKind::HomePathValue
            | FindingKind::MachineValue
    )
}

/// Saves on this Mac, then sends to GitHub. The local save happens first and stays whatever GitHub
/// says. The outcome is the only thing a Run could wait on, and none does.
pub fn save(
    runner: &dyn CommandRunner,
    tools: &Tools,
    catalog: &Catalog,
    request: &SaveRequest<'_>,
) -> Result<SaveOutcome, SaveError> {
    let config = request.config;
    let text = config.to_canonical_json();
    Config::parse(text.as_bytes()).map_err(SaveError::Invalid)?;
    let blocked: Vec<Finding> = lint(config, catalog, DEFAULT_CLONE_LOCATION)
        .into_iter()
        .filter(must_stay_on_this_mac)
        .collect();
    if !blocked.is_empty() {
        return Err(SaveError::Blocked(blocked));
    }
    let local = keep_own(&tools.home, &config.name, &text).map_err(SaveError::LocalWrite)?;
    let waiting = |why| SaveOutcome::Waiting {
        why,
        local: local.clone(),
    };
    if tools.gh.is_none() || tools.git.is_none() {
        return Ok(waiting(WaitReason::SignIn));
    }
    let github = GitHub::new(runner, tools);
    let identity = match github.sign_in() {
        SignIn::SignedIn(identity) => identity.clone(),
        SignIn::SignedOut => return Ok(waiting(WaitReason::SignIn)),
        SignIn::Unreachable => return Ok(waiting(WaitReason::Offline)),
    };
    let sender = Sender {
        runner,
        tools,
        github: &github,
        identity,
        repo_name: request.repo_name,
        base: request.base,
        text: &text,
        path: format!("configs/{}.json", slug(&config.name)),
    };
    let mut created_repo = false;
    for _ in 0..TRIES {
        match sender.try_once(&mut created_repo) {
            Try::Rejected => {}
            Try::Wait(why) => return Ok(waiting(why)),
            Try::Done(outcome) => return Ok(outcome),
        }
    }
    Ok(waiting(WaitReason::Busy))
}

enum Try {
    /// GitHub moved while this was being sent, so go again on top of what's there now.
    Rejected,
    Wait(WaitReason),
    Done(SaveOutcome),
}

fn wait_for_fetch(error: &FetchError) -> WaitReason {
    match error {
        FetchError::NotVisible { .. } => WaitReason::SignIn,
        FetchError::Malformed => WaitReason::Denied,
        _ => WaitReason::Offline,
    }
}

/// Git's words, read in one language and never repeated.
fn classify_git(output: &CommandOutput) -> Try {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let has = |word: &str| stderr.contains(word);
    if has("GH006") || has("Permission") || has("denied") || has("protected") || has("403") {
        Try::Wait(WaitReason::Denied)
    } else if has("could not read Username")
        || has("Authentication failed")
        || has("terminal prompts disabled")
        || has("401")
    {
        Try::Wait(WaitReason::SignIn)
    } else if has("rejected") || has("non-fast-forward") || has("fetch first") {
        Try::Rejected
    } else {
        Try::Wait(WaitReason::Offline)
    }
}

/// A working folder that is gone when the save is done, whatever happened.
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    fn new(home: &Path) -> Result<Self, PathTrouble> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = SafeRoot::open(home)?;
        let name = format!(
            "save-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let mut parts: Vec<&str> = SUPPORT.to_vec();
        parts.push("work");
        parts.push(&name);
        root.make_folder(&parts)?;
        Ok(Self {
            path: root.path().join(parts.join("/")),
        })
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct Sender<'a> {
    runner: &'a dyn CommandRunner,
    tools: &'a Tools,
    github: &'a GitHub<'a>,
    identity: Identity,
    repo_name: &'a str,
    base: Option<&'a str>,
    text: &'a str,
    path: String,
}

impl Sender<'_> {
    fn git(&self, cwd: &Path, args: &[&str]) -> Option<CommandOutput> {
        let command = self
            .tools
            .git(GitCred::GitHub, Some(cwd))?
            .args(args.iter().copied());
        self.runner.run(&command, &mut |_, _| {}).ok()
    }

    /// GitHub's answer about the person's repo, making it private if there isn't one.
    fn repo(&self, created: &mut bool) -> Result<RepoFacts, WaitReason> {
        let id =
            RepoId::new(&self.identity.login, self.repo_name).map_err(|_| WaitReason::Denied)?;
        let facts = match self.github.repo_facts(&id) {
            Ok(facts) => facts,
            Err(FetchError::NotVisible { .. }) => {
                let command = self
                    .tools
                    .gh(["repo", "create", &id.full_name(), "--private"])
                    .ok_or(WaitReason::SignIn)?;
                let output = self.github.run(&command).ok_or(WaitReason::Offline)?;
                if !output.is_success() {
                    return Err(WaitReason::Denied);
                }
                *created = true;
                self.github
                    .repo_facts(&id)
                    .map_err(|e| wait_for_fetch(&e))?
            }
            Err(error) => return Err(wait_for_fetch(&error)),
        };
        // Never write to a repo that isn't the person's, wherever a name led.
        let theirs = facts
            .repo
            .owner()
            .eq_ignore_ascii_case(&self.identity.login);
        if !theirs || facts.archived || facts.can_push == Some(false) {
            return Err(WaitReason::Denied);
        }
        Ok(facts)
    }

    /// The newest commit on the default branch. `None` is a repo with nothing in it yet.
    fn head(&self, facts: &RepoFacts) -> Result<Option<CommitSha>, WaitReason> {
        match self.github.commit(&facts.repo, &facts.default_branch) {
            Ok(commit) => Ok(Some(commit)),
            Err(FetchError::Empty) => Ok(None),
            Err(error) => Err(wait_for_fetch(&error)),
        }
    }

    fn try_once(&self, created: &mut bool) -> Try {
        let facts = match self.repo(created) {
            Ok(facts) => facts,
            Err(why) => return Try::Wait(why),
        };
        let head = match self.head(&facts) {
            Ok(head) => head,
            Err(why) => return Try::Wait(why),
        };
        let Ok(work) = WorkDir::new(&self.tools.home) else {
            return Try::Wait(WaitReason::Denied);
        };
        self.in_work_dir(&work.path, &facts, head.as_ref(), *created)
    }

    fn in_work_dir(
        &self,
        work: &Path,
        facts: &RepoFacts,
        head: Option<&CommitSha>,
        created: bool,
    ) -> Try {
        macro_rules! git_or_wait {
            ($($arg:expr),+ $(,)?) => {{
                match self.git(work, &[$($arg),+]) {
                    Some(output) if output.is_success() => output,
                    Some(output) => return classify_git(&output),
                    None => return Try::Wait(WaitReason::Offline),
                }
            }};
        }
        let url = facts.repo.clone_url();
        let work_text = work.to_string_lossy().into_owned();
        git_or_wait!("init", "--quiet", "--template=", "--", &work_text);
        git_or_wait!("remote", "add", "--", "origin", &url);
        if let Some(head) = head {
            git_or_wait!(
                "fetch",
                "--depth=1",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-auto-gc",
                "--",
                "origin",
                head.as_str(),
            );
            git_or_wait!("checkout", "--detach", "--quiet", head.as_str());
        }

        let Ok(root) = SafeRoot::open(work) else {
            return Try::Wait(WaitReason::Denied);
        };
        let parts: Vec<&str> = self.path.split('/').collect();
        let public = !facts.private;
        let existing = match root.read_file(&parts, MAX_BYTES as u64 + 1) {
            Ok(existing) => existing,
            Err(_) => return Try::Wait(WaitReason::Denied),
        };
        if let Some(existing) = existing {
            if existing == self.text.as_bytes() {
                return Try::Done(SaveOutcome::AlreadyThere {
                    repo: facts.repo.clone(),
                    path: self.path.clone(),
                    public,
                });
            }
            if self.base.is_none_or(|base| base.as_bytes() != existing) {
                return Try::Done(SaveOutcome::ChangedElsewhere {
                    path: self.path.clone(),
                });
            }
        }
        if root
            .write_file(&parts, self.text.as_bytes(), 0o644)
            .is_err()
        {
            return Try::Wait(WaitReason::Denied);
        }

        git_or_wait!("add", "--", &self.path);
        let name = format!("user.name={}", self.identity.login);
        let email = format!("user.email={}", self.identity.noreply());
        let message = format!("Save {}", self.path);
        git_or_wait!(
            "-c",
            &name,
            "-c",
            &email,
            "commit",
            "--quiet",
            "--no-verify",
            "--no-gpg-sign",
            "-m",
            &message,
        );
        let refspec = format!("HEAD:refs/heads/{}", facts.default_branch.as_str());
        git_or_wait!("push", "--quiet", "--no-verify", "--", "origin", &refspec);
        let output = git_or_wait!("rev-parse", "--verify", "--end-of-options", "HEAD^{commit}");
        let Some(commit) = CommitSha::parse(output.stdout_text().trim()) else {
            return Try::Wait(WaitReason::Denied);
        };
        Try::Done(SaveOutcome::Sent {
            repo: facts.repo.clone(),
            path: self.path.clone(),
            commit,
            public,
            created_repo: created,
        })
    }
}
