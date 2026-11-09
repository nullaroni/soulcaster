//! A person's Fleet, in their Config Repo: the Machine Types in `configs/`, and one Machine Entry
//! per Mac in `machines/`. See ADR 0002. Reading sends nothing, and each Mac writes only its own
//! entry.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soulcaster_core::approval::CommitSha;
use soulcaster_core::command::{Command, CommandRunner};
use soulcaster_core::config::{Config, MAX_BYTES};

use crate::address::RepoId;
use crate::copies::{CopyError, SUPPORT, slug};
use crate::github::{DirEntry, FetchError, GitHub, SignIn};
use crate::safe_fs::SafeRoot;
use crate::save::{Outgoing, SaveOutcome, WaitReason, send, wait_for_fetch};
use crate::text::escaped;
use crate::tools::Tools;

pub const ENTRY_SCHEMA_VERSION: u32 = 1;

const CONFIGS: &str = "configs";
const MACHINES: &str = "machines";
const ID_FILE: &str = "machine-id";
/// An entry is a handful of short fields, so anything bigger isn't one.
const MAX_ENTRY_BYTES: usize = 4 * 1024;
/// How many files in one folder get read. One repo can't make this many reads into a long wait.
const MAX_FILES: usize = 64;
const MAX_NAME_CHARS: usize = 64;
const SCUTIL: &str = "/usr/sbin/scutil";
/// What a Mac calls itself when it can't find its computer name.
pub const FALLBACK_NAME: &str = "This Mac";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MachineEntry {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub machine_type: String,
    pub last_seen: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub differences: Option<u32>,
}

impl MachineEntry {
    pub fn new(id: String, name: String, machine_type: String, last_seen: u64) -> Self {
        Self {
            schema_version: ENTRY_SCHEMA_VERSION,
            id,
            name,
            machine_type,
            last_seen,
            differences: None,
        }
    }

    fn path(&self) -> String {
        format!("{MACHINES}/{}.json", self.id)
    }

    fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("plain fields");
        text.push('\n');
        text
    }
}

/// 32 lowercase hex digits, which is also safe as a file name.
pub fn is_machine_id(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn new_machine_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut bytes)
        .ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// This Mac's ID, made the first time it's asked for. It's kept apart from the Machine Record,
/// which is never sent anywhere.
pub fn machine_id(home: &Path) -> Result<String, CopyError> {
    let root = SafeRoot::open(home)?;
    let mut parts: Vec<&str> = SUPPORT.to_vec();
    parts.push(ID_FILE);
    if let Some(kept) = root.read_file(&parts, 128)? {
        let kept = String::from_utf8_lossy(&kept);
        let kept = kept.trim();
        if is_machine_id(kept) {
            return Ok(kept.to_string());
        }
    }
    let id = new_machine_id().ok_or(CopyError::Write)?;
    root.make_folder(SUPPORT)?;
    root.write_file(&parts, format!("{id}\n").as_bytes(), 0o600)?;
    Ok(id)
}

/// The name a person gave this Mac in System Settings.
pub fn computer_name(runner: &dyn CommandRunner) -> String {
    let command = Command::new(SCUTIL).args(["--get", "ComputerName"]);
    runner
        .run(&command, &mut |_, _| {})
        .ok()
        .filter(|output| output.is_success())
        .map(|output| showable_name(&output.stdout_text()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| FALLBACK_NAME.to_string())
}

fn showable_name(text: &str) -> String {
    escaped(text.trim()).chars().take(MAX_NAME_CHARS).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Published {
    Sent,
    Waiting(WaitReason),
}

/// Sends this Mac's entry. An entry is the latest state only, so a Mac that can't reach GitHub
/// keeps nothing to send later. It says what's true the next time something happens.
pub fn publish(
    runner: &dyn CommandRunner,
    tools: &Tools,
    repo_name: &str,
    entry: &MachineEntry,
) -> Published {
    if !is_machine_id(&entry.id) {
        return Published::Waiting(WaitReason::Denied);
    }
    let outcome = send(
        runner,
        tools,
        &Outgoing {
            repo_name,
            path: entry.path(),
            text: &entry.to_text(),
            base: None,
            own_entry: true,
            local: PathBuf::new(),
        },
    );
    match outcome {
        SaveOutcome::Sent { .. } | SaveOutcome::AlreadyThere { .. } => Published::Sent,
        SaveOutcome::Waiting { why, .. } => Published::Waiting(why),
        // Only for files others can change, which an entry isn't.
        SaveOutcome::ChangedElsewhere { .. } => Published::Waiting(WaitReason::Busy),
    }
}

/// A Machine Type as the repo holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeEntry {
    pub path: String,
    pub config: Config,
    /// The file as GitHub has it, which is what a save goes on top of.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fleet {
    pub repo: RepoId,
    pub public: bool,
    pub types: Vec<TypeEntry>,
    pub machines: Vec<MachineEntry>,
    /// Files that were there and couldn't be used.
    pub unreadable: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FleetRead {
    Read(Box<Fleet>),
    /// The person has no Config Repo yet, or nothing in it.
    NothingYet,
    Waiting(WaitReason),
}

/// Reads the whole Fleet at one commit. It sends nothing.
pub fn read(runner: &dyn CommandRunner, tools: &Tools, repo_name: &str) -> FleetRead {
    if tools.gh.is_none() || tools.git.is_none() {
        return FleetRead::Waiting(WaitReason::SignIn);
    }
    let github = GitHub::new(runner, tools);
    let login = match github.sign_in() {
        SignIn::SignedIn(identity) => identity.login.clone(),
        SignIn::SignedOut => return FleetRead::Waiting(WaitReason::SignIn),
        SignIn::Unreachable => return FleetRead::Waiting(WaitReason::Offline),
    };
    let Ok(id) = RepoId::new(&login, repo_name) else {
        return FleetRead::Waiting(WaitReason::Denied);
    };
    let facts = match github.repo_facts(&id) {
        Ok(facts) => facts,
        Err(FetchError::NotVisible { .. }) => return FleetRead::NothingYet,
        Err(error) => return FleetRead::Waiting(wait_for_fetch(&error)),
    };
    let head = match github.commit(&facts.repo, &facts.default_branch) {
        Ok(head) => head,
        Err(FetchError::Empty) => return FleetRead::NothingYet,
        Err(error) => return FleetRead::Waiting(wait_for_fetch(&error)),
    };
    let reader = Reader {
        github: &github,
        repo: &facts.repo,
        head: &head,
    };
    let mut unreadable = 0;
    let types = match reader.types(&mut unreadable) {
        Ok(types) => types,
        Err(why) => return FleetRead::Waiting(why),
    };
    let machines = match reader.machines(&mut unreadable) {
        Ok(machines) => machines,
        Err(why) => return FleetRead::Waiting(why),
    };
    FleetRead::Read(Box::new(Fleet {
        repo: facts.repo.clone(),
        public: !facts.private,
        types,
        machines,
        unreadable,
    }))
}

struct Reader<'a> {
    github: &'a GitHub<'a>,
    repo: &'a RepoId,
    head: &'a CommitSha,
}

impl Reader<'_> {
    /// The JSON files in one folder. A folder that isn't there has nothing in it.
    fn files(&self, dir: &str, unreadable: &mut usize) -> Result<Vec<DirEntry>, WaitReason> {
        let listed = match self.github.list_dir(self.repo, self.head, dir) {
            Ok(listed) => listed,
            Err(FetchError::NotVisible { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(wait_for_fetch(&error)),
        };
        let mut files: Vec<DirEntry> = listed
            .into_iter()
            .filter(|entry| entry.kind == "file" && entry.name.ends_with(".json"))
            .collect();
        if files.len() > MAX_FILES {
            *unreadable += files.len() - MAX_FILES;
            files.truncate(MAX_FILES);
        }
        Ok(files)
    }

    /// One file's bytes, or `None` for one that can't be used. Being cut off stops the whole read.
    fn bytes(&self, path: &str, max: usize) -> Result<Option<Vec<u8>>, WaitReason> {
        match self.github.read_file(self.repo, self.head, path, max) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(FetchError::Malformed | FetchError::TooLarge | FetchError::NotVisible { .. }) => {
                Ok(None)
            }
            Err(error) => Err(wait_for_fetch(&error)),
        }
    }

    fn types(&self, unreadable: &mut usize) -> Result<Vec<TypeEntry>, WaitReason> {
        let mut types = Vec::new();
        for file in self.files(CONFIGS, unreadable)? {
            let path = format!("{CONFIGS}/{}", file.name);
            let parsed = self.bytes(&path, MAX_BYTES)?.and_then(|bytes| {
                let config = Config::parse(&bytes).ok()?;
                // A save lands on the file its name makes, so a mismatch would fork on saving.
                let right_file = format!("{}.json", slug(&config.name)) == file.name;
                right_file.then_some((config, String::from_utf8(bytes).ok()?))
            });
            match parsed {
                Some((config, text)) => types.push(TypeEntry { path, config, text }),
                None => *unreadable += 1,
            }
        }
        Ok(types)
    }

    fn machines(&self, unreadable: &mut usize) -> Result<Vec<MachineEntry>, WaitReason> {
        let mut machines = Vec::new();
        for file in self.files(MACHINES, unreadable)? {
            let path = format!("{MACHINES}/{}", file.name);
            let entry = self
                .bytes(&path, MAX_ENTRY_BYTES)?
                .and_then(|bytes| parse_entry(&bytes, &file.name));
            match entry {
                Some(entry) => machines.push(entry),
                None => *unreadable += 1,
            }
        }
        machines.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(machines)
    }
}

/// An entry is only believed if it says it's the Mac whose file it's in.
fn parse_entry(bytes: &[u8], file_name: &str) -> Option<MachineEntry> {
    let mut entry: MachineEntry = serde_json::from_slice(bytes).ok()?;
    let right_file = file_name.strip_suffix(".json") == Some(entry.id.as_str());
    if entry.schema_version != ENTRY_SCHEMA_VERSION || !is_machine_id(&entry.id) || !right_file {
        return None;
    }
    entry.name = showable_name(&entry.name);
    entry.machine_type = showable_name(&entry.machine_type);
    Some(entry)
}
