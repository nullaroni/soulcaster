//! Adopt: saving what a person reviewed on top of whatever GitHub has now. Only the reviewed
//! entries change. An entry someone else changed comes back as a row instead of being replaced.

use std::collections::BTreeSet;
use std::path::PathBuf;

use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::CommandRunner;
use soulcaster_core::config::{Config, MAX_BYTES};
use soulcaster_core::drift::{Reviewed, adopt_onto};

use crate::address::RepoId;
use crate::copies::{keep_own, slug};
use crate::github::{FetchError, GitHub, SignIn};
use crate::save::{SaveError, SaveOutcome, SaveRequest, WaitReason, save};
use crate::tools::Tools;

const TRIES: usize = 3;

pub struct AdoptRequest<'a> {
    pub repo_name: &'a str,
    /// The Config this Machine follows, which names its file in the repo.
    pub config_name: &'a str,
    /// What this Mac last read. It's only used when GitHub can't be asked.
    pub held: &'a Config,
    pub reviewed: &'a [Reviewed],
    /// Whether this Mac still holds what the person chose to save.
    pub still_true: &'a dyn Fn(&Reviewed) -> bool,
    /// False for a Config that has never been sent, whose file isn't expected to be there.
    pub ever_saved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdoptOutcome {
    /// GitHub has it. Only now may the last-applied record move.
    Saved {
        public: bool,
    },
    Waiting {
        why: WaitReason,
        local: PathBuf,
    },
    ChangedElsewhere {
        ids: Vec<String>,
    },
    ChangedAgain {
        ids: Vec<String>,
    },
    /// The file on GitHub can't be read. Adopt waits, and nothing was written.
    Unreadable,
    /// The file is gone from GitHub. Writes stop, and nothing was written.
    Missing {
        path: String,
    },
}

enum Latest {
    Text(String),
    /// Never sent, so there's nothing there to go on top of.
    Nothing,
}

enum Stop {
    Wait(WaitReason),
    Unreadable,
    Missing(String),
}

fn wait_for(error: &FetchError) -> WaitReason {
    match error {
        FetchError::NotVisible { .. } => WaitReason::SignIn,
        FetchError::Malformed => WaitReason::Denied,
        _ => WaitReason::Offline,
    }
}

fn read_latest(
    github: &GitHub<'_>,
    request: &AdoptRequest<'_>,
    path: &str,
) -> Result<Latest, Stop> {
    let identity = match github.sign_in() {
        SignIn::SignedIn(identity) => identity.clone(),
        SignIn::SignedOut => return Err(Stop::Wait(WaitReason::SignIn)),
        SignIn::Unreachable => return Err(Stop::Wait(WaitReason::Offline)),
    };
    let repo = RepoId::new(&identity.login, request.repo_name)
        .map_err(|_| Stop::Wait(WaitReason::Denied))?;
    let gone = || {
        if request.ever_saved {
            Err(Stop::Missing(path.to_string()))
        } else {
            Ok(Latest::Nothing)
        }
    };
    let facts = match github.repo_facts(&repo) {
        Ok(facts) => facts,
        Err(FetchError::NotVisible { signed_in: true }) => return gone(),
        Err(error) => return Err(Stop::Wait(wait_for(&error))),
    };
    let head = match github.commit(&facts.repo, &facts.default_branch) {
        Ok(head) => head,
        Err(FetchError::Empty) => return gone(),
        Err(error) => return Err(Stop::Wait(wait_for(&error))),
    };
    match github.read_file(&facts.repo, &head, path, MAX_BYTES) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Latest::Text)
            .map_err(|_| Stop::Unreadable),
        Err(FetchError::NotVisible { signed_in: true }) => gone(),
        Err(FetchError::TooLarge) => Err(Stop::Unreadable),
        Err(error) => Err(Stop::Wait(wait_for(&error))),
    }
}

fn parse_latest(text: &str, config_name: &str) -> Result<Config, Stop> {
    let config = Config::parse(text.as_bytes()).map_err(|_| Stop::Unreadable)?;
    // The file's own name picks where a save lands, so a renamed one would fork into a new file.
    if slug(&config.name) != slug(config_name) {
        return Err(Stop::Unreadable);
    }
    Ok(config)
}

enum Sent {
    Done { public: bool },
    Moved,
    Waiting(WaitReason, PathBuf),
}

fn send(
    runner: &dyn CommandRunner,
    tools: &Tools,
    catalog: &Catalog,
    request: &AdoptRequest<'_>,
    config: &Config,
    base: Option<&str>,
) -> Result<Sent, SaveError> {
    let outcome = save(
        runner,
        tools,
        catalog,
        &SaveRequest {
            config,
            repo_name: request.repo_name,
            base,
        },
    )?;
    Ok(match outcome {
        SaveOutcome::Sent { public, .. } | SaveOutcome::AlreadyThere { public, .. } => {
            Sent::Done { public }
        }
        SaveOutcome::ChangedElsewhere { .. } => Sent::Moved,
        SaveOutcome::Waiting { why, local } => Sent::Waiting(why, local),
    })
}

/// Keeps the change on this Mac when GitHub can't be asked, built on what this Mac last read.
fn wait_here(
    tools: &Tools,
    request: &AdoptRequest<'_>,
    always: &BTreeSet<&str>,
    why: WaitReason,
) -> Result<AdoptOutcome, SaveError> {
    let merged = adopt_onto(request.held, request.reviewed, always);
    let text = merged.as_ref().unwrap_or(request.held).to_canonical_json();
    let local = keep_own(&tools.home, request.config_name, &text).map_err(SaveError::LocalWrite)?;
    Ok(AdoptOutcome::Waiting { why, local })
}

/// Reads the latest Config, puts the reviewed entries on it, and sends that. A hand-edited file
/// gets its reformat as a save of its own first, so the entry's change stays readable.
pub fn adopt(
    runner: &dyn CommandRunner,
    tools: &Tools,
    catalog: &Catalog,
    request: &AdoptRequest<'_>,
) -> Result<AdoptOutcome, SaveError> {
    let always: BTreeSet<&str> = catalog.base_items().map(|entry| entry.id()).collect();
    let path = format!("configs/{}.json", slug(request.config_name));
    if tools.gh.is_none() || tools.git.is_none() {
        return wait_here(tools, request, &always, WaitReason::SignIn);
    }
    let github = GitHub::new(runner, tools);
    for _ in 0..TRIES {
        let again: Vec<String> = request
            .reviewed
            .iter()
            .filter(|entry| !(request.still_true)(entry))
            .map(|entry| entry.id.clone())
            .collect();
        if !again.is_empty() {
            return Ok(AdoptOutcome::ChangedAgain { ids: again });
        }
        let latest = match read_latest(&github, request, &path) {
            Ok(latest) => latest,
            Err(Stop::Wait(why)) => return wait_here(tools, request, &always, why),
            Err(Stop::Unreadable) => return Ok(AdoptOutcome::Unreadable),
            Err(Stop::Missing(path)) => return Ok(AdoptOutcome::Missing { path }),
        };
        let (config, text) = match latest {
            Latest::Nothing => (request.held.clone(), None),
            Latest::Text(text) => match parse_latest(&text, request.config_name) {
                Ok(config) => (config, Some(text)),
                Err(_) => return Ok(AdoptOutcome::Unreadable),
            },
        };
        let merged = match adopt_onto(&config, request.reviewed, &always) {
            Ok(merged) => merged,
            Err(ids) => return Ok(AdoptOutcome::ChangedElsewhere { ids }),
        };
        let canonical = config.to_canonical_json();
        let mut base = text.clone();
        if let Some(text) = &text
            && *text != canonical
        {
            match send(runner, tools, catalog, request, &config, Some(text))? {
                Sent::Done { .. } => base = Some(canonical),
                Sent::Moved => continue,
                Sent::Waiting(why, local) => return Ok(AdoptOutcome::Waiting { why, local }),
            }
        }
        match send(runner, tools, catalog, request, &merged, base.as_deref())? {
            Sent::Done { public } => return Ok(AdoptOutcome::Saved { public }),
            Sent::Moved => {}
            Sent::Waiting(why, local) => return Ok(AdoptOutcome::Waiting { why, local }),
        }
    }
    wait_here(tools, request, &always, WaitReason::Busy)
}
