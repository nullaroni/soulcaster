//! Import: from a paste to one validated Config at one commit, read by nothing but the strict
//! parser until it passes. Reading runs nothing from the file, and a failure keeps nothing.

use std::fmt;

use soulcaster_core::approval::CommitSha;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::CommandRunner;
use soulcaster_core::config::{Config, ConfigError, MAX_BYTES};

use crate::address::{AddressError, ConfigPath, DotfilesSource, ImportTarget, RepoId};
use crate::github::{FetchError, GitHub};
use crate::review::{DotfilesFacts, Review};
use crate::text::{escaped, showable_script};
use crate::tools::{GitCred, Tools};

/// The longest script Soulcaster will show. A longer one can't be read, so it can't be approved.
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;

/// How many files in `configs/` a repo can offer. One repo can't make this many reads into more.
pub const MAX_LISTED: usize = 50;

const CONFIG_FOLDER: &str = "configs";

/// Where a copy came from: GitHub's name for the repo, the path, and the one commit it was read at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub repo: RepoId,
    pub path: ConfigPath,
    pub commit: CommitSha,
}

/// One Config at one commit, already through the strict parser.
#[derive(Clone, Debug, PartialEq)]
pub struct Fetched {
    pub source: Source,
    pub config: Config,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalKind {
    TooLarge,
    NotStrictJson,
    DuplicateKey,
    NewerVersion,
    UnknownField,
    BadText,
    Invalid,
}

/// The whole file was refused. `detail` is printable ASCII, so nothing in it can pass for something
/// else on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub at: String,
    pub detail: String,
}

impl Refusal {
    fn of(error: &ConfigError) -> Self {
        let (kind, at) = match error {
            ConfigError::TooLarge { .. } => (RefusalKind::TooLarge, String::new()),
            ConfigError::NewerVersion { .. } => (RefusalKind::NewerVersion, String::new()),
            ConfigError::Parse(message) if message.contains("duplicate key") => {
                (RefusalKind::DuplicateKey, String::new())
            }
            ConfigError::Parse(_) => (RefusalKind::NotStrictJson, String::new()),
            ConfigError::Invalid { path, message } => {
                let kind = if message.contains("Additional properties") {
                    RefusalKind::UnknownField
                } else if path == "/name" {
                    RefusalKind::BadText
                } else {
                    RefusalKind::Invalid
                };
                (kind, path.clone())
            }
        };
        let mut detail = escaped(&error.to_string());
        detail.truncate(400);
        Self {
            kind,
            at: escaped(&at),
            detail,
        }
    }

    pub fn words(&self) -> &'static str {
        match self.kind {
            RefusalKind::TooLarge => "This file is too large to be a Config.",
            RefusalKind::NotStrictJson => "This file isn't valid JSON.",
            RefusalKind::DuplicateKey => {
                "The same key is in this file twice, and programs disagree on which one counts."
            }
            RefusalKind::NewerVersion => "This Config needs a newer version of Soulcaster.",
            RefusalKind::UnknownField => {
                "This file has a field a Config can't have, such as one that would approve something for you."
            }
            RefusalKind::BadText => "The name has characters in it that change how text reads.",
            RefusalKind::Invalid => "This file breaks the Config format.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    Address(AddressError),
    Fetch(FetchError),
    /// The file was fetched and refused in full.
    Refused(Refusal),
    /// A repo with no `configs/*.json`.
    NoConfigs,
    /// A pick that isn't one of the rows offered.
    NotOffered,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(error) => error.fmt(f),
            Self::Fetch(error) => error.fmt(f),
            Self::Refused(refusal) => f.write_str(refusal.words()),
            Self::NoConfigs => {
                f.write_str("No Configs there. Paste a link to the file if it lives somewhere else.")
            }
            Self::NotOffered => f.write_str("That Config wasn't offered."),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<AddressError> for ImportError {
    fn from(error: AddressError) -> Self {
        Self::Address(error)
    }
}

impl From<FetchError> for ImportError {
    fn from(error: FetchError) -> Self {
        Self::Fetch(error)
    }
}

/// One line of the choice between Configs in a repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRow {
    pub path: ConfigPath,
    pub name: Option<String>,
    pub install: usize,
    pub settings: usize,
    pub approvals: usize,
    pub refused: Option<Refusal>,
}

/// A repo with several Configs. A Machine follows one, so a person picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub repo: RepoId,
    pub commit: CommitSha,
    pub rows: Vec<PickRow>,
    /// The folder held more than [`MAX_LISTED`] files.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Opened {
    One(Fetched),
    Pick(Picker),
}

pub struct Importer<'a> {
    github: GitHub<'a>,
    runner: &'a dyn CommandRunner,
    tools: &'a Tools,
    catalog: &'a Catalog,
    clone_location: &'a str,
}

impl<'a> Importer<'a> {
    pub fn connect(
        runner: &'a dyn CommandRunner,
        tools: &'a Tools,
        catalog: &'a Catalog,
        clone_location: &'a str,
    ) -> Self {
        Self {
            github: GitHub::new(runner, tools),
            runner,
            tools,
            catalog,
            clone_location,
        }
    }

    /// The signed-in login, if anyone is.
    pub fn login(&self) -> Option<&str> {
        self.github.login()
    }

    pub fn open(&self, paste: &str) -> Result<Opened, ImportError> {
        match ImportTarget::parse(paste)? {
            ImportTarget::Repo(asked) => self.open_repo(&asked),
            ImportTarget::File {
                repo,
                git_ref,
                path,
            } => {
                let facts = self.github.repo_facts(&repo)?;
                let commit = self.github.commit(&facts.repo, &git_ref)?;
                Ok(Opened::One(self.fetch(&facts.repo, &commit, &path)?))
            }
        }
    }

    fn open_repo(&self, asked: &RepoId) -> Result<Opened, ImportError> {
        let facts = self.github.repo_facts(asked)?;
        let repo = facts.repo;
        let commit = match self.github.commit(&repo, &facts.default_branch) {
            Ok(commit) => commit,
            Err(FetchError::Empty) => return Err(ImportError::NoConfigs),
            Err(error) => return Err(error.into()),
        };
        let listing = match self.github.list_dir(&repo, &commit, CONFIG_FOLDER) {
            Ok(listing) => listing,
            Err(FetchError::NotVisible { .. }) => return Err(ImportError::NoConfigs),
            Err(error) => return Err(error.into()),
        };
        let mut paths: Vec<ConfigPath> = listing
            .iter()
            .filter(|entry| {
                entry.kind == "file" && entry.path == format!("{CONFIG_FOLDER}/{}", entry.name)
            })
            .filter_map(|entry| ConfigPath::parse(&entry.path))
            .collect();
        paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let truncated = paths.len() > MAX_LISTED;
        paths.truncate(MAX_LISTED);
        match paths.as_slice() {
            [] => Err(ImportError::NoConfigs),
            [only] => Ok(Opened::One(self.fetch(&repo, &commit, only)?)),
            _ => {
                let mut rows = Vec::with_capacity(paths.len());
                for path in paths {
                    rows.push(self.pick_row(&repo, &commit, path)?);
                }
                Ok(Opened::Pick(Picker {
                    repo,
                    commit,
                    rows,
                    truncated,
                }))
            }
        }
    }

    fn pick_row(
        &self,
        repo: &RepoId,
        commit: &CommitSha,
        path: ConfigPath,
    ) -> Result<PickRow, ImportError> {
        let refused = |refusal: Refusal, path: ConfigPath| PickRow {
            path,
            name: None,
            install: 0,
            settings: 0,
            approvals: 0,
            refused: Some(refusal),
        };
        match self.fetch(repo, commit, &path) {
            Ok(fetched) => {
                let review = Review::build(
                    &fetched,
                    self.github.login(),
                    &DotfilesFacts::default(),
                    self.catalog,
                    self.clone_location,
                );
                Ok(PickRow {
                    path,
                    name: Some(review.name.clone()),
                    install: review.install_count(),
                    settings: review.settings.len(),
                    approvals: review.approval_count(),
                    refused: None,
                })
            }
            Err(ImportError::Refused(refusal)) => Ok(refused(refusal, path)),
            Err(ImportError::Fetch(FetchError::TooLarge)) => Ok(refused(
                Refusal {
                    kind: RefusalKind::TooLarge,
                    at: String::new(),
                    detail: String::new(),
                },
                path,
            )),
            Err(error) => Err(error),
        }
    }

    /// Reads a row the choice offered, again at the same commit.
    pub fn pick(&self, picker: &Picker, path: &ConfigPath) -> Result<Fetched, ImportError> {
        let offered = picker
            .rows
            .iter()
            .any(|row| row.path == *path && row.refused.is_none());
        if !offered {
            return Err(ImportError::NotOffered);
        }
        self.fetch(&picker.repo, &picker.commit, path)
    }

    fn fetch(
        &self,
        repo: &RepoId,
        commit: &CommitSha,
        path: &ConfigPath,
    ) -> Result<Fetched, ImportError> {
        let bytes = self
            .github
            .read_file(repo, commit, path.as_str(), MAX_BYTES)?;
        let config = Config::parse(&bytes).map_err(|e| ImportError::Refused(Refusal::of(&e)))?;
        Ok(Fetched {
            source: Source {
                repo: repo.clone(),
                path: path.clone(),
                commit: commit.clone(),
            },
            config,
        })
    }

    /// Everything the one review page shows, after asking GitHub about the Dotfiles Repo. Nothing
    /// is downloaded but the script's text, and that only to show it.
    pub fn review(&self, fetched: &Fetched) -> Review {
        Review::build(
            fetched,
            self.github.login(),
            &self.dotfiles_facts(fetched),
            self.catalog,
            self.clone_location,
        )
    }

    fn dotfiles_facts(&self, fetched: &Fetched) -> DotfilesFacts {
        let Some(dotfiles) = &fetched.config.dotfiles else {
            return DotfilesFacts::default();
        };
        match DotfilesSource::parse(&dotfiles.repo) {
            Some(DotfilesSource::GitHub(repo)) => {
                self.github_dotfiles_facts(&repo, dotfiles.install_script.as_deref())
            }
            Some(other) => DotfilesFacts {
                commit: self.ask_git_for_commit(&other.clone_url()),
                current_owner: None,
                // Reading a file on another host needs a clone, so its script stays unshown.
                script_text: None,
            },
            None => DotfilesFacts::default(),
        }
    }

    fn github_dotfiles_facts(&self, repo: &RepoId, script: Option<&str>) -> DotfilesFacts {
        let Ok(facts) = self.github.repo_facts(repo) else {
            return DotfilesFacts::default();
        };
        let commit = self.github.commit(&facts.repo, &facts.default_branch).ok();
        let script_text = match (&commit, script) {
            (Some(commit), Some(path)) => self
                .github
                .read_file(&facts.repo, commit, path, MAX_SCRIPT_BYTES)
                .ok()
                .and_then(|bytes| showable_script(&bytes)),
            _ => None,
        };
        DotfilesFacts {
            commit,
            current_owner: Some(facts.repo.owner().to_string()),
            script_text,
        }
    }

    /// A Dotfiles Repo on another host. Git's own answer, with no sign-in offered to that host.
    fn ask_git_for_commit(&self, url: &str) -> Option<CommitSha> {
        let command =
            self.tools
                .git(GitCred::None, None)?
                .args(["ls-remote", "--", url, "HEAD"]);
        let output = self.runner.run(&command, &mut |_, _| {}).ok()?;
        if !output.is_success() {
            return None;
        }
        let text = output.stdout_text();
        CommitSha::parse(text.split_whitespace().next()?)
    }
}
