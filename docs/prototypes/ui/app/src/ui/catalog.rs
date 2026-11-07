//! Choose: browse the Catalog with checkboxes, search, and a running count.

use std::borrow::Cow;

use rune::{Color, El, GrowthSpec, ScrollSpec, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, Select, TextInput};

use super::kit::{self, Tones, icons};
use crate::catalog::{BASE, catalog, surface_name};
use crate::state::{App, CatRow, Screen};

const SIDEBAR: f32 = 252.0;

pub fn screen(app: &App, k: &'static Kit, tones: Tones, layer: &mut Vec<El>) -> El {
    col()
        .key("catalog-screen")
        .w_full()
        .h_full()
        .child(
            row().w_full().grow(1.0).min_h(0.0)
                .child(sidebar(app, k, tones))
                .child(col().grow(1.0).min_w(0.0).h_full().fill(k.c.bg_base).child(search_bar(app, k)).child(list(app, k)).child(overlay_for_select(app, k, layer))),
        )
        .child(footer(app, k, tones))
}

fn sidebar(app: &App, k: &Kit, tones: Tones) -> El {
    let cat = catalog();
    let row_for = |key: String, glyph: &'static rune::Icon, label: String, count: usize, on: bool| {
        let mut r = row()
            .key(key)
            .role("button")
            .label(label.clone())
            .w_full()
            .min_h(28.0)
            .px(8.0)
            .py(4.0)
            .gap(8.0)
            .center_y()
            .radius(6.0)
            .focusable(true)
            .hover(k.c.hover)
            .child(icon(glyph, 15.0).color(if on { tones.accent_ink } else { k.c.icon_muted }))
            .child(text(label).font(if on { k.ty.text_small_medium } else { k.ty.text_small }).color(if on { k.c.text_base } else { k.c.text_muted }).grow(1.0).shrink().wrap());
        if on {
            r = r.fill(k.c.layer_03);
        }
        if count > 0 {
            r = r.child(text(count.to_string()).font(k.ty.text_xsmall_medium).color(tones.accent_ink));
        }
        r
    };
    let browsing = !app.only_selected && app.search.text.is_empty();
    let mut sections = col().key("sidebar-sections").w_full().gap(1.0);
    sections = sections.child(
        row_for("nav-selected".to_string(), &icons::LIST, "Your selection".to_string(), app.selection_count(), app.only_selected)
            .on_press_tx(|app: &mut App, tx| app.set_filter_selected(!app.only_selected, tx)),
    );
    sections = sections.child(el().h(10.0));
    sections = sections.child(text("APPS AND TOOLS").font(k.ty.text_xsmall_medium).color(k.c.text_faint).px(8.0).pb(4.0));
    for (i, section) in cat.sections.iter().enumerate() {
        let count = cat.items.iter().filter(|x| x.section == section.id && app.selected.contains(&x.id)).count();
        sections = sections.child(
            row_for(format!("nav-section-{i}"), kit::section_icon(&section.id), section.name.clone(), count, browsing && app.current_section == i)
                .on_press_tx(move |app: &mut App, tx| app.jump_to_section(i, tx)),
        );
    }
    sections = sections.child(el().h(10.0));
    sections = sections.child(text("SETTINGS").font(k.ty.text_xsmall_medium).color(k.c.text_faint).px(8.0).pb(4.0));
    sections = sections.child(
        row_for("nav-settings".to_string(), &icons::SLIDERS, "macOS and Git".to_string(), app.settings.len(), browsing && app.current_section == usize::MAX)
            .on_press_tx(|app: &mut App, tx| app.jump_to_section(usize::MAX, tx)),
    );
    col()
        .key("catalog-sidebar")
        .role("navigation")
        .label("Catalog sections")
        .w(SIDEBAR)
        .h_full()
        .fill(k.c.bg_deep)
        .child(col().key("sidebar-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(sections.padding(10.0)))
}

fn search_bar(app: &App, k: &Kit) -> El {
    let input = TextInput::new(&app.search)
        .placeholder("Search apps, tools and settings")
        .label("Search the Catalog")
        .leading_icon(k.glyph.search)
        .clear_button()
        .on_edit(|app: &mut App| &mut app.search)
        .on_change(|app: &mut App, _| app.rebuild_rows())
        .build(k)
        .key("search");
    row().key("search-bar").w_full().px(16.0).py(10.0).gap(10.0).center_y().child(input.grow(1.0))
}

fn list(app: &App, _k: &Kit) -> El {
    col()
        .key("catalog-list")
        .role("list")
        .label("Catalog")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(
            col()
                .key("catalog-rows")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    app.rows.len(),
                    50.0,
                    |s: &App, i| Cow::Owned(row_key(s, i)),
                    |s: &App, i| {
                        let (k, tones) = kit::theme_now();
                        build_row(s, i, k, tones)
                    },
                )
                .virtual_rows_version(app.rows_version)
                .on_visible_rows(|app: &mut App, range| {
                    let cat = catalog();
                    let top = app.rows.get(range.start..range.end.min(app.rows.len())).and_then(|rows| {
                        rows.iter().find_map(|r| match r {
                            CatRow::Item(i) => cat.sections.iter().position(|s| s.id == cat.items[*i].section),
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
        CatRow::Base => "row-base".to_string(),
        CatRow::Section(s) => format!("row-section-{s}"),
        CatRow::Group(name) => format!("row-group-{name}-{i}"),
        CatRow::Choice(c) => format!("row-choice-{c}"),
        CatRow::Item(x) => format!("row-item-{}", catalog().items[*x].id),
        CatRow::Surface(name) => {
            if app.rows.iter().position(|r| matches!(r, CatRow::Surface(_))) == Some(i) {
                "row-surface-first".to_string()
            } else {
                format!("row-surface-{name}")
            }
        }
        CatRow::Setting(x) => format!("row-setting-{}", catalog().settings[*x].id),
        CatRow::Package(p) => format!("row-package-{p}"),
        CatRow::AddPackage(_) => "row-add-package".to_string(),
        CatRow::Empty => "row-empty".to_string(),
    }
}

fn build_row(app: &App, i: usize, k: &'static Kit, tones: Tones) -> El {
    match &app.rows[i] {
        CatRow::Base => base_row(k, tones),
        CatRow::Section(s) => {
            let section = &catalog().sections[*s];
            col().w_full().px(16.0).pt(20.0).pb(4.0).gap(2.0).child(
                row().gap(8.0).center_y()
                    .child(icon(kit::section_icon(&section.id), 16.0).color(k.c.icon_muted))
                    .child(text(section.name.clone()).font(rune::TextStyle::new(15.0, 600.0)).color(k.c.text_strong).level(2).role("heading")),
            )
        }
        CatRow::Group(name) => row().w_full().px(16.0).pt(10.0).pb(2.0).child(text(name.clone()).font(k.ty.text_small_medium).color(k.c.text_muted)),
        CatRow::Choice(c) => choice_row(app, *c, k, tones),
        CatRow::Item(x) => item_row(app, *x, k, tones),
        CatRow::Surface(name) => {
            let first = app.rows.iter().position(|r| matches!(r, CatRow::Surface(_))) == Some(i);
            let mut c = col().w_full().px(16.0).pt(if first { 20.0 } else { 10.0 }).pb(2.0).gap(3.0);
            if first {
                c = c.child(
                    row().gap(8.0).center_y()
                        .child(icon(&icons::SLIDERS, 16.0).color(k.c.icon_muted))
                        .child(text("macOS and Git settings").font(rune::TextStyle::new(15.0, 600.0)).color(k.c.text_strong).level(2).role("heading")),
                );
                c = c.child(text("Pick the settings you want Soulcaster to set. Anything you leave out stays as it is.").font(k.ty.text_xsmall).color(k.c.text_muted).pb(6.0));
            }
            c.child(text(name.clone()).font(k.ty.text_small_medium).color(k.c.text_muted))
        }
        CatRow::Setting(x) => setting_row(app, *x, k, tones),
        CatRow::Package(p) => package_row(p, k, tones),
        CatRow::AddPackage(q) => add_package_row(q, k, tones),
        CatRow::Empty => col().w_full().padding(40.0).center().child(text("Nothing selected yet. Tick anything in the list to add it.").font(k.ty.text_small).color(k.c.text_muted)),
    }
}

fn base_row(k: &Kit, tones: Tones) -> El {
    let mut names = row().w_full().gap(16.0).flex_wrap();
    for base in BASE {
        names = names.child(
            row().gap(8.0).center_y().child(kit::check_mark(k, tones, true, true))
                .child(col().child(text(base.name).font(k.ty.text_small_medium).color(k.c.text_base)).child(text(base.why).font(k.ty.text_xsmall).color(k.c.text_muted))),
        );
    }
    col().w_full().px(16.0).pt(8.0).child(
        kit::card(k).key("base-card").padding(12.0).gap(10.0)
            .child(row().gap(6.0).center_y().child(icon(&icons::LOCK_CLOSED, 13.0).color(k.c.icon_muted)).child(text("Base Items").font(k.ty.text_small_medium).color(k.c.text_base)).child(text("Every Run needs these three. A Config can't select or omit them.").font(k.ty.text_xsmall).color(k.c.text_muted)))
            .child(names),
    )
}

fn choice_row(app: &App, c: usize, k: &Kit, tones: Tones) -> El {
    let choice = &catalog().choices[c];
    let picked: Vec<String> = choice.members.iter().filter(|m| app.selected.contains(*m)).map(|m| catalog().name_of(m)).collect();
    let clash = picked.len() > 1;
    let (fill, glyph, glyph_color) = if clash { (tones.warn_soft, &icons::WARN, tones.warn) } else { (k.c.layer_02, &icons::INFO, k.c.icon_muted) };
    let mut body = col().grow(1.0).shrink().gap(2.0)
        .child(text(choice.title.clone()).font(k.ty.text_small_medium).color(k.c.text_base))
        .child(text(choice.advice.clone()).font(k.ty.text_xsmall.lh(16.0)).color(k.c.text_muted).w_full().wrap());
    if clash {
        body = body.child(text(format!("You picked {}. Both will install, and they do the same job.", picked.join(" and "))).font(k.ty.text_xsmall_medium).color(tones.warn_ink).w_full().wrap());
    }
    col().w_full().px(16.0).pt(6.0).pb(2.0).child(
        row().w_full().padding(10.0).gap(10.0).items_start().radius(8.0).fill(fill).child(icon(glyph, 15.0).color(glyph_color).mt(1.0)).child(body),
    )
}

/// Everything about an Item that isn't free and unattended goes here, so it's visible while
/// scanning, not only after selecting it.
fn item_tags(item: &crate::catalog::Item, k: &Kit, tones: Tones) -> Vec<El> {
    let cat = catalog();
    let mut tags = Vec::new();
    if let Some(tag) = kit::price_tag(k, tones, cat.price(item)) {
        tags.push(tag);
    }
    let has = |kind: &str| item.needs_from_person.iter().any(|n| n.kind == kind);
    if has("apple-account") {
        tags.push(
            row().h(20.0).px(7.0).gap(4.0).radius(5.0).center().fill(k.c.layer_03).child(icon(&icons::APPLE, 11.0).color(k.c.icon_base)).child(text("Needs Apple Account").font(k.ty.text_xsmall_medium).color(k.c.text_base)),
        );
    }
    if has("license") {
        tags.push(kit::tag(k, "License to accept", k.c.layer_03, k.c.text_muted));
    }
    if item.tap.is_some() {
        tags.push(kit::tag(k, "Third-party source", k.c.layer_03, k.c.text_muted));
    }
    if has("admin-password") {
        tags.push(row().size(20.0, 20.0).radius(5.0).center().fill(k.c.layer_03).help("Needs your Mac password to install").label("Needs your Mac password").child(icon(&icons::KEY, 12.0).color(k.c.icon_muted)));
    }
    tags
}

fn item_row(app: &App, x: usize, k: &Kit, tones: Tones) -> El {
    let cat = catalog();
    let item = &cat.items[x];
    let on = app.selected.contains(&item.id);
    let id = item.id.clone();
    let mut tags = row().gap(5.0).center_y();
    for tag in item_tags(item, k, tones) {
        tags = tags.child(tag);
    }
    // A hard conflict shows the moment both sides are ticked, and Review asks which one wins.
    let clash: Vec<String> = cat.conflicts_of(item).iter().filter(|(other, _)| app.selected.contains(&other.id)).map(|(other, _)| other.name.clone()).collect();
    let mut words = col().grow(1.0).shrink().min_w(0.0).gap(1.0)
        .child(text(item.name.clone()).font(k.ty.text_small_medium).color(k.c.text_base).truncate())
        .child(text(item.description.clone()).font(k.ty.text_xsmall).color(k.c.text_muted).truncate());
    if on && !clash.is_empty() {
        words = words.child(text(format!("Can't sit beside {}. You'll pick one on the next step.", clash.join(", "))).font(k.ty.text_xsmall_medium).color(tones.bad));
    }
    row()
        .key(format!("item-{}", item.id))
        .role("checkbox")
        .label(item.name.clone())
        .description(item.description.clone())
        .checked(on)
        .focusable(true)
        .focus_ring(2.0, -2.0, tones.accent)
        .w_full()
        .min_h(50.0)
        .px(16.0)
        .py(7.0)
        .gap(11.0)
        .center_y()
        .hover(k.c.hover)
        .on_press(move |app: &mut App| app.toggle_item(&id))
        .child(kit::check_mark(k, tones, on, false))
        .child(kit::tile(k, &item.name, 30.0))
        .child(words)
        .child(tags)
}

fn value_control(app: &App, x: usize, k: &'static Kit, tones: Tones) -> El {
    let setting = &catalog().settings[x];
    let id = setting.id.clone();
    let on = app.settings.contains_key(&id);
    let value = app.settings.get(&id).cloned();
    match setting.value_type.as_str() {
        "boolean" => {
            let current = value.clone().unwrap_or_default();
            let mut seg = row().key(format!("value-{id}")).role("radiogroup").label(setting.title.clone()).gap(2.0).padding(2.0).radius(6.0).fill(k.c.layer_02);
            for (label, v) in [("On", "true"), ("Off", "false")] {
                let id = id.clone();
                seg = seg.child(kit::mini_seg(k, tones, format!("value-{id}-{v}"), label, on && current == v, on, move |app: &mut App| {
                    app.settings.insert(id.clone(), v.to_string());
                }));
            }
            if on { seg } else { seg.opacity(0.45) }
        }
        "integer" => {
            let n: i64 = value.as_deref().and_then(|v| v.parse().ok()).unwrap_or(setting.minimum.unwrap_or(0));
            let (lo, hi) = (setting.minimum.unwrap_or(i64::MIN), setting.maximum.unwrap_or(i64::MAX));
            let step = |delta: i64, label: &'static str, glyph: &'static rune::Icon| {
                let id = id.clone();
                IconButton::new(glyph).variant(IconButtonVariant::Ghost).label(label).disabled(!on).on_press(move |app: &mut App| {
                    let now: i64 = app.settings.get(&id).and_then(|v| v.parse().ok()).unwrap_or(lo);
                    app.settings.insert(id.clone(), (now + delta).clamp(lo, hi).to_string());
                }).build(k)
            };
            row().key(format!("value-{id}")).gap(4.0).center_y().opacity(if on { 1.0 } else { 0.45 })
                .child(step(-1, "Less", k.glyph.dash))
                .child(text(n.to_string()).font(k.ty.text_small_medium).color(k.c.text_base).w(28.0))
                .child(step(1, "More", &icons::CHEVRON))
        }
        _ if !setting.values.is_empty() => {
            let open = app.open_select.as_deref() == Some(id.as_str());
            let (a, b) = (id.clone(), id.clone());
            let _ = b;
            Select::new(format!("select-{id}"), &setting.values)
                .width(170.0)
                .current(value.as_deref().or(setting.values.first().map(String::as_str)))
                .label(setting.title.clone())
                .open(open)
                .disabled(!on)
                .on_toggle(move |app: &mut App| {
                    app.open_select = if app.open_select.as_deref() == Some(a.as_str()) { None } else { Some(a.clone()) };
                })
                .trigger(k)
        }
        _ => {
            let edit = app.text_values.get(&id);
            let shown = edit.map(|e| e.text.clone()).or(value).unwrap_or_default();
            row().key(format!("value-{id}")).h(26.0).w(170.0).px(8.0).center_y().radius(6.0).fill(k.c.layer_02).opacity(if on { 1.0 } else { 0.45 })
                .child(text(if shown.is_empty() { "Set in your Config".to_string() } else { shown }).font(k.ty.text_xsmall).color(k.c.text_muted).truncate())
        }
    }
}

/// Selects can't open from inside a virtual row, because a row can't reach the overlay layer.
/// So the open one builds here, against the trigger's key.
fn overlay_for_select(app: &App, k: &'static Kit, layer: &mut Vec<El>) -> El {
    if let Some(id) = &app.open_select {
        if let Some(setting) = catalog().setting(id) {
            let (close, choose) = (id.clone(), id.clone());
            let _ = close;
            let value = app.settings.get(id).cloned();
            let _ = Select::new(format!("select-{id}"), &setting.values)
                .width(170.0)
                .current(value.as_deref())
                .label(setting.title.clone())
                .open(true)
                .on_close(|app: &mut App| app.open_select = None)
                .on_choose(move |app: &mut App, picked: &str, _| {
                    app.settings.insert(choose.clone(), picked.to_string());
                    app.open_select = None;
                })
                .build(k, layer);
        }
    }
    el().h(0.0)
}

fn setting_row(app: &App, x: usize, k: &'static Kit, tones: Tones) -> El {
    let setting = &catalog().settings[x];
    let id = setting.id.clone();
    let on = app.settings.contains_key(&id);
    let broken = setting.evidence.group == "broken";
    let list = setting.value_type == "string-list";
    let mut sub = if broken {
        setting.notes.first().map_or("Soulcaster can't set this on this macOS.".to_string(), |n| n.trim_start_matches("Skipped: ").to_string())
    } else if setting.evidence.group == "unconfirmed" {
        "Soulcaster can store this, but nobody has confirmed macOS still honors it.".to_string()
    } else if list {
        "A list. In this prototype, edit it in your Config file.".to_string()
    } else {
        String::new()
    };
    if !setting.requires.is_empty() && !broken && setting.requires.iter().any(|r| r != "command-line-tools") {
        let names: Vec<String> = setting.requires.iter().filter(|r| *r != "command-line-tools").map(|r| catalog().name_of(r)).collect();
        sub = format!("Also installs {}. {sub}", names.join(", "));
    }
    let mut words = col().grow(1.0).shrink().min_w(0.0).gap(1.0).child(text(setting.title.clone()).font(k.ty.text_small_medium).color(if broken { k.c.text_muted } else { k.c.text_base }).truncate());
    if !sub.is_empty() {
        words = words.child(text(sub).font(k.ty.text_xsmall).color(k.c.text_muted).truncate());
    }
    let disabled = broken || list;
    let mut r = row()
        .key(format!("setting-{id}"))
        .w_full()
        .min_h(44.0)
        .px(16.0)
        .py(5.0)
        .gap(11.0)
        .center_y();
    let mut toggle = row()
        .key(format!("toggle-{id}"))
        .role("checkbox")
        .label(setting.title.clone())
        .checked(on)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(11.0)
        .center_y()
        .disabled(disabled)
        .child(kit::check_mark(k, tones, on, false).opacity(if disabled { 0.35 } else { 1.0 }))
        .child(words);
    if !disabled {
        let id = id.clone();
        toggle = toggle.focusable(true).on_press(move |app: &mut App| app.toggle_setting(&id));
        r = r.hover(k.c.hover);
    }
    r = r.child(toggle);
    if broken {
        r = r.child(kit::tag(k, "Doesn't work here", k.c.layer_03, k.c.text_muted));
    } else if !list {
        r = r.child(value_control(app, x, k, tones));
    }
    r
}

fn package_row(package: &str, k: &Kit, tones: Tones) -> El {
    let (p, q) = (package.to_string(), package.to_string());
    row().key(format!("package-{package}")).role("checkbox").label(package.to_string()).checked(true).focusable(true).w_full().min_h(44.0).px(16.0).gap(11.0).center_y().hover(k.c.hover)
        .on_press(move |app: &mut App| {
            app.packages.remove(&p);
            app.rebuild_rows();
        })
        .child(kit::check_mark(k, tones, true, false))
        .child(kit::tile(k, &q, 30.0))
        .child(col().child(text(q).font(k.ty.text_small_medium).color(k.c.text_base)).child(text("A Homebrew package you added by name").font(k.ty.text_xsmall).color(k.c.text_muted)))
}

fn add_package_row(query: &str, k: &Kit, tones: Tones) -> El {
    let name = query.to_string();
    row().key("add-package").role("button").label(format!("Add {query} as a Homebrew package")).focusable(true).w_full().min_h(44.0).px(16.0).gap(11.0).center_y().hover(k.c.hover)
        .on_press(move |app: &mut App| {
            app.packages.insert(name.clone());
            app.search.clear();
            app.rebuild_rows();
        })
        .child(icon(&icons::CHEVRON, 14.0).color(tones.accent_ink))
        .child(text(format!("Not what you're after? Add \u{201c}{query}\u{201d} as a Homebrew package")).font(k.ty.text_small).color(tones.accent_ink))
}

fn footer(app: &App, k: &Kit, tones: Tones) -> El {
    let apps = app.selected.len() + app.packages.len();
    let settings = app.settings.len();
    let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let headline = if apps + settings == 0 {
        "Nothing selected yet".to_string()
    } else {
        format!("{} and {}", plural(apps, "app or tool", "apps and tools"), plural(settings, "setting", "settings"))
    };
    let plan = app.plan();
    let waits = plan.waiting().count();
    let sub = format!(
        "Plus {} Base Items{}{}. Nothing else gets installed.",
        BASE.len(),
        if plan.count_where(|e| matches!(e.why, crate::plan::Why::PulledIn { .. })) > 0 { ", and what your picks need" } else { "" },
        if waits > 0 { format!(", {} will need you at the end", waits) } else { String::new() }
    );
    row()
        .key("catalog-footer")
        .w_full()
        .h(64.0)
        .px(16.0)
        .gap(14.0)
        .center_y()
        .fill(k.c.bg_base)
        .child(
            col().grow(1.0).shrink().min_w(0.0).gap(1.0)
                .child(text(headline).key("selection-count").font(k.ty.body_medium).color(k.c.text_strong).live_region(rune::LiveRegion::Polite))
                .child(text(sub).font(k.ty.text_xsmall).color(k.c.text_muted).truncate()),
        )
        .child(kit::primary(k, tones, "go-review", "Continue", true, |app: &mut App, _| app.go_to_review()))
}

#[allow(dead_code)]
fn unused(_: Screen, _: Color, _: &str) -> &'static str {
    surface_name("")
}
