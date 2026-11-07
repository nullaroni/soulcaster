//! Reading one Config at one commit, with GitHub played by a fake. Every response here is hostile
//! or merely wrong in some way a real server could be.

use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::Command;
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_repo::address::AddressError;
use soulcaster_repo::fake::{FakeGitHub, RepoShape};
use soulcaster_repo::github::FetchError;
use soulcaster_repo::import::{ImportError, Importer, Opened, RefusalKind};
use soulcaster_repo::tools::Tools;

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const NEWER: &str = "9c8d7e6000000000000000000000000000000000";
const CLEAN: &[u8] = include_bytes!("fixtures/import/clean.json");

fn tools() -> Tools {
    Tools::fixed(std::env::temp_dir().join("soulcaster-repo-tests-never-written"))
}

fn stub() -> Catalog {
    Catalog::parse(include_str!("fixtures/catalog-stub.json")).unwrap()
}

fn github() -> FakeGitHub {
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    gh.add_repo("example-dev", "mac-setup", RepoShape::public("main", SHA));
    gh.add_file("example-dev", "mac-setup", SHA, "configs/laptop.json", CLEAN);
    gh
}

fn open(gh: &FakeGitHub, paste: &str) -> Result<Opened, ImportError> {
    let tools = tools();
    let catalog = stub();
    Importer::connect(gh, &tools, &catalog, DEFAULT_CLONE_LOCATION).open(paste)
}

fn one(gh: &FakeGitHub, paste: &str) -> soulcaster_repo::import::Fetched {
    match open(gh, paste) {
        Ok(Opened::One(fetched)) => fetched,
        Ok(Opened::Pick(_)) => panic!("expected one Config"),
        Err(error) => panic!("{error:?}"),
    }
}

#[test]
fn a_repo_with_one_config_opens_it_pinned_to_the_commit_it_read() {
    let gh = github();
    let fetched = one(&gh, "example-dev/mac-setup");
    assert_eq!(fetched.source.commit.as_str(), SHA);
    assert_eq!(fetched.source.path.as_str(), "configs/laptop.json");
    assert_eq!(fetched.source.repo.full_name(), "example-dev/mac-setup");
    // The file was read at the commit, never at a name that can move.
    assert!(gh.api_paths().iter().any(|p| p
        == &format!("repos/example-dev/mac-setup/contents/configs/laptop.json?ref={SHA}")));
    assert!(
        !gh.api_paths()
            .iter()
            .any(|p| p.contains("contents") && p.contains("ref=main"))
    );
}

#[test]
fn a_repo_with_several_configs_is_a_choice_with_what_each_would_do() {
    let gh = github();
    gh.add_file(
        "example-dev",
        "mac-setup",
        SHA,
        "configs/mobile.json",
        include_bytes!("fixtures/import/one-tap.json"),
    );
    gh.add_file(
        "example-dev",
        "mac-setup",
        SHA,
        "configs/broken.json",
        br#"{"schemaVersion":1,"name":"x","trusted":true}"#,
    );
    gh.add_file("example-dev", "mac-setup", SHA, "configs/notes.txt", b"hello");
    let Ok(Opened::Pick(picker)) = open(&gh, "example-dev/mac-setup") else {
        panic!("expected a choice");
    };
    let names: Vec<&str> = picker.rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "configs/broken.json",
            "configs/laptop.json",
            "configs/mobile.json"
        ]
    );
    assert!(picker.rows[0].refused.is_some());
    assert!(picker.rows[1].refused.is_none());
    assert!(picker.rows[1].install > 0);
    assert_eq!(picker.rows[1].approvals, 0);
    assert_eq!(picker.rows[2].approvals, 1);

    let tools = tools();
    let catalog = stub();
    let importer = Importer::connect(&gh, &tools, &catalog, DEFAULT_CLONE_LOCATION);
    let fetched = importer.pick(&picker, &picker.rows[1].path).unwrap();
    assert_eq!(fetched.source.commit.as_str(), SHA);
    assert!(
        importer.pick(&picker, &picker.rows[0].path).is_err(),
        "a refused file can't be picked"
    );
}

#[test]
fn a_hostile_listing_cannot_name_a_path_outside_configs() {
    let gh = github();
    gh.set_response(
        &format!("repos/example-dev/mac-setup/contents/configs?ref={SHA}"),
        200,
        &serde_json::json!([
            {"name": "../../etc/passwd.json", "path": "../../etc/passwd.json", "type": "file", "size": 10},
            {"name": "-rf.json", "path": "configs/-rf.json", "type": "file", "size": 10},
            {"name": "x\n.json", "path": "configs/x\n.json", "type": "file", "size": 10},
            {"name": "a.json", "path": ".git/config.json", "type": "file", "size": 10},
            {"name": "link.json", "path": "configs/link.json", "type": "symlink", "size": 10},
            {"name": "laptop.json", "path": "configs/laptop.json", "type": "file", "size": CLEAN.len()},
        ])
        .to_string(),
    );
    let fetched = one(&gh, "example-dev/mac-setup");
    assert_eq!(fetched.source.path.as_str(), "configs/laptop.json");
    for path in gh.api_paths() {
        assert!(
            !path.contains("passwd") && !path.contains("-rf") && !path.contains(".git"),
            "{path}"
        );
    }
}

#[test]
fn a_file_link_opens_that_file_and_resolves_a_moveable_name_once() {
    let gh = github();
    let fetched = one(
        &gh,
        "https://github.com/example-dev/mac-setup/blob/main/configs/laptop.json",
    );
    assert_eq!(fetched.source.commit.as_str(), SHA);

    // The branch moves after the read. A new read gets a new commit, and the first copy keeps its own.
    gh.add_repo("example-dev", "mac-setup", RepoShape::public("main", NEWER));
    gh.add_file("example-dev", "mac-setup", NEWER, "configs/laptop.json", CLEAN);
    let later = one(
        &gh,
        "https://github.com/example-dev/mac-setup/blob/main/configs/laptop.json",
    );
    assert_eq!(later.source.commit.as_str(), NEWER);
    assert_eq!(fetched.source.commit.as_str(), SHA);
}

#[test]
fn a_commit_in_the_link_must_come_back_as_that_commit() {
    let gh = github();
    gh.set_response(
        &format!("repos/example-dev/mac-setup/commits/{SHA}"),
        200,
        &format!(r#"{{"sha":"{NEWER}"}}"#),
    );
    let error = open(
        &gh,
        &format!("https://github.com/example-dev/mac-setup/blob/{SHA}/configs/laptop.json"),
    )
    .err()
    .unwrap();
    assert_eq!(error, ImportError::Fetch(FetchError::Malformed));
}

#[test]
fn a_commit_that_is_not_a_full_sha_is_refused() {
    let gh = github();
    for bad in ["main", "3f9a1c2", "--upload-pack=evil", ""] {
        gh.set_response(
            "repos/example-dev/mac-setup/commits/main",
            200,
            &serde_json::json!({ "sha": bad }).to_string(),
        );
        assert_eq!(
            open(&gh, "example-dev/mac-setup").err().unwrap(),
            ImportError::Fetch(FetchError::Malformed),
            "{bad}"
        );
    }
}

#[test]
fn a_hostile_default_branch_never_reaches_a_request() {
    let gh = github();
    gh.set_response(
        "repos/example-dev/mac-setup",
        200,
        &serde_json::json!({
            "full_name": "example-dev/mac-setup",
            "default_branch": "--upload-pack=evil",
            "private": false,
            "owner": {"login": "example-dev"},
        })
        .to_string(),
    );
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::Malformed)
    );
    assert!(!gh.api_paths().iter().any(|p| p.contains("upload-pack")));
}

#[test]
fn an_oversized_file_is_refused_before_it_is_parsed() {
    let gh = github();
    let path = format!("repos/example-dev/mac-setup/contents/configs/laptop.json?ref={SHA}");
    // GitHub says the size up front.
    gh.set_response(
        &path,
        200,
        &serde_json::json!({
            "type": "file", "encoding": "none", "size": 5_000_000,
            "path": "configs/laptop.json", "content": ""
        })
        .to_string(),
    );
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::TooLarge)
    );
    // Or lies about it, and the bytes are counted anyway.
    use base64::Engine;
    let big = base64::engine::general_purpose::STANDARD.encode(vec![b' '; 1024 * 1024 + 1]);
    gh.set_response(
        &path,
        200,
        &serde_json::json!({
            "type": "file", "encoding": "base64", "size": 10,
            "path": "configs/laptop.json", "content": big
        })
        .to_string(),
    );
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::TooLarge)
    );
    // Or sends far more than any file could need.
    gh.set_response(&path, 200, &" ".repeat(8 * 1024 * 1024));
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::TooLarge)
    );
}

#[test]
fn a_response_for_some_other_path_or_a_symlink_is_refused() {
    let gh = github();
    let path = format!("repos/example-dev/mac-setup/contents/configs/laptop.json?ref={SHA}");
    use base64::Engine;
    let body = base64::engine::general_purpose::STANDARD.encode(CLEAN);
    gh.set_response(
        &path,
        200,
        &serde_json::json!({
            "type": "file", "encoding": "base64", "size": CLEAN.len(),
            "path": "configs/other.json", "content": body
        })
        .to_string(),
    );
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::Malformed)
    );
    gh.set_response(
        &path,
        200,
        &serde_json::json!({
            "type": "symlink", "size": 3, "target": "/etc/passwd", "path": "configs/laptop.json"
        })
        .to_string(),
    );
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::Malformed)
    );
    gh.set_response(&path, 200, "not json at all");
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::Malformed)
    );
}

fn refused_kind(bytes: &[u8]) -> (RefusalKind, String) {
    let gh = github();
    gh.add_file("example-dev", "mac-setup", SHA, "configs/laptop.json", bytes);
    match open(&gh, "example-dev/mac-setup") {
        Err(ImportError::Refused(refusal)) => (refusal.kind, refusal.detail),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_strict_parser_refuses_the_whole_file_before_anything_reads_it() {
    assert_eq!(
        refused_kind(include_bytes!("fixtures/import/refused-duplicate-key.json")).0,
        RefusalKind::DuplicateKey
    );
    assert_eq!(
        refused_kind(include_bytes!("fixtures/import/refused-newer-version.json")).0,
        RefusalKind::NewerVersion
    );
    assert_eq!(
        refused_kind(include_bytes!("fixtures/import/refused-self-approval.json")).0,
        RefusalKind::UnknownField
    );
    let (kind, detail) = refused_kind(include_bytes!("fixtures/import/refused-bidi.json"));
    assert_eq!(kind, RefusalKind::BadText);
    assert!(detail.contains("\\u202e"), "{detail}");
    assert!(!detail.contains('\u{202e}'), "the character itself isn't shown");
}

#[test]
fn a_refusal_shows_no_control_or_bidi_character_from_the_file() {
    let (_, detail) = refused_kind(
        "{\"schemaVersion\":1,\"name\":\"x\",\"dotfiles\":{\"repo\":\"a/b\",\"links\":{\"~/.x\u{202e}\u{200b}\":\"y\"}}}"
            .as_bytes(),
    );
    assert!(detail.chars().all(|c| (' '..='~').contains(&c)), "{detail}");
}

#[test]
fn failures_keep_nothing_and_say_what_happened() {
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    assert_eq!(
        open(&gh, "example-dev/private-setup").err().unwrap(),
        ImportError::Fetch(FetchError::NotVisible { signed_in: true })
    );

    let gh = FakeGitHub::new();
    assert_eq!(
        open(&gh, "example-dev/private-setup").err().unwrap(),
        ImportError::Fetch(FetchError::NotVisible { signed_in: false })
    );

    let gh = github();
    gh.go_offline();
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::Offline)
    );

    let gh = FakeGitHub::new();
    gh.add_repo("example-dev", "empty", RepoShape::public("main", SHA));
    assert_eq!(
        open(&gh, "example-dev/empty").err().unwrap(),
        ImportError::NoConfigs
    );

    let gh = github();
    gh.set_response("repos/example-dev/mac-setup", 403, "API rate limit exceeded");
    assert_eq!(
        open(&gh, "example-dev/mac-setup").err().unwrap(),
        ImportError::Fetch(FetchError::RateLimited)
    );
}

#[test]
fn not_signed_in_reads_public_repos_without_gh_and_sends_no_token() {
    let gh = github();
    gh.sign_out();
    let fetched = one(&gh, "example-dev/mac-setup");
    assert_eq!(fetched.source.commit.as_str(), SHA);
    let curl: Vec<Command> = gh
        .calls()
        .into_iter()
        .filter(|c| c.program.ends_with("curl"))
        .collect();
    assert!(!curl.is_empty());
    for call in &curl {
        assert_eq!(call.args[0], "-q", "no curlrc");
        assert!(
            call.args.windows(2).any(|w| w == ["--proto", "=https"]),
            "{:?}",
            call.args
        );
        assert!(call.args.iter().any(|a| a == "--max-filesize"));
        assert!(
            call.args
                .last()
                .unwrap()
                .starts_with("https://api.github.com/repos/example-dev/")
        );
        assert!(!call.env.iter().any(|(k, _)| k.contains("TOKEN")));
    }
}

#[test]
fn a_gh_token_never_reaches_a_command_line_a_log_or_an_error() {
    let gh = github();
    gh.fail_all_with("gh: Bad credentials ghp_abcdefghijklmnopqrstuvwxyz (HTTP 401)");
    let error = open(&gh, "example-dev/mac-setup").err().unwrap();
    assert!(!format!("{error:?} {error}").contains("ghp_"));
    for call in gh.calls() {
        let line = format!("{:?}", (&call.args, &call.env));
        assert!(!line.contains("ghp_") && !line.contains("GH_TOKEN"), "{line}");
    }
}

#[test]
fn a_bad_paste_is_refused_before_any_command_runs() {
    let gh = FakeGitHub::new();
    for (paste, expected) in [
        ("http://github.com/a/b", AddressError::NotHttps),
        ("https://user:pw@github.com/a/b", AddressError::Credentials),
    ] {
        assert_eq!(
            open(&gh, paste).err().unwrap(),
            ImportError::Address(expected)
        );
    }
    assert!(gh.calls().is_empty());
}

#[test]
fn every_failure_is_worded_without_git_words() {
    let errors = [
        ImportError::Fetch(FetchError::Offline),
        ImportError::Fetch(FetchError::NotVisible { signed_in: false }),
        ImportError::Fetch(FetchError::NotVisible { signed_in: true }),
        ImportError::Fetch(FetchError::RateLimited),
        ImportError::Fetch(FetchError::TooLarge),
        ImportError::Fetch(FetchError::Malformed),
        ImportError::Fetch(FetchError::Empty),
        ImportError::NoConfigs,
        ImportError::Address(AddressError::Empty),
        ImportError::Address(AddressError::NotHttps),
        ImportError::Address(AddressError::Credentials),
        ImportError::Address(AddressError::HiddenCharacters),
        ImportError::Address(AddressError::Malformed),
    ];
    for error in errors {
        soulcaster_repo::assert_no_git_words(&error.to_string());
    }
}
