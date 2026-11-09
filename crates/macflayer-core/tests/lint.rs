//! `LINT_EXPECT` from the prototype's check script, plus its secret-shape cases.

mod common;

use common::{example, stub};
use macflayer_core::config::Config;
use macflayer_core::lint::{DEFAULT_CLONE_LOCATION, Finding, FindingKind, lint};

fn findings(path: &str) -> Vec<Finding> {
    let config = Config::parse(&example(path)).unwrap();
    lint(&config, &stub(), DEFAULT_CLONE_LOCATION)
}

fn kinds(path: &str) -> Vec<FindingKind> {
    let mut kinds: Vec<FindingKind> = findings(path).into_iter().map(|f| f.kind).collect();
    kinds.sort();
    kinds
}

fn with_setting(value: &str) -> Vec<Finding> {
    let text =
        serde_json::json!({"schemaVersion": 1, "name": "x", "settings": {"example.note": value}})
            .to_string();
    lint(
        &Config::parse(text.as_bytes()).unwrap(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    )
}

use FindingKind as K;

#[test]
fn each_lint_example_flags_exactly_what_the_prototype_expects() {
    let expect: &[(&str, &[FindingKind])] = &[
        ("secret-shaped-key.json", &[K::SecretKey]),
        ("secret-shaped-value.json", &[K::SecretValue]),
        ("git-identity-in-settings.json", &[K::MachineValue]),
        ("home-folder-value.json", &[K::HomePathValue]),
        ("email-value.json", &[K::EmailValue]),
        ("token-in-name.json", &[K::SecretValue]),
        (
            "link-protected-folder.json",
            &[K::LinkProtected, K::LinkProtected],
        ),
        ("link-case-duplicate.json", &[K::LinkDuplicate]),
        ("link-unicode-duplicate.json", &[K::LinkDuplicate]),
        ("link-ancestor-pair.json", &[K::LinkOverlap]),
        (
            "link-at-clone-location.json",
            &[
                K::LinkOverlap,
                K::LinkOverlap,
                K::LinkOverlap,
                K::LinkClone,
                K::LinkClone,
                K::LinkClone,
            ],
        ),
        (
            "write-through-link.json",
            &[K::WritesThroughLink, K::WritesThroughLink],
        ),
        ("base-items-listed.json", &[]),
        ("item-package-conflict.json", &[]),
        ("rename-collision.json", &[]),
        ("sign-in-item.json", &[]),
    ];
    for (name, want) in expect {
        let mut want = want.to_vec();
        want.sort();
        assert_eq!(kinds(&format!("lint/{name}")), want, "{name}");
    }
}

#[test]
fn the_valid_examples_have_no_findings() {
    for name in [
        "developer.json",
        "minimal.json",
        "owner-personal.json",
        "owner-work.json",
    ] {
        assert_eq!(findings(name), vec![], "{name}");
    }
}

#[test]
fn token_shapes_are_flagged() {
    for value in [
        "ghp_abcdefghijklmnop",
        " ghp_abcdefghijklmno",
        "Bearer abcdefghijklm",
        "sk_live_abcdefghijkl",
        "npm_abcdefghijkl",
        "hf_abcdefghijkl",
        "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.abc",
        "-----BEGIN OPENSSH PRIVATE KEY",
        "AKIAABCDEFGHIJKLMNOP",
    ] {
        let found = with_setting(value);
        assert_eq!(found.len(), 1, "{value}");
        assert_eq!(found[0].kind, K::SecretValue, "{value}");
    }
}

#[test]
fn real_values_with_secret_looking_names_pass() {
    assert_eq!(with_setting("sk-SK"), vec![]);
    let text = r#"{"schemaVersion":1,"name":"x","settings":{"sudo.pass":"x"}}"#;
    let found = lint(
        &Config::parse(text.as_bytes()).unwrap(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    assert_eq!(found[0].kind, K::SecretKey);
}

#[test]
fn a_token_in_a_link_source_is_flagged() {
    let text = r#"{"schemaVersion":1,"name":"x","dotfiles":{"repo":"a/b","links":{"~/.x":"ghp_abcdefghijklmnop"}}}"#;
    let found = lint(
        &Config::parse(text.as_bytes()).unwrap(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    assert_eq!(found[0].kind, K::SecretValue);
    assert_eq!(found[0].at, "~/.x");
}

#[test]
fn the_clone_location_is_an_input_because_a_machine_can_override_it() {
    let text =
        r#"{"schemaVersion":1,"name":"x","dotfiles":{"repo":"a/b","links":{"~/code/dots/x":"x"}}}"#;
    let config = Config::parse(text.as_bytes()).unwrap();
    assert_eq!(lint(&config, &stub(), DEFAULT_CLONE_LOCATION), vec![]);
    assert_eq!(lint(&config, &stub(), "~/code/dots")[0].kind, K::LinkClone);
}

#[test]
fn a_link_cannot_land_in_macflayers_own_folder() {
    for target in [
        "~/Library/Application Support/MacFlayer",
        "~/Library/Application Support/MacFlayer/machine.json",
        "~/library/application support/MACFLAYER/machine.json",
    ] {
        let text = serde_json::json!({
            "schemaVersion": 1,
            "name": "x",
            "dotfiles": {"repo": "a/b", "links": {target: "x"}}
        })
        .to_string();
        let config = Config::parse(text.as_bytes()).unwrap();
        let found = lint(&config, &stub(), DEFAULT_CLONE_LOCATION);
        assert!(
            found.iter().any(|f| f.kind == K::LinkOwnFolder),
            "{target}: {found:?}"
        );
    }
}

#[test]
fn a_folded_spelling_does_not_get_past_the_protected_list() {
    // U+017F folds to s on the Mac's volume, and Rust's to_lowercase leaves it alone.
    let text = "{\"schemaVersion\":1,\"name\":\"x\",\"dotfiles\":{\"repo\":\"a/b\",\"links\":{\"~/De\u{17f}ktop\":\"x\"}}}";
    let config = Config::parse(text.as_bytes()).unwrap();
    assert_eq!(
        lint(&config, &stub(), DEFAULT_CLONE_LOCATION)[0].kind,
        K::LinkProtected
    );
}

#[test]
fn both_sides_of_a_clash_are_refused_so_the_file_cannot_choose_the_winner() {
    use macflayer_core::lint::refused_links;
    let refused = refused_links(
        ["~/.zshrc", "~/.ZSHRC", "~/.a", "~/.a/b", "~/.fine"].into_iter(),
        DEFAULT_CLONE_LOCATION,
    );
    for target in ["~/.zshrc", "~/.ZSHRC", "~/.a", "~/.a/b"] {
        assert!(refused.contains_key(target), "{target}: {refused:?}");
    }
    assert!(!refused.contains_key("~/.fine"));
}
