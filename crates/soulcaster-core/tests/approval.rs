//! Approvals are typed so a value from a file can't become a flag or a short, moveable name.

use std::collections::BTreeMap;

use soulcaster_core::approval::{Approvals, CommitSha, DotfilesApproval};

#[test]
fn a_commit_is_exactly_forty_lowercase_hex_characters() {
    let good = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(CommitSha::parse(good).unwrap().as_str(), good);
    for bad in [
        "",
        "main",
        "v1.0",
        "HEAD",
        "3f9a1c2",
        "--upload-pack=evil",
        "0123456789ABCDEF0123456789ABCDEF01234567",
        "0123456789abcdef0123456789abcdef01234567\n",
        "0123456789abcdef0123456789abcdef012345678",
    ] {
        assert_eq!(CommitSha::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn a_record_holding_a_bad_commit_or_an_unknown_field_does_not_load() {
    let moved = r#"{"dotfiles":{"repo":"github.com/a/b","commit":"main","links":{}}}"#;
    assert!(serde_json::from_str::<Approvals>(moved).is_err());
    let extra = r#"{"trustEverything":true}"#;
    assert!(serde_json::from_str::<Approvals>(extra).is_err());
}

#[test]
fn approvals_round_trip_with_repo_and_commit_together() {
    let approvals = Approvals {
        taps: Default::default(),
        packages: ["mobile-dev-inc/tap/maestro".to_string()].into(),
        dotfiles: Some(DotfilesApproval {
            repo: "github.com/a/b".to_string(),
            commit: CommitSha::parse("0123456789abcdef0123456789abcdef01234567").unwrap(),
            links: BTreeMap::from([("~/.zshrc".to_string(), "zsh/zshrc".to_string())]),
        }),
        script: None,
    };
    let text = serde_json::to_string(&approvals).unwrap();
    assert_eq!(serde_json::from_str::<Approvals>(&text).unwrap(), approvals);
}
