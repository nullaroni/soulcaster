//! The plain CLI. It can read a Config and say what a Run from it would do, run it on this Mac,
//! Import one for review, and Save one to the person's own GitHub repo. Exit code 2 means
//! refused, and 1 means a save is waiting on this Mac. A Run has its own codes: see
//! [`run_command`].

mod repo_commands;
mod run_command;

use std::path::PathBuf;
use std::process::ExitCode;

use macflayer_core::catalog::Catalog;
use macflayer_core::config::Config;
use macflayer_core::lifecycle::Stage;
use macflayer_core::plan::{NeedKind, Plan, Why};
use macflayer_repo::process::runner;
use macflayer_repo::tools::Tools;

use crate::repo_commands::{Done, ImportArgs, SaveArgs};

const USAGE: &str = "usage: macflayer validate <config.json>
       macflayer plan <config.json>
       macflayer run <config.json> [--json]
       macflayer import <owner/repo or URL> [--config <path>] [--keep]
       macflayer save <config.json> [--repo <name>]";

const REFUSED: u8 = 2;
const WAITING: u8 = 1;

fn main() -> ExitCode {
    if let Some(home) = std::env::var_os("HOME") {
        macflayer_engine::adopt_old_support(std::path::Path::new(&home));
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["validate", path] => read(path).map(|config| validate(&config)),
        ["plan", path] => read(path).map(|config| print_plan(&config)),
        ["run", rest @ ..] => return run(rest),
        ["import", rest @ ..] => return import(rest),
        ["save", rest @ ..] => return save(rest),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(REFUSED)
        }
    }
}

/// The real home and the real programs. Nothing above this line has been refused yet, and every
/// command that reaches here goes through `Tools`, with its environment set in full.
fn real_tools() -> Option<Tools> {
    std::env::var_os("HOME").map(|home| Tools::for_mac(PathBuf::from(home)))
}

fn finish(done: Done) -> ExitCode {
    match done {
        Done::Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Done::Refused(text) => {
            eprintln!("{text}");
            ExitCode::from(REFUSED)
        }
        Done::Waiting(text) => {
            eprintln!("{text}");
            ExitCode::from(WAITING)
        }
    }
}

/// A real Run, on this Mac. Everything is the real thing here, which is why the tests go through
/// [`run_command::run_plan`] with fakes instead.
fn run(rest: &[&str]) -> ExitCode {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use macflayer_core::lint::DEFAULT_CLONE_LOCATION;
    use macflayer_engine::{
        Clock, KeepAwake, MachineRecord, ProcessRunner, SystemClock, gate_beside,
    };
    use macflayer_repo::in_run::RunDotfiles;

    let (path, json) = match rest {
        [path] => (*path, false),
        [path, "--json"] | ["--json", path] => (*path, true),
        _ => return refuse_usage(),
    };
    let config = match read(path) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(REFUSED);
        }
    };
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return refuse_usage();
    };
    let Ok(executable) = std::env::current_exe() else {
        return refuse_usage();
    };
    let mut plan = Plan::new(&config, Catalog::shipped());
    let gate = gate_beside(&executable);
    let engine_runner = ProcessRunner::new();
    let repo_runner = runner();
    let tools = Tools::for_mac(home.clone());
    let clone_location = MachineRecord::load(&home)
        .ok()
        .and_then(|record| record.clone_path)
        .unwrap_or_else(|| DEFAULT_CLONE_LOCATION.to_string());
    let stamp = SystemClock.now_ms().to_string();
    let dotfiles = RunDotfiles {
        runner: &repo_runner,
        tools: &tools,
        clone_location: &clone_location,
        stamp: &stamp,
    };
    let cancel = AtomicBool::new(false);
    let setup = run_command::Setup {
        runner: &engine_runner,
        admin: &gate,
        dotfiles: Some(&dotfiles),
        asks: &run_command::TerminalAsks,
        clock: &SystemClock,
        cancel: &cancel,
        home: &home,
        json,
    };
    let _awake = KeepAwake::start();
    let out = Arc::new(Mutex::new(std::io::stdout()));
    ExitCode::from(run_command::run_plan(&mut plan, &setup, out))
}

fn refuse_usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(REFUSED)
}

fn import(rest: &[&str]) -> ExitCode {
    let (paste, config, keep) = match rest {
        [paste] => (*paste, None, false),
        [paste, "--keep"] => (*paste, None, true),
        [paste, "--config", config] => (*paste, Some(*config), false),
        [paste, "--config", config, "--keep"] | [paste, "--keep", "--config", config] => {
            (*paste, Some(*config), true)
        }
        _ => return refuse_usage(),
    };
    let Some(tools) = real_tools() else {
        return refuse_usage();
    };
    finish(repo_commands::import(
        &runner(),
        &tools,
        Catalog::shipped(),
        &ImportArgs {
            paste,
            config,
            keep,
        },
    ))
}

fn save(rest: &[&str]) -> ExitCode {
    let (path, repo) = match rest {
        [path] => (*path, None),
        [path, "--repo", repo] => (*path, Some(*repo)),
        _ => return refuse_usage(),
    };
    let Some(tools) = real_tools() else {
        return refuse_usage();
    };
    finish(repo_commands::save_config(
        &runner(),
        &tools,
        Catalog::shipped(),
        &SaveArgs { path, repo },
    ))
}

fn read(path: &str) -> Result<Config, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Config::parse(&bytes).map_err(|e| format!("{path}: {e}"))
}

fn validate(config: &Config) {
    let plan = Plan::new(config, Catalog::shipped());
    println!("{:?} is a valid Machine Type.", config.name);
    print_notes(&plan);
}

fn print_notes(plan: &Plan) {
    for note in &plan.notes {
        println!("note: {note}");
    }
    for finding in &plan.findings {
        println!("lint: {}", finding.message);
    }
}

fn print_plan(config: &Config) {
    let plan = Plan::new(config, Catalog::shipped());
    // The name is the file author's text, so it stays quoted as data.
    println!("Plan for {:?}", plan.config_name);
    for (stage, heading) in [(Stage::Admin, "Admin Stage"), (Stage::User, "User Stage")] {
        println!("\n{heading}");
        for entry in plan.entries.iter().filter(|e| e.stage == stage) {
            let why = match &entry.why {
                Why::PulledIn { by } => format!("  (pulled in by {by})"),
                _ => String::new(),
            };
            println!("  {}{why}", entry.id);
            for need in &entry.needs {
                println!("    needs {}: {}", need_name(need.kind), need.text);
            }
        }
    }
    if !plan.taps.is_empty() {
        println!("\nThird-party taps, each needing approval");
        for (tap, items) in &plan.taps {
            println!("  {tap}: {}", items.join(", "));
        }
    }
    if let Some(dotfiles) = &plan.dotfiles {
        println!("\nDotfiles Repo, needing approval: {}", dotfiles.repo);
        for (target, source) in &dotfiles.links {
            println!("  {target} -> {source}");
        }
        if let Some(script) = &dotfiles.install_script {
            println!("  install script: {script}");
        }
    }
    if !plan.notes.is_empty() || !plan.findings.is_empty() {
        println!();
        print_notes(&plan);
    }
}

fn need_name(kind: NeedKind) -> &'static str {
    match kind {
        NeedKind::AdminPassword => "password",
        NeedKind::TapApproval => "tap approval",
        NeedKind::License => "license",
        NeedKind::AppleAccount => "Apple Account",
        NeedKind::Person => "you",
        NeedKind::MachineValue => "local value",
    }
}
