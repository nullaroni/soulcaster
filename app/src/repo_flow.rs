//! Save, Import and the Dotfiles Repo in the window. The rules are the real `soulcaster-repo`: paste
//! parsing, the strict parser, the review, the link safety and the saving all run. What's simulated
//! is the outside world: GitHub and git are a fake, and every file goes in a temporary folder.
//! Wiring a real runner, the real home and the Machine Record is the wiring ticket's one change here,
//! in [`RepoSim`], along with moving these calls off the window's thread.

use rune::TextEdit;
use std::path::PathBuf;

use soulcaster_core::approval::{ApprovalStore, Approvals, CommitSha, DotfilesApproval};
use soulcaster_core::command::CommandRunner;
use soulcaster_core::command::Reply;
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::address::ConfigPath;
use soulcaster_repo::copies::keep_import;
use soulcaster_repo::dotfiles::{DotfilesStep, LinkReport, MovePreview, UserStage};
use soulcaster_repo::fake::{FakeGitHub, FileApprovals, RepoShape};
use soulcaster_repo::import::{ImportError, Importer, Opened, Picker, Refusal};
use soulcaster_repo::review::{Boxes, Review, Tick};
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveOutcome, SaveRequest, save};
use soulcaster_repo::tools::Tools;

use crate::catalog_view::view;
use crate::fleet_flow::{FleetSample, seeded_fleet};
use crate::state::{App, Screen};

/// What's on the Import screen.
pub enum ImportPhase {
    Paste { error: Option<String> },
    Pick(Picker),
    Refused { from: String, refusal: Refusal },
    Review(Box<ReviewPage>),
    Kept { name: String, path: String },
}

pub struct ReviewPage {
    pub review: Review,
    pub boxes: Boxes,
    pub show_all: bool,
}

pub enum SavePhase {
    Ready,
    Done(SaveOutcome),
    Blocked(String),
}

pub enum DotfilesPhase {
    Approved,
    Applied(LinkReport),
    Failed(String),
    Behind(MovePreview),
    Rewritten,
}

pub struct RepoFlow {
    pub sim: RepoSim,
    pub paste: TextEdit,
    pub import: ImportPhase,
    pub save: SavePhase,
    /// The last text saved from this window, so a change on this Mac can go on top of it.
    pub last_saved: Option<String>,
    pub dotfiles: DotfilesPhase,
}

/// Where a sample repo comes from, on the Import screen's list of simulated ones.
pub const SAMPLES: &[&str] = &[
    "example-dev/mac-setup",
    "example-dev/one-config",
    "example-stranger/setup",
    "example-you/setup",
    "s0ulcaster-official/verified-configs",
    "example-dev/odd-config",
    "evil-tools/preapproved",
    "evil-tools/two-answers",
    "example-dev/future",
    "example-dev/private-setup",
];

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const STRANGER_DOTS: &str = "a1b2c3d000000000000000000000000000000000";
const OWN_DOTS: &str = "77aa012000000000000000000000000000000000";
const OWN_NEWER: &str = "88bb123000000000000000000000000000000000";

/// The outside world, simulated. Everything it writes is under `dir`, a temporary folder removed
/// when the window closes.
pub struct RepoSim {
    /// Held so the folder lasts as long as the window. A real window has none.
    _dir: Option<tempfile::TempDir>,
    pub github: FakeGitHub,
    /// The Fleet the window shows, kept apart from the samples above.
    pub fleet: FakeGitHub,
    /// The real programs. Without this, everything goes to `github` above.
    real: Option<soulcaster_engine::ProcessRunner>,
    pub tools: Tools,
    pub store: Box<dyn ApprovalStore>,
}

impl RepoSim {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let home = std::fs::canonicalize(dir.path()).expect("the folder just made");
        let tools = Tools::fixed(home.clone());
        let store = Box::new(FileApprovals::new(home.join("approvals-test-double.json")));
        Self {
            github: seeded_github(),
            fleet: seeded_fleet(FleetSample::Several),
            real: None,
            tools,
            store,
            _dir: Some(dir),
        }
    }

    fn real(home: PathBuf) -> Self {
        Self {
            github: FakeGitHub::new(),
            fleet: FakeGitHub::new(),
            real: Some(soulcaster_repo::process::runner()),
            tools: Tools::for_mac(home.clone()),
            store: Box::new(soulcaster_engine::RecordApprovals::new(home)),
            _dir: None,
        }
    }

    fn runner(&self) -> &dyn CommandRunner {
        match &self.real {
            Some(real) => real,
            None => &self.github,
        }
    }

    fn importer(&self) -> Importer<'_> {
        Importer::connect(
            self.runner(),
            &self.tools,
            view().catalog,
            DEFAULT_CLONE_LOCATION,
        )
    }

    fn step(&self) -> DotfilesStep<'_> {
        DotfilesStep {
            runner: self.runner(),
            tools: &self.tools,
            store: self.store.as_ref(),
            clone_location: DEFAULT_CLONE_LOCATION,
            stamp: "simulated-run",
        }
    }

    /// A clone already in place, and files for the links to find and replace.
    fn prepare_dotfiles(&self) {
        let home = &self.tools.home;
        let clone = home.join("Developer/dotfiles");
        for (path, text) in [
            ("zsh/zshrc", "export EDITOR=nvim\n"),
            ("git/gitconfig", "[pull]\n\trebase = true\n"),
            ("nvim/init.lua", "-- hello\n"),
        ] {
            let at = clone.join(path);
            std::fs::create_dir_all(at.parent().expect("has a parent")).expect("a test folder");
            std::fs::write(at, text).expect("a test file");
        }
        std::fs::create_dir_all(clone.join(".git/objects")).expect("a test folder");
        std::fs::write(home.join(".zshrc"), "# what was here before\n").expect("a test file");
        std::fs::create_dir_all(home.join("Documents")).expect("a test folder");
        self.github.on_git(
            "--get",
            Reply::ok("https://github.com/example-you/dotfiles.git\n"),
        );
        self.github
            .on_git("rev-parse", Reply::ok(format!("{OWN_DOTS}\n")));
        self.github.on_git(
            "diff",
            Reply::ok(
                "--- a/zsh/zshrc\n+++ b/zsh/zshrc\n@@ -1 +1,2 @@\n export EDITOR=nvim\n+source ~/.config/extras.zsh\n",
            ),
        );
    }
}

fn seeded_github() -> FakeGitHub {
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    let repo = |owner: &str, name: &str| gh.add_repo(owner, name, RepoShape::public("main", SHA));
    let file = |owner: &str, name: &str, path: &str, bytes: &[u8]| {
        gh.add_file(owner, name, SHA, path, bytes);
    };
    macro_rules! sample {
        ($name:literal) => {
            include_bytes!(concat!(
                "../../crates/soulcaster-repo/tests/fixtures/import/",
                $name
            ))
        };
    }
    repo("example-dev", "mac-setup");
    file(
        "example-dev",
        "mac-setup",
        "configs/laptop.json",
        sample!("clean.json"),
    );
    file(
        "example-dev",
        "mac-setup",
        "configs/mobile.json",
        sample!("one-tap.json"),
    );
    file(
        "example-dev",
        "mac-setup",
        "configs/broken.json",
        sample!("refused-self-approval.json"),
    );
    repo("example-dev", "one-config");
    file(
        "example-dev",
        "one-config",
        "configs/mobile.json",
        sample!("one-tap.json"),
    );
    repo("example-stranger", "setup");
    file(
        "example-stranger",
        "setup",
        "configs/terminal.json",
        sample!("stranger-dotfiles.json"),
    );
    gh.add_repo(
        "example-stranger",
        "dotfiles",
        RepoShape::public("main", STRANGER_DOTS),
    );
    gh.add_file(
        "example-stranger",
        "dotfiles",
        STRANGER_DOTS,
        "script/setup",
        b"#!/bin/sh\n# script/setup\nset -e\nbrew bundle --file=./Brewfile\nmkdir -p ~/.config\ncurl -fsSL https://example.invalid/theme.tar.gz | tar -xz -C ~/.config\n./script/lib/post-install.sh\n",
    );
    repo("example-you", "setup");
    file(
        "example-you",
        "setup",
        "configs/other-mac.json",
        sample!("own-dotfiles.json"),
    );
    gh.add_repo(
        "example-you",
        "dotfiles",
        RepoShape::public("main", OWN_DOTS),
    );
    gh.add_file(
        "example-you",
        "dotfiles",
        OWN_DOTS,
        "install.sh",
        b"#!/bin/sh\n# install.sh\nmkdir -p ~/Developer\n",
    );
    repo("s0ulcaster-official", "verified-configs");
    file(
        "s0ulcaster-official",
        "verified-configs",
        "configs/recommended.json",
        sample!("hostile.json"),
    );
    repo("example-dev", "odd-config");
    file(
        "example-dev",
        "odd-config",
        "configs/x.json",
        sample!("refused-bidi.json"),
    );
    repo("evil-tools", "preapproved");
    file(
        "evil-tools",
        "preapproved",
        "configs/x.json",
        sample!("refused-self-approval.json"),
    );
    repo("evil-tools", "two-answers");
    file(
        "evil-tools",
        "two-answers",
        "configs/x.json",
        sample!("refused-duplicate-key.json"),
    );
    repo("example-dev", "future");
    file(
        "example-dev",
        "future",
        "configs/x.json",
        sample!("refused-newer-version.json"),
    );
    gh.on_git("rev-parse", Reply::ok(format!("{SHA}\n")));
    gh
}

impl RepoFlow {
    /// Import and Save on real GitHub, and approvals in the Machine Record.
    pub fn real(home: PathBuf) -> Self {
        Self {
            sim: RepoSim::real(home),
            ..Self::simulated()
        }
    }

    pub fn simulated() -> Self {
        Self {
            sim: RepoSim::new(),
            paste: TextEdit::new(""),
            import: ImportPhase::Paste { error: None },
            save: SavePhase::Ready,
            last_saved: None,
            dotfiles: DotfilesPhase::Approved,
        }
    }

    pub fn dotfiles_plan() -> DotfilesPlan {
        DotfilesPlan {
            repo: "example-you/dotfiles".to_string(),
            links: [
                ("~/.config/nvim", "nvim"),
                ("~/.gitconfig", "git/gitconfig"),
                ("~/.zshrc", "zsh/zshrc"),
                ("~/Documents", "nvim"),
            ]
            .into_iter()
            .map(|(target, source)| (target.to_string(), source.to_string()))
            .collect(),
            refused_links: vec![],
            install_script: None,
        }
    }

    pub fn own_dots() -> CommitSha {
        CommitSha::parse(OWN_DOTS).expect("a fixed commit")
    }

    pub fn own_newer() -> CommitSha {
        CommitSha::parse(OWN_NEWER).expect("a fixed commit")
    }

    /// The approval a Start Run would have recorded for the Dotfiles screen's own repo.
    fn approve_dotfiles(&self) {
        let plan = Self::dotfiles_plan();
        let approvals = Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: "github.com/example-you/dotfiles".to_string(),
                commit: Self::own_dots(),
                links: plan.links,
            }),
            ..Approvals::default()
        };
        let _ = self.sim.store.write(&approvals);
    }
}

impl App {
    pub fn read_paste(&mut self) {
        let paste = self.repo.paste.text.clone();
        let opened = self.repo.sim.importer().open(&paste);
        self.repo.import = match opened {
            Ok(Opened::One(fetched)) => self.review_page(&fetched),
            Ok(Opened::Pick(picker)) => ImportPhase::Pick(picker),
            Err(ImportError::Refused(refusal)) => ImportPhase::Refused {
                from: paste,
                refusal,
            },
            // The paste stays on screen, so it can be fixed.
            Err(error) => ImportPhase::Paste {
                error: Some(error.to_string()),
            },
        };
    }

    fn review_page(&self, fetched: &soulcaster_repo::import::Fetched) -> ImportPhase {
        let review = self.repo.sim.importer().review(fetched);
        let approvals = self.repo.sim.store.read().unwrap_or_default();
        let boxes = review.starting_boxes(&approvals);
        ImportPhase::Review(Box::new(ReviewPage {
            review,
            boxes,
            show_all: false,
        }))
    }

    pub fn read_sample(&mut self, sample: &str) {
        self.repo.paste = TextEdit::new(sample);
        self.read_paste();
    }

    pub fn pick_config(&mut self, path: &ConfigPath) {
        let ImportPhase::Pick(picker) = &self.repo.import else {
            return;
        };
        let fetched = self.repo.sim.importer().pick(picker, path);
        self.repo.import = match fetched {
            Ok(fetched) => self.review_page(&fetched),
            Err(error) => ImportPhase::Paste {
                error: Some(error.to_string()),
            },
        };
    }

    pub fn review_tick(&mut self, tick: &Tick) {
        if let ImportPhase::Review(page) = &mut self.repo.import {
            let ReviewPage { review, boxes, .. } = page.as_mut();
            review.toggle(boxes, tick);
        }
    }

    pub fn review_select_safe(&mut self) {
        if let ImportPhase::Review(page) = &mut self.repo.import {
            let ReviewPage { review, boxes, .. } = page.as_mut();
            review.select_what_needs_no_approval(boxes);
        }
    }

    pub fn review_show_all(&mut self) {
        if let ImportPhase::Review(page) = &mut self.repo.import {
            page.show_all = true;
        }
    }

    /// Cancel, or Close without importing. The copy is dropped and the paste is still there.
    pub fn close_import(&mut self) {
        self.repo.import = ImportPhase::Paste { error: None };
    }

    /// Keep a copy: it records no approvals.
    pub fn keep_copy(&mut self) {
        let ImportPhase::Review(page) = &self.repo.import else {
            return;
        };
        let fetched = soulcaster_repo::import::Fetched {
            source: page.review.source.clone(),
            config: page.review.fetched_config().clone(),
        };
        let name = page.review.name.clone();
        self.repo.import = match keep_import(&self.repo.sim.tools.home, &fetched) {
            Ok(path) => ImportPhase::Kept {
                name,
                path: path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
            Err(error) => ImportPhase::Paste {
                error: Some(error.to_string()),
            },
        };
    }

    /// Start Run: the only thing that records approvals. It hands what the boxes allow to Review,
    /// which asks for the password and GitHub sign-in before the simulated Run.
    pub fn start_from_review(&mut self) {
        let ImportPhase::Review(page) = &self.repo.import else {
            return;
        };
        if page.review.nothing_selected(&page.boxes) {
            return;
        }
        let Ok(run) = page
            .review
            .record_start(&page.boxes, self.repo.sim.store.as_ref())
        else {
            self.toast = Some("Couldn't record your approvals, so nothing started.".to_string());
            return;
        };
        self.set_type_name(&run.config.name);
        self.own_dotfiles = None;
        self.selected = run.config.items.iter().cloned().collect();
        self.settings = run.config.settings.clone();
        self.packages = run.config.packages.formulae.iter().cloned().collect();
        self.packages.extend(
            run.config
                .packages
                .casks
                .iter()
                .map(|c| format!("cask:{c}")),
        );
        self.answers.taps = run.taps.iter().cloned().collect();
        self.imported_taps = run.taps.iter().cloned().collect();
        self.answers.licenses = run.licenses.clone();
        self.keep_dotfiles(run.config.dotfiles.clone());
        self.rebuild_rows();
        self.repo.import = ImportPhase::Paste { error: None };
        self.screen = Screen::Review;
    }

    /// Saves the current selection. A Run never waits on this, and nothing here blocks one.
    pub fn save_config(&mut self) {
        let config = self.config_to_save();
        let request = SaveRequest {
            config: &config,
            repo_name: CONFIG_REPO_NAME,
            base: self.repo.last_saved.as_deref(),
        };
        let sim = &self.repo.sim;
        let outcome = save(sim.runner(), &sim.tools, view().catalog, &request);
        self.repo.save = match outcome {
            Ok(outcome) => {
                if matches!(
                    outcome,
                    SaveOutcome::Sent { .. } | SaveOutcome::AlreadyThere { .. }
                ) {
                    self.repo.last_saved = Some(config.to_canonical_json());
                    self.engine.follow(&config, true);
                    self.saved_to_fleet(&config);
                }
                SavePhase::Done(outcome)
            }
            Err(error) => SavePhase::Blocked(error.to_string()),
        };
    }

    /// Runs the real link step on the temporary folder. In a real Run this is the wiring ticket's
    /// call, between `ThirdParty` and `GitValues`, with no button.
    pub fn apply_dotfiles(&mut self) {
        let plan = RepoFlow::dotfiles_plan();
        self.repo.sim.prepare_dotfiles();
        self.repo.approve_dotfiles();
        self.repo.dotfiles = match self
            .repo
            .sim
            .step()
            .apply_links(&plan, UserStage::password_is_gone())
        {
            Ok(report) => DotfilesPhase::Applied(report),
            Err(error) => DotfilesPhase::Failed(error.to_string()),
        };
    }

    /// A newer version on GitHub, with its changes to look at. Nothing moves.
    pub fn review_dotfiles_update(&mut self) {
        let plan = RepoFlow::dotfiles_plan();
        self.repo.sim.prepare_dotfiles();
        self.repo.approve_dotfiles();
        self.repo.dotfiles = match self
            .repo
            .sim
            .step()
            .preview_move(&plan, &RepoFlow::own_newer())
        {
            Ok(preview) => DotfilesPhase::Behind(preview),
            Err(error) => DotfilesPhase::Failed(error.to_string()),
        };
    }

    pub fn move_dotfiles(&mut self) {
        let DotfilesPhase::Behind(preview) = &self.repo.dotfiles else {
            return;
        };
        let preview = preview.clone();
        self.repo
            .sim
            .github
            .on_git("rev-parse", Reply::ok(format!("{OWN_NEWER}\n")));
        let plan = RepoFlow::dotfiles_plan();
        let moved = self.repo.sim.step().move_after_review(&plan, &preview);
        match moved {
            Ok(()) => {
                self.toast = Some("Dotfiles Repo updated.".to_string());
                self.repo.dotfiles = DotfilesPhase::Approved;
            }
            Err(error) => self.repo.dotfiles = DotfilesPhase::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use soulcaster_core::ask::Password;
    use soulcaster_core::event::Event;
    use soulcaster_core::plan::Plan;
    use soulcaster_repo::review::Primary;

    use super::*;
    use crate::answers::Answers;
    use crate::engine::Engine;
    use crate::sim::{Scenario, SimEngine, SimMachine};
    use crate::state::with_engine;

    fn app() -> App {
        with_engine(Box::new(SimEngine::new(
            Scenario::AllGood,
            SimMachine::Bare,
        )))
    }

    fn page(app: &App) -> &ReviewPage {
        match &app.repo.import {
            ImportPhase::Review(page) => page,
            _ => panic!("not on the review"),
        }
    }

    #[test]
    fn every_sample_lands_where_the_design_says() {
        let mut app = app();
        app.read_sample("example-dev/mac-setup");
        assert!(matches!(app.repo.import, ImportPhase::Pick(_)));

        app.read_sample("s0ulcaster-official/verified-configs");
        let hostile = page(&app);
        assert_eq!(hostile.review.primary(&hostile.boxes), Primary::Close);
        assert!(hostile.review.nothing_selected(&hostile.boxes));

        app.read_sample("example-you/setup");
        let own = page(&app);
        assert!(own.boxes.repo && !own.boxes.script);

        app.read_sample("example-stranger/setup");
        let stranger = page(&app);
        assert!(!stranger.boxes.repo && !stranger.boxes.script);
        assert!(
            stranger
                .review
                .dotfiles
                .as_ref()
                .unwrap()
                .script_text
                .is_some()
        );

        for refused in [
            "example-dev/odd-config",
            "evil-tools/two-answers",
            "example-dev/future",
        ] {
            app.read_sample(refused);
            assert!(
                matches!(app.repo.import, ImportPhase::Refused { .. }),
                "{refused}"
            );
        }

        app.read_sample("example-dev/private-setup");
        let ImportPhase::Paste { error: Some(words) } = &app.repo.import else {
            panic!("expected a failure");
        };
        assert!(words.contains("can't see"), "{words}");
        assert_eq!(
            &app.repo.paste.text, "example-dev/private-setup",
            "the paste stays"
        );
    }

    #[test]
    fn keep_a_copy_records_no_approval_and_start_does() {
        let mut app = app();
        app.read_sample("example-stranger/setup");
        app.review_tick(&Tick::Repo);
        app.keep_copy();
        assert!(matches!(app.repo.import, ImportPhase::Kept { .. }));
        assert_eq!(app.repo.sim.store.read().unwrap(), Approvals::default());

        app.read_sample("example-stranger/setup");
        app.review_tick(&Tick::Repo);
        app.start_from_review();
        assert_eq!(app.screen, Screen::Review);
        assert!(app.repo.sim.store.read().unwrap().dotfiles.is_some());
    }

    #[test]
    fn an_imported_dotfiles_repo_does_not_follow_a_later_catalog_plan() {
        let mut app = app();
        app.read_sample("example-stranger/setup");
        app.review_tick(&Tick::Repo);
        app.start_from_review();
        let imported = app
            .dotfiles
            .as_ref()
            .map(|dotfiles| dotfiles.repo.clone())
            .expect("starting from that import should keep its Dotfiles Repo for the Run");
        assert!(imported.contains("example-stranger/dotfiles"), "{imported}");
        app.screen = Screen::Catalog;
        app.selected.clear();
        app.packages.clear();
        app.settings.clear();
        app.selected.insert("jq".to_string());
        let plan = app.plan();
        assert_eq!(
            plan.dotfiles
                .as_ref()
                .map(|dotfiles| dotfiles.repo.as_str()),
            None,
            "a Catalog plan that has nothing to do with the import still carries {imported}"
        );
    }

    /// The review records `owner/tap/name`. A tap approval covers every package from that tap.
    struct Capture(Arc<Mutex<Vec<String>>>);

    impl Engine for Capture {
        fn start_run(&mut self, _plan: &Plan, answers: &Answers) {
            *self.0.lock().unwrap() = answers.taps.iter().cloned().collect();
        }

        fn start_scoped(&mut self, _: &str, _: &[String], _: Option<&str>) {}

        fn submit_password(&mut self, _: Password) {}

        fn start_github_sign_in(&mut self) -> String {
            String::new()
        }

        fn poll(&mut self, _: f64) -> Vec<Event> {
            Vec::new()
        }

        fn next_due(&self, _: f64) -> Option<f64> {
            None
        }
    }

    #[test]
    fn a_package_approval_is_not_widened_to_the_whole_tap() {
        let mut app = app();
        app.read_sample("example-dev/one-config");
        app.review_tick(&Tick::Tap("mobile-dev-inc/tap".to_string()));
        app.start_from_review();
        assert!(
            app.plan().taps.contains_key("mobile-dev-inc/tap"),
            "the imported package should put its tap on the plan"
        );
        let held = app.repo.sim.store.read().unwrap();
        assert!(
            held.packages.contains("mobile-dev-inc/tap/maestro"),
            "the review records the package, not the tap: {:?}",
            held.packages
        );
        assert!(
            !held.taps.contains("mobile-dev-inc/tap"),
            "the review approved the whole tap: {:?}",
            held.taps
        );
        let got = Arc::new(Mutex::new(Vec::new()));
        app.engine = Box::new(Capture(Arc::clone(&got)));
        app.start_run();
        let handed = got.lock().unwrap().clone();
        assert!(
            !handed.iter().any(|tap| tap == "mobile-dev-inc/tap"),
            "start handed the engine the tap {handed:?}, which approves every package from it, not only mobile-dev-inc/tap/maestro"
        );
    }

    fn imported() -> App {
        let mut app = app();
        app.read_sample("example-dev/one-config");
        app.review_tick(&Tick::Tap("mobile-dev-inc/tap".to_string()));
        app.start_from_review();
        app
    }

    fn taps_handed(app: &mut App) -> Vec<String> {
        let got = Arc::new(Mutex::new(Vec::new()));
        app.engine = Box::new(Capture(Arc::clone(&got)));
        app.start_run();
        got.lock().unwrap().clone()
    }

    /// The import stores the package and not the tap. A later Review is a new answer.
    #[test]
    fn a_tap_ticked_on_a_later_review_is_not_held_back() {
        let mut app = imported();
        let held = app.repo.sim.store.read().unwrap();
        assert!(
            held.packages.contains("mobile-dev-inc/tap/maestro"),
            "the package approval was not stored: {:?}",
            held.packages
        );
        assert!(
            !held.taps.contains("mobile-dev-inc/tap"),
            "the import wrote a tap approval: {:?}",
            held.taps
        );
        let brew_names: Vec<String> = app
            .plan()
            .entries
            .iter()
            .flat_map(|entry| entry.recipe.steps())
            .filter_map(|step| step.brew_name().map(str::to_string))
            .collect();
        assert!(
            brew_names
                .iter()
                .any(|name| name == "mobile-dev-inc/tap/maestro"),
            "the plan's brew name is not the stored package, so it would not run: {brew_names:?}"
        );

        app.selected.clear();
        app.packages.clear();
        app.settings.clear();
        app.selected.insert("aerospace".to_string());
        app.answers.taps.clear();
        app.answers.taps.insert("nikitabobko/tap".to_string());
        assert!(app.plan().taps.contains_key("nikitabobko/tap"));
        let unrelated = taps_handed(&mut app);

        let mut app = imported();
        app.selected.clear();
        app.packages.clear();
        app.settings.clear();
        app.packages
            .insert("mobile-dev-inc/tap/other-tool".to_string());
        // Changed from setting the field directly: a person's tick goes through `answer_tap`.
        app.answers.taps.clear();
        app.answer_tap("mobile-dev-inc/tap", true);
        assert!(
            app.plan().taps.contains_key("mobile-dev-inc/tap"),
            "the later plan should show the tap the person ticked"
        );
        let later = taps_handed(&mut app);

        let mut problems = Vec::new();
        if !unrelated.iter().any(|tap| tap == "nikitabobko/tap") {
            problems.push(format!(
                "a different tap ticked on a later review was held back: {unrelated:?}"
            ));
        }
        if !later.iter().any(|tap| tap == "mobile-dev-inc/tap") {
            problems.push(format!(
                "a tap the person ticked on a later review was held back ({later:?}) because an earlier import used that tap"
            ));
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// The import copies its tap into the Review answers so that row shows Allowed. That copy
    /// is how the package shows as answered. It is not a tap approval, and a tap approval
    /// covers every package from the tap. Editing the selection does not tick the row again.
    #[test]
    fn a_selection_edit_does_not_widen_an_imported_package_approval_to_the_tap() {
        let mut app = imported();
        assert!(
            app.answers.taps.contains("mobile-dev-inc/tap"),
            "the import should show its tap as answered"
        );
        assert!(
            app.plan().taps.contains_key("mobile-dev-inc/tap"),
            "the imported package should put its tap on the plan"
        );
        let answers_before = app.answers.taps.clone();
        app.selected.insert("ripgrep".to_string());
        assert_eq!(
            app.answers.taps, answers_before,
            "the edit should not tick or clear the tap"
        );
        assert!(
            app.plan().taps.contains_key("mobile-dev-inc/tap"),
            "the imported package should stay on the plan"
        );
        let handed = taps_handed(&mut app);
        assert!(
            !handed.iter().any(|tap| tap == "mobile-dev-inc/tap"),
            "editing the selection handed the engine the tap {handed:?}, which approves every package from it; the import only stored mobile-dev-inc/tap/maestro and the tap was not ticked again"
        );
    }

    #[test]
    fn saving_never_gets_in_the_way_of_a_run() {
        // With nothing saved yet and no network, a Run still starts from the same selection.
        let mut app = app();
        app.fill_typical();
        app.repo.sim.github.go_offline();
        app.save_config();
        assert!(matches!(
            app.repo.save,
            SavePhase::Done(SaveOutcome::Waiting { .. })
        ));
        let before = app.config();
        app.start_run();
        assert!(app.run_started);
        assert_eq!(app.config(), before);
    }

    #[test]
    fn first_run_saves_the_selection_to_a_new_private_repo() {
        let mut app = app();
        app.fill_typical();
        app.save_config();
        let SavePhase::Done(SaveOutcome::Sent {
            public,
            created_repo,
            ..
        }) = &app.repo.save
        else {
            panic!("expected it to be sent");
        };
        assert!(!public);
        assert!(created_repo);
    }

    #[test]
    fn the_dotfiles_screen_runs_the_real_link_rules_inside_the_temporary_folder() {
        let mut app = app();
        app.apply_dotfiles();
        let DotfilesPhase::Applied(report) = &app.repo.dotfiles else {
            panic!("expected a report");
        };
        assert_eq!(report.rows.len(), 4);
        let home = app.repo.sim.tools.home.clone();
        assert!(
            std::fs::read_link(home.join(".zshrc"))
                .unwrap()
                .starts_with(&home)
        );
        assert!(home.join("Documents").is_dir(), "a protected folder stays");
    }
}
