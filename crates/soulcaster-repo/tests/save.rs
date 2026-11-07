//! Saving a Config to the person's own repo, with GitHub played by a fake. It must work with
//! nothing saved yet, keep the change when GitHub can't take it, and never force anything.

use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, Reply};
use soulcaster_core::config::{Config, SCHEMA_VERSION};
use soulcaster_repo::fake::{FakeGitHub, RepoShape};
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveError, SaveOutcome, SaveRequest, WaitReason, save};
use soulcaster_repo::tools::Tools;

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const PUSHED: &str = "bbbbbbb000000000000000000000000000000000";

struct Sandbox {
    _dir: tempfile::TempDir,
    tools: Tools,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let tools = Tools::fixed(dir.path().to_path_buf());
    Sandbox { _dir: dir, tools }
}

fn config() -> Config {
    Config::parse(br#"{"schemaVersion":1,"name":"My Mac","items":["ghostty"]}"#).unwrap()
}

fn request(config: &Config) -> SaveRequest<'_> {
    SaveRequest {
        config,
        repo_name: CONFIG_REPO_NAME,
        base: None,
    }
}

fn run_save(gh: &FakeGitHub, sandbox: &Sandbox, config: &Config) -> Result<SaveOutcome, SaveError> {
    save(gh, &sandbox.tools, Catalog::shipped(), &request(config))
}

fn git_calls(gh: &FakeGitHub) -> Vec<Command> {
    gh.calls()
        .into_iter()
        .filter(|c| c.program.ends_with("git"))
        .collect()
}

fn has(call: &Command, word: &str) -> bool {
    call.args.iter().any(|a| a == word)
}

fn signed_in() -> FakeGitHub {
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    gh.on_git("rev-parse", Reply::ok(format!("{PUSHED}\n")));
    gh
}

#[test]
fn with_nothing_saved_yet_it_creates_a_private_repo_and_saves_the_canonical_form() {
    let gh = signed_in();
    let sandbox = sandbox();
    let config = config();
    let outcome = run_save(&gh, &sandbox, &config).unwrap();
    let SaveOutcome::Sent {
        repo,
        path,
        commit,
        public,
        created_repo,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(repo.full_name(), format!("example-you/{CONFIG_REPO_NAME}"));
    assert_eq!(path, "configs/my-mac.json");
    assert_eq!(commit.as_str(), PUSHED);
    assert!(!public);
    assert!(created_repo);

    let create = gh
        .calls()
        .into_iter()
        .find(|c| c.args.first().is_some_and(|a| a == "repo"))
        .expect("the repo was created");
    assert_eq!(
        create.args,
        ["repo", "create", "example-you/soulcaster-configs", "--private"]
    );
    // The file on disk at commit time is exactly the canonical form.
    assert_eq!(
        gh.file_at_git("add", "configs/my-mac.json").unwrap(),
        config.to_canonical_json()
    );
}

#[test]
fn it_commits_as_the_github_noreply_address_and_nothing_else_about_this_mac() {
    let gh = signed_in();
    let sandbox = sandbox();
    run_save(&gh, &sandbox, &config()).unwrap();
    let commit = git_calls(&gh)
        .into_iter()
        .find(|c| has(c, "commit"))
        .expect("a commit");
    let line = commit.args.join(" ");
    assert!(line.contains("user.email=42+example-you@users.noreply.github.com"), "{line}");
    assert!(line.contains("user.name=example-you"), "{line}");
    assert!(!line.contains("peyton"), "{line}");
    let message = commit
        .args
        .iter()
        .skip_while(|a| *a != "-m")
        .nth(1)
        .unwrap();
    assert_eq!(message, "Save configs/my-mac.json");
}

#[test]
fn every_git_command_is_hardened_and_no_token_reaches_a_command_line() {
    let gh = signed_in();
    let sandbox = sandbox();
    run_save(&gh, &sandbox, &config()).unwrap();
    let calls = git_calls(&gh);
    assert!(!calls.is_empty());
    for call in &calls {
        let env = |k: &str| call.env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(env("GIT_CONFIG_GLOBAL"), Some("/dev/null"));
        assert_eq!(env("GIT_CONFIG_NOSYSTEM"), Some("1"));
        assert_eq!(env("GIT_TERMINAL_PROMPT"), Some("0"));
        assert_eq!(env("GIT_ALLOW_PROTOCOL"), Some("https"));
        let line = call.args.join(" ");
        assert!(line.contains("core.hooksPath=/dev/null"), "{line}");
        assert!(line.contains("core.fsmonitor=false"), "{line}");
        assert!(line.contains("protocol.allow=never"), "{line}");
        assert!(line.contains("credential.helper="), "{line}");
        assert!(!line.contains("--force") && !line.contains("+HEAD"), "{line}");
        assert!(!line.contains("ghp_") && !line.contains("x-access-token"), "{line}");
        assert!(!call.env.iter().any(|(k, _)| k.contains("TOKEN")));
    }
    // Positional values come after `--`, so none of them can be read as a flag.
    let push = calls.iter().find(|c| has(c, "push")).unwrap();
    let dash = push.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(push.args[dash + 1], "origin");
    assert_eq!(push.args[dash + 2], "HEAD:refs/heads/main");
    let remote = calls.iter().find(|c| has(c, "remote")).unwrap();
    assert_eq!(
        remote.args.last().unwrap(),
        "https://github.com/example-you/soulcaster-configs.git"
    );
}

#[test]
fn an_existing_repo_is_built_on_not_replaced() {
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("trunk", SHA));
    gh.set_git_files("checkout", vec![("configs/other.json", "{}\n")]);
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    let SaveOutcome::Sent { created_repo, public, .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(!created_repo);
    assert!(!public);
    assert!(!gh.calls().iter().any(|c| c.args.first().is_some_and(|a| a == "repo")));
    let calls = git_calls(&gh);
    let fetch = calls.iter().find(|c| has(c, "fetch")).unwrap();
    assert!(has(fetch, SHA), "{:?}", fetch.args);
    let push = calls.iter().find(|c| has(c, "push")).unwrap();
    assert_eq!(push.args.last().unwrap(), "HEAD:refs/heads/trunk");
}

#[test]
fn a_public_repo_is_said_once_when_saving() {
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::public("main", SHA));
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(matches!(outcome, SaveOutcome::Sent { public: true, .. }), "{outcome:?}");
}

#[test]
fn a_hostile_branch_name_from_github_stops_the_save() {
    let gh = signed_in();
    gh.set_response(
        &format!("repos/example-you/{CONFIG_REPO_NAME}"),
        200,
        &serde_json::json!({
            "full_name": format!("example-you/{CONFIG_REPO_NAME}"),
            "default_branch": "--receive-pack=evil",
            "private": true,
            "owner": {"login": "example-you"},
            "permissions": {"push": true},
        })
        .to_string(),
    );
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(matches!(outcome, SaveOutcome::Waiting { .. }), "{outcome:?}");
    assert!(git_calls(&gh).is_empty());
}

#[test]
fn a_repo_that_github_says_belongs_to_someone_else_is_not_written() {
    let gh = signed_in();
    gh.set_response(
        &format!("repos/example-you/{CONFIG_REPO_NAME}"),
        200,
        &serde_json::json!({
            "full_name": "someone-else/moved",
            "default_branch": "main",
            "private": true,
            "owner": {"login": "someone-else"},
            "permissions": {"push": true},
        })
        .to_string(),
    );
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Waiting { why: WaitReason::Denied, .. }),
        "{outcome:?}"
    );
    assert!(git_calls(&gh).is_empty());
}

#[test]
fn a_secret_stops_the_save_before_any_command_runs() {
    let gh = signed_in();
    let sandbox = sandbox();
    for (key, value) in [
        ("example.note", "ghp_abcdefghijklmnopqrstuvwxyz"),
        ("example.password", "hunter2"),
        ("example.owner", "person@example.com"),
        ("example.path", "/Users/somebody/Developer"),
        ("git.user.email", "person@example.com"),
    ] {
        let text = serde_json::json!({
            "schemaVersion": SCHEMA_VERSION,
            "name": "x",
            "settings": {key: value}
        })
        .to_string();
        let config = Config::parse(text.as_bytes()).unwrap();
        let error = run_save(&gh, &sandbox, &config).unwrap_err();
        assert!(matches!(error, SaveError::Blocked(_)), "{key}");
        assert!(!format!("{error}").contains(value), "the value isn't repeated");
    }
    assert!(gh.calls().is_empty());
}

#[test]
fn a_config_that_would_not_read_back_is_never_sent() {
    let gh = signed_in();
    let sandbox = sandbox();
    let mut config = config();
    config.name = "bad\u{202e}name".to_string();
    assert!(matches!(
        run_save(&gh, &sandbox, &config).unwrap_err(),
        SaveError::Invalid(_)
    ));
    config.name = "x".to_string();
    config.items = vec!["--upload-pack=evil".to_string()];
    assert!(matches!(
        run_save(&gh, &sandbox, &config).unwrap_err(),
        SaveError::Invalid(_)
    ));
    assert!(gh.calls().is_empty());
}

#[test]
fn the_name_on_disk_is_a_plain_slug_whatever_the_config_is_called() {
    for (name, path) in [
        ("My Mac", "configs/my-mac.json"),
        ("../../evil", "configs/evil.json"),
        ("-rf", "configs/rf.json"),
        ("日本語", "configs/config.json"),
        (".git", "configs/git.json"),
    ] {
        let gh = signed_in();
        let sandbox = sandbox();
        let mut config = config();
        config.name = name.to_string();
        let SaveOutcome::Sent { path: saved, .. } = run_save(&gh, &sandbox, &config).unwrap() else {
            panic!("{name}");
        };
        assert_eq!(saved, path, "{name}");
    }
}

#[test]
fn no_network_leaves_the_change_on_this_mac() {
    let gh = signed_in();
    gh.go_offline();
    let sandbox = sandbox();
    let config = config();
    let SaveOutcome::Waiting { why, local } = run_save(&gh, &sandbox, &config).unwrap() else {
        panic!();
    };
    assert_eq!(why, WaitReason::Offline);
    assert_eq!(
        std::fs::read_to_string(&local).unwrap(),
        config.to_canonical_json()
    );
    assert!(local.starts_with(std::fs::canonicalize(&sandbox.tools.home).unwrap()));
}

#[test]
fn not_signed_in_waits_and_a_bare_mac_with_no_tools_waits_too() {
    let gh = FakeGitHub::new();
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Waiting { why: WaitReason::SignIn, .. }),
        "{outcome:?}"
    );

    let gh = FakeGitHub::new();
    let mut bare = sandbox_tools();
    bare.0.gh = None;
    bare.0.git = None;
    let config = config();
    let outcome = save(&gh, &bare.0, Catalog::shipped(), &request(&config)).unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Waiting { why: WaitReason::SignIn, .. }),
        "{outcome:?}"
    );
    assert!(gh.calls().is_empty(), "nothing to start");
}

fn sandbox_tools() -> (Tools, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Tools::fixed(dir.path().to_path_buf()), dir)
}

#[test]
fn github_moving_is_tried_again_on_top_and_three_times_is_enough() {
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    gh.on_git_replies(
        "push",
        vec![
            Reply::fails(1, "! [rejected] HEAD -> main (fetch first)"),
            Reply::ok(""),
        ],
    );
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(matches!(outcome, SaveOutcome::Sent { .. }), "{outcome:?}");
    let pushes = git_calls(&gh).into_iter().filter(|c| has(c, "push")).count();
    assert_eq!(pushes, 2);

    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    gh.on_git("push", Reply::fails(1, "! [rejected] HEAD -> main (non-fast-forward)"));
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Waiting { why: WaitReason::Busy, .. }),
        "{outcome:?}"
    );
    let pushes = git_calls(&gh).into_iter().filter(|c| has(c, "push")).count();
    assert_eq!(pushes, 3);
}

#[test]
fn github_refusing_is_not_tried_again_and_says_so() {
    for (stderr, expected) in [
        ("remote: error: GH006: Protected branch update failed", WaitReason::Denied),
        ("remote: Permission to x denied. fatal: unable to access (403)", WaitReason::Denied),
        ("fatal: could not read Username for 'https://github.com'", WaitReason::SignIn),
        ("fatal: Authentication failed for 'https://github.com/'", WaitReason::SignIn),
        ("fatal: unable to access: Could not resolve host: github.com", WaitReason::Offline),
    ] {
        let gh = signed_in();
        gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
        gh.on_git("push", Reply::fails(128, stderr));
        let sandbox = sandbox();
        let outcome = run_save(&gh, &sandbox, &config()).unwrap();
        let SaveOutcome::Waiting { why, .. } = outcome else {
            panic!("{stderr}: {outcome:?}");
        };
        assert_eq!(why, expected, "{stderr}");
        let pushes = git_calls(&gh).into_iter().filter(|c| has(c, "push")).count();
        assert_eq!(pushes, 1, "{stderr}");
    }
}

#[test]
fn someone_elses_change_to_the_same_file_comes_back_instead_of_being_replaced() {
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    gh.set_git_files("checkout", vec![("configs/my-mac.json", "{\"changed\":\"elsewhere\"}\n")]);
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(matches!(outcome, SaveOutcome::ChangedElsewhere { .. }), "{outcome:?}");
    assert!(!git_calls(&gh).iter().any(|c| has(c, "push")));

    // With the text this Mac last sent as the base, a change on this Mac goes on top.
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    let before = "{\"before\":true}\n";
    gh.set_git_files("checkout", vec![("configs/my-mac.json", before)]);
    let config = config();
    let outcome = save(
        &gh,
        &sandbox.tools,
        Catalog::shipped(),
        &SaveRequest {
            config: &config,
            repo_name: CONFIG_REPO_NAME,
            base: Some(before),
        },
    )
    .unwrap();
    assert!(matches!(outcome, SaveOutcome::Sent { .. }), "{outcome:?}");
}

#[test]
fn the_same_bytes_already_there_write_nothing() {
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    let config = config();
    let text = config.to_canonical_json();
    gh.set_git_files("checkout", vec![("configs/my-mac.json", Box::leak(text.into_boxed_str()))]);
    let sandbox = sandbox();
    let outcome = run_save(&gh, &sandbox, &config).unwrap();
    assert!(matches!(outcome, SaveOutcome::AlreadyThere { .. }), "{outcome:?}");
    assert!(!git_calls(&gh).iter().any(|c| has(c, "commit") || has(c, "push")));
}

#[test]
fn a_symlink_planted_in_the_repo_is_not_written_through() {
    let gh = signed_in();
    gh.add_repo("example-you", CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    let sandbox = sandbox();
    let outside = sandbox.tools.home.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    gh.set_git_symlink("checkout", "configs", outside.clone());
    let outcome = run_save(&gh, &sandbox, &config()).unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Waiting { why: WaitReason::Denied, .. }),
        "{outcome:?}"
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn the_working_copy_is_gone_after_every_save() {
    let gh = signed_in();
    let sandbox = sandbox();
    run_save(&gh, &sandbox, &config()).unwrap();
    let work = sandbox.tools.home.join("Library/Application Support/Soulcaster/work");
    assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0);
}

#[test]
fn the_screen_words_have_no_git_words_in_them() {
    let outcomes = [
        WaitReason::Offline,
        WaitReason::SignIn,
        WaitReason::Denied,
        WaitReason::Busy,
    ];
    for why in outcomes {
        soulcaster_repo::assert_no_git_words(why.words());
    }
}
