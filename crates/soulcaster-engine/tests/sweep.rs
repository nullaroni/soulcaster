//! Seeded sweep of the shipped Catalog. A tiny PRNG picks items, settings, failures, a crash and
//! a person's answers. The runner is a fake of this Machine (test, brew, defaults, git), so
//! nothing here installs or writes a setting on the Mac the suite is running on.
//!
//! Each scenario has to ask at most once, run no third-party code while the password is held,
//! run no admin step after the discard, finish with the password gone, act on nothing it could
//! not check, block only for a reason, and settle once the injected failures stop.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::{Catalog, Lifecycle};
use soulcaster_core::command::{
    Command, CommandEnd, CommandOutput, CommandRunner, OutputStream, SpawnError,
};
use soulcaster_core::config::Config;
use soulcaster_core::event::{ConfigRef, Event, EventKind, EventSink, RunScope, StepPhase};
use soulcaster_core::lifecycle::ItemState;
use soulcaster_core::plan::Plan;
use soulcaster_core::recipe::{
    BuiltinProbe, BuiltinStore, Probe, Recipe, SettingRecipe, Step, Store, ValueType,
};
use soulcaster_engine::{Clock, EngineError, InProgress, MachineRecord, RunInput, perform};

const SCENARIOS: u64 = 1_000;
const SECRET: &str = "sweep-secret";

/// Hands the command back unwrapped, so a test's runner sees `installer` rather than `sudo`.
/// `tests/admin.rs` covers the real gate.
struct OpenGate;

struct OpenWindow;

impl soulcaster_engine::AdminWindow for OpenWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }
}

impl soulcaster_engine::AdminGate for OpenGate {
    fn validate(
        &self,
        _runner: &dyn CommandRunner,
        _home: &Path,
        _password: &Password,
    ) -> soulcaster_engine::Validation {
        soulcaster_engine::Validation::Accepted
    }

    fn open(
        &self,
        _password: &Password,
        _budget: soulcaster_engine::Budget,
    ) -> Result<Box<dyn soulcaster_engine::AdminWindow>, String> {
        Ok(Box::new(OpenWindow))
    }

    fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) -> bool {
        true
    }
}

struct Rng(u64);

impl Rng {
    /// SplitMix64. A bare multiply-and-add has low bits with a short period, and `below` and
    /// `one_in` read exactly those, so seeds in a row would repeat the same few scenarios.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() as usize) % n
        }
    }

    fn one_in(&mut self, n: u64) -> bool {
        n > 0 && self.next().is_multiple_of(n)
    }
}

struct Facts {
    sha: BTreeMap<String, String>,
    team: BTreeMap<String, String>,
    /// Check paths a brew install of this leaf should make true.
    brew_paths: BTreeMap<String, Vec<String>>,
    /// Package receipts a brew leaf or a pkg filename installs.
    receipts_by_brew: BTreeMap<String, Vec<String>>,
    receipts_by_file: BTreeMap<String, Vec<String>>,
    sdk_paths: Vec<String>,
    items: Vec<String>,
    settings: Vec<(String, serde_json::Value)>,
    may_unknown: BTreeSet<String>,
}

struct World {
    admin: bool,
    clt: bool,
    brew: bool,
    xcode_ready: bool,
    receipts: BTreeSet<String>,
    formulae: BTreeSet<String>,
    casks: BTreeSet<String>,
    files: BTreeSet<String>,
    prefixes: Vec<String>,
    git: BTreeMap<String, String>,
    defaults: BTreeMap<String, String>,
    dock: Vec<String>,
    library_hidden: bool,
    fail_left: u32,
    fail_kind: FailKind,
    saw_git_without_clt: bool,
}

#[derive(Clone, Copy)]
enum FailKind {
    Network,
    Tool,
    Stall,
}

enum Mark {
    Event(EventKind),
    Command(Command),
}

struct SweepClock {
    now: AtomicU64,
}

impl Clock for SweepClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }

    fn wait(&self, duration: Duration) {
        self.now
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }
}

struct SweepAsks {
    asked: AtomicUsize,
}

impl Asks for SweepAsks {
    fn password(&self, _request: &PasswordRequest) -> Option<Password> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Some(Password::new(SECRET.to_string()))
    }

    fn show_github_code(&self, _code: &str, _url: &str) {}

    fn github_status(&self, _status: SignInStatus) {}
}

struct LogSink {
    log: Arc<Mutex<Vec<Mark>>>,
}

impl EventSink for LogSink {
    fn send(&mut self, event: Event) {
        self.log.lock().unwrap().push(Mark::Event(event.kind));
    }
}

struct Sim {
    world: Mutex<World>,
    log: Arc<Mutex<Vec<Mark>>>,
    facts: Arc<Facts>,
    home: PathBuf,
    cancel: AtomicBool,
    commands: AtomicUsize,
    cancel_at: AtomicUsize,
}

impl CommandRunner for Sim {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError> {
        self.log
            .lock()
            .unwrap()
            .push(Mark::Command(command.clone()));
        let n = self.commands.fetch_add(1, Ordering::Relaxed);
        if n == self.cancel_at.load(Ordering::Relaxed) {
            self.cancel.store(true, Ordering::Relaxed);
        }
        let mut world = self.world.lock().unwrap();
        let reply = answer(&mut world, &self.facts, &self.home, command);
        drop(world);
        finish(reply, on_line)
    }
}

enum Answer {
    Out {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Stall,
    Missing(String),
}

fn finish(
    answer: Answer,
    on_line: &mut dyn FnMut(OutputStream, &str),
) -> Result<CommandOutput, SpawnError> {
    match answer {
        Answer::Missing(message) => Err(SpawnError {
            program: PathBuf::from("sweep"),
            message,
        }),
        Answer::Stall => Ok(CommandOutput {
            end: CommandEnd::Stalled,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }),
        Answer::Out {
            code,
            stdout,
            stderr,
        } => {
            for line in stdout.lines() {
                on_line(OutputStream::Stdout, line);
            }
            for line in stderr.lines() {
                on_line(OutputStream::Stderr, line);
            }
            Ok(CommandOutput {
                end: CommandEnd::Exited(code),
                stdout: stdout.into_bytes(),
                stderr: stderr.into_bytes(),
            })
        }
    }
}

fn ok(stdout: impl Into<String>) -> Answer {
    Answer::Out {
        code: 0,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

fn fail(stderr: impl Into<String>) -> Answer {
    Answer::Out {
        code: 1,
        stdout: String::new(),
        stderr: stderr.into(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string()
}

fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn is_mutation(command: &Command) -> bool {
    let name = file_name(&command.program);
    let args = &command.args;
    match name.as_str() {
        "brew" => args.iter().any(|arg| arg == "install"),
        "softwareupdate" | "curl" | "installer" | "chflags" => true,
        "defaults" => args.first().map(String::as_str) == Some("write"),
        "git" => args.iter().any(|arg| arg == "config") && !args.iter().any(|arg| arg == "--get"),
        "dockutil" => args.iter().any(|arg| arg == "--add" || arg == "--remove"),
        "xcodebuild" => args
            .iter()
            .any(|arg| arg == "-runFirstLaunch" || arg == "-license"),
        "sdkmanager" => true,
        _ => false,
    }
}

fn is_brew(command: &Command) -> bool {
    file_name(&command.program) == "brew"
}

fn env_has(command: &Command, key: &str) -> bool {
    command.env.iter().any(|(name, _)| name == key)
}

fn third_party(command: &Command) -> bool {
    let program = command.program.to_string_lossy();
    if program.contains("/tools/") || program.contains(".local/bin") {
        return true;
    }
    is_brew(command)
        && command
            .args
            .iter()
            .any(|arg| arg.split('/').count() >= 3 && !arg.contains("://"))
}

fn admin_command(command: &Command) -> bool {
    let name = file_name(&command.program);
    match name.as_str() {
        "softwareupdate" | "installer" => true,
        "xcodebuild" => command
            .args
            .iter()
            .any(|arg| arg == "-runFirstLaunch" || arg == "-license"),
        "brew" => !env_has(command, "HOMEBREW_NO_SUDO"),
        _ => false,
    }
}

fn answer(world: &mut World, facts: &Facts, home: &Path, command: &Command) -> Answer {
    let name = file_name(&command.program);
    if name == "git" && !world.clt {
        world.saw_git_without_clt = true;
        return fail("git is not available until Command Line Tools are installed");
    }
    if world.fail_left > 0 && is_mutation(command) {
        world.fail_left -= 1;
        return match world.fail_kind {
            FailKind::Network => fail("curl: (22) The requested URL returned error: 503"),
            FailKind::Tool => fail("Error: the tool refused"),
            FailKind::Stall => Answer::Stall,
        };
    }
    match name.as_str() {
        "id" => {
            if world.admin {
                ok("staff admin\n")
            } else {
                ok("staff\n")
            }
        }
        "sw_vers" => ok("15.6.1\n"),
        "plutil" => fail("Could not extract value, error: No value at that key path"),
        "test" => answer_test(world, &command.args),
        "brew" => answer_brew(world, facts, home, &command.args),
        "softwareupdate" => {
            if command.args.first().map(String::as_str) == Some("-l") {
                return ok("* Label: Command Line Tools for Xcode-16.4\n");
            }
            world.clt = true;
            ok("")
        }
        // The bootstrap copy and Homebrew's both answer. Sign-in is covered by name elsewhere.
        "gh" => {
            if command.args.iter().any(|arg| arg == "user") {
                ok("someone\n")
            } else {
                ok("")
            }
        }
        "touch" | "xcode-select" => ok(""),
        "curl" => ok(""),
        "shasum" => answer_shasum(facts, &command.args),
        "codesign" => ok(""),
        "pkgutil" => answer_pkgutil(world, facts, &command.args),
        "installer" => {
            if command.args.iter().any(|arg| arg.contains("Homebrew.pkg")) {
                world.brew = true;
            }
            if let Some(file) = command.args.iter().find_map(|arg| {
                Path::new(arg)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            }) && let Some(ids) = facts.receipts_by_file.get(&file)
            {
                world.receipts.extend(ids.iter().cloned());
            }
            ok("")
        }
        "defaults" => answer_defaults(world, &command.args),
        "git" => answer_git(world, &command.args),
        "dockutil" => answer_dock(world, &command.args),
        "xcodebuild" => answer_xcodebuild(world, &command.args),
        "chflags" => {
            world.library_hidden = command.args.first().map(String::as_str) == Some("hidden");
            ok("")
        }
        "ls" => {
            if world.library_hidden {
                ok("drwx------  3 staff hidden 96 Library\n")
            } else {
                ok("drwx------  3 staff 96 Library\n")
            }
        }
        "mkdir" => {
            if let Some(path) = command.args.last() {
                world.files.insert(path.clone());
            }
            ok("")
        }
        "mv" => {
            if let Some(dest) = command.args.last() {
                world.prefixes.push(dest.clone());
            }
            ok("")
        }
        "ln" => {
            if let Some(path) = command.args.last() {
                world.files.insert(path.clone());
            }
            ok("")
        }
        "sdkmanager" => {
            for path in &facts.sdk_paths {
                world.files.insert(expand(home, path));
            }
            ok("")
        }
        "cat" | "killall" | "ditto" | "tar" => ok(""),
        _ => {
            let program = command.program.to_string_lossy();
            if program.starts_with("/usr/")
                || program.starts_with("/bin/")
                || program.starts_with("/sbin/")
                || world.has_path(&program)
            {
                ok("99.0.0\n")
            } else {
                Answer::Missing(format!("{program} is not on this Machine"))
            }
        }
    }
}

fn answer_test(world: &World, args: &[String]) -> Answer {
    let (negate, path) = if args.first().map(String::as_str) == Some("!") {
        (true, args.get(2).map(String::as_str).unwrap_or(""))
    } else {
        (false, args.get(1).map(String::as_str).unwrap_or(""))
    };
    let present = world.has_path(path);
    if present == negate { fail("") } else { ok("") }
}

fn answer_brew(world: &mut World, facts: &Facts, home: &Path, args: &[String]) -> Answer {
    let Some(name) = args.last() else {
        return ok("");
    };
    if !args.iter().any(|arg| arg == "install") {
        return ok("");
    }
    let leaf = leaf(name).to_string();
    if args.iter().any(|arg| arg == "--cask") {
        world.casks.insert(leaf.clone());
    } else {
        world.formulae.insert(leaf.clone());
        world.files.insert(format!("/opt/homebrew/bin/{leaf}"));
    }
    if let Some(ids) = facts.receipts_by_brew.get(&leaf) {
        world.receipts.extend(ids.iter().cloned());
    }
    if let Some(paths) = facts.brew_paths.get(&leaf) {
        for path in paths {
            world.files.insert(expand(home, path));
        }
    }
    ok("")
}

fn answer_shasum(facts: &Facts, args: &[String]) -> Answer {
    let file = args
        .last()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match facts.sha.get(file) {
        Some(sha) => ok(format!("{sha}\n")),
        None => fail(format!("no known checksum for {file}")),
    }
}

fn answer_pkgutil(world: &World, facts: &Facts, args: &[String]) -> Answer {
    if args.first().map(String::as_str) == Some("--pkg-info") {
        let id = args.get(1).map(String::as_str).unwrap_or("");
        return if world.receipts.contains(id) {
            ok("install-time: 1\n")
        } else {
            fail("no receipt")
        };
    }
    let file = args
        .last()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match facts.team.get(file) {
        Some(team) => ok(format!(
            "Status: signed by a developer certificate issued by Apple\n    Certificate Chain:\n     1. Developer ID Installer: Example ({team})\n"
        )),
        None => fail("no signature"),
    }
}

fn answer_defaults(world: &mut World, args: &[String]) -> Answer {
    let Some(op) = args.first().map(String::as_str) else {
        return fail("usage");
    };
    let Some((id, value_at)) = defaults_identity(args) else {
        return fail("usage");
    };
    if op == "read" {
        return match world.defaults.get(&id) {
            Some(text) => ok(format!("{text}\n")),
            None => fail("does not exist"),
        };
    }
    if op == "write" {
        world
            .defaults
            .insert(id, defaults_readback(&args[value_at..]));
        return ok("");
    }
    ok("")
}

fn defaults_identity(args: &[String]) -> Option<(String, usize)> {
    let mut index = 1;
    let mut host = String::new();
    if args.get(index).map(String::as_str) == Some("-currentHost") {
        host = "current".into();
        index += 1;
    }
    let domain = args.get(index)?.clone();
    let key = args.get(index + 1)?.clone();
    Some((format!("{host}\0{domain}\0{key}"), index + 2))
}

fn defaults_readback(args: &[String]) -> String {
    let flag = args.first().map(String::as_str).unwrap_or("");
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    match flag {
        "-bool" => {
            if rest.first().map(String::as_str) == Some("true") {
                "1".into()
            } else {
                "0".into()
            }
        }
        "-int" | "-float" | "-string" => rest.first().cloned().unwrap_or_default(),
        "-array" => rest.join("\n"),
        "-dict" => {
            let mut lines = Vec::new();
            let mut index = 0;
            while index < rest.len() {
                let key = &rest[index];
                let on = rest.get(index + 2).map(String::as_str) == Some("true");
                lines.push(format!("{key} = {};", if on { "1" } else { "0" }));
                index += 3;
            }
            lines.join("\n")
        }
        _ => String::new(),
    }
}

fn answer_git(world: &mut World, args: &[String]) -> Answer {
    if args.iter().any(|arg| arg == "--get") {
        let key = args.last().map(String::as_str).unwrap_or("");
        return match world.git.get(key) {
            Some(text) => ok(format!("{text}\n")),
            None => fail("not set"),
        };
    }
    if args.len() >= 2 {
        let value = args[args.len() - 1].clone();
        let key = args[args.len() - 2].clone();
        world.git.insert(key, value);
    }
    ok("")
}

fn answer_dock(world: &mut World, args: &[String]) -> Answer {
    if !world.formulae.contains("dockutil") {
        return fail("dockutil: command not found");
    }
    if args.iter().any(|arg| arg == "--list") {
        let text = world
            .dock
            .iter()
            .map(|path| {
                let name = Path::new(path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(path);
                format!("{name}\t{path}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        return ok(format!("{text}\n"));
    }
    if args.iter().any(|arg| arg == "--remove") {
        world.dock.clear();
        return ok("");
    }
    if let Some(index) = args.iter().position(|arg| arg == "--add")
        && let Some(path) = args.get(index + 1)
    {
        world.dock.push(path.clone());
    }
    ok("")
}

fn answer_xcodebuild(world: &mut World, args: &[String]) -> Answer {
    if args.iter().any(|arg| arg == "-checkFirstLaunchStatus") {
        return if world.xcode_ready {
            ok("")
        } else {
            fail("not ready")
        };
    }
    if args.iter().any(|arg| arg == "-runFirstLaunch") {
        world.xcode_ready = true;
    }
    ok("")
}

impl World {
    fn prepared() -> Self {
        let mut formulae = BTreeSet::new();
        formulae.insert("gh".to_string());
        let mut files = BTreeSet::new();
        files.insert("/opt/homebrew/bin/gh".to_string());
        Self {
            admin: true,
            clt: true,
            brew: true,
            xcode_ready: false,
            receipts: BTreeSet::new(),
            formulae,
            casks: BTreeSet::new(),
            files,
            prefixes: Vec::new(),
            git: BTreeMap::new(),
            defaults: BTreeMap::new(),
            dock: Vec::new(),
            library_hidden: false,
            fail_left: 0,
            fail_kind: FailKind::Tool,
            saw_git_without_clt: false,
        }
    }

    fn bare() -> Self {
        Self {
            clt: false,
            brew: false,
            formulae: BTreeSet::new(),
            files: BTreeSet::new(),
            ..Self::prepared()
        }
    }

    fn has_path(&self, path: &str) -> bool {
        if path.contains("CommandLineTools/usr/bin/git") {
            return self.clt;
        }
        if path == "/opt/homebrew/bin/brew" {
            return self.brew;
        }
        if let Some(name) = path.split("/Cellar/").nth(1) {
            return self.formulae.contains(leaf(name));
        }
        if let Some(name) = path.split("/Caskroom/").nth(1) {
            return self.casks.contains(leaf(name));
        }
        if self.files.contains(path) {
            return true;
        }
        self.prefixes.iter().any(|prefix| {
            path == prefix || path.starts_with(prefix) && path[prefix.len()..].starts_with('/')
        })
    }
}

fn expand(home: &Path, path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest).to_string_lossy().into_owned()
    } else if path == "~" {
        home.to_string_lossy().into_owned()
    } else {
        path.replace('~', &home.to_string_lossy())
    }
}

fn collect(catalog: &Catalog) -> Facts {
    let mut sha = BTreeMap::new();
    let mut team = BTreeMap::new();
    let mut brew_paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut receipts_by_brew: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut receipts_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut sdk_paths = Vec::new();
    let mut items = Vec::new();
    let mut settings = Vec::new();
    let mut may_unknown = BTreeSet::new();
    for entry in catalog.entries() {
        if matches!(entry.lifecycle, Lifecycle::Retired(_)) {
            continue;
        }
        if may_be_unknown(&entry.recipe) {
            may_unknown.insert(entry.id().to_string());
        }
        match &entry.recipe {
            Recipe::Install(install) => {
                let mut paths = Vec::new();
                let mut receipts = Vec::new();
                for probe in &install.checks {
                    match probe {
                        Probe::AppBundle { path, .. } | Probe::File { path } => {
                            paths.push(path.clone());
                        }
                        Probe::PkgReceipt { pkg_id } => receipts.push(pkg_id.clone()),
                        _ => {}
                    }
                }
                for step in &install.steps {
                    match step {
                        Step::Download {
                            url,
                            sha256,
                            signed_by,
                            ..
                        }
                        | Step::PkgInstall {
                            url,
                            sha256,
                            signed_by,
                            ..
                        } => {
                            let file = url.rsplit('/').next().unwrap_or(url).to_string();
                            sha.insert(file.clone(), sha256.clone());
                            team.insert(file.clone(), signed_by.clone());
                            receipts_by_file
                                .entry(file)
                                .or_default()
                                .extend(receipts.iter().cloned());
                        }
                        Step::BrewFormula { name, .. } | Step::BrewCask { name, .. } => {
                            let name = leaf(name).to_string();
                            brew_paths
                                .entry(name.clone())
                                .or_default()
                                .extend(paths.iter().cloned());
                            receipts_by_brew
                                .entry(name)
                                .or_default()
                                .extend(receipts.iter().cloned());
                        }
                        Step::Sdkmanager { .. } => sdk_paths.extend(paths.iter().cloned()),
                        _ => {}
                    }
                }
                if entry.listing.is_some() {
                    items.push(entry.id().to_string());
                }
            }
            Recipe::Setting(setting) => {
                if let Some(value) = setting_value(setting) {
                    settings.push((setting.id.clone(), value));
                }
            }
            Recipe::MachineValues(_) => {}
        }
    }
    Facts {
        sha,
        team,
        brew_paths,
        receipts_by_brew,
        receipts_by_file,
        sdk_paths,
        items,
        settings,
        may_unknown,
    }
}

fn may_be_unknown(recipe: &Recipe) -> bool {
    match recipe {
        Recipe::Setting(setting) => setting.stores.iter().any(|store| {
            matches!(
                store,
                Store::Builtin {
                    name: BuiltinStore::SidebarFavorites
                }
            )
        }),
        Recipe::Install(install) => install.checks.iter().any(|probe| {
            matches!(
                probe,
                Probe::Unknowable { .. }
                    | Probe::Builtin {
                        name: BuiltinProbe::GithubSignedIn
                    }
            )
        }),
        Recipe::MachineValues(_) => true,
    }
}

fn setting_value(setting: &SettingRecipe) -> Option<serde_json::Value> {
    let value = match setting.value_type {
        ValueType::Boolean => serde_json::json!(true),
        ValueType::Integer => {
            let mut number = setting.minimum.unwrap_or(0);
            if let Some(max) = setting.maximum {
                number = number.min(max);
            }
            if let Some(min) = setting.minimum {
                number = number.max(min);
            }
            serde_json::json!(number)
        }
        ValueType::String => {
            if let Some(allowed) = &setting.values {
                serde_json::json!(allowed.first()?.clone())
            } else if setting.pattern.is_some() {
                return None;
            } else {
                serde_json::json!("sweep")
            }
        }
        ValueType::StringList => {
            if let Some(allowed) = &setting.values {
                serde_json::json!([allowed.first()?.clone()])
            } else if setting.pattern.is_some() {
                return None;
            } else if setting.stores.iter().any(|store| {
                matches!(
                    store,
                    Store::Builtin {
                        name: BuiltinStore::DockTiles
                    }
                )
            }) {
                serde_json::json!(["/Applications/Safari.app"])
            } else {
                serde_json::json!(["sweep"])
            }
        }
    };
    let parsed: soulcaster_core::config::SettingValue =
        serde_json::from_value(value.clone()).ok()?;
    setting.check_value(&parsed).ok()?;
    Some(value)
}

#[test]
fn a_seeded_sweep_of_the_shipped_catalog_holds_the_run_invariants() {
    let catalog = Catalog::shipped();
    let facts = Arc::new(collect(catalog));
    assert!(!facts.items.is_empty(), "the catalog has no install items");
    let root = std::env::temp_dir().join(format!("soulcaster-sweep-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    // Each scenario has its own HOME, so they can share the cores. A failure names its seed.
    let next = AtomicU64::new(0);
    let workers = std::thread::available_parallelism().map_or(1, |count| count.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= SCENARIOS {
                        break;
                    }
                    let seed = 0x5A17_0000_u64.wrapping_add(index);
                    scenario(seed, &facts, catalog, &root);
                }
            });
        }
    });
    let _ = fs::remove_dir_all(&root);
}

fn scenario(seed: u64, facts: &Arc<Facts>, catalog: &Catalog, root: &Path) {
    let mut rng = Rng(seed);
    let mut picked = Vec::new();
    for _ in 0..rng.below(4) {
        picked.push(facts.items[rng.below(facts.items.len())].clone());
    }
    picked.sort();
    picked.dedup();
    let mut settings = BTreeMap::new();
    for _ in 0..rng.below(3) {
        let (id, value) = &facts.settings[rng.below(facts.settings.len())];
        settings.insert(id.clone(), value.clone());
    }
    let config = Config::parse(
        &serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "name": "sweep",
            "items": picked,
            "settings": settings,
        }))
        .unwrap(),
    )
    .unwrap_or_else(|error| panic!("seed {seed} config: {error}"));
    let home = root.join(seed.to_string());
    fs::create_dir_all(&home).unwrap();
    let bare = rng.one_in(5);
    let admin = !rng.one_in(8);
    let mut world = if bare {
        World::bare()
    } else {
        World::prepared()
    };
    world.admin = admin;
    world.fail_left = rng.below(3) as u32;
    world.fail_kind = match rng.below(3) {
        0 => FailKind::Network,
        1 => FailKind::Tool,
        _ => FailKind::Stall,
    };
    let cancel_at = if rng.one_in(5) {
        rng.below(40) + 1
    } else {
        usize::MAX
    };
    let crash = rng.one_in(7);
    let log = Arc::new(Mutex::new(Vec::new()));
    let sim = Sim {
        world: Mutex::new(world),
        log: Arc::clone(&log),
        facts: Arc::clone(facts),
        home: home.clone(),
        cancel: AtomicBool::new(false),
        commands: AtomicUsize::new(0),
        cancel_at: AtomicUsize::new(cancel_at),
    };
    let clock = SweepClock {
        now: AtomicU64::new(1_700_000_000_000),
    };
    let asks = SweepAsks {
        asked: AtomicUsize::new(0),
    };
    let mut quiet = 0;
    let mut last_report = None;
    for pass in 0..6 {
        if pass > 0 {
            sim.world.lock().unwrap().fail_left = 0;
            sim.cancel_at.store(usize::MAX, Ordering::Relaxed);
            sim.cancel.store(false, Ordering::Relaxed);
            close_queue(&home);
            if pass == 1 && crash {
                leave_a_crash_marker(&home);
            }
        }
        log.lock().unwrap().clear();
        let asked_before = asks.asked.load(Ordering::Relaxed);
        let mut plan = Plan::new(&config, catalog);
        let mut sink = LogSink {
            log: Arc::clone(&log),
        };
        let report = perform(RunInput {
            admin: &OpenGate,
            plan: &mut plan,
            runner: &sim,
            home: &home,
            events: &mut sink,
            asks: &asks,
            cancel: &sim.cancel,
            clock: &clock,
            scope: RunScope::Full,
            config: ConfigRef {
                name: "sweep".into(),
                repo: None,
                path: None,
                commit: None,
            },
            catalog_version: catalog.version(),
            engine_version: "sweep",
        });
        let asked = asks.asked.load(Ordering::Relaxed) - asked_before;
        let report = match report {
            Ok(report) => report,
            // A standard account cannot install the base tools. The Run stops before it asks.
            Err(EngineError::AdminRequired { .. }) => {
                assert_eq!(asked, 0, "seed {seed} asked before refusing");
                assert!(
                    !sim.world.lock().unwrap().saw_git_without_clt,
                    "seed {seed} ran git before refusing"
                );
                return;
            }
            Err(error) => panic!("seed {seed} pass {pass}: {error}"),
        };
        let marks = log.lock().unwrap();
        check_run(seed, pass, asked, &report, &marks, &sim);
        let mutations = marks
            .iter()
            .filter(|mark| match mark {
                Mark::Command(command) => is_mutation(command),
                Mark::Event(_) => false,
            })
            .count();
        drop(marks);
        if pass > 0 && mutations == 0 {
            quiet += 1;
            if quiet >= 2 {
                last_report = Some(report);
                break;
            }
        } else if pass > 0 {
            quiet = 0;
        }
        last_report = Some(report);
    }
    if quiet < 2 {
        let marks = log.lock().unwrap();
        let mutations: Vec<String> = marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Command(command) if is_mutation(command) => Some(format!(
                    "{} {}",
                    file_name(&command.program),
                    command.args.join(" ")
                )),
                _ => None,
            })
            .collect();
        let states = last_report
            .as_ref()
            .map(|report| format!("{:?}", report.states))
            .unwrap_or_default();
        panic!(
            "seed {seed} did not settle once failures stopped\nstates: {states}\nlast mutations: {mutations:?}"
        );
    }
    let report = last_report.unwrap();
    for (id, state) in &report.states {
        match state {
            ItemState::Satisfied { .. }
            | ItemState::Skipped { .. }
            | ItemState::NeedsYou { .. } => {}
            ItemState::Blocked { by } => {
                let dep = report.states.get(by).unwrap_or_else(|| {
                    panic!("seed {seed} blocked {id} by {by}, which is not in the run")
                });
                assert!(
                    matches!(
                        dep,
                        ItemState::Failed { .. }
                            | ItemState::Blocked { .. }
                            | ItemState::NeedsYou { .. }
                            | ItemState::Skipped { .. }
                    ),
                    "seed {seed} blocked {id} by {by}, which is {dep:?}"
                );
            }
            ItemState::Unknown => assert!(
                facts.may_unknown.contains(id),
                "seed {seed} left {id} unknown"
            ),
            other => panic!("seed {seed} left {id} at {other:?}"),
        }
    }
    let record =
        fs::read_to_string(soulcaster_engine::machine_record_path(&home)).unwrap_or_default();
    assert!(
        !record.contains(SECRET),
        "seed {seed} wrote the password into the record"
    );
}

fn check_run(
    seed: u64,
    pass: usize,
    asked: usize,
    report: &soulcaster_engine::RunReport,
    marks: &[Mark],
    sim: &Sim,
) {
    assert!(asked <= 1, "seed {seed} pass {pass} asked {asked} times");
    assert!(
        !sim.world.lock().unwrap().saw_git_without_clt,
        "seed {seed} pass {pass} ran git before Command Line Tools existed"
    );
    let mut held = false;
    let acted_unknown = acted_while_unknown(report, marks);
    assert!(
        acted_unknown.is_empty(),
        "seed {seed} pass {pass} acted on unknown items: {acted_unknown:?}"
    );
    for mark in marks {
        match mark {
            Mark::Event(EventKind::AdminHeld { .. }) => held = true,
            Mark::Event(EventKind::AdminDiscarded { .. }) => held = false,
            Mark::Event(kind) => {
                let text = format!("{kind:?}");
                assert!(
                    !text.contains(SECRET),
                    "seed {seed} pass {pass} put the password in an event"
                );
            }
            Mark::Command(command) => {
                let rendered = format!("{command:?}");
                assert!(
                    !rendered.contains(SECRET),
                    "seed {seed} pass {pass} put the password on a command"
                );
                assert!(
                    !env_has(command, "SUDO_ASKPASS") || admin_command(command),
                    "seed {seed} pass {pass} gave the helper to a step that isn't admin: {command:?}"
                );
                if held {
                    assert!(
                        !third_party(command),
                        "seed {seed} pass {pass} ran third-party code while the password was held: {command:?}"
                    );
                    assert!(
                        // Fetching a cask is the one unprivileged brew while it's held, and runs
                        // no package's code.
                        !(is_brew(command)
                            && env_has(command, "HOMEBREW_NO_SUDO")
                            && !command.args.iter().any(|arg| arg == "fetch")),
                        "seed {seed} pass {pass} ran a formula while the password was held: {command:?}"
                    );
                }
            }
        }
    }
    let mut seen_hold = false;
    let mut open = false;
    for mark in marks {
        match mark {
            Mark::Event(EventKind::AdminHeld { .. }) => {
                seen_hold = true;
                open = true;
            }
            Mark::Event(EventKind::AdminDiscarded { .. }) => open = false,
            Mark::Command(command) if seen_hold && !open => assert!(
                !admin_command(command),
                "seed {seed} pass {pass} ran an admin command after the discard: {command:?}"
            ),
            _ => {}
        }
    }
    if seen_hold {
        assert!(
            !open,
            "seed {seed} pass {pass} still held the password at the end"
        );
    }
    for (id, state) in &report.states {
        if let ItemState::Blocked { by } = state {
            let Some(dep) = report.states.get(by) else {
                panic!("seed {seed} pass {pass} blocked {id} by a missing {by}");
            };
            assert!(
                matches!(
                    dep,
                    ItemState::Failed { .. }
                        | ItemState::Blocked { .. }
                        | ItemState::NeedsYou { .. }
                        | ItemState::Skipped { .. }
                        | ItemState::Pending
                ),
                "seed {seed} pass {pass} blocked {id} by {by}, which is {dep:?}"
            );
        }
        if matches!(state, ItemState::Unknown) {
            let started = marks.iter().any(|mark| {
                matches!(
                    mark,
                    Mark::Event(EventKind::ItemStep {
                        item,
                        phase: StepPhase::Started,
                        ..
                    }) if item == id
                ) || matches!(
                    mark,
                    Mark::Event(EventKind::ItemState { item, to: ItemState::Acting, .. }) if item == id
                )
            });
            assert!(
                !started,
                "seed {seed} pass {pass} acted on {id} while it was unknown"
            );
        }
    }
}

fn acted_while_unknown(report: &soulcaster_engine::RunReport, marks: &[Mark]) -> Vec<String> {
    report
        .states
        .iter()
        .filter(|(id, state)| {
            matches!(state, ItemState::Unknown)
                && marks.iter().any(|mark| {
                    matches!(
                        mark,
                        Mark::Event(EventKind::ItemStep {
                            item,
                            phase: StepPhase::Started,
                            ..
                        }) if item == *id
                    )
                })
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn close_queue(home: &Path) {
    let mut record =
        MachineRecord::load(home).unwrap_or_else(|error| panic!("record would not load: {error}"));
    let mut changed = false;
    for entry in &mut record.queue {
        if entry.finished_at.is_none() {
            entry.finished_at = Some(1);
            entry.finished_by = Some("person".into());
            changed = true;
        }
    }
    if changed {
        record.save(home).unwrap();
    }
}

fn leave_a_crash_marker(home: &Path) {
    let mut record = MachineRecord::load(home).unwrap();
    record.running = Some(InProgress {
        run_id: "crashed".into(),
        item: "command-line-tools".into(),
    });
    record.save(home).unwrap();
}
