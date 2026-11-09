//! The Fleet in the Config Repo, with GitHub played by a fake. Each Mac writes only its own entry,
//! and nothing is believed just because it's in the repo.

use std::os::unix::fs::PermissionsExt;

use soulcaster_core::command::{Command, Reply};
use soulcaster_core::config::{Config, SCHEMA_VERSION};
use soulcaster_repo::fake::{FakeGitHub, RepoShape};
use soulcaster_repo::fleet::{
    FALLBACK_NAME, FleetRead, MachineEntry, Published, computer_name, is_machine_id, machine_id,
    publish, read,
};
use soulcaster_repo::save::{CONFIG_REPO_NAME, WaitReason};
use soulcaster_repo::tools::Tools;

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const PUSHED: &str = "bbbbbbb000000000000000000000000000000000";
const MAC_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MAC_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Sandbox {
    _dir: tempfile::TempDir,
    tools: Tools,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let tools = Tools::fixed(std::fs::canonicalize(dir.path()).unwrap());
    Sandbox { _dir: dir, tools }
}

fn entry(id: &str, name: &str) -> MachineEntry {
    MachineEntry::new(id.to_string(), name.to_string(), "Dev".to_string(), 1_000)
}

fn entry_text(entry: &MachineEntry) -> String {
    serde_json::to_string(entry).unwrap()
}

fn github(shape: RepoShape) -> FakeGitHub {
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    gh.add_repo("example-you", CONFIG_REPO_NAME, shape);
    gh.on_git("rev-parse", Reply::ok(format!("{PUSHED}\n")));
    gh
}

fn add(gh: &FakeGitHub, path: &str, text: &str) {
    gh.add_file("example-you", CONFIG_REPO_NAME, SHA, path, text.as_bytes());
}

fn add_entry(gh: &FakeGitHub, entry: &MachineEntry) {
    add(
        gh,
        &format!("machines/{}.json", entry.id),
        &entry_text(entry),
    );
}

fn git_words(gh: &FakeGitHub) -> Vec<Command> {
    gh.calls()
        .into_iter()
        .filter(|c| c.program.ends_with("git"))
        .collect()
}

fn pushed(gh: &FakeGitHub) -> bool {
    git_words(gh)
        .iter()
        .any(|c| c.args.iter().any(|a| a == "push"))
}

fn fleet(gh: &FakeGitHub, sandbox: &Sandbox) -> soulcaster_repo::fleet::Fleet {
    match read(gh, &sandbox.tools, CONFIG_REPO_NAME) {
        FleetRead::Read(fleet) => *fleet,
        other => panic!("{other:?}"),
    }
}

fn config(name: &str) -> Config {
    Config {
        schema: None,
        schema_version: SCHEMA_VERSION,
        name: name.to_string(),
        items: vec!["ghostty".to_string()],
        packages: Default::default(),
        settings: Default::default(),
        dotfiles: None,
    }
}

#[test]
fn a_machine_id_is_made_once_private_and_stays() {
    let sandbox = sandbox();
    let first = machine_id(&sandbox.tools.home).unwrap();
    assert!(is_machine_id(&first), "{first}");
    assert_eq!(machine_id(&sandbox.tools.home).unwrap(), first);
    let file = sandbox
        .tools
        .home
        .join("Library/Application Support/Soulcaster/machine-id");
    let mode = std::fs::metadata(file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_damaged_machine_id_is_replaced() {
    let sandbox = sandbox();
    let support = sandbox
        .tools
        .home
        .join("Library/Application Support/Soulcaster");
    std::fs::create_dir_all(&support).unwrap();
    std::fs::write(support.join("machine-id"), "../../evil").unwrap();
    assert!(is_machine_id(&machine_id(&sandbox.tools.home).unwrap()));
}

#[test]
fn the_computer_name_is_cleaned_and_has_a_fallback() {
    let gh = FakeGitHub::new();
    gh.on_program("scutil", Reply::ok("The Mac mini\n"));
    assert_eq!(computer_name(&gh), "The Mac mini");

    let gh = FakeGitHub::new();
    gh.on_program("scutil", Reply::fails(1, "no"));
    assert_eq!(computer_name(&gh), FALLBACK_NAME);
}

#[test]
fn an_entry_goes_to_its_own_file_in_a_private_repo() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    let outcome = publish(
        &gh,
        &sandbox.tools,
        CONFIG_REPO_NAME,
        &entry(MAC_A, "MacBook"),
    );
    assert_eq!(outcome, Published::Sent);
    assert!(
        git_words(&gh).iter().any(|c| {
            c.args
                .iter()
                .any(|a| a == &format!("machines/{MAC_A}.json"))
        }),
        "the add names the entry's own file"
    );
    assert!(pushed(&gh));
}

#[test]
fn a_public_repo_gets_no_entry() {
    let gh = github(RepoShape::public("main", SHA));
    let sandbox = sandbox();
    let outcome = publish(
        &gh,
        &sandbox.tools,
        CONFIG_REPO_NAME,
        &entry(MAC_A, "MacBook"),
    );
    assert_eq!(outcome, Published::Waiting(WaitReason::Public));
    assert!(!pushed(&gh));
}

#[test]
fn being_offline_or_signed_out_waits_and_the_next_try_sends() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    let mac = entry(MAC_A, "MacBook");
    gh.sign_out();
    assert_eq!(
        publish(&gh, &sandbox.tools, CONFIG_REPO_NAME, &mac),
        Published::Waiting(WaitReason::SignIn)
    );
    assert!(!pushed(&gh));

    let gh = github(RepoShape::private("main", SHA));
    assert_eq!(
        publish(&gh, &sandbox.tools, CONFIG_REPO_NAME, &mac),
        Published::Sent,
        "it says what's true when it can"
    );
}

#[test]
fn an_id_that_is_not_one_is_never_sent() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    let outcome = publish(
        &gh,
        &sandbox.tools,
        CONFIG_REPO_NAME,
        &entry("../configs/dev", "x"),
    );
    assert_eq!(outcome, Published::Waiting(WaitReason::Denied));
    assert!(gh.calls().is_empty());
}

#[test]
fn reading_finds_each_type_and_mac_in_name_order() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    add(&gh, "configs/dev.json", &config("Dev").to_canonical_json());
    add(
        &gh,
        "configs/agents.json",
        &config("Agents").to_canonical_json(),
    );
    add_entry(&gh, &entry(MAC_B, "Mini"));
    add_entry(&gh, &entry(MAC_A, "MacBook"));

    let found = fleet(&gh, &sandbox);
    assert_eq!(found.types.len(), 2);
    let names: Vec<&str> = found.machines.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["MacBook", "Mini"]);
    assert_eq!(found.unreadable, 0);
    assert!(!found.public);
}

#[test]
fn reading_sends_nothing() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    add_entry(&gh, &entry(MAC_A, "MacBook"));
    fleet(&gh, &sandbox);
    assert!(git_words(&gh).is_empty());
}

#[test]
fn an_entry_claiming_to_be_another_mac_is_not_believed() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    let imposter = entry(MAC_B, "Not really");
    add(
        &gh,
        &format!("machines/{MAC_A}.json"),
        &entry_text(&imposter),
    );
    let found = fleet(&gh, &sandbox);
    assert!(found.machines.is_empty());
    assert_eq!(found.unreadable, 1);
}

#[test]
fn an_entry_with_extra_fields_or_a_newer_schema_is_not_believed() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    add(
        &gh,
        &format!("machines/{MAC_A}.json"),
        &format!(
            r#"{{"schemaVersion":1,"id":"{MAC_A}","name":"x","machineType":"Dev","lastSeen":1,"apps":["secret"]}}"#
        ),
    );
    add(
        &gh,
        &format!("machines/{MAC_B}.json"),
        &format!(
            r#"{{"schemaVersion":2,"id":"{MAC_B}","name":"x","machineType":"Dev","lastSeen":1}}"#
        ),
    );
    let found = fleet(&gh, &sandbox);
    assert!(found.machines.is_empty());
    assert_eq!(found.unreadable, 2);
}

#[test]
fn a_type_in_the_wrong_file_is_not_offered() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    add(
        &gh,
        "configs/other.json",
        &config("Dev").to_canonical_json(),
    );
    let found = fleet(&gh, &sandbox);
    assert!(found.types.is_empty());
    assert_eq!(found.unreadable, 1);
}

#[test]
fn a_hidden_character_in_a_name_is_shown_not_obeyed() {
    let gh = github(RepoShape::private("main", SHA));
    let sandbox = sandbox();
    add_entry(&gh, &entry(MAC_A, "Mac\u{202e}Book"));
    let found = fleet(&gh, &sandbox);
    assert!(!found.machines[0].name.contains('\u{202e}'));
}

#[test]
fn no_repo_yet_is_nothing_yet() {
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    let sandbox = sandbox();
    assert_eq!(
        read(&gh, &sandbox.tools, CONFIG_REPO_NAME),
        FleetRead::NothingYet
    );
}

#[test]
fn signed_out_or_offline_waits() {
    let sandbox = sandbox();
    let gh = FakeGitHub::new();
    assert_eq!(
        read(&gh, &sandbox.tools, CONFIG_REPO_NAME),
        FleetRead::Waiting(WaitReason::SignIn)
    );
    let gh = github(RepoShape::private("main", SHA));
    gh.go_offline();
    assert!(matches!(
        read(&gh, &sandbox.tools, CONFIG_REPO_NAME),
        FleetRead::Waiting(WaitReason::Offline | WaitReason::SignIn)
    ));
}
