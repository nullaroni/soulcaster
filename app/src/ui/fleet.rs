//! Fleet: your Machine Types and your Macs, and whether each is in step. It opens here.

use macflayer_repo::fleet::{MachineEntry, TypeEntry};
use macflayer_repo::save::WaitReason;
use rune::{El, ScrollSpec, col, el, row, text};

use super::catalog::tile;
use super::kit;
use crate::fleet_flow::{seen_words, status_words, waiting_words};
use crate::state::App;

const TYPE_COLUMN: f32 = 170.0;
const SEEN_COLUMN: f32 = 120.0;
const STATUS_COLUMN: f32 = 180.0;

fn heading(words: &str) -> El {
    row()
        .w_full()
        .px(14.0)
        .pt(16.0)
        .pb(4.0)
        .child(kit::group(words.to_string()).role("heading").level(2))
}

fn note(key: &str, words: &str) -> El {
    kit::ruled(
        format!("fleet-{key}"),
        row().help(words.to_string()).child(
            text(words.to_string())
                .font(kit::ui_style())
                .color(kit::ink())
                .grow(1.0)
                .shrink()
                .wrap(),
        ),
    )
}

fn cell(words: String, width: f32, good: bool) -> El {
    let color = if good { kit::forest() } else { kit::ink() };
    text(words)
        .font(kit::ui_style())
        .color(color)
        .w(width)
        .truncate()
}

fn mac_row(app: &App, mac: &MachineEntry) -> El {
    let this = app.fleet.this_mac.as_deref() == Some(mac.id.as_str());
    let mut name = row()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(9.0)
        .center_y()
        .child(tile("", &mac.name))
        .child(kit::label(mac.name.clone()).shrink().truncate());
    if this {
        name = name.child(text("This Mac").font(kit::ui_style()).color(kit::dim()));
    }
    let (status, good) = status_words(mac, app.fleet.now);
    kit::ruled(
        format!("mac-{}", mac.id),
        row()
            .child(name)
            .child(cell(mac.machine_type.clone(), TYPE_COLUMN, false))
            .child(cell(
                seen_words(mac.last_seen, app.fleet.now),
                SEEN_COLUMN,
                false,
            ))
            .child(cell(status, STATUS_COLUMN, good)),
    )
}

fn holds_words(entry: &TypeEntry) -> String {
    let config = &entry.config;
    let apps = config.items.len() + config.packages.formulae.len() + config.packages.casks.len();
    format!("{apps} items, {} settings", config.settings.len())
}

fn used_words(count: usize) -> String {
    match count {
        0 => "No Macs yet".to_string(),
        1 => "1 Mac".to_string(),
        n => format!("{n} Macs"),
    }
}

fn type_row(app: &App, entry: &TypeEntry) -> El {
    let name = entry.config.name.clone();
    let here = app.fleet.follows() == Some(name.as_str());
    let edit = {
        let entry = entry.clone();
        kit::action(
            format!("edit-{name}"),
            "Edit",
            false,
            move |app: &mut App, _| app.edit_type(&entry),
        )
    };
    let follow = if here {
        text("On this Mac").font(kit::ui_style()).color(kit::dim())
    } else {
        let entry = entry.clone();
        kit::action(
            format!("use-{name}"),
            "Use on this Mac",
            false,
            move |app: &mut App, _| app.use_type_here(&entry),
        )
    };
    kit::ruled(
        format!("type-{name}"),
        row()
            .child(
                row()
                    .grow(1.0)
                    .shrink()
                    .min_w(0.0)
                    .gap(9.0)
                    .center_y()
                    .child(tile("", &name))
                    .child(kit::label(name.clone()).shrink().truncate()),
            )
            .child(cell(holds_words(entry), TYPE_COLUMN, false))
            .child(cell(
                used_words(app.fleet.users_of(&name)),
                SEEN_COLUMN,
                false,
            ))
            .child(
                row()
                    .w(STATUS_COLUMN)
                    .gap(14.0)
                    .center_y()
                    .child(edit)
                    .child(follow),
            ),
    )
}

fn body(app: &App) -> El {
    let view = &app.fleet;
    let mut out = col().key("fleet-body").w_full();
    if let Some(why) = view.waiting {
        out = out.child(note("waiting", waiting_words(why)));
    }
    if view.nothing_yet {
        return out.child(note(
            "empty",
            "No Machine Types yet. Make one from what's on this Mac, or start from the Catalog.",
        ));
    }
    let Some(fleet) = &view.fleet else {
        if view.loading {
            out = out.child(note("loading", "Looking for your Macs."));
        }
        return out;
    };
    if fleet.public {
        out = out.child(note("public", WaitReason::Public.words()));
    }
    out = out.child(heading("Macs"));
    if fleet.machines.is_empty() {
        out = out.child(note("no-macs", "No Macs have listed themselves yet."));
    }
    // This Mac first, then the rest in name order.
    let is_this = |mac: &&MachineEntry| view.this_mac.as_deref() == Some(mac.id.as_str());
    let (this, others): (Vec<_>, Vec<_>) = fleet.machines.iter().partition(is_this);
    for mac in this.into_iter().chain(others) {
        out = out.child(mac_row(app, mac));
    }
    out = out.child(heading("Machine Types"));
    for entry in &fleet.types {
        out = out.child(type_row(app, entry));
    }
    if fleet.unreadable > 0 {
        let s = if fleet.unreadable == 1 { "" } else { "s" };
        out = out.child(note(
            "unreadable",
            &format!(
                "{} file{s} in your repo couldn't be read.",
                fleet.unreadable
            ),
        ));
    }
    out
}

pub fn screen(app: &App) -> El {
    let running = app.run_started && !app.run_finished;
    let survey: El = if app.fleet.surveying {
        kit::label("Looking at this Mac")
    } else if running {
        row()
            .key("fleet-survey")
            .help("Only one thing can change this Mac at a time.")
            .child(
                text("Make one from this Mac")
                    .font(kit::ui_style())
                    .color(kit::dim()),
            )
    } else {
        kit::action(
            "fleet-survey".to_string(),
            "Make one from this Mac",
            false,
            |app: &mut App, _| app.start_survey(),
        )
    };
    let right = row().gap(14.0).center_y().child(survey).child(kit::primary(
        "fleet-new",
        "New Machine Type",
        true,
        |app: &mut App, _| app.new_type(),
    ));
    col()
        .key("fleet-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Fleet")))
        .child(
            col()
                .key("fleet-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .mt(6.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body(app))
                .child(el().h(12.0)),
        )
        .child(kit::foot(
            "fleet-footer",
            kit::action(
                "fleet-refresh".to_string(),
                "Refresh",
                false,
                |app: &mut App, _| app.refresh_fleet(),
            ),
            right,
        ))
}
