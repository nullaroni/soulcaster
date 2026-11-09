//! Reading GitHub. Signed in, that's `gh api`. Not signed in, or before `gh` exists on a bare Mac,
//! it's an anonymous `curl`, because `gh api` refuses to run with no sign-in and the Import design
//! reads public repos before sign-in. A response is capped by size and checked by shape before
//! anything uses it, and its error text is classified, never repeated, so nothing a server or `gh`
//! says can reach a log or a screen.

use std::fmt;
use std::sync::OnceLock;

use base64::Engine;
use serde::Deserialize;
use soulcaster_core::approval::CommitSha;
use soulcaster_core::command::{Command, CommandOutput, CommandRunner};

use crate::address::{GitRef, RepoId, is_safe_repo_path};
use crate::tools::Tools;

/// The most any listing or small JSON answer may be.
const MAX_ANSWER: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Offline,
    NotVisible {
        signed_in: bool,
    },
    RateLimited,
    TooLarge,
    /// The server's answer isn't what GitHub sends.
    Malformed,
    /// The repo exists and has nothing in it yet.
    Empty,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Offline => "Soulcaster couldn't reach GitHub. Try again.",
            Self::NotVisible { signed_in: false } => {
                "It doesn't exist, or it's private. GitHub gives the same answer for both, so Soulcaster can't tell which. Sign in, in the browser."
            }
            Self::NotVisible { signed_in: true } => {
                "This account can't see it. Check the spelling, or ask the owner."
            }
            Self::RateLimited => {
                "GitHub is limiting this network. Sign in for a higher limit, or try again later."
            }
            Self::TooLarge => "This file is too large to be a Machine Type, so it wasn't read.",
            Self::Malformed => "GitHub's answer didn't make sense, so nothing was kept.",
            Self::Empty => "There's nothing in that repo yet.",
        })
    }
}

impl std::error::Error for FetchError {}

/// The person signed in to `gh`, as GitHub reports them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub login: String,
    pub id: u64,
}

impl Identity {
    /// The address saves use, so the Config Repo never learns a person's real email.
    pub fn noreply(&self) -> String {
        format!("{}+{}@users.noreply.github.com", self.id, self.login)
    }
}

/// What asking `gh` who's signed in came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignIn {
    SignedIn(Identity),
    SignedOut,
    Unreachable,
}

/// GitHub's own answer about a repo. Where it lives now can differ from what was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoFacts {
    pub repo: RepoId,
    pub default_branch: GitRef,
    pub private: bool,
    pub archived: bool,
    pub can_push: Option<bool>,
}

/// One file in a folder listing. Its name and path are checked before they're used.
#[derive(Clone, Debug, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// How a newer commit relates to an approved one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ancestry {
    /// The approved commit is in the newer one's history.
    Ahead,
    /// It isn't, or GitHub wouldn't say.
    Rewritten,
}

pub struct GitHub<'a> {
    runner: &'a dyn CommandRunner,
    tools: &'a Tools,
    sign_in: OnceLock<SignIn>,
}

impl<'a> GitHub<'a> {
    pub fn new(runner: &'a dyn CommandRunner, tools: &'a Tools) -> Self {
        Self {
            runner,
            tools,
            sign_in: OnceLock::new(),
        }
    }

    pub(crate) fn run(&self, command: &Command) -> Option<CommandOutput> {
        // Output goes nowhere but the result: a line from `gh` could hold anything.
        self.runner.run(command, &mut |_, _| {}).ok()
    }

    pub fn sign_in(&self) -> &SignIn {
        self.sign_in.get_or_init(|| self.ask_who_is_signed_in())
    }

    pub fn login(&self) -> Option<&str> {
        match self.sign_in() {
            SignIn::SignedIn(identity) => Some(&identity.login),
            _ => None,
        }
    }

    fn signed_in(&self) -> bool {
        matches!(self.sign_in(), SignIn::SignedIn(_))
    }

    fn ask_who_is_signed_in(&self) -> SignIn {
        #[derive(Deserialize)]
        struct User {
            login: String,
            id: u64,
        }
        let Some(command) = self.tools.gh(["api", "--", "user"]) else {
            return SignIn::SignedOut;
        };
        let Some(output) = self.run(&command) else {
            return SignIn::SignedOut;
        };
        if !output.is_success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return if stderr.contains("HTTP") || stderr.contains("gh auth login") {
                SignIn::SignedOut
            } else {
                SignIn::Unreachable
            };
        }
        match serde_json::from_slice::<User>(&output.stdout) {
            Ok(user) if RepoId::new(&user.login, "x").is_ok() && user.id > 0 => {
                SignIn::SignedIn(Identity {
                    login: user.login,
                    id: user.id,
                })
            }
            _ => SignIn::SignedOut,
        }
    }

    /// `api_path` is built from checked names only, and has no leading `-` or `{`.
    fn get(&self, api_path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        if self.signed_in() {
            self.get_with_gh(api_path, max_bytes)
        } else {
            self.get_with_curl(api_path, max_bytes)
        }
    }

    fn get_with_gh(&self, api_path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        let command = self
            .tools
            .gh([
                "api",
                "--method",
                "GET",
                "--header",
                "Accept: application/vnd.github+json",
                "--",
                api_path,
            ])
            .ok_or(FetchError::Offline)?;
        let output = self.run(&command).ok_or(FetchError::Offline)?;
        if output.is_success() {
            return checked_size(output.stdout, max_bytes);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(classify_status(http_status(&stderr), &stderr, true))
    }

    fn get_with_curl(&self, api_path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        let command = self.tools.curl_get(api_path, max_bytes);
        let output = self.run(&command).ok_or(FetchError::Offline)?;
        if !output.is_success() {
            // curl's own exit code for --max-filesize.
            return Err(
                if output.end == soulcaster_core::command::CommandEnd::Exited(63) {
                    FetchError::TooLarge
                } else {
                    FetchError::Offline
                },
            );
        }
        let mut body = output.stdout;
        let split = body
            .iter()
            .rposition(|b| *b == b'\n')
            .ok_or(FetchError::Malformed)?;
        let status: u16 = std::str::from_utf8(&body[split + 1..])
            .ok()
            .and_then(|code| code.trim().parse().ok())
            .ok_or(FetchError::Malformed)?;
        body.truncate(split);
        if status == 200 {
            return checked_size(body, max_bytes);
        }
        let text = String::from_utf8_lossy(&body);
        Err(classify_status(Some(status), &text, false))
    }

    /// GitHub's answer about a repo, checked. A name that could become a flag stops here.
    pub fn repo_facts(&self, repo: &RepoId) -> Result<RepoFacts, FetchError> {
        #[derive(Deserialize)]
        struct Owner {
            login: String,
        }
        #[derive(Deserialize)]
        struct Permissions {
            push: bool,
        }
        #[derive(Deserialize)]
        struct Repo {
            full_name: String,
            default_branch: String,
            private: bool,
            #[serde(default)]
            archived: bool,
            owner: Owner,
            permissions: Option<Permissions>,
        }
        let bytes = self.get(&format!("repos/{}", repo.full_name()), MAX_ANSWER)?;
        let answer: Repo = serde_json::from_slice(&bytes).map_err(|_| FetchError::Malformed)?;
        let found = RepoId::parse(&answer.full_name).ok_or(FetchError::Malformed)?;
        if !found.owner().eq_ignore_ascii_case(&answer.owner.login) {
            return Err(FetchError::Malformed);
        }
        Ok(RepoFacts {
            repo: found,
            default_branch: GitRef::parse(&answer.default_branch).ok_or(FetchError::Malformed)?,
            private: answer.private,
            archived: answer.archived,
            can_push: answer.permissions.map(|p| p.push),
        })
    }

    /// Resolves a name to one full commit. Everything after this is read at that commit.
    pub fn commit(&self, repo: &RepoId, git_ref: &GitRef) -> Result<CommitSha, FetchError> {
        #[derive(Deserialize)]
        struct Commit {
            sha: String,
        }
        // The answer for a branch can be large, and only its first field matters.
        let path = format!("repos/{}/commits/{}", repo.full_name(), git_ref.as_str());
        let bytes = self.get(&path, 16 * MAX_ANSWER)?;
        let answer: Commit = serde_json::from_slice(&bytes).map_err(|_| FetchError::Malformed)?;
        let found = CommitSha::parse(&answer.sha).ok_or(FetchError::Malformed)?;
        match git_ref.as_commit() {
            Some(asked) if asked != found => Err(FetchError::Malformed),
            _ => Ok(found),
        }
    }

    /// The files directly in one folder, at one commit.
    pub fn list_dir(
        &self,
        repo: &RepoId,
        commit: &CommitSha,
        dir: &str,
    ) -> Result<Vec<DirEntry>, FetchError> {
        if !is_safe_repo_path(dir) {
            return Err(FetchError::Malformed);
        }
        let path = format!("repos/{}/contents/{dir}?ref={commit}", repo.full_name());
        let bytes = self.get(&path, MAX_ANSWER)?;
        serde_json::from_slice(&bytes).map_err(|_| FetchError::Malformed)
    }

    /// One regular file at one commit, up to `max_bytes` once decoded. A symlink or a folder isn't
    /// a file, so it's refused rather than followed.
    pub fn read_file(
        &self,
        repo: &RepoId,
        commit: &CommitSha,
        file: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, FetchError> {
        #[derive(Deserialize)]
        struct File {
            #[serde(rename = "type")]
            kind: String,
            path: String,
            size: u64,
            encoding: Option<String>,
            content: Option<String>,
        }
        if !is_safe_repo_path(file) {
            return Err(FetchError::Malformed);
        }
        let path = format!("repos/{}/contents/{file}?ref={commit}", repo.full_name());
        // Base64 grows by a third and GitHub wraps it, and the JSON around it adds a little.
        let bytes = self.get(&path, max_bytes / 2 * 3 + 64 * 1024)?;
        let answer: File = serde_json::from_slice(&bytes).map_err(|_| FetchError::Malformed)?;
        if answer.kind != "file" || answer.path != file {
            return Err(FetchError::Malformed);
        }
        if answer.size > max_bytes as u64 {
            return Err(FetchError::TooLarge);
        }
        if answer.encoding.as_deref() != Some("base64") {
            return Err(FetchError::Malformed);
        }
        let wrapped = answer.content.ok_or(FetchError::Malformed)?;
        let plain: Vec<u8> = wrapped.bytes().filter(|b| *b != b'\n').collect();
        if plain.len() / 4 * 3 > max_bytes + 3 {
            return Err(FetchError::TooLarge);
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(plain)
            .map_err(|_| FetchError::Malformed)?;
        if decoded.len() > max_bytes {
            return Err(FetchError::TooLarge);
        }
        if decoded.len() as u64 != answer.size {
            return Err(FetchError::Malformed);
        }
        Ok(decoded)
    }
}

fn checked_size(bytes: Vec<u8>, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
    if bytes.len() > max_bytes {
        Err(FetchError::TooLarge)
    } else {
        Ok(bytes)
    }
}

/// `gh` ends a failed request's error with `(HTTP 404)`.
fn http_status(stderr: &str) -> Option<u16> {
    let at = stderr.rfind("(HTTP ")?;
    stderr[at + 6..].split(')').next()?.trim().parse().ok()
}

fn classify_status(status: Option<u16>, text: &str, signed_in: bool) -> FetchError {
    let lower = text.to_ascii_lowercase();
    match status {
        Some(409) => FetchError::Empty,
        Some(429) => FetchError::RateLimited,
        Some(403) if lower.contains("rate limit") => FetchError::RateLimited,
        Some(301 | 302 | 307 | 308 | 401 | 403 | 404 | 422) => FetchError::NotVisible { signed_in },
        _ => FetchError::Offline,
    }
}
