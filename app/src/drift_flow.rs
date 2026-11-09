//! What Drift does for real, apart from the window: which Config this Mac follows, a look at the
//! Machine against it, and carrying out what a person decided. Everything it starts goes through
//! the runners it's given, so tests use fakes and a temp `HOME`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::CommandRunner;
use soulcaster_core::config::Config;
use soulcaster_core::drift::{
    Choice, Report, Reviewed, Row, changed_again, changed_elsewhere, not_adoptable, value_in,
};
use soulcaster_core::plan::Plan;
use soulcaster_engine::{
    Clock, MachineRecord, Removal, RemovalConfirmed, check_drift, keep_here, leave_here,
    record_saved, remove_from_mac, set_followed, set_waiting_saves, waiting_saves,
};
use soulcaster_repo::adopt::{AdoptOutcome, AdoptRequest, Followed, adopt, read_followed};
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveError};
use soulcaster_repo::tools::Tools;

pub struct Deps<'a> {
    pub mac: &'a dyn CommandRunner,
    pub repo: &'a dyn CommandRunner,
    pub tools: &'a Tools,
    pub home: &'a Path,
    pub catalog: &'a Catalog,
    pub clock: &'a dyn Clock,
}

/// The one Config this Mac follows, as it last read it. Kept in the Machine Record.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Following {
    config: String,
    ever_saved: bool,
    read_at: u64,
    /// The Config that saves waiting on this Mac were reviewed for, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    waiting_for: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// No Run or save has given this Mac a Config to compare with.
    NoConfig,
    Current,
    Offline {
        when: u64,
    },
    /// Apply and Adopt wait until the file can be read.
    Unreadable,
    /// Writes stop, and nothing is removed.
    Missing {
        path: String,
    },
}

impl Status {
    pub fn blocks_changes(&self) -> bool {
        matches!(
            self,
            Self::NoConfig | Self::Unreadable | Self::Missing { .. }
        )
    }
}

#[derive(Clone, Debug)]
pub struct Checked {
    pub report: Report,
    pub status: Status,
    pub waiting: usize,
}

struct Look {
    checked: Checked,
    config: Option<Config>,
    following: Option<Following>,
    plan: Option<Plan>,
}

fn following(home: &Path) -> Option<Following> {
    let record = MachineRecord::load(home).ok()?;
    serde_json::from_value(record.follows?).ok()
}

fn store(home: &Path, following: &Following) {
    if let Ok(value) = serde_json::to_value(following) {
        let _ = set_followed(home, value);
    }
}

/// Starts following a Config. Saving or running one that was already saved stays saved.
pub fn follow(home: &Path, config: &Config, saved: bool, now: u64) {
    let text = config.to_canonical_json();
    let before = following(home);
    let earlier_saved = before.as_ref().is_some_and(|earlier| earlier.ever_saved);
    let waiting_for = before
        .as_ref()
        .and_then(|earlier| earlier.waiting_for.clone());
    let same = before.as_ref().is_some_and(|earlier| {
        Config::parse(earlier.config.as_bytes()).is_ok_and(|held| held.name == config.name)
    });
    store(
        home,
        &Following {
            config: text,
            ever_saved: saved || (same && earlier_saved),
            read_at: now,
            waiting_for,
        },
    );
}

fn look(deps: &Deps<'_>) -> Look {
    let waiting = waiting_saves(deps.home).map_or(0, |saves| saves.len());
    let nothing = |status| Look {
        checked: Checked {
            report: Report::default(),
            status,
            waiting,
        },
        config: None,
        following: None,
        plan: None,
    };
    let Some(mut held) = following(deps.home) else {
        return nothing(Status::NoConfig);
    };
    let Ok(held_config) = Config::parse(held.config.as_bytes()) else {
        return nothing(Status::NoConfig);
    };
    let read = read_followed(
        deps.repo,
        deps.tools,
        CONFIG_REPO_NAME,
        &held_config.name,
        held.ever_saved,
    );
    let (config, status) = match read {
        Followed::Latest(latest) => {
            held.config = latest.to_canonical_json();
            held.read_at = deps.clock.now_ms();
            // It's on GitHub, so if it goes away that's a missing file, not a new one.
            held.ever_saved = true;
            store(deps.home, &held);
            (latest, Status::Current)
        }
        Followed::NeverSent => (held_config, Status::Current),
        Followed::Waiting(_) => (held_config, Status::Offline { when: held.read_at }),
        Followed::Unreadable => (held_config, Status::Unreadable),
        Followed::Missing(path) => (held_config, Status::Missing { path }),
    };
    let plan = Plan::new(&config, deps.catalog);
    let Ok(report) = check_drift(deps.mac, deps.home, deps.catalog, &plan, &config) else {
        return nothing(status);
    };
    Look {
        checked: Checked {
            report,
            status,
            waiting,
        },
        config: Some(config),
        following: Some(held),
        plan: Some(plan),
    }
}

pub fn check(deps: &Deps<'_>) -> Checked {
    look(deps).checked
}

#[derive(Debug, Default)]
pub struct Carried {
    pub notes: Vec<String>,
    /// Entries to bring in line through a Run, with the plan to do it from.
    pub apply: Option<(Plan, Vec<String>)>,
}

fn name_of(rows: &[Row], id: &str) -> String {
    rows.iter()
        .find(|row| row.id == id)
        .map_or_else(|| id.to_string(), |row| row.name.clone())
}

const BUSY: &str =
    "A Run is using this Mac's record, so that wasn't saved. Try again when it ends.";

/// Sends reviewed saves and says how it went. The last-applied record moves only for what GitHub
/// has, and what couldn't be sent waits on this Mac.
fn send(
    deps: &Deps<'_>,
    view: &Look,
    reviewed: &[Reviewed],
    rows: &[Row],
    still: &dyn Fn(&Reviewed) -> bool,
    notes: &mut Vec<String>,
) {
    let (Some(config), Some(following)) = (&view.config, &view.following) else {
        return;
    };
    let name = |id: &String| name_of(rows, id);
    let request = AdoptRequest {
        repo_name: CONFIG_REPO_NAME,
        config_name: &config.name,
        held: config,
        reviewed,
        still_true: still,
        ever_saved: following.ever_saved,
    };
    let mut saved: Vec<String> = Vec::new();
    let mut public = false;
    match adopt(deps.repo, deps.tools, deps.catalog, &request) {
        Ok(AdoptOutcome::Saved { public: open }) => {
            saved = reviewed.iter().map(|entry| entry.id.clone()).collect();
            public = open;
        }
        Ok(AdoptOutcome::Partly {
            public: open,
            saved: went,
            changed_elsewhere: moved,
            changed_again: again,
        }) => {
            saved = went;
            public = open;
            notes.extend(moved.iter().map(|id| changed_elsewhere(&name(id))));
            notes.extend(again.iter().map(|id| changed_again(&name(id))));
        }
        Ok(AdoptOutcome::Waiting { why, .. }) => {
            let mut waiting = waiting_saves(deps.home).unwrap_or_default();
            // One Config's saves wait at a time, or sending would put them all in the wrong file.
            if !waiting.is_empty()
                && let Some(owner) = following.waiting_for.as_deref()
                && owner != config.name
            {
                notes.push(switching(owner));
                return;
            }
            notes.push(why.words().to_string());
            waiting.retain(|earlier| !reviewed.iter().any(|entry| entry.id == earlier.id));
            waiting.extend(reviewed.iter().cloned());
            if set_waiting_saves(deps.home, &waiting).is_err() {
                notes.push(BUSY.to_string());
            }
            let mut marked = following.clone();
            marked.waiting_for = Some(config.name.clone());
            store(deps.home, &marked);
            return;
        }
        Ok(AdoptOutcome::ChangedElsewhere { ids }) => {
            notes.extend(ids.iter().map(|id| changed_elsewhere(&name(id))));
        }
        Ok(AdoptOutcome::ChangedAgain { ids }) => {
            notes.extend(ids.iter().map(|id| changed_again(&name(id))));
        }
        Ok(AdoptOutcome::Unreadable) => notes.push(unreadable()),
        Ok(AdoptOutcome::Missing { path }) => notes.push(missing(&path)),
        Err(SaveError::Blocked(findings)) => {
            for finding in findings {
                notes.push(not_adoptable(&finding.at, "it has to stay on this Mac"));
            }
        }
        Err(error) => notes.push(error.to_string()),
    }
    if saved.is_empty() {
        return;
    }
    let went: Vec<Reviewed> = reviewed
        .iter()
        .filter(|entry| saved.contains(&entry.id))
        .cloned()
        .collect();
    if record_saved(deps.home, &went, deps.catalog.version(), deps.clock).is_err() {
        notes.push(BUSY.to_string());
    }
    let mut now_saved = following.clone();
    now_saved.ever_saved = true;
    store(deps.home, &now_saved);
    if public {
        notes.push("This Machine Type is public. Anyone can read what you save.".to_string());
    }
}

fn switching(owner: &str) -> String {
    format!("You have changes waiting to send to {owner}. Send or discard them before you switch.")
}

pub fn unreadable() -> String {
    "Your Machine Type on GitHub can't be read: it isn't a file this version understands. Nothing on \
     this Mac has changed. Fix it on GitHub, or go back to the last version that worked."
        .to_string()
}

pub fn missing(path: &str) -> String {
    format!(
        "{path} isn't on GitHub any more. Nothing on this Mac has changed, and nothing will be removed."
    )
}

/// Carries out what the person chose, against a fresh look. A row that no longer says what they
/// saw is left for them to review again.
pub fn carry_out(deps: &Deps<'_>, decisions: &[(Row, Choice)]) -> Carried {
    let view = look(deps);
    let mut carried = Carried::default();
    if view.checked.status.blocks_changes() {
        carried.notes.push(match &view.checked.status {
            Status::Missing { path } => missing(path),
            _ => unreadable(),
        });
        return carried;
    }
    let fresh: BTreeMap<&str, &Row> = view
        .checked
        .report
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    let mut saves = Vec::new();
    let mut removals = Vec::new();
    let mut apply = Vec::new();
    for (seen, choice) in decisions {
        let unchanged = fresh.get(seen.id.as_str()).is_some_and(|now| {
            now.kind == seen.kind && now.on_mac == seen.on_mac && now.in_config == seen.in_config
        });
        let offered = fresh
            .get(seen.id.as_str())
            .is_some_and(|now| now.choices().contains(choice));
        if !unchanged || !offered {
            carried.notes.push(changed_again(&seen.name));
            continue;
        }
        match choice {
            Choice::Adopt | Choice::AddBack => saves.push(seen.reviewed()),
            Choice::Apply => apply.push(seen.id.clone()),
            Choice::Remove => removals.push(seen.id.clone()),
            Choice::NotHere => {
                if leave_here(deps.home, &seen.id).is_err() {
                    carried.notes.push(BUSY.to_string());
                }
            }
            Choice::Keep | Choice::KeepHere => {
                if keep_here(deps.home, &seen.id).is_err() {
                    carried.notes.push(BUSY.to_string());
                }
            }
        }
    }
    let rows: Vec<Row> = decisions.iter().map(|(row, _)| row.clone()).collect();
    if !saves.is_empty() {
        let still = |entry: &Reviewed| {
            fresh
                .get(entry.id.as_str())
                .is_some_and(|now| now.on_mac == entry.write)
        };
        send(deps, &view, &saves, &rows, &still, &mut carried.notes);
    }
    if !removals.is_empty()
        && let Some(plan) = &view.plan
    {
        match remove_from_mac(
            deps.mac,
            deps.home,
            deps.catalog,
            plan,
            &removals,
            RemovalConfirmed::by_the_person(),
        ) {
            Ok(out) => {
                for (id, result) in out {
                    let name = name_of(&rows, &id);
                    match result {
                        Removal::Removed => {}
                        Removal::Refused => carried.notes.push(format!(
                            "{name} wasn't removed. Soulcaster can't safely take it off this Mac."
                        )),
                        Removal::Failed => carried.notes.push(format!(
                            "Homebrew couldn't remove {name}. It's still on this Mac."
                        )),
                    }
                }
            }
            Err(_) => carried.notes.push(BUSY.to_string()),
        }
    }
    if !apply.is_empty()
        && let Some(plan) = view.plan
    {
        carried.apply = Some((plan, apply));
    }
    carried
}

/// "Send to GitHub": tries the waiting saves again on top of whatever GitHub has now.
pub fn send_waiting(deps: &Deps<'_>) -> Vec<String> {
    let mut notes = Vec::new();
    let view = look(deps);
    let Ok(waiting) = waiting_saves(deps.home) else {
        return notes;
    };
    if waiting.is_empty() || view.checked.status.blocks_changes() {
        return notes;
    }
    // A save reviewed for one Config never lands in another.
    if let (Some(following), Some(config)) = (&view.following, &view.config)
        && let Some(owner) = following.waiting_for.as_deref()
        && owner != config.name
    {
        notes.push(switching(owner));
        return notes;
    }
    let rows = view.checked.report.rows.clone();
    let held = view.config.clone();
    // No row means this Mac and the Config agree, which is only a success if they agree on this.
    let still = |entry: &Reviewed| match rows.iter().find(|row| row.id == entry.id) {
        Some(now) => now.on_mac == entry.write,
        None => held
            .as_ref()
            .is_some_and(|config| value_in(config, &entry.key) == entry.write),
    };
    send(deps, &view, &waiting, &rows, &still, &mut notes);
    notes
}

/// "Discard, and keep this Mac as it is." The rows come back.
pub fn discard_waiting(home: &Path) -> Vec<String> {
    match set_waiting_saves(home, &[]) {
        Ok(()) => {
            if let Some(mut held) = following(home) {
                held.waiting_for = None;
                store(home, &held);
            }
            Vec::new()
        }
        Err(_) => vec![BUSY.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use soulcaster_core::command::{FakeRunner, Reply};
    use soulcaster_core::drift::RowKind;
    use soulcaster_repo::fake::{FakeGitHub, RepoShape};

    use super::*;

    const SHA: &str = "3f9a1c2000000000000000000000000000000000";
    const PUSHED: &str = "bbbbbbb000000000000000000000000000000000";
    const PATH: &str = "configs/my-mac.json";
    const TEXT: &str =
        r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":2}}"#;

    struct Fixed;

    impl Clock for Fixed {
        fn now_ms(&self) -> u64 {
            5
        }

        fn wait(&self, _: Duration) {}
    }

    struct World {
        dir: tempfile::TempDir,
        tools: Tools,
        mac: FakeRunner,
        gh: FakeGitHub,
    }

    fn line(command: &soulcaster_core::command::Command) -> String {
        let mut out = vec![command.program.to_string_lossy().into_owned()];
        out.extend(command.args.iter().cloned());
        out.join(" ")
    }

    impl World {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let tools = Tools::fixed(dir.path().to_path_buf());
            let gh = FakeGitHub::new();
            gh.sign_in("example-you", 42);
            gh.on_git("rev-parse", Reply::ok(format!("{PUSHED}\n")));
            gh.add_repo(
                "example-you",
                CONFIG_REPO_NAME,
                RepoShape::private("main", SHA),
            );
            Self {
                dir,
                tools,
                mac: FakeRunner::new(),
                gh,
            }
        }

        fn deps(&self) -> Deps<'_> {
            Deps {
                mac: &self.mac,
                repo: &self.gh,
                tools: &self.tools,
                home: self.dir.path(),
                catalog: Catalog::shipped(),
                clock: &Fixed,
            }
        }

        fn mac_says(&self, needle: &'static str, reply: Reply) {
            self.mac.on(move |c| line(c).contains(needle), [reply]);
        }

        fn nothing_else_here(&self) {
            self.mac.on(|_| true, [Reply::fails(1, "")]);
        }

        fn on_github(&self, text: &str) {
            let config = Config::parse(text.as_bytes()).unwrap();
            let canonical = config.to_canonical_json();
            self.gh.add_file(
                "example-you",
                CONFIG_REPO_NAME,
                SHA,
                PATH,
                canonical.as_bytes(),
            );
            self.gh.set_git_files("checkout", vec![(PATH, &canonical)]);
        }

        fn follow(&self, text: &str, saved: bool) {
            follow(
                self.dir.path(),
                &Config::parse(text.as_bytes()).unwrap(),
                saved,
                1,
            );
        }

        fn remember(&self, id: &str, value: serde_json::Value) {
            let mut record = MachineRecord::load(self.dir.path()).unwrap();
            record.last_applied.insert(
                id.to_string(),
                soulcaster_engine::LastApplied {
                    value,
                    applied_at: 1,
                    run_id: "run".into(),
                    catalog_version: "1".into(),
                    macos_version: None,
                    assurance: soulcaster_core::lifecycle::Assurance::Stored,
                    prior_value: None,
                },
            );
            record.save(self.dir.path()).unwrap();
        }

        fn record(&self) -> MachineRecord {
            MachineRecord::load(self.dir.path()).unwrap()
        }

        fn key_repeat_moved(&self) -> Checked {
            self.follow(TEXT, true);
            self.on_github(TEXT);
            self.remember("keyboard.key-repeat", json!(2));
            self.mac_says("read NSGlobalDomain KeyRepeat", Reply::ok("6\n"));
            self.nothing_else_here();
            check(&self.deps())
        }

        fn pushes(&self) -> usize {
            self.gh
                .calls()
                .iter()
                .filter(|c| c.program.ends_with("git") && c.args.iter().any(|a| a == "push"))
                .count()
        }
    }

    #[test]
    fn with_no_config_there_is_nothing_to_compare_and_nothing_runs() {
        let world = World::new();
        let checked = check(&world.deps());
        assert_eq!(checked.status, Status::NoConfig);
        assert!(world.mac.calls().is_empty());
    }

    #[test]
    fn a_check_finds_the_row_and_sends_nothing() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        assert_eq!(checked.status, Status::Current);
        assert_eq!(checked.report.rows[0].kind, RowKind::ChangedHere);
        assert_eq!(world.pushes(), 0);
    }

    #[test]
    fn offline_it_compares_with_the_config_it_last_read() {
        let world = World::new();
        world.follow(TEXT, true);
        world.gh.go_offline();
        world.mac_says("read NSGlobalDomain KeyRepeat", Reply::ok("6\n"));
        world.nothing_else_here();
        let checked = check(&world.deps());
        assert_eq!(checked.status, Status::Offline { when: 1 });
    }

    #[test]
    fn a_file_that_cannot_be_read_stops_changes() {
        let world = World::new();
        world.follow(TEXT, true);
        world.gh.add_file(
            "example-you",
            CONFIG_REPO_NAME,
            SHA,
            PATH,
            br#"{"schemaVersion":99,"name":"My Mac"}"#,
        );
        world.nothing_else_here();
        let checked = check(&world.deps());
        assert_eq!(checked.status, Status::Unreadable);
        assert!(checked.status.blocks_changes());
    }

    #[test]
    fn a_save_moves_last_applied_only_once_github_has_it() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        let row = checked.report.rows[0].clone();
        let carried = carry_out(&world.deps(), &[(row, Choice::Adopt)]);
        assert_eq!(carried.notes, Vec::<String>::new());
        assert_eq!(world.pushes(), 1);
        assert_eq!(
            world.record().last_applied["keyboard.key-repeat"].value,
            json!(6)
        );
    }

    #[test]
    fn a_save_that_cannot_be_sent_waits_and_leaves_last_applied_alone() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        let row = checked.report.rows[0].clone();
        world.gh.go_offline();
        let carried = carry_out(&world.deps(), &[(row, Choice::Adopt)]);
        assert_eq!(world.pushes(), 0);
        assert!(
            carried.notes[0].contains("not on GitHub yet"),
            "{:?}",
            carried.notes
        );
        assert_eq!(
            world.record().last_applied["keyboard.key-repeat"].value,
            json!(2)
        );
        assert_eq!(world.record().waiting_saves.len(), 1);
        assert_eq!(discard_waiting(world.dir.path()), Vec::<String>::new());
        assert!(world.record().waiting_saves.is_empty());
    }

    #[test]
    fn a_row_that_changed_since_the_person_looked_is_not_carried_out() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        let mut stale = checked.report.rows[0].clone();
        stale.on_mac = Some(json!(99));
        let carried = carry_out(&world.deps(), &[(stale, Choice::Adopt)]);
        assert_eq!(world.pushes(), 0);
        assert!(
            carried.notes[0].contains("changed again"),
            "{:?}",
            carried.notes
        );
    }

    #[test]
    fn a_choice_the_row_no_longer_offers_is_not_carried_out() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        let row = checked.report.rows[0].clone();
        let carried = carry_out(&world.deps(), &[(row, Choice::Remove)]);
        assert!(
            carried.notes[0].contains("changed again"),
            "{:?}",
            carried.notes
        );
        assert!(!line_ran(&world, "uninstall"));
    }

    fn line_ran(world: &World, needle: &str) -> bool {
        world.mac.calls().iter().any(|c| line(c).contains(needle))
    }

    #[test]
    fn apply_hands_back_a_plan_and_only_the_chosen_entries() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        let row = checked.report.rows[0].clone();
        let carried = carry_out(&world.deps(), &[(row, Choice::Apply)]);
        let (_, items) = carried.apply.unwrap();
        assert_eq!(items, ["keyboard.key-repeat"]);
        assert!(
            !line_ran(&world, "defaults write"),
            "a Run does the writing"
        );
    }

    #[test]
    fn leaving_it_alone_and_keeping_it_are_recorded() {
        let world = World::new();
        let checked = world.key_repeat_moved();
        let row = checked.report.rows[0].clone();
        carry_out(&world.deps(), &[(row, Choice::NotHere)]);
        assert_eq!(
            world.record().unavailable["keyboard.key-repeat"],
            "marked-by-person"
        );
    }

    #[test]
    fn removal_uninstalls_what_was_confirmed() {
        let world = World::new();
        let text = r#"{"schemaVersion":1,"name":"My Mac"}"#;
        world.follow(text, true);
        world.on_github(text);
        world.remember("formula:nobody-covers-this", json!(true));
        world.mac_says("CommandLineTools", Reply::ok(""));
        world.mac.on(
            |c| line(c).contains("Cellar/nobody-covers-this"),
            [
                Reply::ok(""),
                Reply::ok(""),
                Reply::ok(""),
                Reply::fails(1, ""),
            ],
        );
        world.mac_says("brew uninstall", Reply::ok(""));
        world.mac_says("brew l", Reply::ok(""));
        world.nothing_else_here();
        let checked = check(&world.deps());
        let row = checked.report.rows[0].clone();
        assert!(row.removable);
        let carried = carry_out(&world.deps(), &[(row, Choice::Remove)]);
        assert_eq!(carried.notes, Vec::<String>::new());
        assert!(line_ran(&world, "brew uninstall nobody-covers-this"));
    }

    #[test]
    fn following_a_config_that_was_saved_stays_saved_when_a_run_follows_it() {
        let world = World::new();
        world.follow(TEXT, true);
        world.follow(TEXT, false);
        assert!(following(world.dir.path()).unwrap().ever_saved);
    }

    // Review of PR #43, round 1. Each of these fails on the reviewed commit.

    /// GitHub comes back, holding exactly these files.
    fn github_holds(world: &mut World, files: &[(&str, &str)]) {
        let gh = FakeGitHub::new();
        gh.sign_in("example-you", 42);
        gh.on_git("rev-parse", Reply::ok(format!("{PUSHED}\n")));
        gh.add_repo(
            "example-you",
            CONFIG_REPO_NAME,
            RepoShape::private("main", SHA),
        );
        let canonical: Vec<(String, String)> = files
            .iter()
            .map(|(path, text)| {
                let config = Config::parse(text.as_bytes()).unwrap();
                ((*path).to_string(), config.to_canonical_json())
            })
            .collect();
        for (path, text) in &canonical {
            gh.add_file("example-you", CONFIG_REPO_NAME, SHA, path, text.as_bytes());
        }
        gh.set_git_files(
            "checkout",
            canonical
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect(),
        );
        world.gh = gh;
    }

    #[test]
    fn review_a_second_save_while_offline_keeps_the_first_one_waiting() {
        const TWO: &str = r#"{"schemaVersion":1,"name":"My Mac","settings":{"dock.autohide":true,"keyboard.key-repeat":2}}"#;
        let world = World::new();
        world.follow(TWO, true);
        world.on_github(TWO);
        world.remember("keyboard.key-repeat", json!(2));
        world.remember("dock.autohide", json!(true));
        world.mac_says("read NSGlobalDomain KeyRepeat", Reply::ok("6\n"));
        world.mac_says("read com.apple.dock autohide", Reply::ok("0\n"));
        world.nothing_else_here();
        let rows = check(&world.deps()).report.rows;
        let row = |id: &str| rows.iter().find(|row| row.id == id).unwrap().clone();
        world.gh.go_offline();
        carry_out(
            &world.deps(),
            &[(row("keyboard.key-repeat"), Choice::Adopt)],
        );
        carry_out(&world.deps(), &[(row("dock.autohide"), Choice::Adopt)]);
        let waiting: Vec<String> = world
            .record()
            .waiting_saves
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        assert!(
            waiting.contains(&"keyboard.key-repeat".to_string())
                && waiting.contains(&"dock.autohide".to_string()),
            "the person saved both while offline, and only these are waiting: {waiting:?}"
        );
    }

    #[test]
    fn review_a_waiting_save_does_not_go_into_a_config_followed_later() {
        const WORK: &str =
            r#"{"schemaVersion":1,"name":"Work","settings":{"keyboard.key-repeat":2}}"#;
        const HOME: &str =
            r#"{"schemaVersion":1,"name":"Home","settings":{"keyboard.key-repeat":2}}"#;
        let mut world = World::new();
        world.follow(WORK, true);
        github_holds(&mut world, &[("configs/work.json", WORK)]);
        world.remember("keyboard.key-repeat", json!(2));
        world.mac_says("read NSGlobalDomain KeyRepeat", Reply::ok("6\n"));
        world.nothing_else_here();
        let row = check(&world.deps()).report.rows[0].clone();
        world.gh.go_offline();
        carry_out(&world.deps(), &[(row, Choice::Adopt)]);
        assert_eq!(world.record().waiting_saves.len(), 1, "saved for Work");

        // Later, a Run or a save of another Config, and GitHub is back.
        world.follow(HOME, true);
        github_holds(
            &mut world,
            &[("configs/work.json", WORK), ("configs/home.json", HOME)],
        );
        send_waiting(&world.deps());
        assert_eq!(
            world.pushes(),
            0,
            "Send to GitHub wrote a save reviewed for Work into configs/home.json: {:?}",
            world.gh.file_at_git("add", "configs/home.json")
        );
        assert_eq!(
            world.record().last_applied["keyboard.key-repeat"].value,
            json!(2),
            "GitHub's Work Machine Type doesn't have this"
        );
    }

    #[test]
    fn review_a_config_read_from_github_stops_changes_once_its_file_is_gone() {
        // A second Mac imports its Config from GitHub and Runs it, which follows it unsaved.
        let mut world = World::new();
        world.follow(TEXT, false);
        github_holds(&mut world, &[(PATH, TEXT)]);
        world.remember("keyboard.key-repeat", json!(2));
        world.mac_says("read NSGlobalDomain KeyRepeat", Reply::ok("6\n"));
        world.nothing_else_here();
        let checked = check(&world.deps());
        assert_eq!(checked.status, Status::Current, "GitHub has it");
        let row = checked.report.rows[0].clone();

        github_holds(&mut world, &[]);
        let carried = carry_out(&world.deps(), &[(row, Choice::Adopt)]);
        assert_eq!(
            world.pushes(),
            0,
            "the file was deleted on GitHub and this put it back: {:?}",
            carried.notes
        );
        assert_eq!(
            check(&world.deps()).status,
            Status::Missing {
                path: PATH.to_string()
            }
        );
    }

    #[test]
    fn review_send_settles_a_waiting_save_github_already_has() {
        const SIX: &str =
            r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":6}}"#;
        let mut world = World::new();
        let row = world.key_repeat_moved().report.rows[0].clone();
        world.gh.go_offline();
        carry_out(&world.deps(), &[(row, Choice::Adopt)]);
        assert_eq!(world.record().waiting_saves.len(), 1);

        // The push landed before the app died, or another Mac sent the same value.
        github_holds(&mut world, &[(PATH, SIX)]);
        let notes = send_waiting(&world.deps());
        assert_eq!(
            notes,
            Vec::<String>::new(),
            "GitHub and this Mac both say 6, and the person is told it wasn't saved"
        );
        assert!(world.record().waiting_saves.is_empty(), "it's on GitHub");
        assert_eq!(
            world.record().last_applied["keyboard.key-repeat"].value,
            json!(6)
        );
    }

    // Review of PR #43, round 2.

    #[test]
    fn review_an_offline_save_for_another_config_does_not_take_over_a_waiting_save() {
        const WORK: &str =
            r#"{"schemaVersion":1,"name":"Work","settings":{"keyboard.key-repeat":2}}"#;
        const HOME: &str = r#"{"schemaVersion":1,"name":"Home","settings":{"dock.autohide":true,"keyboard.key-repeat":2}}"#;
        let mut world = World::new();
        world.follow(WORK, true);
        github_holds(&mut world, &[("configs/work.json", WORK)]);
        world.remember("keyboard.key-repeat", json!(2));
        world.remember("dock.autohide", json!(true));
        world.mac_says("read NSGlobalDomain KeyRepeat", Reply::ok("6\n"));
        world.mac_says("read com.apple.dock autohide", Reply::ok("0\n"));
        world.nothing_else_here();
        let rows = check(&world.deps()).report.rows;
        let repeat = rows
            .iter()
            .find(|row| row.id == "keyboard.key-repeat")
            .unwrap()
            .clone();

        // Key repeat is saved for Work and waits.
        world.gh.go_offline();
        carry_out(&world.deps(), &[(repeat, Choice::Adopt)]);
        assert_eq!(world.record().waiting_saves.len(), 1, "saved for Work");

        // Home is followed next, and only its Dock row is saved, still offline.
        world.follow(HOME, true);
        let rows = check(&world.deps()).report.rows;
        let dock = rows
            .iter()
            .find(|row| row.id == "dock.autohide")
            .unwrap()
            .clone();
        carry_out(&world.deps(), &[(dock, Choice::Adopt)]);

        github_holds(
            &mut world,
            &[("configs/work.json", WORK), ("configs/home.json", HOME)],
        );
        send_waiting(&world.deps());
        let home = world.gh.file_at_git("add", "configs/home.json");
        assert!(
            home.as_ref().is_none_or(|text| {
                let held: serde_json::Value = serde_json::from_str(text).unwrap();
                held["settings"]["keyboard.key-repeat"] == json!(2)
            }),
            "the key repeat save was reviewed for Work and went into configs/home.json: {home:?}"
        );
        assert_eq!(
            world.record().last_applied["keyboard.key-repeat"].value,
            json!(2),
            "GitHub's Work Machine Type doesn't have this"
        );
    }
}
