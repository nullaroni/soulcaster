//! The review's rules: where each box starts, what each box allows, and that only a person's boxes
//! ever become approvals. The samples and `expected.json` come from the Import prototype.

use macflayer_core::approval::{
    ApprovalStore, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use macflayer_core::catalog::Catalog;
use macflayer_core::config::Config;
use macflayer_core::lint::DEFAULT_CLONE_LOCATION;
use macflayer_repo::address::{ConfigPath, RepoId};
use macflayer_repo::fake::FileApprovals;
use macflayer_repo::import::{Fetched, Source};
use macflayer_repo::review::{DotfilesFacts, Primary, Review, Tick};

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const DOTS: &str = "a1b2c3d000000000000000000000000000000000";

fn stub() -> Catalog {
    Catalog::parse(include_str!("fixtures/catalog-stub.json")).unwrap()
}

fn fetched(bytes: &[u8], owner: &str) -> Fetched {
    Fetched {
        source: Source {
            repo: RepoId::new(owner, "setup").unwrap(),
            path: ConfigPath::parse("configs/x.json").unwrap(),
            commit: CommitSha::parse(SHA).unwrap(),
        },
        config: Config::parse(bytes).unwrap(),
    }
}

struct Case {
    bytes: &'static [u8],
    owner: &'static str,
    signed_in: Option<&'static str>,
    facts: DotfilesFacts,
}

fn facts() -> DotfilesFacts {
    DotfilesFacts {
        commit: CommitSha::parse(DOTS),
        current_owner: Some("example-stranger".to_string()),
        script_text: Some("#!/bin/sh\necho hello\n".to_string()),
    }
}

fn review(case: Case) -> Review {
    Review::build(
        &fetched(case.bytes, case.owner),
        case.signed_in,
        &case.facts,
        &stub(),
        DEFAULT_CLONE_LOCATION,
    )
}

const CLEAN: &[u8] = include_bytes!("fixtures/import/clean.json");
const ONE_TAP: &[u8] = include_bytes!("fixtures/import/one-tap.json");
const HOSTILE: &[u8] = include_bytes!("fixtures/import/hostile.json");
const OWN: &[u8] = include_bytes!("fixtures/import/own-dotfiles.json");
const STRANGER: &[u8] = include_bytes!("fixtures/import/stranger-dotfiles.json");

fn clean() -> Review {
    review(Case {
        bytes: CLEAN,
        owner: "example-dev",
        signed_in: Some("example-you"),
        facts: DotfilesFacts::default(),
    })
}

fn one_tap() -> Review {
    review(Case {
        bytes: ONE_TAP,
        owner: "example-dev",
        signed_in: Some("example-you"),
        facts: DotfilesFacts::default(),
    })
}

fn hostile() -> Review {
    review(Case {
        bytes: HOSTILE,
        owner: "macf1ayer-official",
        signed_in: Some("example-you"),
        facts: DotfilesFacts {
            commit: CommitSha::parse(DOTS),
            current_owner: None,
            script_text: None,
        },
    })
}

fn stranger() -> Review {
    review(Case {
        bytes: STRANGER,
        owner: "example-stranger",
        signed_in: Some("example-you"),
        facts: facts(),
    })
}

fn own_facts() -> DotfilesFacts {
    DotfilesFacts {
        commit: CommitSha::parse(DOTS),
        current_owner: Some("example-you".to_string()),
        script_text: Some("#!/bin/sh\nmkdir -p ~/Developer\n".to_string()),
    }
}

fn own() -> Review {
    review(Case {
        bytes: OWN,
        owner: "example-you",
        signed_in: Some("example-you"),
        facts: own_facts(),
    })
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

#[test]
fn the_samples_match_what_the_prototype_decided() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/import/expected.json")).unwrap();
    let list = |name: &str, field: &str| -> Vec<String> {
        expected[name][field]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    for (name, review) in [
        ("clean.json", clean()),
        ("one-tap.json", one_tap()),
        ("stranger-dotfiles.json", stranger()),
        ("own-dotfiles.json", own()),
        ("hostile.json", hostile()),
    ] {
        let taps = sorted(review.taps.iter().map(|t| t.tap.clone()).collect());
        assert_eq!(taps, list(name, "taps"), "{name} taps");
        let blocked = sorted(
            review
                .taps
                .iter()
                .filter(|t| t.blocked)
                .map(|t| t.tap.clone())
                .collect(),
        );
        assert_eq!(blocked, list(name, "blockedTaps"), "{name} blocked");
        let signals = sorted(review.signals.iter().map(|s| s.code.to_string()).collect());
        assert_eq!(signals, list(name, "signals"), "{name} signals");
        let refused = sorted(
            review
                .dotfiles
                .iter()
                .flat_map(|d| d.links.iter())
                .filter(|l| l.refused.is_some())
                .map(|l| l.target.clone())
                .collect(),
        );
        assert_eq!(refused, list(name, "refusedLinks"), "{name} refused links");
        let wont = sorted(review.wont_apply.iter().map(|w| w.id.clone()).collect());
        assert_eq!(wont, list(name, "wontApply"), "{name} won't apply");
        assert_eq!(
            review.dotfiles.as_ref().map(|d| d.repo.clone()),
            expected[name]["dotfiles"].as_str().map(str::to_string),
            "{name} dotfiles"
        );
    }
}

#[test]
fn a_clean_config_starts_safe_groups_on_with_nothing_to_approve() {
    let review = clean();
    let boxes = review.starting_boxes(&Approvals::default());
    assert!(boxes.catalog && boxes.formulae && boxes.settings);
    assert!(review.signals.is_empty());
    assert_eq!(review.primary(&boxes), Primary::Start("Start".to_string()));
    let run = review.run_contents(&boxes);
    assert!(run.taps.is_empty() && run.clone.is_none() && run.script.is_none());
    assert_eq!(run.config.items, review.fetched_config().items);
}

#[test]
fn a_misleading_config_starts_with_everything_off_and_close_as_the_primary_button() {
    let review = hostile();
    // Even if this Machine had approved something from it before.
    let dotfiles = review.dotfiles.as_ref().unwrap();
    let approvals = Approvals {
        taps: Default::default(),
        packages: ["mesa-tools/tap/libxml2".to_string()].into(),
        dotfiles: Some(DotfilesApproval {
            repo: dotfiles.key.clone().unwrap(),
            commit: CommitSha::parse(DOTS).unwrap(),
            links: dotfiles.live_links(),
        }),
        script: None,
    };
    let boxes = review.starting_boxes(&approvals);
    assert!(!(boxes.catalog || boxes.formulae || boxes.casks));
    assert!(!(boxes.settings || boxes.repo || boxes.script));
    assert!(boxes.taps.values().all(|on| !on));
    assert!(review.nothing_selected(&boxes));
    assert_eq!(review.primary(&boxes), Primary::Close);
    // Start Run does nothing while nothing is selected.
    let run = review.run_contents(&boxes);
    assert!(run.is_empty());
    assert!(run.config.items.is_empty() && run.config.packages.is_empty());
    assert!(run.config.settings.is_empty() && run.config.dotfiles.is_none());
}

#[test]
fn a_tap_hidden_in_a_long_package_list_is_surfaced_with_where_it_was() {
    let review = hostile();
    let tap = review
        .taps
        .iter()
        .find(|t| t.tap == "mesa-tools/tap")
        .expect("the buried tap");
    assert!(
        tap.packages[0].via.contains("of 480 in packages.formulae"),
        "{:?}",
        tap.packages[0].via
    );
    assert!(
        !review
            .official_formulae
            .iter()
            .any(|name| name.contains('/')),
        "a third-party name never counts as official Homebrew"
    );
}

#[test]
fn a_package_named_like_a_base_item_cannot_be_approved_or_installed() {
    let review = hostile();
    let tap = review
        .taps
        .iter()
        .find(|t| t.tap == "evil-tools/tap")
        .unwrap();
    assert!(tap.blocked);
    let mut boxes = review.starting_boxes(&Approvals::default());
    assert!(!review.can_tick(&boxes, &Tick::Tap("evil-tools/tap".to_string())));
    review.toggle(&mut boxes, &Tick::Tap("evil-tools/tap".to_string()));
    assert!(!boxes.taps["evil-tools/tap"], "the toggle does nothing");
    // Even a forced box installs nothing.
    boxes.taps.insert("evil-tools/tap".to_string(), true);
    let run = review.run_contents(&boxes);
    assert!(!run.taps.contains(&"evil-tools/tap".to_string()));
    assert!(
        !run.config
            .packages
            .casks
            .contains(&"evil-tools/tap/gh".to_string())
    );
    let after = review.approvals_after_run(&boxes, &Approvals::default());
    assert!(!after.packages.contains("evil-tools/tap/gh"));
}

#[test]
fn a_tap_box_starts_off_and_leaving_it_off_skips_only_its_packages() {
    let review = one_tap();
    let boxes = review.starting_boxes(&Approvals::default());
    assert!(!boxes.taps["mobile-dev-inc/tap"]);
    assert_eq!(
        review.primary(&boxes),
        Primary::Start("Start without them".to_string())
    );
    let run = review.run_contents(&boxes);
    assert!(
        !run.config
            .packages
            .formulae
            .iter()
            .any(|p| p.starts_with("mobile-dev-inc/")),
        "{:?}",
        run.config.packages.formulae
    );
    assert!(!run.config.items.is_empty() || !run.config.packages.is_empty());

    let mut on = boxes.clone();
    review.toggle(&mut on, &Tick::Tap("mobile-dev-inc/tap".to_string()));
    assert_eq!(
        review.primary(&on),
        Primary::Start("Start with 1 approval".to_string())
    );
    let run = review.run_contents(&on);
    assert_eq!(run.taps, ["mobile-dev-inc/tap"]);
    assert!(
        run.packages
            .contains(&"mobile-dev-inc/tap/maestro".to_string())
    );
}

#[test]
fn an_approval_covers_the_packages_named_not_the_tap() {
    let review = one_tap();
    let mut boxes = review.starting_boxes(&Approvals::default());
    review.toggle(&mut boxes, &Tick::Tap("mobile-dev-inc/tap".to_string()));
    let after = review.approvals_after_run(&boxes, &Approvals::default());
    assert!(review.starting_boxes(&after).taps["mobile-dev-inc/tap"]);
    // A new package from the same tap starts off again.
    let mut more = after.clone();
    more.packages.clear();
    more.packages
        .insert("mobile-dev-inc/tap/something-else".to_string());
    assert!(!review.starting_boxes(&more).taps["mobile-dev-inc/tap"]);
}

#[test]
fn a_strangers_dotfiles_repo_starts_off_and_the_script_needs_the_repo_first() {
    let review = stranger();
    let mut boxes = review.starting_boxes(&Approvals::default());
    assert!(!boxes.repo);
    assert!(!boxes.script);
    assert!(!review.can_tick(&boxes, &Tick::Script), "the repo isn't on");
    review.toggle(&mut boxes, &Tick::Script);
    assert!(!boxes.script);

    review.toggle(&mut boxes, &Tick::Repo);
    assert!(boxes.repo);
    assert!(!boxes.script, "the repo doesn't imply the script");
    review.toggle(&mut boxes, &Tick::Script);
    assert!(boxes.script);
    let run = review.run_contents(&boxes);
    assert_eq!(run.script.as_deref(), Some("script/setup"));

    // Turning the repo off turns the script off.
    review.toggle(&mut boxes, &Tick::Repo);
    assert_eq!((boxes.repo, boxes.script), (false, false));
    let run = review.run_contents(&boxes);
    assert!(run.clone.is_none());
    assert!(run.config.dotfiles.is_none());
}

#[test]
fn your_own_repo_starts_on_but_its_script_stays_off() {
    let review = own();
    let boxes = review.starting_boxes(&Approvals::default());
    assert!(boxes.repo);
    assert!(!boxes.script);
}

#[test]
fn own_means_github_says_so_and_you_own_the_config_too() {
    // Not signed in: MacFlayer can't know.
    let signed_out = review(Case {
        bytes: OWN,
        owner: "example-you",
        signed_in: None,
        facts: own_facts(),
    });
    assert!(!signed_out.starting_boxes(&Approvals::default()).repo);

    // A stranger's Config naming your repo still wrote that list of links.
    let strangers_config = review(Case {
        bytes: OWN,
        owner: "example-stranger",
        signed_in: Some("example-you"),
        facts: own_facts(),
    });
    assert!(strangers_config.dotfiles.as_ref().unwrap().names_your_repo);
    assert!(!strangers_config.starting_boxes(&Approvals::default()).repo);

    // The repo was transferred: the name in the file is yours, and GitHub says otherwise.
    let transferred = review(Case {
        bytes: OWN,
        owner: "example-you",
        signed_in: Some("example-you"),
        facts: DotfilesFacts {
            current_owner: Some("someone-else".to_string()),
            ..own_facts()
        },
    });
    assert!(!transferred.starting_boxes(&Approvals::default()).repo);

    // Case doesn't matter to GitHub, so it doesn't here.
    let case = review(Case {
        bytes: OWN,
        owner: "Example-You",
        signed_in: Some("EXAMPLE-you"),
        facts: own_facts(),
    });
    assert!(case.starting_boxes(&Approvals::default()).repo);

    // No commit, nothing to approve.
    let no_commit = review(Case {
        bytes: OWN,
        owner: "example-you",
        signed_in: Some("example-you"),
        facts: DotfilesFacts {
            commit: None,
            ..own_facts()
        },
    });
    let boxes = no_commit.starting_boxes(&Approvals::default());
    assert!(!boxes.repo);
    assert!(!no_commit.can_tick(&boxes, &Tick::Repo));
}

#[test]
fn a_look_alike_host_is_never_yours() {
    let bytes = serde_json::json!({
        "schemaVersion": 1,
        "name": "Mine",
        "dotfiles": {"repo": "https://github.com.evil.example/example-you/dotfiles", "links": {"~/.zshrc": "zshrc"}}
    })
    .to_string();
    let review = Review::build(
        &fetched(bytes.as_bytes(), "example-you"),
        Some("example-you"),
        &own_facts(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    assert!(!review.starting_boxes(&Approvals::default()).repo);
    assert!(review.signals.iter().any(|s| s.code == "host-lookalike"));
}

#[test]
fn a_script_that_cannot_be_shown_cannot_be_approved() {
    let unshown = review(Case {
        bytes: STRANGER,
        owner: "example-stranger",
        signed_in: Some("example-you"),
        facts: DotfilesFacts {
            script_text: None,
            ..facts()
        },
    });
    let mut boxes = unshown.starting_boxes(&Approvals::default());
    unshown.toggle(&mut boxes, &Tick::Repo);
    assert!(!unshown.can_tick(&boxes, &Tick::Script));
    unshown.toggle(&mut boxes, &Tick::Script);
    assert!(!boxes.script);
    boxes.script = true;
    assert_eq!(unshown.run_contents(&boxes).script, None);
}

#[test]
fn approvals_are_tied_to_repo_commit_and_links() {
    let first = stranger();
    let mut boxes = first.starting_boxes(&Approvals::default());
    first.toggle(&mut boxes, &Tick::Repo);
    first.toggle(&mut boxes, &Tick::Script);
    let approvals = first.approvals_after_run(&boxes, &Approvals::default());
    let held = approvals.dotfiles.as_ref().unwrap();
    assert_eq!(held.repo, "github.com/example-stranger/dotfiles");
    assert_eq!(held.commit.as_str(), DOTS);
    assert_eq!(
        approvals.script,
        Some(ScriptApproval {
            repo: "github.com/example-stranger/dotfiles".to_string(),
            commit: CommitSha::parse(DOTS).unwrap(),
            path: "script/setup".to_string(),
            ran: false,
        })
    );
    // The same repo and commit is on again.
    let again = first.starting_boxes(&approvals);
    assert!(again.repo && again.script);

    // A newer commit is off, script included.
    let newer = review(Case {
        bytes: STRANGER,
        owner: "example-stranger",
        signed_in: Some("example-you"),
        facts: DotfilesFacts {
            commit: CommitSha::parse("b1b2c3d000000000000000000000000000000000"),
            ..facts()
        },
    });
    let boxes = newer.starting_boxes(&approvals);
    assert!(!boxes.repo && !boxes.script);

    // One more link at the same commit is off.
    let mut config: serde_json::Value = serde_json::from_slice(STRANGER).unwrap();
    config["dotfiles"]["links"]["~/.zshenv"] = "zsh/zshenv".into();
    let bytes = config.to_string();
    let more_links = Review::build(
        &fetched(bytes.as_bytes(), "example-stranger"),
        Some("example-you"),
        &facts(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    assert!(!more_links.starting_boxes(&approvals).repo);

    // A different repo in the file is off.
    let mut config: serde_json::Value = serde_json::from_slice(STRANGER).unwrap();
    config["dotfiles"]["repo"] = "example-stranger/other".into();
    let bytes = config.to_string();
    let other = Review::build(
        &fetched(bytes.as_bytes(), "example-stranger"),
        Some("example-you"),
        &facts(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    assert!(!other.starting_boxes(&approvals).repo);
}

#[test]
fn turning_a_box_off_at_the_next_run_voids_the_approval() {
    let review = stranger();
    let mut boxes = review.starting_boxes(&Approvals::default());
    review.toggle(&mut boxes, &Tick::Repo);
    let approvals = review.approvals_after_run(&boxes, &Approvals::default());
    assert!(approvals.dotfiles.is_some());
    let off = review.starting_boxes(&Approvals::default());
    let after = review.approvals_after_run(&off, &approvals);
    assert_eq!(after.dotfiles, None);
}

#[test]
fn a_refused_link_is_shown_and_never_applied_and_a_note_explains_but_does_not_refuse() {
    let review = hostile();
    let dotfiles = review.dotfiles.as_ref().unwrap();
    let row = |target: &str| dotfiles.links.iter().find(|l| l.target == target).unwrap();
    assert!(row("~/Documents").refused.is_some());
    assert!(
        row("~/.zshenv").refused.is_some(),
        "both sides of the clash"
    );
    assert!(
        row("~/.ZSHENV").refused.is_some(),
        "both sides of the clash"
    );
    assert!(row("~/Developer/dotfiles/hooks").refused.is_some());
    let sudo = row("~/.local/bin/sudo");
    assert!(sudo.refused.is_none());
    assert!(sudo.note.is_some());
    let lookalike = dotfiles
        .links
        .iter()
        .find(|l| l.target.starts_with("~/.zshr"))
        .unwrap();
    assert!(lookalike.lookalike);
    assert_eq!(lookalike.target_escaped, "~/.zshr\\u0441");

    let mut boxes = review.starting_boxes(&Approvals::default());
    review.toggle(&mut boxes, &Tick::Repo);
    let run = review.run_contents(&boxes);
    let links = &run.config.dotfiles.as_ref().unwrap().links;
    assert!(!links.contains_key("~/Documents"));
    assert!(!links.contains_key("~/.zshenv"));
    assert!(links.contains_key("~/.local/bin/sudo"));
    assert_eq!(run.linked.len(), links.len());
}

#[test]
fn the_name_is_data_and_mixed_alphabets_are_written_out() {
    let review = hostile();
    assert!(review.name_has_lookalikes);
    assert!(
        review.name_escaped.contains("\\u0410"),
        "{}",
        review.name_escaped
    );
    assert!(review.name_escaped.is_ascii());
}

#[test]
fn a_setting_writing_through_a_link_only_skips_when_the_repo_is_on() {
    let review = stranger();
    let git = review
        .settings
        .iter()
        .find(|s| s.id == "git.pull-rebase")
        .expect("still reviewed");
    assert!(git.if_repo_on.is_some());
    let mut boxes = review.starting_boxes(&Approvals::default());
    assert!(
        review
            .run_contents(&boxes)
            .config
            .settings
            .contains_key("git.pull-rebase"),
        "the repo is off, so nothing is linked"
    );
    review.toggle(&mut boxes, &Tick::Repo);
    let run = review.run_contents(&boxes);
    let plan = macflayer_core::plan::Plan::new(&run.config, &stub());
    assert!(
        !plan.entries.iter().any(|e| e.id == "git.pull-rebase"),
        "linked ~/.gitconfig: the write never goes through into the repo"
    );
}

#[test]
fn a_long_official_list_starts_off_even_on_an_ordinary_config() {
    let mut value: serde_json::Value = serde_json::from_slice(HOSTILE).unwrap();
    value["name"] = "Ordinary".into();
    value.as_object_mut().unwrap().remove("dotfiles");
    value["packages"]["casks"] = serde_json::json!(["firefox"]);
    let names = value["packages"]["formulae"].as_array_mut().unwrap();
    names.retain(|n| !n.as_str().unwrap().contains('/'));
    let bytes = value.to_string();
    let review = Review::build(
        &fetched(bytes.as_bytes(), "example-dev"),
        Some("example-you"),
        &DotfilesFacts::default(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    let boxes = review.starting_boxes(&Approvals::default());
    assert!(review.big_formulae);
    assert!(!boxes.formulae);
    assert!(boxes.casks);
}

#[test]
fn nothing_in_a_config_can_set_a_box() {
    // Every shape of self-approval fails the closed schema, so it never gets this far.
    for field in [
        r#""approved":true"#,
        r#""trusted":true"#,
        r#""approvals":{"taps":["evil/tap"]}"#,
        r#""commit":"0123456789abcdef0123456789abcdef01234567""#,
    ] {
        let text = format!(r#"{{"schemaVersion":1,"name":"x",{field}}}"#);
        assert!(Config::parse(text.as_bytes()).is_err(), "{field}");
        let inside =
            format!(r#"{{"schemaVersion":1,"name":"x","dotfiles":{{"repo":"a/b",{field}}}}}"#);
        assert!(Config::parse(inside.as_bytes()).is_err(), "{field}");
    }
}

#[test]
fn start_run_is_what_writes_approvals_and_a_clean_run_approves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileApprovals::new(dir.path().join("record.json"));
    let review = clean();
    let boxes = review.starting_boxes(&Approvals::default());
    review.record_start(&boxes, &store).unwrap();
    assert_eq!(store.read().unwrap(), Approvals::default());

    let review = stranger();
    let mut boxes = review.starting_boxes(&Approvals::default());
    review.toggle(&mut boxes, &Tick::Repo);
    let run = review.record_start(&boxes, &store).unwrap();
    assert_eq!(
        store.read().unwrap().dotfiles.unwrap().commit.as_str(),
        DOTS
    );
    assert_eq!(run.clone.unwrap().1.as_str(), DOTS);
}

#[test]
fn an_address_that_looks_like_a_token_is_never_approvable() {
    let bytes = serde_json::json!({
        "schemaVersion": 1,
        "name": "x",
        "dotfiles": {"repo": "someone/ghp_abcdefghijklmnopqrstuvwxyz", "links": {"~/.zshrc": "zshrc"}}
    })
    .to_string();
    let review = Review::build(
        &fetched(bytes.as_bytes(), "someone"),
        Some("example-you"),
        &facts(),
        &stub(),
        DEFAULT_CLONE_LOCATION,
    );
    let boxes = review.starting_boxes(&Approvals::default());
    assert!(!review.can_tick(&boxes, &Tick::Repo));
    assert!(review.run_contents(&boxes).clone.is_none());
}
