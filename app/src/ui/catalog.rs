//! Catalog: checkboxes over the shipped Catalog, a search and a running count.

use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, Select, TextInput};
use soulcaster_core::config::SettingValue;
use soulcaster_core::recipe::{EvidenceGroup, ValueType};

use super::kit;
use crate::catalog_view::view;
use crate::state::{ALWAYS_SECTION, App, CatRow, Screen};

const SIDEBAR: f32 = 188.0;

pub fn screen(app: &App, k: &'static Kit, layer: &mut Vec<El>) -> El {
    row()
        .key("catalog-screen")
        .w_full()
        .h_full()
        .child(sidebar(app))
        .child(el().w(1.0).h_full().fill(kit::ink().alpha(0.22)))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .child(search_bar(app, k))
                .child(list(app))
                .child(overlay_for_select(app, k, layer))
                .child(footer(app)),
        )
}

fn side_row(key: String, words: String, count: usize, on: bool) -> El {
    let style = if on {
        kit::bold_style()
    } else {
        kit::ui_style()
    };
    let mut r = row()
        .key(key)
        .role("button")
        .label(words.clone())
        .w_full()
        .min_h(26.0)
        .px(12.0)
        .py(5.0)
        .gap(8.0)
        .focusable(true)
        .focus_ring(1.5, -1.5, kit::forest())
        .hover(kit::ink().alpha(0.05))
        .child(
            text(words)
                .font(style)
                .color(if on { kit::forest() } else { kit::ink() })
                .grow(1.0)
                .shrink()
                .wrap(),
        );
    if count > 0 {
        r = r.child(text(count.to_string()).font(style).color(if on {
            kit::forest()
        } else {
            kit::ink()
        }));
    }
    r
}

fn sidebar(app: &App) -> El {
    let cat = view();
    let browsing = !app.only_selected && app.search.text.is_empty();
    let mut sections = col().key("sidebar-sections").w_full().py(6.0).child(
        side_row(
            "nav-always".to_string(),
            "Always".to_string(),
            0,
            browsing && app.current_section == ALWAYS_SECTION,
        )
        .on_press_tx(|app: &mut App, tx| app.jump_to_section(ALWAYS_SECTION, tx)),
    );
    for (i, section) in cat.catalog.sections().iter().enumerate() {
        let count = cat
            .items
            .iter()
            .filter(|e| {
                e.listing.as_ref().is_some_and(|l| l.section == section.id)
                    && app.selected.contains(e.id())
            })
            .count();
        sections = sections.child(
            side_row(
                format!("nav-section-{i}"),
                section.name.clone(),
                count,
                browsing && app.current_section == i,
            )
            .on_press_tx(move |app: &mut App, tx| app.jump_to_section(i, tx)),
        );
    }
    sections = sections.child(
        side_row(
            "nav-settings".to_string(),
            "macOS and Git".to_string(),
            app.settings.len(),
            browsing && app.current_section == usize::MAX,
        )
        .on_press_tx(|app: &mut App, tx| app.jump_to_section(usize::MAX, tx)),
    );
    col()
        .key("catalog-sidebar")
        .role("navigation")
        .label("Catalog sections")
        .w(SIDEBAR)
        .h_full()
        .child(
            col()
                .key("sidebar-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(sections),
        )
}

fn search_bar(app: &App, k: &Kit) -> El {
    let input = TextInput::new(&app.search)
        .plain()
        .placeholder("Search")
        .label("Search the Catalog")
        .on_edit(|app: &mut App| &mut app.search)
        .on_change(|app: &mut App, _| app.rebuild_rows())
        .build(k)
        .key("search");
    col()
        .key("search-bar")
        .w_full()
        .child(
            row()
                .w_full()
                .px(6.0)
                .py(4.0)
                .center_y()
                .child(input.grow(1.0)),
        )
        .child(kit::hairline())
}

fn list(app: &App) -> El {
    col()
        .key("catalog-list")
        .role("list")
        .label("Catalog")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("catalog-rows")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    app.rows.len(),
                    32.0,
                    |s: &App, i| Cow::Owned(row_key(s, i)),
                    |s: &App, i| build_row(s, i, kit::theme()),
                )
                .virtual_rows_version(app.rows_version)
                .on_visible_rows(|app: &mut App, range| {
                    let cat = view();
                    let sections = cat.catalog.sections();
                    let top = app
                        .rows
                        .get(range.start..range.end.min(app.rows.len()))
                        .and_then(|rows| {
                            rows.iter().find_map(|r| match r {
                                CatRow::Base(_) => Some(ALWAYS_SECTION),
                                CatRow::Item(i) => sections.iter().position(|s| {
                                    Some(&s.id)
                                        == cat.items[*i].listing.as_ref().map(|l| &l.section)
                                }),
                                CatRow::Section(s) => Some(*s),
                                CatRow::Setting(_) | CatRow::Surface(_) => Some(usize::MAX),
                                _ => None,
                            })
                        });
                    match top {
                        Some(section) if section != app.current_section => {
                            app.current_section = section;
                            true
                        }
                        _ => false,
                    }
                }),
        )
}

fn row_key(app: &App, i: usize) -> String {
    match &app.rows[i] {
        CatRow::Base(b) => format!("row-base-{b}"),
        CatRow::Section(s) => format!("row-section-{s}"),
        CatRow::Group(name) => format!("row-group-{name}-{i}"),
        CatRow::Item(x) => format!("row-item-{}", view().items[*x].id()),
        CatRow::Surface(name) => {
            if app
                .rows
                .iter()
                .position(|r| matches!(r, CatRow::Surface(_)))
                == Some(i)
            {
                "row-surface-first".to_string()
            } else {
                format!("row-surface-{name}")
            }
        }
        CatRow::Setting(x) => format!("row-setting-{}", view().settings[*x].id),
        CatRow::Package(p) => format!("row-package-{p}"),
        CatRow::AddPackage(_) => "row-add-package".to_string(),
        CatRow::Empty => "row-empty".to_string(),
    }
}

fn heading(words: String, role_heading: bool) -> El {
    let t = if role_heading {
        kit::group(words).role("heading").level(2)
    } else {
        text(words).font(kit::ui_style()).color(kit::dim())
    };
    row().w_full().px(14.0).pt(12.0).pb(2.0).child(t)
}

fn build_row(app: &App, i: usize, k: &'static Kit) -> El {
    match &app.rows[i] {
        CatRow::Base(b) => base_row(*b, k),
        CatRow::Section(s) => heading(view().catalog.sections()[*s].name.clone(), true),
        CatRow::Group(name) => heading(name.clone(), false),
        CatRow::Item(x) => item_row(app, *x, k),
        CatRow::Surface(name) => {
            let first = app
                .rows
                .iter()
                .position(|r| matches!(r, CatRow::Surface(_)))
                == Some(i);
            if first {
                col()
                    .w_full()
                    .child(heading("macOS and Git".to_string(), true))
                    .child(heading(name.clone(), false))
            } else {
                heading(name.clone(), false)
            }
        }
        CatRow::Setting(x) => setting_row(app, *x, k),
        CatRow::Package(p) => package_row(p, k),
        CatRow::AddPackage(q) => add_package_row(q),
        CatRow::Empty => row()
            .w_full()
            .px(14.0)
            .h(kit::ROW)
            .center_y()
            .child(kit::label("Nothing selected")),
    }
}

fn base_row(b: usize, k: &Kit) -> El {
    let entry = view().catalog.base_items().nth(b).expect("a Base Item");
    let content = row()
        .child(kit::check_box(k, true, true))
        .child(kit::label(entry.name().to_string()));
    let mut wrapped = col().w_full();
    if b == 0 {
        wrapped = wrapped.child(heading("Always".to_string(), true));
    }
    wrapped.child(kit::ruled(format!("base-{}", entry.id()), content))
}

fn item_row(app: &App, x: usize, k: &Kit) -> El {
    let entry = view().items[x];
    let on = app.selected.contains(entry.id());
    let id = entry.id().to_string();
    let listing = entry.listing.as_ref().expect("a listed Item");
    // Competing Items sit together, and picking two says so on the row.
    let rivals: Vec<String> = view()
        .choice_of(entry.id())
        .map(|choice| {
            choice
                .members
                .iter()
                .filter(|m| *m != entry.id() && app.selected.contains(*m))
                .map(|m| view().name_of(m))
                .collect()
        })
        .unwrap_or_default();
    let mut content = row()
        .role("checkbox")
        .label(entry.name().to_string())
        .description(listing.description.clone())
        .help(listing.description.clone())
        .checked(on)
        .focusable(true)
        .focus_ring(1.5, -1.5, kit::forest())
        .hover(kit::ink().alpha(0.05))
        .on_press(move |app: &mut App| app.toggle_item(&id))
        .child(kit::check_box(k, on, false))
        .child(kit::label(entry.name().to_string()).shrink().truncate())
        .child(el().grow(1.0));
    if on && !rivals.is_empty() {
        content = content.child(
            text(format!("Also {}", rivals.join(", ")))
                .font(kit::ui_style())
                .color(kit::dim())
                .shrink()
                .truncate(),
        );
    }
    kit::ruled(format!("item-{}", entry.id()), content)
}

fn value_control(app: &App, x: usize, k: &'static Kit) -> El {
    let setting = view().settings[x];
    let id = setting.id.clone();
    let on = app.settings.contains_key(&id);
    let value = app.settings.get(&id);
    match setting.value_type {
        ValueType::Boolean => {
            let mut choice = row()
                .key(format!("value-{id}"))
                .role("radiogroup")
                .label(setting.title.clone())
                .gap(8.0);
            for (words, v) in [("On", true), ("Off", false)] {
                let id = id.clone();
                let current = on && value == Some(&SettingValue::Bool(v));
                choice = choice.child(kit::action(
                    format!("value-{id}-{words}"),
                    words,
                    current,
                    move |app: &mut App, _| {
                        app.settings.insert(id.clone(), SettingValue::Bool(v));
                    },
                ));
            }
            choice
        }
        ValueType::Integer => {
            let (lo, hi) = (
                setting.minimum.unwrap_or(i64::MIN),
                setting.maximum.unwrap_or(i64::MAX),
            );
            let n = match value {
                Some(SettingValue::Number(n)) => n.as_i64().unwrap_or(lo),
                _ => setting.minimum.unwrap_or(0),
            };
            let step = |delta: i64, label: &'static str, glyph: &'static rune::Icon| {
                let id = id.clone();
                IconButton::new(glyph)
                    .variant(IconButtonVariant::Ghost)
                    .label(label)
                    .disabled(!on)
                    .on_press(move |app: &mut App| {
                        let now = match app.settings.get(&id) {
                            Some(SettingValue::Number(n)) => n.as_i64().unwrap_or(lo),
                            _ => lo,
                        };
                        app.settings.insert(
                            id.clone(),
                            SettingValue::Number((now + delta).clamp(lo, hi).into()),
                        );
                    })
                    .build(k)
            };
            row()
                .key(format!("value-{id}"))
                .gap(4.0)
                .center_y()
                .child(step(-1, "Less", k.glyph.dash))
                .child(kit::label(n.to_string()))
                .child(step(1, "More", &kit::PLUS))
        }
        ValueType::String if setting.values.is_some() => {
            let values = setting.values.as_deref().unwrap_or_default();
            let open = app.open_select.as_deref() == Some(id.as_str());
            let toggled = id.clone();
            let current = match value {
                Some(SettingValue::String(text)) => Some(text.as_str()),
                _ => values.first().map(String::as_str),
            };
            Select::new(format!("select-{id}"), values)
                .width(150.0)
                .current(current)
                .label(setting.title.clone())
                .open(open)
                .disabled(!on)
                .on_toggle(move |app: &mut App| {
                    app.open_select = if app.open_select.as_deref() == Some(toggled.as_str()) {
                        None
                    } else {
                        Some(toggled.clone())
                    };
                })
                .trigger(k)
        }
        _ => kit::label("In your Config"),
    }
}

/// Selects can't open from inside a virtual row, because a row can't reach the overlay layer. So
/// the open one builds here, against the trigger's key.
fn overlay_for_select(app: &App, k: &'static Kit, layer: &mut Vec<El>) -> El {
    if let Some(id) = &app.open_select
        && let Some(setting) = view().setting(id)
    {
        let choose = id.clone();
        let current = match app.settings.get(id) {
            Some(SettingValue::String(text)) => Some(text.as_str()),
            _ => None,
        };
        let _ = Select::new(
            format!("select-{id}"),
            setting.values.as_deref().unwrap_or_default(),
        )
        .width(150.0)
        .current(current)
        .label(setting.title.clone())
        .open(true)
        .on_close(|app: &mut App| app.open_select = None)
        .on_choose(move |app: &mut App, picked: &str, _| {
            app.settings
                .insert(choose.clone(), SettingValue::String(picked.to_string()));
            app.open_select = None;
        })
        .build(k, layer);
    }
    el().h(0.0)
}

fn setting_row(app: &App, x: usize, k: &'static Kit) -> El {
    let setting = view().settings[x];
    let id = setting.id.clone();
    let on = app.settings.contains_key(&id);
    let broken = setting.evidence.group == EvidenceGroup::Broken;
    let free_form = setting.value_type == ValueType::StringList
        || (setting.value_type == ValueType::String && setting.values.is_none());
    let disabled = broken || free_form;
    let mut toggle = row()
        .key(format!("toggle-{id}"))
        .role("checkbox")
        .label(setting.title.clone())
        .checked(on)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .min_h(kit::ROW)
        .gap(10.0)
        .center_y()
        .disabled(disabled)
        .child(kit::check_box(k, on, false).opacity(if disabled { 0.35 } else { 1.0 }))
        .child(kit::label(setting.title.clone()).shrink().truncate());
    if let Some(note) = setting.notes.first() {
        toggle = toggle.help(note.clone());
    }
    if !disabled {
        let id = id.clone();
        toggle = toggle
            .focusable(true)
            .focus_ring(1.5, -1.5, kit::forest())
            .on_press(move |app: &mut App| app.toggle_setting(&id));
    }
    let content = if broken {
        row().child(toggle).child(kit::label("Doesn't work here"))
    } else {
        row().child(toggle).child(value_control(app, x, k))
    };
    kit::ruled(format!("setting-{id}"), content)
}

fn package_row(package: &str, k: &Kit) -> El {
    let name = package.to_string();
    let content = row()
        .role("checkbox")
        .label(package.to_string())
        .checked(true)
        .focusable(true)
        .focus_ring(1.5, -1.5, kit::forest())
        .hover(kit::ink().alpha(0.05))
        .on_press(move |app: &mut App| {
            app.packages.remove(&name);
            app.rebuild_rows();
        })
        .child(kit::check_box(k, true, false))
        .child(kit::label(package.to_string()));
    kit::ruled(format!("package-{package}"), content)
}

fn add_package_row(query: &str) -> El {
    let name = query.to_string();
    let content = row()
        .child(
            kit::label(format!("\u{201c}{query}\u{201d} from Homebrew"))
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .child(kit::action(
            "add-package".to_string(),
            "Add",
            false,
            move |app: &mut App, _| app.add_package(&name),
        ));
    kit::ruled("add-package-row".to_string(), content)
}

fn footer(app: &App) -> El {
    let words = format!("{} selected", app.selection_count());
    let left = kit::action(
        "selection-count".to_string(),
        &words,
        app.only_selected,
        |app: &mut App, tx| app.set_filter_selected(!app.only_selected, tx),
    );
    kit::foot(
        "catalog-footer",
        left,
        kit::primary("go-review", "Continue", true, |app: &mut App, _| {
            app.screen = Screen::Review
        }),
    )
}
