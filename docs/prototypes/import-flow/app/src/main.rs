//! PROTOTYPE of Soulcaster's Import flow, answering wayfinder ticket #15. Fetches nothing, installs nothing, changes
//! nothing. Every screen is a scene, so it can render to PNG with no person present:
//!
//! ```sh
//! cargo run -- --snap out.png --scene review-hostile --size 980x1400 --scale 1 --theme light
//! cargo run                # and click through it
//! ```

mod entry;
mod later;
mod plan;
mod review;
mod ui;

use std::collections::{BTreeMap, BTreeSet};

use rune::{AppSpec, Cx, El, TextEdit};
use rune_kit::Kit;

#[derive(Clone, PartialEq)]
pub enum Screen {
    Paste,
    Reading(String),
    Choose,
    Failed(entry::Failure),
    Refused(String),
    Review,
    FirstMinute,
    Report,
    UpdateConfig,
    UpdateDotfiles(later::Moved),
    Save,
}

/// Every box on the review. Nothing else decides what a Run does.
#[derive(Default)]
pub struct Boxes {
    pub taps: BTreeMap<String, bool>,
    pub repo: bool,
    pub script: bool,
    pub catalog: bool,
    pub formulae: bool,
    pub casks: bool,
    pub settings: bool,
    pub license: bool,
}

pub struct App {
    pub screen: Screen,
    pub paste: TextEdit,
    pub signed_in: bool,
    pub plan: String,
    pub boxes: Boxes,
    pub show_all: bool,
    pub show_values: BTreeSet<String>,
    pub update_tap: bool,
    pub update_repo: bool,
    pub move_commit: bool,
    pub move_script: bool,
}

impl App {
    fn new() -> Self {
        App {
            screen: Screen::Paste,
            paste: TextEdit::new(""),
            signed_in: false,
            plan: String::new(),
            boxes: Boxes::default(),
            show_all: false,
            show_values: BTreeSet::new(),
            update_tap: false,
            update_repo: false,
            move_commit: false,
            move_script: false,
        }
    }

    /// Where each box starts. check/import_model.py's starting_state, for a first review with no earlier approvals.
    pub fn open_review(&mut self, id: &str) {
        let p = plan::plan(id);
        let warned = !plan::list(p, "signals").is_empty();
        let big = plan::strs(p, "bigLists");
        self.plan = id.to_string();
        self.show_all = false;
        self.show_values.clear();
        self.boxes = Boxes {
            taps: plan::list(p, "taps").iter().map(|t| (plan::s(t, "tap").to_string(), false)).collect(),
            repo: p["dotfiles"]["own"].as_bool().unwrap_or(false) && self.signed_in && !warned,
            script: false,
            catalog: !warned,
            formulae: !warned && !big.contains(&"formulae"),
            casks: !warned && !big.contains(&"casks"),
            settings: !warned,
            license: false,
        };
        self.screen = Screen::Review;
    }

    pub fn nothing_selected(&self) -> bool {
        let b = &self.boxes;
        !(b.catalog || b.formulae || b.casks || b.settings || b.repo || b.taps.values().any(|on| *on))
    }
}

pub fn kit(dark: bool) -> &'static Kit {
    static DARK: std::sync::OnceLock<Kit> = std::sync::OnceLock::new();
    static LIGHT: std::sync::OnceLock<Kit> = std::sync::OnceLock::new();
    if dark { DARK.get_or_init(|| Kit::neutral(true)) } else { LIGHT.get_or_init(|| Kit::neutral(false)) }
}

fn view(app: &App, cx: &mut Cx<App>) -> El {
    let k = kit(cx.system_dark());
    match &app.screen {
        Screen::Paste => entry::paste(app, k),
        Screen::Reading(source) => entry::reading(k, source),
        Screen::Choose => entry::choose(k),
        Screen::Failed(f) => entry::failed(k, *f),
        Screen::Refused(id) => entry::refused(k, id),
        Screen::Review => review::review(app, k),
        Screen::FirstMinute => later::first_minute(app, k),
        Screen::Report => later::report(app, k),
        Screen::UpdateConfig => later::update_config(app, k),
        Screen::UpdateDotfiles(m) => later::update_dotfiles(app, k, *m),
        Screen::Save => later::save(k),
    }
}

fn signed(a: &mut App) {
    a.signed_in = true;
}

fn main() {
    use entry::Failure as F;
    use later::Moved as M;
    rune::run(AppSpec {
        scenes: &[
            ("paste", |a: &mut App| a.screen = Screen::Paste),
            ("reading", |a: &mut App| a.screen = Screen::Reading("example-dev/mac-setup".into())),
            ("choose", |a: &mut App| a.screen = Screen::Choose),
            ("failed-offline", |a: &mut App| a.screen = Screen::Failed(F::Offline)),
            ("failed-not-visible", |a: &mut App| a.screen = Screen::Failed(F::NotVisible)),
            ("failed-not-visible-signed-in", |a: &mut App| { signed(a); a.screen = Screen::Failed(F::NotVisibleSignedIn) }),
            ("failed-rate-limited", |a: &mut App| a.screen = Screen::Failed(F::RateLimited)),
            ("failed-no-configs", |a: &mut App| a.screen = Screen::Failed(F::NoConfigs)),
            ("refused-bidi", |a: &mut App| a.screen = Screen::Refused("refused-bidi".into())),
            ("refused-self-approval", |a: &mut App| a.screen = Screen::Refused("refused-self-approval".into())),
            ("refused-duplicate-key", |a: &mut App| a.screen = Screen::Refused("refused-duplicate-key".into())),
            ("refused-newer-version", |a: &mut App| a.screen = Screen::Refused("refused-newer-version".into())),
            ("review-clean", |a: &mut App| a.open_review("clean")),
            ("review-one-tap", |a: &mut App| a.open_review("one-tap")),
            ("review-one-tap-approved", |a: &mut App| {
                a.open_review("one-tap");
                a.boxes.taps.insert("mobile-dev-inc/tap".into(), true);
                a.boxes.license = true;
            }),
            ("review-stranger", |a: &mut App| a.open_review("stranger-dotfiles")),
            ("review-stranger-approved", |a: &mut App| {
                a.open_review("stranger-dotfiles");
                a.boxes.repo = true;
                a.boxes.script = true;
            }),
            ("review-own", |a: &mut App| { signed(a); a.open_review("own-dotfiles") }),
            ("review-own-signed-out", |a: &mut App| a.open_review("own-dotfiles")),
            ("review-hostile", |a: &mut App| a.open_review("hostile")),
            ("first-minute", |a: &mut App| {
                a.open_review("one-tap");
                a.boxes.taps.insert("mobile-dev-inc/tap".into(), true);
                a.boxes.license = true;
                a.screen = Screen::FirstMinute;
            }),
            ("report-left-out", |a: &mut App| {
                a.open_review("one-tap");
                a.screen = Screen::Report;
            }),
            ("update-config", |a: &mut App| a.screen = Screen::UpdateConfig),
            ("update-dotfiles-stranger", |a: &mut App| a.screen = Screen::UpdateDotfiles(M::Stranger)),
            ("update-dotfiles-own", |a: &mut App| {
                a.move_commit = true;
                a.screen = Screen::UpdateDotfiles(M::Own);
            }),
            ("update-dotfiles-rewritten", |a: &mut App| a.screen = Screen::UpdateDotfiles(M::Rewritten)),
            ("save-copy", |a: &mut App| a.screen = Screen::Save),
        ],
        ..AppSpec::new("Soulcaster Import (prototype)", (980.0, 760.0), App::new, view)
    });
}
