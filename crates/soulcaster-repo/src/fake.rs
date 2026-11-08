//! A fake GitHub and git for tests and the simulated window, in the spirit of core's `FakeRunner`.
//! It starts nothing. It answers `gh api` and `curl` from a table, and git from scripted replies.
//! [`FileApprovals`] is a test double: real approvals belong in the Machine Record.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use serde_json::{Value, json};
use soulcaster_core::approval::{ApprovalStore, ApprovalStoreError, Approvals};
use soulcaster_core::command::{
    Command, CommandEnd, CommandOutput, CommandRunner, OutputStream, Reply, SpawnError,
};

#[derive(Clone, Debug)]
pub struct RepoShape {
    pub default_branch: String,
    pub private: bool,
    /// `None` is a repo with nothing in it yet.
    pub head: Option<String>,
}

impl RepoShape {
    pub fn public(default_branch: &str, head: &str) -> Self {
        Self {
            default_branch: default_branch.to_string(),
            private: false,
            head: Some(head.to_string()),
        }
    }

    pub fn private(default_branch: &str, head: &str) -> Self {
        Self {
            private: true,
            ..Self::public(default_branch, head)
        }
    }
}

#[derive(Default)]
struct State {
    login: Option<(String, u64)>,
    responses: BTreeMap<String, (u16, String)>,
    calls: Vec<Command>,
    offline: bool,
    fail_all: Option<String>,
    scripted: Vec<(String, VecDeque<Reply>)>,
    programs: Vec<(String, Reply)>,
    effects: Vec<(String, Effect)>,
    seen: BTreeMap<(String, String), String>,
}

#[derive(Clone)]
enum Effect {
    Files(Vec<(String, String)>),
    Symlink(String, PathBuf),
}

#[derive(Default)]
pub struct FakeGitHub {
    state: Mutex<State>,
}

impl FakeGitHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sign_in(&self, login: &str, id: u64) {
        self.state.lock().unwrap().login = Some((login.to_string(), id));
    }

    pub fn sign_out(&self) {
        self.state.lock().unwrap().login = None;
    }

    pub fn go_offline(&self) {
        self.state.lock().unwrap().offline = true;
    }

    /// Every `gh` and `curl` fails with this on stderr.
    pub fn fail_all_with(&self, stderr: &str) {
        self.state.lock().unwrap().fail_all = Some(stderr.to_string());
    }

    pub fn set_response(&self, api_path: &str, status: u16, body: &str) {
        self.state
            .lock()
            .unwrap()
            .responses
            .insert(api_path.to_string(), (status, body.to_string()));
    }

    pub fn add_repo(&self, owner: &str, name: &str, shape: RepoShape) {
        let full = format!("{owner}/{name}");
        self.set_response(
            &format!("repos/{full}"),
            200,
            &json!({
                "full_name": full,
                "default_branch": shape.default_branch,
                "private": shape.private,
                "owner": {"login": owner},
                "permissions": {"push": true},
            })
            .to_string(),
        );
        let branch = format!("repos/{full}/commits/{}", shape.default_branch);
        match shape.head {
            Some(head) => {
                let answer = json!({ "sha": head }).to_string();
                self.set_response(&branch, 200, &answer);
                self.set_response(&format!("repos/{full}/commits/{head}"), 200, &answer);
            }
            None => self.set_response(&branch, 409, "Git Repository is empty."),
        }
    }

    /// A file at a commit, and its line in its folder's listing.
    pub fn add_file(&self, owner: &str, name: &str, commit: &str, path: &str, bytes: &[u8]) {
        let full = format!("{owner}/{name}");
        self.set_response(
            &format!("repos/{full}/contents/{path}?ref={commit}"),
            200,
            &json!({
                "type": "file",
                "encoding": "base64",
                "size": bytes.len(),
                "path": path,
                "content": base64::engine::general_purpose::STANDARD.encode(bytes),
            })
            .to_string(),
        );
        let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
        let listing = format!("repos/{full}/contents/{dir}?ref={commit}");
        let mut entries: Vec<Value> = self
            .state
            .lock()
            .unwrap()
            .responses
            .get(&listing)
            .and_then(|(_, body)| serde_json::from_str(body).ok())
            .unwrap_or_default();
        entries.retain(|entry| entry["name"] != file);
        entries.push(json!({"name": file, "path": path, "type": "file", "size": bytes.len()}));
        self.set_response(&listing, 200, &Value::Array(entries).to_string());
    }

    /// git commands holding `word` answer with `reply`. The latest script for a word wins.
    pub fn on_git(&self, word: &str, reply: Reply) {
        self.on_git_replies(word, vec![reply]);
    }

    /// As [`Self::on_git`], with replies in turn. The last one repeats.
    pub fn on_git_replies(&self, word: &str, replies: Vec<Reply>) {
        self.state
            .lock()
            .unwrap()
            .scripted
            .insert(0, (word.to_string(), replies.into()));
    }

    /// A program whose path ends in `suffix` answers with `reply`.
    pub fn on_program(&self, suffix: &str, reply: Reply) {
        self.state
            .lock()
            .unwrap()
            .programs
            .insert(0, (suffix.to_string(), reply));
    }

    /// When a git command holding `word` runs, these files appear under its working folder, as a
    /// real checkout would leave them.
    pub fn set_git_files(&self, word: &str, files: Vec<(&str, &str)>) {
        let files = files
            .into_iter()
            .map(|(p, c)| (p.to_string(), c.to_string()))
            .collect();
        self.state
            .lock()
            .unwrap()
            .effects
            .push((word.to_string(), Effect::Files(files)));
    }

    /// As [`Self::set_git_files`], for a symlink a repo planted.
    pub fn set_git_symlink(&self, word: &str, at: &str, target: PathBuf) {
        self.state
            .lock()
            .unwrap()
            .effects
            .push((word.to_string(), Effect::Symlink(at.to_string(), target)));
    }

    /// What a file held when a git command holding `word` named it after `--`.
    pub fn file_at_git(&self, word: &str, path: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .seen
            .get(&(word.to_string(), path.to_string()))
            .cloned()
    }

    pub fn calls(&self) -> Vec<Command> {
        self.state.lock().unwrap().calls.clone()
    }

    /// Every API path asked for, by `gh` or `curl`, in order.
    pub fn api_paths(&self) -> Vec<String> {
        self.calls()
            .iter()
            .filter_map(|call| {
                if call.program.ends_with("gh") && call.args.first().is_some_and(|a| a == "api") {
                    call.args.last().cloned()
                } else if call.program.ends_with("curl") {
                    call.args
                        .last()
                        .and_then(|url| url.strip_prefix("https://api.github.com/"))
                        .map(str::to_string)
                } else {
                    None
                }
            })
            .collect()
    }

    fn answer_gh(state: &mut State, command: &Command) -> Reply {
        if let Some(stderr) = &state.fail_all {
            return Reply::fails(1, stderr.clone());
        }
        if state.offline {
            return Reply::fails(1, "dial tcp: lookup api.github.com: no such host");
        }
        if command.args.first().is_some_and(|a| a == "repo") {
            if let Some(full) = command.args.get(2) {
                let (owner, name) = full.split_once('/').unwrap_or((full, ""));
                let body = json!({
                    "full_name": full,
                    "default_branch": "main",
                    "private": command.args.iter().any(|a| a == "--private"),
                    "owner": {"login": owner},
                    "permissions": {"push": true},
                })
                .to_string();
                state
                    .responses
                    .insert(format!("repos/{owner}/{name}"), (200, body));
                state.responses.insert(
                    format!("repos/{owner}/{name}/commits/main"),
                    (409, "Git Repository is empty.".to_string()),
                );
            }
            return Reply::ok("");
        }
        let path = command.args.last().map_or("", String::as_str);
        if path == "user" {
            return match &state.login {
                Some((login, id)) => Reply::ok(json!({"login": login, "id": id}).to_string()),
                None => Reply::Output {
                    code: 4,
                    stdout: String::new(),
                    stderr: "To get started with GitHub CLI, please run:  gh auth login"
                        .to_string(),
                },
            };
        }
        match state.responses.get(path) {
            Some((200, body)) => Reply::ok(body.clone()),
            Some((status, body)) => Reply::fails(1, format!("gh: {body} (HTTP {status})")),
            None => Reply::fails(1, "gh: Not Found (HTTP 404)"),
        }
    }

    fn answer_curl(state: &State, command: &Command) -> Reply {
        if state.fail_all.is_some() {
            return Reply::fails(22, "curl: (22) failed");
        }
        if state.offline {
            return Reply::fails(6, "curl: (6) Could not resolve host: api.github.com");
        }
        let url = command.args.last().map_or("", String::as_str);
        let path = url.strip_prefix("https://api.github.com/").unwrap_or(url);
        match state.responses.get(path) {
            Some((status, body)) => Reply::ok(format!("{body}\n{status}")),
            None => Reply::ok("{\"message\":\"Not Found\"}\n404"),
        }
    }

    fn apply_effects(state: &mut State, command: &Command) {
        let Some(cwd) = command.cwd.clone() else {
            return;
        };
        for (word, effect) in state.effects.clone() {
            if !command.args.contains(&word) {
                continue;
            }
            match effect {
                Effect::Files(files) => {
                    for (path, content) in files {
                        let at = cwd.join(path);
                        if let Some(parent) = at.parent() {
                            std::fs::create_dir_all(parent).expect("a test folder");
                        }
                        std::fs::write(at, content).expect("a test file");
                    }
                }
                Effect::Symlink(at, target) => {
                    std::os::unix::fs::symlink(target, cwd.join(at)).expect("a test symlink");
                }
            }
        }
        if let Some(dash) = command.args.iter().position(|a| a == "--") {
            let subcommand = command.args[..dash]
                .iter()
                .rev()
                .find(|a| !a.starts_with('-') && !a.contains('='))
                .cloned()
                .unwrap_or_default();
            for path in &command.args[dash + 1..] {
                if let Ok(text) = std::fs::read_to_string(cwd.join(path)) {
                    state.seen.insert((subcommand.clone(), path.clone()), text);
                }
            }
        }
    }

    fn answer_git(state: &mut State, command: &Command) -> Reply {
        Self::apply_effects(state, command);
        for (word, replies) in &mut state.scripted {
            if command.args.iter().any(|a| a == word) {
                return match replies.len() {
                    0 => Reply::ok(""),
                    1 => replies[0].clone(),
                    _ => replies.pop_front().expect("checked the length"),
                };
            }
        }
        if command.args.iter().any(|a| a == "ls-tree") {
            return Self::list_as_on_disk(command);
        }
        Reply::ok("")
    }

    /// A commit that is exactly what's on disk, as a clean clone at the approved commit would
    /// be. Tests of a clone that differs use real git.
    fn list_as_on_disk(command: &Command) -> Reply {
        use std::os::unix::ffi::OsStrExt;
        let Some(cwd) = command.cwd.as_deref() else {
            return Reply::fails(128, "fatal: not a git repository");
        };
        fn walk(root: &Path, at: &Path, out: &mut Vec<u8>) {
            let Ok(entries) = std::fs::read_dir(at) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                if relative == Path::new(".git") || entry.file_name() == ".DS_Store" {
                    continue;
                }
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let (mode, bytes) = if meta.is_dir() {
                    walk(root, &path, out);
                    continue;
                } else if meta.file_type().is_symlink() {
                    let target = std::fs::read_link(&path).unwrap_or_default();
                    ("120000", target.as_os_str().as_bytes().to_vec())
                } else {
                    ("100644", std::fs::read(&path).unwrap_or_default())
                };
                let mut hasher = sha1_smol::Sha1::new();
                hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
                hasher.update(&bytes);
                out.extend_from_slice(format!("{mode} blob {}\t", hasher.digest()).as_bytes());
                out.extend_from_slice(relative.as_os_str().as_bytes());
                out.push(0);
            }
        }
        let mut out = Vec::new();
        walk(cwd, cwd, &mut out);
        Reply::ok(String::from_utf8_lossy(&out).into_owned())
    }
}

impl CommandRunner for FakeGitHub {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(command.clone());
        let scripted = state
            .programs
            .iter()
            .find(|(suffix, _)| command.program.ends_with(suffix))
            .map(|(_, reply)| reply.clone());
        let reply = if let Some(reply) = scripted {
            reply
        } else if command.program.ends_with("gh") {
            Self::answer_gh(&mut state, command)
        } else if command.program.ends_with("curl") {
            Self::answer_curl(&state, command)
        } else if command.program.ends_with("git") {
            Self::answer_git(&mut state, command)
        } else {
            Reply::ok("")
        };
        drop(state);
        match reply {
            Reply::SpawnFails(message) => Err(SpawnError {
                program: command.program.clone(),
                message,
            }),
            Reply::Stall => Ok(CommandOutput {
                end: CommandEnd::Stalled,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Reply::Output {
                code,
                stdout,
                stderr,
            } => {
                stdout
                    .lines()
                    .for_each(|line| on_line(OutputStream::Stdout, line));
                stderr
                    .lines()
                    .for_each(|line| on_line(OutputStream::Stderr, line));
                Ok(CommandOutput {
                    end: CommandEnd::Exited(code),
                    stdout: stdout.into_bytes(),
                    stderr: stderr.into_bytes(),
                })
            }
        }
    }
}

/// A test double for the Machine Record's approvals. The wiring ticket binds the real one.
pub struct FileApprovals {
    path: PathBuf,
}

impl FileApprovals {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ApprovalStore for FileApprovals {
    fn read(&self) -> Result<Approvals, ApprovalStoreError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let value =
                    soulcaster_core::config::strict_parse(&bytes).map_err(ApprovalStoreError)?;
                serde_json::from_value(value).map_err(|e| ApprovalStoreError(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Approvals::default()),
            Err(e) => Err(ApprovalStoreError(e.to_string())),
        }
    }

    fn write(&self, approvals: &Approvals) -> Result<(), ApprovalStoreError> {
        let text = serde_json::to_string_pretty(approvals)
            .map_err(|e| ApprovalStoreError(e.to_string()))?;
        std::fs::write(&self.path, text).map_err(|e| ApprovalStoreError(e.to_string()))
    }
}
