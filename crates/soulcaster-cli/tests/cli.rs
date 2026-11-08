//! Runs the real binary on files only. It reads a Config and prints, so nothing here can touch a
//! Machine. `import` and `save` are run only on input refused before any process starts, with `HOME`
//! pointed at a temporary folder: the rest of their behaviour is tested on a fake, in the crate.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn soulcaster(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soulcaster"))
        .args(args)
        .output()
        .unwrap()
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/examples")
        .join(name)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn validate_accepts_a_good_config() {
    let out = soulcaster(&["validate", example("minimal.json").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("valid"), "{}", stdout(&out));
}

#[test]
fn validate_refuses_a_bad_config_with_exit_code_2_and_names_the_problem() {
    let out = soulcaster(&[
        "validate",
        example("invalid/duplicate-schema-version.json")
            .to_str()
            .unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("duplicate key"), "{}", stderr(&out));
}

#[test]
fn validate_reports_what_would_be_skipped() {
    let out = soulcaster(&[
        "validate",
        example("lint/secret-shaped-value.json").to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a lint finding skips one entry and keeps the file"
    );
    assert!(
        stdout(&out).contains("looks like a token"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn plan_prints_both_stages_in_order_with_what_each_item_needs() {
    let out = soulcaster(&["plan", example("owner-work.json").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    let admin = text.find("Admin Stage").expect("Admin Stage");
    let brew = text.find("homebrew").expect("homebrew");
    let user = text.find("User Stage").expect("User Stage");
    let gh = text[user..].find("gh").expect("gh");
    assert!(admin < brew && brew < user && gh > 0, "{text}");
    assert!(text.contains("tap approval"), "{text}");
    assert!(text.contains("nikitabobko/tap"), "{text}");
}

#[test]
fn a_name_from_the_file_is_shown_as_quoted_data() {
    let out = soulcaster(&["plan", example("minimal.json").to_str().unwrap()]);
    assert!(
        stdout(&out).lines().next().unwrap().contains('"'),
        "{}",
        stdout(&out)
    );
}

#[test]
fn a_missing_file_and_bad_usage_are_refused() {
    assert_eq!(
        soulcaster(&["validate", "/nonexistent/nope.json"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(soulcaster(&["install"]).status.code(), Some(2));
    assert_eq!(soulcaster(&[]).status.code(), Some(2));
}

fn in_sandbox(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soulcaster"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .output()
        .unwrap()
}

#[test]
fn import_refuses_a_bad_address_before_anything_runs() {
    let home = tempfile::tempdir().unwrap();
    for paste in [
        "http://github.com/a/b",
        "https://user:hunter2@github.com/a/b",
        "https://github.com.evil.example/a/b",
        "--upload-pack=evil/b",
        "a/b\nc",
    ] {
        let out = in_sandbox(home.path(), &["import", paste]);
        assert_eq!(out.status.code(), Some(2), "{paste}");
        assert!(!stderr(&out).contains("hunter2"), "{}", stderr(&out));
    }
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn save_refuses_a_secret_or_a_bad_file_before_anything_runs() {
    let home = tempfile::tempdir().unwrap();
    let secret = home.path().join("secret.json");
    std::fs::write(
        &secret,
        r#"{"schemaVersion":1,"name":"x","settings":{"example.note":"ghp_abcdefghijklmnopqrst"}}"#,
    )
    .unwrap();
    let out = in_sandbox(home.path(), &["save", secret.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(!stderr(&out).contains("ghp_"), "{}", stderr(&out));
    let bad = example("invalid/duplicate-key.json");
    let out = in_sandbox(home.path(), &["save", bad.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!home.path().join("Library").exists());
}

#[test]
fn import_and_save_refuse_bad_usage() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        &["import"][..],
        &["import", "a/b", "--nope"][..],
        &["save"][..],
        &["save", "x.json", "--repo"][..],
    ] {
        assert_eq!(
            in_sandbox(home.path(), args).status.code(),
            Some(2),
            "{args:?}"
        );
    }
}

/// Only refusals: a valid Config here would start a real Run on this Mac.
#[test]
fn run_refuses_bad_usage_and_a_missing_or_bad_file_before_anything_starts() {
    let home = tempfile::tempdir().unwrap();
    let missing = home.path().join("missing.json");
    let bad = example("invalid/duplicate-key.json");
    for args in [
        vec!["run"],
        vec!["run", "a.json", "b.json"],
        vec!["run", "a.json", "--nope"],
        vec!["run", missing.to_str().unwrap()],
        vec!["run", bad.to_str().unwrap(), "--json"],
    ] {
        let out = in_sandbox(home.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    assert!(!home.path().join("Library").exists());
}
