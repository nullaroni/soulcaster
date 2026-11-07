//! The plain CLI. Only what the core can honestly do today: read a Config and say what a Run from
//! it would do. It changes nothing on the Machine. Exit code 2 means refused.

use std::process::ExitCode;

use soulcaster_core::catalog::Catalog;
use soulcaster_core::config::Config;
use soulcaster_core::lifecycle::Stage;
use soulcaster_core::plan::{NeedKind, Plan, Why};

const USAGE: &str =
    "usage: soulcaster validate <config.json>\n       soulcaster plan <config.json>";

const REFUSED: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["validate", path] => read(path).map(|config| validate(&config)),
        ["plan", path] => read(path).map(|config| print_plan(&config)),
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

fn read(path: &str) -> Result<Config, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Config::parse(&bytes).map_err(|e| format!("{path}: {e}"))
}

fn validate(config: &Config) {
    let plan = Plan::new(config, Catalog::shipped());
    println!("{:?} is a valid Config.", config.name);
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
