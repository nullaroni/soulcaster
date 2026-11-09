//! The Fleet in the window: what it knows about your Macs and Machine Types, opening a Machine Type
//! in the Catalog, and starting one from what's on this Mac. A real Mac does the slow parts on the
//! engine's thread. The simulator plays them against a fake GitHub, with the same code reading it.

use std::time::{SystemTime, UNIX_EPOCH};

use macflayer_core::config::{Config, Dotfiles};
use macflayer_engine::Found;
use macflayer_repo::copies::slug;
use macflayer_repo::fake::{FakeGitHub, RepoShape};
use macflayer_repo::fleet::{Fleet, FleetRead, MachineEntry, TypeEntry, read};
use macflayer_repo::save::{CONFIG_REPO_NAME, WaitReason};

use crate::catalog_view::view;
use crate::state::{App, Screen, TYPICAL, typical_settings};

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const OWNER: &str = "example-you";
/// The simulated Mac's ID. A real one is random.
pub const SIM_THIS_MAC: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";
const SIM_MINI: &str = "1f2e3d4c5b6a79880716253443526170";
const SIM_IMAC: &str = "2a3b4c5d6e7f8091a2b3c4d5e6f70819";
const SIM_NAME: &str = "This MacBook Pro";
const DAY: u64 = 24 * 60 * 60;
/// A Mac unheard from for this long is shown as stale, whatever it last said.
const STALE_AFTER: u64 = 14 * DAY;

/// What came back from the Fleet's work on the engine's thread.
pub enum FleetNews {
    Read {
        read: FleetRead,
        this_mac: Option<String>,
    },
    Surveyed(Found),
}

#[derive(Default)]
pub struct FleetView {
    pub fleet: Option<Box<Fleet>>,
    pub nothing_yet: bool,
    pub waiting: Option<WaitReason>,
    pub loading: bool,
    pub surveying: bool,
    pub this_mac: Option<String>,
    pub now: u64,
}

impl FleetView {
    fn take(&mut self, read: FleetRead) {
        self.loading = false;
        match read {
            FleetRead::Read(fleet) => {
                self.fleet = Some(fleet);
                self.nothing_yet = false;
                self.waiting = None;
            }
            FleetRead::NothingYet => {
                self.fleet = None;
                self.nothing_yet = true;
                self.waiting = None;
            }
            FleetRead::Waiting(why) => self.waiting = Some(why),
        }
    }

    pub fn follows(&self) -> Option<&str> {
        let this = self.this_mac.as_deref()?;
        let fleet = self.fleet.as_ref()?;
        fleet
            .machines
            .iter()
            .find(|mac| mac.id == this)
            .map(|mac| mac.machine_type.as_str())
    }

    pub fn users_of(&self, type_name: &str) -> usize {
        self.fleet.as_ref().map_or(0, |fleet| {
            fleet
                .machines
                .iter()
                .filter(|mac| mac.machine_type == type_name)
                .count()
        })
    }
}

/// Which simulated Fleet the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FleetSample {
    Several,
    One,
    Empty,
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// How long ago, in words.
pub fn seen_words(last_seen: u64, now: u64) -> String {
    let ago = now.saturating_sub(last_seen);
    let count = |n: u64, unit: &str| {
        let s = if n == 1 { "" } else { "s" };
        format!("{n} {unit}{s} ago")
    };
    match ago {
        0..60 => "Just now".to_string(),
        60..3_600 => count(ago / 60, "minute"),
        3_600..86_400 => count(ago / 3_600, "hour"),
        _ => count(ago / DAY, "day"),
    }
}

/// A Mac's state, and whether it's a good one.
pub fn status_words(mac: &MachineEntry, now: u64) -> (String, bool) {
    if now.saturating_sub(mac.last_seen) > STALE_AFTER {
        return ("Not seen lately".to_string(), false);
    }
    match mac.differences {
        None => ("Not checked yet".to_string(), false),
        Some(0) => ("In step".to_string(), true),
        Some(1) => ("1 difference".to_string(), false),
        Some(n) => (format!("{n} differences"), false),
    }
}

/// What a reading problem means on this screen, where nothing is being sent.
pub fn waiting_words(why: WaitReason) -> &'static str {
    match why {
        WaitReason::SignIn => "Sign in to GitHub to see your other Macs.",
        WaitReason::Offline | WaitReason::Busy => {
            "Can't reach GitHub right now. This is your Fleet as of the last time it was read."
        }
        WaitReason::Denied | WaitReason::Public => why.words(),
    }
}

fn sample_config(name: &str, take: usize, settings: usize) -> Config {
    Config {
        schema: None,
        schema_version: macflayer_core::config::SCHEMA_VERSION,
        name: name.to_string(),
        items: TYPICAL
            .iter()
            .take(take)
            .map(|id| (*id).to_string())
            .collect(),
        packages: Default::default(),
        settings: typical_settings()
            .into_iter()
            .take(settings)
            .map(|(id, value)| (id.to_string(), value))
            .collect(),
        dotfiles: None,
    }
}

fn add_type(gh: &FakeGitHub, config: &Config) {
    gh.add_file(
        OWNER,
        CONFIG_REPO_NAME,
        SHA,
        &format!("configs/{}.json", slug(&config.name)),
        config.to_canonical_json().as_bytes(),
    );
}

fn add_mac(gh: &FakeGitHub, entry: &MachineEntry) {
    let text = serde_json::to_string_pretty(entry).expect("plain fields");
    gh.add_file(
        OWNER,
        CONFIG_REPO_NAME,
        SHA,
        &format!("machines/{}.json", entry.id),
        text.as_bytes(),
    );
}

/// A simulated GitHub holding a Fleet.
pub fn seeded_fleet(sample: FleetSample) -> FakeGitHub {
    let gh = FakeGitHub::new();
    gh.sign_in(OWNER, 42);
    if sample == FleetSample::Empty {
        return gh;
    }
    gh.add_repo(OWNER, CONFIG_REPO_NAME, RepoShape::private("main", SHA));
    let now = now_seconds();
    let dev = "Dev MacBook";
    add_type(&gh, &sample_config(dev, 30, 10));
    let mut this = MachineEntry::new(
        SIM_THIS_MAC.to_string(),
        SIM_NAME.to_string(),
        dev.to_string(),
        now - 40,
    );
    this.differences = Some(0);
    add_mac(&gh, &this);
    if sample == FleetSample::Several {
        let agents = "Agent Mac Mini";
        add_type(&gh, &sample_config(agents, 12, 3));
        let mut mini = MachineEntry::new(
            SIM_MINI.to_string(),
            "The Mac mini".to_string(),
            agents.to_string(),
            now - 8 * 60,
        );
        mini.differences = Some(3);
        add_mac(&gh, &mini);
        add_mac(
            &gh,
            &MachineEntry::new(
                SIM_IMAC.to_string(),
                "Old iMac".to_string(),
                dev.to_string(),
                now - 41 * DAY,
            ),
        );
    }
    gh
}

fn sim_found() -> Found {
    Found {
        items: TYPICAL
            .iter()
            .take(14)
            .map(|id| (*id).to_string())
            .collect(),
        packages: vec!["htop".to_string()],
        settings: typical_settings()
            .into_iter()
            .take(4)
            .map(|(id, value)| (id.to_string(), value))
            .collect(),
    }
}

impl App {
    pub fn open_fleet(&mut self) {
        self.screen = Screen::Fleet;
        self.refresh_fleet();
    }

    pub fn refresh_fleet(&mut self) {
        self.fleet.loading = true;
        self.fleet.now = now_seconds();
        if self.engine.is_real() {
            self.engine.refresh_fleet();
            return;
        }
        let sim = &self.repo.sim;
        let read = read(&sim.fleet, &sim.tools, CONFIG_REPO_NAME);
        self.fleet.take(read);
        self.fleet.this_mac = Some(SIM_THIS_MAC.to_string());
    }

    pub fn fleet_news(&mut self, news: FleetNews) {
        match news {
            FleetNews::Read { read, this_mac } => {
                self.fleet.take(read);
                self.fleet.now = now_seconds();
                self.fleet.this_mac = this_mac.or(self.fleet.this_mac.take());
            }
            FleetNews::Surveyed(found) => self.take_found(found),
        }
    }

    /// Opens a Machine Type from the repo in the Catalog. Saving goes back to the same file.
    pub fn edit_type(&mut self, entry: &TypeEntry) {
        self.load_type(&entry.config);
        self.own_dotfiles = entry.config.dotfiles.clone();
        self.repo.last_saved = Some(entry.text.clone());
        self.repo.save = crate::repo_flow::SavePhase::Ready;
        self.show_section(0);
        self.screen = Screen::Catalog;
    }

    pub fn new_type(&mut self) {
        self.selected.clear();
        self.settings.clear();
        self.packages.clear();
        self.set_type_name("New Machine Type");
        self.forget_saved();
        self.show_section(0);
        self.screen = Screen::Catalog;
    }

    fn load_type(&mut self, config: &Config) {
        self.set_type_name(&config.name);
        self.selected = config.items.iter().cloned().collect();
        self.settings = config.settings.clone();
        self.packages = config.packages.formulae.iter().cloned().collect();
        self.packages
            .extend(config.packages.casks.iter().map(|c| format!("cask:{c}")));
    }

    fn forget_saved(&mut self) {
        self.own_dotfiles = None;
        self.repo.last_saved = None;
        self.repo.save = crate::repo_flow::SavePhase::Ready;
    }

    /// Makes this Mac follow a Machine Type. This says what it should have. It changes nothing on
    /// this Mac, and Drift shows the difference.
    pub fn use_type_here(&mut self, entry: &TypeEntry) {
        self.engine.follow(&entry.config, true);
        self.toast = Some(format!(
            "This Mac now follows {}. Drift shows what's different.",
            entry.config.name
        ));
        if self.engine.is_real() {
            self.engine.publish_self(None);
            return;
        }
        self.sim_record_this_mac(&entry.config.name);
        self.refresh_fleet();
    }

    fn sim_record_this_mac(&self, type_name: &str) {
        let entry = MachineEntry::new(
            SIM_THIS_MAC.to_string(),
            SIM_NAME.to_string(),
            type_name.to_string(),
            now_seconds(),
        );
        add_mac(&self.repo.sim.fleet, &entry);
    }

    /// What happens to the Fleet once a Machine Type has reached GitHub.
    pub fn saved_to_fleet(&mut self, config: &Config) {
        if self.engine.is_real() {
            self.engine.publish_self(None);
            return;
        }
        let gh = &self.repo.sim.fleet;
        gh.add_repo(OWNER, CONFIG_REPO_NAME, RepoShape::private("main", SHA));
        add_type(gh, config);
        self.sim_record_this_mac(&config.name);
    }

    pub fn start_survey(&mut self) {
        self.fleet.surveying = true;
        if self.engine.is_real() {
            self.engine.survey_this_mac();
        } else {
            self.take_found(sim_found());
        }
    }

    /// Fills the Catalog with what's here, for review. Nothing is saved or sent until the person
    /// saves it.
    pub fn take_found(&mut self, found: Found) {
        self.fleet.surveying = false;
        let listed = |id: &String| view().items.iter().any(|entry| entry.id() == id);
        self.selected = found.items.into_iter().filter(|id| listed(id)).collect();
        self.settings = found.settings;
        self.packages = found.packages.into_iter().collect();
        self.set_type_name("My Mac");
        self.forget_saved();
        self.show_section(0);
        self.screen = Screen::Catalog;
        self.toast = Some(format!(
            "Found {} on this Mac. Nothing is saved until you save it.",
            self.selection_count()
        ));
    }

    /// The Dotfiles Repo a Machine Type opened from your own repo already had.
    pub fn kept_dotfiles(&self) -> Option<Dotfiles> {
        self.own_dotfiles.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Scenario, SimEngine, SimMachine};
    use crate::state::with_engine;

    fn app() -> App {
        with_engine(Box::new(SimEngine::new(
            Scenario::AllGood,
            SimMachine::Bare,
        )))
    }

    fn with_sample(sample: FleetSample) -> App {
        let mut app = app();
        app.repo.sim.fleet = seeded_fleet(sample);
        app.open_fleet();
        app
    }

    fn entry(app: &App, name: &str) -> TypeEntry {
        app.fleet
            .fleet
            .as_ref()
            .unwrap()
            .types
            .iter()
            .find(|t| t.config.name == name)
            .unwrap()
            .clone()
    }

    #[test]
    fn the_sample_fleet_reads_through_the_real_reader() {
        let app = with_sample(FleetSample::Several);
        let fleet = app.fleet.fleet.as_ref().unwrap();
        assert_eq!(fleet.types.len(), 2);
        assert_eq!(fleet.machines.len(), 3);
        assert_eq!(fleet.unreadable, 0);
        assert_eq!(app.fleet.follows(), Some("Dev MacBook"));
        assert_eq!(app.fleet.users_of("Dev MacBook"), 2);
    }

    #[test]
    fn with_nothing_saved_the_fleet_says_so() {
        let app = with_sample(FleetSample::Empty);
        assert!(app.fleet.nothing_yet);
        assert!(app.fleet.fleet.is_none());
    }

    #[test]
    fn two_macs_each_see_the_other_after_saving() {
        let mut app = with_sample(FleetSample::Empty);
        app.fill_typical();
        app.set_type_name("Work");
        app.save_config();
        app.open_fleet();
        let fleet = app.fleet.fleet.as_ref().unwrap();
        assert_eq!(fleet.types[0].config.name, "Work");
        assert_eq!(app.fleet.follows(), Some("Work"));

        let other = MachineEntry::new(SIM_MINI.to_string(), "Mini".into(), "Work".into(), 1);
        add_mac(&app.repo.sim.fleet, &other);
        app.refresh_fleet();
        assert_eq!(app.fleet.users_of("Work"), 2);
    }

    #[test]
    fn editing_a_type_fills_the_catalog_and_saves_over_the_same_file() {
        let mut app = with_sample(FleetSample::Several);
        let agents = entry(&app, "Agent Mac Mini");
        app.edit_type(&agents);
        assert_eq!(app.screen, Screen::Catalog);
        assert_eq!(app.config_name, "Agent Mac Mini");
        assert_eq!(app.selected.len(), 12);
        assert_eq!(app.repo.last_saved.as_deref(), Some(agents.text.as_str()));
    }

    #[test]
    fn editing_a_type_with_dotfiles_keeps_them_when_saved_and_new_types_have_none() {
        let mut app = with_sample(FleetSample::Several);
        let mut dev = entry(&app, "Dev MacBook");
        let dotfiles: Dotfiles = serde_json::from_value(serde_json::json!({
            "repo": "example-you/dotfiles",
            "links": {"~/.zshrc": "zsh/zshrc"}
        }))
        .unwrap();
        dev.config.dotfiles = Some(dotfiles.clone());
        app.edit_type(&dev);
        app.toggle_item("ripgrep");
        assert_eq!(app.config_to_save().dotfiles, Some(dotfiles));
        app.new_type();
        assert_eq!(app.config_to_save().dotfiles, None);
    }

    #[test]
    fn following_a_type_says_so_in_the_fleet() {
        let mut app = with_sample(FleetSample::Several);
        let agents = entry(&app, "Agent Mac Mini");
        app.use_type_here(&agents);
        assert_eq!(app.fleet.follows(), Some("Agent Mac Mini"));
    }

    #[test]
    fn a_survey_ticks_exactly_what_was_found_and_saves_nothing() {
        let mut app = with_sample(FleetSample::Empty);
        app.take_found(Found {
            items: vec!["ripgrep".to_string(), "no-such-item".to_string()],
            packages: vec!["htop".to_string(), "cask:foo".to_string()],
            settings: typical_settings()
                .into_iter()
                .take(1)
                .map(|(id, value)| (id.to_string(), value))
                .collect(),
        });
        assert_eq!(app.screen, Screen::Catalog);
        assert_eq!(app.selected.len(), 1, "only what the Catalog lists");
        assert!(app.selected.contains("ripgrep"));
        assert_eq!(app.packages.len(), 2);
        assert_eq!(app.settings.len(), 1);
        assert!(matches!(app.repo.save, crate::repo_flow::SavePhase::Ready));
        assert!(app.toast.as_deref().unwrap().contains("Found 4"));
    }

    #[test]
    fn being_cut_off_keeps_what_was_last_read() {
        let mut app = with_sample(FleetSample::Several);
        app.repo.sim.fleet.go_offline();
        app.refresh_fleet();
        assert!(app.fleet.waiting.is_some());
        assert!(app.fleet.fleet.is_some(), "still shows the last read");
    }

    #[test]
    fn seen_words_read_naturally() {
        assert_eq!(seen_words(100, 130), "Just now");
        assert_eq!(seen_words(0, 60), "1 minute ago");
        assert_eq!(seen_words(0, 7_200), "2 hours ago");
        assert_eq!(seen_words(0, 3 * DAY), "3 days ago");
        assert_eq!(
            seen_words(500, 100),
            "Just now",
            "a fast clock isn't the future"
        );
    }

    #[test]
    fn a_stale_mac_is_stale_whatever_it_last_said() {
        let mut mac = MachineEntry::new(SIM_MINI.to_string(), "M".into(), "T".into(), 0);
        mac.differences = Some(0);
        assert_eq!(status_words(&mac, 15 * DAY).0, "Not seen lately");
        assert_eq!(status_words(&mac, DAY), ("In step".to_string(), true));
        mac.differences = Some(2);
        assert_eq!(status_words(&mac, DAY).0, "2 differences");
        mac.differences = None;
        assert_eq!(status_words(&mac, DAY).0, "Not checked yet");
    }
}
