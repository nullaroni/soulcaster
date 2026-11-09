mod common;

use common::{example, stub};
use macflayer_core::catalog::Catalog;
use macflayer_core::config::{Config, SettingValue};
use macflayer_core::lifecycle::{SkipReason, Stage};
use macflayer_core::plan::{NeedKind, Plan, PlanEntry, RunGroup, Why};
use macflayer_core::recipe::Probe;
use serde_json::json;

fn config(value: serde_json::Value) -> Config {
    let mut value = value;
    value["schemaVersion"] = json!(1);
    value["name"] = json!("probe");
    Config::parse(value.to_string().as_bytes()).unwrap()
}

fn shipped(value: serde_json::Value) -> Plan {
    Plan::new(&config(value), Catalog::shipped())
}

fn ids(plan: &Plan) -> Vec<&str> {
    plan.entries.iter().map(|e| e.id.as_str()).collect()
}

fn entry<'a>(plan: &'a Plan, id: &str) -> &'a PlanEntry {
    plan.entries
        .iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("{id} isn't in {:?}", ids(plan)))
}

fn position(plan: &Plan, id: &str) -> usize {
    plan.entries
        .iter()
        .position(|e| e.id == id)
        .unwrap_or_else(|| panic!("{id} isn't in {:?}", ids(plan)))
}

fn from_example(name: &str) -> Plan {
    Plan::new(&Config::parse(&example(name)).unwrap(), &stub())
}

fn skip_reason(plan: &Plan, raw: &str) -> SkipReason {
    plan.skipped
        .iter()
        .find(|s| s.raw == raw)
        .unwrap_or_else(|| panic!("{raw} isn't skipped: {:?}", plan.skipped))
        .reason
}

#[test]
fn an_empty_config_still_runs_the_base_items_in_the_lifecycle_order() {
    let plan = shipped(json!({}));
    assert_eq!(ids(&plan), ["command-line-tools", "homebrew", "gh"]);
    assert_eq!(entry(&plan, "command-line-tools").stage, Stage::Admin);
    assert_eq!(entry(&plan, "homebrew").stage, Stage::Admin);
    assert_eq!(
        entry(&plan, "gh").stage,
        Stage::User,
        "the bootstrap gh comes first; its Homebrew hand-off is user work"
    );
    assert!(plan.entries.iter().all(|e| e.why == Why::Base));
}

#[test]
fn base_items_come_first_within_each_stage_and_admin_comes_before_user() {
    let plan = shipped(json!({"items": ["tailscale", "ripgrep", "firefox"]}));
    assert_eq!(
        ids(&plan),
        [
            "command-line-tools",
            "homebrew",
            "tailscale",
            "gh",
            "ripgrep",
            "firefox"
        ]
    );
}

#[test]
fn the_user_stage_follows_the_lifecycle_order() {
    let plan = shipped(json!({
        "items": ["aerospace", "kotlin-lsp", "git-identity", "ripgrep"],
        "settings": {"dock.autohide": true, "git.pull-rebase": true}
    }));
    let order: Vec<usize> = [
        "gh",
        "ripgrep",
        "kotlin-lsp",
        "dock.autohide",
        "aerospace",
        "git.pull-rebase",
    ]
    .iter()
    .map(|id| position(&plan, id))
    .collect();
    assert!(order.windows(2).all(|p| p[0] < p[1]), "{:?}", ids(&plan));
    assert_eq!(entry(&plan, "git-identity").group, RunGroup::GitValues);
    assert_eq!(entry(&plan, "git.pull-rebase").group, RunGroup::GitValues);
    assert_eq!(entry(&plan, "aerospace").group, RunGroup::ThirdParty);
}

#[test]
fn third_party_tap_items_never_run_in_the_admin_stage_even_when_the_listing_says_admin() {
    let plan = shipped(json!({"items": ["yabai"]}));
    assert_eq!(entry(&plan, "yabai").stage, Stage::User);
    assert_eq!(plan.taps["asmvik/formulae"], ["yabai"]);
}

#[test]
fn requirements_are_pulled_in_with_why_and_run_first() {
    let plan = shipped(json!({"items": ["android-sdk"]}));
    assert_eq!(entry(&plan, "android-sdk").why, Why::Selected);
    assert_eq!(
        entry(&plan, "openjdk").why,
        Why::PulledIn {
            by: "android-sdk".to_string()
        }
    );
    assert!(position(&plan, "openjdk") < position(&plan, "android-sdk"));
    let summary = plan.summary();
    let openjdk = summary.iter().find(|i| i.id == "openjdk").unwrap();
    assert_eq!(openjdk.pulled_in_by.as_deref(), Some("android-sdk"));
}

#[test]
fn a_setting_pulls_in_what_it_requires() {
    let plan = shipped(json!({"settings": {"dock.apps": ["/Applications/Safari.app"]}}));
    assert_eq!(
        entry(&plan, "dockutil").why,
        Why::PulledIn {
            by: "dock.apps".to_string()
        }
    );
    assert_eq!(
        entry(&plan, "dock.apps").value,
        Some(SettingValue::List(vec![
            "/Applications/Safari.app".to_string()
        ]))
    );
}

#[test]
fn each_item_lists_what_it_needs_from_a_person() {
    let plan = shipped(
        json!({"items": ["xcode", "android-sdk", "aerospace", "tailscale", "ripgrep", "git-identity"]}),
    );
    let kinds = |id: &str| {
        entry(&plan, id)
            .needs
            .iter()
            .map(|n| n.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds("xcode"),
        [
            NeedKind::AppleAccount,
            NeedKind::Person,
            NeedKind::License,
            NeedKind::AdminPassword
        ]
    );
    assert_eq!(kinds("android-sdk"), [NeedKind::License]);
    assert_eq!(
        kinds("aerospace"),
        [NeedKind::TapApproval, NeedKind::Person]
    );
    assert_eq!(kinds("tailscale"), [NeedKind::AdminPassword]);
    assert_eq!(kinds("ripgrep"), []);
    assert_eq!(
        kinds("git-identity"),
        [NeedKind::MachineValue, NeedKind::MachineValue]
    );
    let xcode = entry(&plan, "xcode");
    let person = xcode
        .needs
        .iter()
        .find(|n| n.kind == NeedKind::Person)
        .unwrap();
    assert!(
        person
            .link
            .as_deref()
            .unwrap()
            .starts_with("macappstore://")
    );
}

#[test]
fn xcode_holds_nothing_open_while_it_waits_for_a_person() {
    let plan = shipped(json!({"items": ["xcode"]}));
    assert_eq!(entry(&plan, "xcode").stage, Stage::User);
}

#[test]
fn an_unknown_id_skips_only_that_entry_and_suggests_a_fix() {
    let plan = shipped(json!({"items": ["ghosty", "ripgrep"]}));
    assert_eq!(skip_reason(&plan, "ghosty"), SkipReason::UnknownId);
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("Did you mean ghostty?")),
        "{:?}",
        plan.notes
    );
    assert!(ids(&plan).contains(&"ripgrep"));
    assert!(plan.has_unknown_ids());
}

#[test]
fn old_ids_resolve_retired_ones_skip_and_deprecated_ones_still_install() {
    let plan = from_example("lifecycle.json");
    assert!(ids(&plan).contains(&"ghostty"));
    assert_eq!(
        skip_reason(&plan, "example-retired-tool"),
        SkipReason::Retired
    );
    assert!(ids(&plan).contains(&"example-deprecated-tool"));
    assert_eq!(
        skip_reason(&plan, "example-from-a-newer-catalog"),
        SkipReason::UnknownId
    );
    assert_eq!(
        skip_reason(&plan, "example.retired-setting"),
        SkipReason::Retired
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("renamed example-old-name -> ghostty")),
        "{:?}",
        plan.notes
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("deprecated example-deprecated-tool")),
        "{:?}",
        plan.notes
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("renamed example.old-setting -> dock.show-recents")),
        "{:?}",
        plan.notes
    );
}

#[test]
fn a_value_the_catalog_refuses_skips_only_that_entry() {
    let plan = from_example("lifecycle.json");
    assert_eq!(skip_reason(&plan, "dock.autohide"), SkipReason::BadValue);
    let bad = Plan::new(
        &config(
            json!({"settings": {"keyboard.key-repeat": -5, "keyboard.initial-key-repeat": 25}}),
        ),
        &stub(),
    );
    assert_eq!(
        skip_reason(&bad, "keyboard.key-repeat"),
        SkipReason::BadValue
    );
    assert!(ids(&bad).contains(&"keyboard.initial-key-repeat"));
}

#[test]
fn a_broken_setting_is_unavailable_here_and_not_written() {
    let plan = from_example("owner-personal.json");
    let reason = skip_reason(&plan, "menu-bar.extras");
    assert_eq!(reason, SkipReason::BrokenHere);
    assert!(reason.is_unavailable_here());
    assert!(!ids(&plan).contains(&"menu-bar.extras"));
}

#[test]
fn base_entries_in_a_config_are_ignored_with_a_note() {
    let plan = from_example("lint/base-items-listed.json");
    let notes = plan.notes.join("\n");
    assert_eq!(
        notes.matches("gh is always installed").count(),
        2,
        "{notes}"
    );
    assert!(notes.contains("homebrew is always installed"), "{notes}");
    assert!(notes.contains("evil/tap/gh has the same name"), "{notes}");
    assert_eq!(ids(&plan).iter().filter(|id| **id == "gh").count(), 1);
    assert!(ids(&plan).contains(&"formula:evil/tap/gh"));
}

#[test]
fn a_conflict_installs_neither_side_until_a_person_picks_one() {
    let plan = from_example("lint/item-package-conflict.json");
    let notes = plan.notes.join("\n");
    assert!(notes.contains("conflict: opencode"), "{notes}");
    assert!(notes.contains("conflict: tailscale"), "{notes}");
    assert!(!ids(&plan).contains(&"tailscale"));
    assert!(!ids(&plan).contains(&"formula:tailscale"));
    assert_eq!(plan.conflicts.len(), 2);
}

#[test]
fn two_items_that_conflict_in_homebrew_also_wait_for_a_person() {
    let plan = shipped(json!({"items": ["rust", "rustup", "ripgrep"]}));
    assert_eq!(plan.conflicts.len(), 1);
    assert!(!ids(&plan).contains(&"rust"));
    assert!(!ids(&plan).contains(&"rustup"));
    assert!(ids(&plan).contains(&"ripgrep"));
}

#[test]
fn a_covered_package_installs_once_through_the_item() {
    let plan =
        shipped(json!({"items": ["ripgrep"], "packages": {"formulae": ["ripgrep", "neovim"]}}));
    assert_eq!(
        ids(&plan)
            .iter()
            .filter(|id| id.contains("ripgrep"))
            .count(),
        1
    );
    assert!(
        ids(&plan).contains(&"neovim"),
        "neovim is a Catalog Item, so it becomes the Item"
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("covered by Item")),
        "{:?}",
        plan.notes
    );
}

#[test]
fn a_package_the_catalog_doesnt_cover_gets_a_generated_recipe() {
    let plan = shipped(
        json!({"packages": {"formulae": ["mobile-dev-inc/tap/maestro-x", "tree"], "casks": ["some-cask"]}}),
    );
    let tree = entry(&plan, "formula:tree");
    assert_eq!(tree.recipe.requires(), ["homebrew"]);
    assert_eq!(tree.group, RunGroup::Official);
    assert_eq!(entry(&plan, "cask:some-cask").stage, Stage::User);
    let tapped = entry(&plan, "formula:mobile-dev-inc/tap/maestro-x");
    assert_eq!(tapped.group, RunGroup::ThirdParty);
    assert_eq!(tapped.needs[0].kind, NeedKind::TapApproval);
    assert_eq!(
        plan.taps["mobile-dev-inc/tap"],
        ["formula:mobile-dev-inc/tap/maestro-x"]
    );
}

#[test]
fn two_spellings_of_one_setting_skip_both_unless_they_agree() {
    let differ = from_example("lint/rename-collision.json");
    assert!(
        differ
            .notes
            .iter()
            .any(|n| n.contains("are one setting with different values")),
        "{:?}",
        differ.notes
    );
    assert!(!ids(&differ).contains(&"dock.show-recents"));

    let same = Plan::new(
        &config(json!({"settings": {"example.old-setting": true, "dock.show-recents": true}})),
        &stub(),
    );
    assert_eq!(
        ids(&same)
            .iter()
            .filter(|id| **id == "dock.show-recents")
            .count(),
        1
    );
    assert!(
        same.notes.iter().any(|n| n.contains("same value")),
        "{:?}",
        same.notes
    );
}

#[test]
fn a_lint_finding_skips_that_entry_and_the_rest_applies() {
    let plan = from_example("lint/write-through-link.json");
    assert_eq!(
        skip_reason(&plan, "git-identity"),
        SkipReason::WritesThroughLink
    );
    assert!(!plan.findings.is_empty());
    let secret = shipped(
        json!({"settings": {"example.note": "ghp_abcdefghijklmnop", "dock.autohide": true}}),
    );
    assert_eq!(skip_reason(&secret, "example.note"), SkipReason::Lint);
    assert!(ids(&secret).contains(&"dock.autohide"));
}

#[test]
fn a_refused_link_is_skipped_and_the_other_links_stay() {
    let plan = from_example("lint/link-protected-folder.json");
    let dotfiles = plan.dotfiles.as_ref().unwrap();
    assert!(
        !dotfiles.links.contains_key("~/Library"),
        "{:?}",
        dotfiles.links
    );
    assert_eq!(dotfiles.refused_links.len(), 2);
}

#[test]
fn a_plan_keeps_neither_side_of_two_links_to_one_location() {
    let plan = shipped(json!({"dotfiles": {
        "repo": "a/b",
        "links": {"~/.zshrc": "one", "~/.ZSHRC": "two", "~/.fine": "three"}
    }}));
    let dotfiles = plan.dotfiles.as_ref().unwrap();
    assert_eq!(dotfiles.links.keys().collect::<Vec<_>>(), ["~/.fine"]);
    assert_eq!(dotfiles.refused_links.len(), 2);
}

#[test]
fn approvals_are_worked_out_from_the_file_and_the_catalog_alone() {
    let plan = from_example("owner-work.json");
    let taps: Vec<&String> = plan.taps.keys().collect();
    assert_eq!(taps, ["mobile-dev-inc/tap", "nikitabobko/tap"]);
    assert_eq!(
        plan.dotfiles.as_ref().unwrap().repo,
        "your-github-username/dotfiles"
    );

    let developer = from_example("developer.json");
    assert_eq!(
        developer
            .dotfiles
            .as_ref()
            .unwrap()
            .install_script
            .as_deref(),
        Some("script/setup")
    );
}

#[test]
fn stages_can_be_reassigned_after_the_machine_is_checked_and_the_order_follows() {
    let mut plan = shipped(json!({"items": ["xcode", "ripgrep"]}));
    assert_eq!(entry(&plan, "xcode").stage, Stage::User);
    // Xcode has appeared and Homebrew is already here, so a later Run finds different work.
    let appeared = |p: &Probe| matches!(p, Probe::AppBundle { .. });
    plan.assign_stages(|e| {
        if e.id == "homebrew" {
            Stage::User
        } else {
            e.recipe.stage(appeared)
        }
    });
    assert_eq!(entry(&plan, "xcode").stage, Stage::Admin);
    assert_eq!(entry(&plan, "homebrew").stage, Stage::User);
    assert!(position(&plan, "xcode") < position(&plan, "gh"));
    assert!(
        position(&plan, "homebrew") < position(&plan, "ripgrep"),
        "requirements still run first"
    );
}

#[test]
fn the_plan_says_when_the_password_is_needed() {
    let mut plan = shipped(json!({"items": ["ripgrep"]}));
    assert!(plan.needs_admin());
    plan.assign_stages(|_| Stage::User);
    assert!(!plan.needs_admin());
}
