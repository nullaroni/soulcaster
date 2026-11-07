//! Runs the real binary on files only. It reads a Config and prints, so nothing here can touch a Machine.

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
