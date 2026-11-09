//! Catalog: checkboxes over the shipped Catalog, a search and a running count.

use macflayer_core::config::SettingValue;
use macflayer_core::recipe::{EvidenceGroup, ValueType};
use rune::color::hex;
use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, Select, TextInput};

use super::kit;
use crate::brand_icons::{self, Brand};
use crate::catalog_view::view;
use crate::state::{ALWAYS_SECTION, App, CatRow, PACKAGES_SECTION, SETTINGS_SECTION, Screen};

const SIDEBAR: f32 = 188.0;
/// One row in a column, without its rule.
const CELL: f32 = 32.0;
const HEADING: f32 = 28.0;
const PAD: f32 = 14.0;
const COLUMN_GAP: f32 = 22.0;
const MIN_COLUMN: f32 = 230.0;
/// A setting needs room for its value beside its name.
const MIN_SETTINGS_COLUMN: f32 = 340.0;
const MAX_COLUMNS: usize = 5;
const TILE: f32 = 24.0;
const TILE_GLYPH: f32 = 15.0;
const HEADING_TILE: f32 = 18.0;
const HEADING_GLYPH: f32 = 11.0;

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
    let browsing = app.browsing();
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
            browsing && app.current_section == SETTINGS_SECTION,
        )
        .on_press_tx(|app: &mut App, tx| app.jump_to_section(SETTINGS_SECTION, tx)),
    );
    if !app.packages.is_empty() {
        sections = sections.child(
            side_row(
                "nav-packages".to_string(),
                "Added from Homebrew".to_string(),
                app.packages.len(),
                browsing && app.current_section == PACKAGES_SECTION,
            )
            .on_press_tx(|app: &mut App, tx| app.jump_to_section(PACKAGES_SECTION, tx)),
        );
    }
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
    let name = TextInput::new(&app.name_edit)
        .plain()
        .placeholder("Name it")
        .label("Machine Type name")
        .on_edit(|app: &mut App| &mut app.name_edit)
        .on_change(|app: &mut App, _| app.name_edited())
        .build(k)
        .key("type-name");
    col()
        .key("search-bar")
        .w_full()
        .child(
            row()
                .w_full()
                .pl(PAD)
                .pr(6.0)
                .py(4.0)
                .gap(8.0)
                .center_y()
                .child(kit::group("Machine Type"))
                .child(name.w(190.0))
                .child(el().w(1.0).h(18.0).fill(kit::ink().alpha(0.22)))
                .child(input.grow(1.0)),
        )
        .child(kit::hairline())
}

/// A run of rows that stay together under one heading.
#[derive(Debug, PartialEq)]
struct Group {
    heading: Option<String>,
    rows: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Items,
    Settings,
    /// One message across the whole width.
    Single,
}

/// Groups laid out together, under a category's name when a search shows several categories.
#[derive(Debug, PartialEq)]
struct Block {
    title: Option<String>,
    shape: Shape,
    groups: Vec<Group>,
}

impl Block {
    fn new(title: Option<String>, shape: Shape) -> Self {
        Block {
            title,
            shape,
            groups: Vec::new(),
        }
    }

    fn open_group(&mut self, heading: &str) -> &mut Group {
        self.groups.push(Group {
            heading: Some(heading.to_string()),
            rows: Vec::new(),
        });
        self.groups.last_mut().expect("just pushed")
    }
}

/// Reads the flat rows into blocks of groups. Rows with no heading are groups of one, so a
/// category with no groups (Browsers, Fonts) still spreads across the columns.
fn blocks(rows: &[CatRow], browsing: bool) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for (i, row_data) in rows.iter().enumerate() {
        match row_data {
            CatRow::Base(_) => {
                if i == 0 {
                    let mut block = Block::new(None, Shape::Items);
                    block.open_group("Always");
                    out.push(block);
                }
                push_row(&mut out, i);
            }
            CatRow::Section(s) => {
                let name = view().catalog.sections()[*s].name.clone();
                out.push(Block::new((!browsing).then_some(name), Shape::Items));
            }
            CatRow::Group(name) => {
                if let Some(block) = out.last_mut() {
                    block.open_group(name);
                }
            }
            CatRow::Item(_) => match out.last_mut() {
                Some(block) if block.groups.last().is_some_and(|g| g.heading.is_some()) => {
                    push_row(&mut out, i);
                }
                Some(block) => block.groups.push(Group {
                    heading: None,
                    rows: vec![i],
                }),
                None => {}
            },
            CatRow::Surface(name) => {
                if out.last().is_none_or(|b| b.shape != Shape::Settings) {
                    let title = (!browsing).then(|| "macOS and Git".to_string());
                    out.push(Block::new(title, Shape::Settings));
                }
                if let Some(block) = out.last_mut() {
                    block.open_group(name);
                }
            }
            CatRow::Setting(_) => push_row(&mut out, i),
            CatRow::Package(_) => {
                if i == 0 || !matches!(rows[i - 1], CatRow::Package(_)) {
                    let mut block = Block::new(None, Shape::Items);
                    block.open_group("Added from Homebrew");
                    out.push(block);
                }
                push_row(&mut out, i);
            }
            CatRow::AddPackage(_) | CatRow::Empty => {
                let mut block = Block::new(None, Shape::Single);
                block.groups.push(Group {
                    heading: None,
                    rows: vec![i],
                });
                out.push(block);
            }
        }
    }
    // A group of competing Items leaves the heading it was listed under with nothing in it.
    for block in &mut out {
        block.groups.retain(|group| !group.rows.is_empty());
    }
    out.retain(|block| !block.groups.is_empty());
    out
}

fn push_row(out: &mut [Block], row: usize) {
    if let Some(group) = out.last_mut().and_then(|b| b.groups.last_mut()) {
        group.rows.push(row);
    }
}

/// How many columns of at least `min` points fit in `width`.
fn columns_for(width: f32, min: f32) -> usize {
    let usable = width - 2.0 * PAD + COLUMN_GAP;
    ((usable / (min + COLUMN_GAP)).floor() as usize).clamp(1, MAX_COLUMNS)
}

fn group_height(group: &Group) -> f32 {
    let heading = if group.heading.is_some() {
        HEADING
    } else {
        0.0
    };
    heading + group.rows.len() as f32 * (CELL + 1.0)
}

/// Deals groups into columns in reading order, so a language stays next to its managers, with the
/// tallest column as short as it can be. Groups are never split.
fn place(heights: &[f32], columns: usize) -> Vec<Vec<usize>> {
    let (mut too_short, mut tall_enough) = (
        heights.iter().copied().fold(0.0, f32::max) - 1.0,
        heights.iter().sum::<f32>() + 1.0,
    );
    // Halving 32 times takes this far below a point, for any height a screen could have.
    for _ in 0..32 {
        let limit = (too_short + tall_enough) / 2.0;
        if fill_columns(heights, usize::MAX, limit).len() <= columns {
            tall_enough = limit;
        } else {
            too_short = limit;
        }
    }
    let mut placed = fill_columns(heights, columns, tall_enough);
    placed.resize(columns, Vec::new());
    placed
}

/// Fills each column up to `limit` before starting the next, and the last column takes the rest.
fn fill_columns(heights: &[f32], columns: usize, limit: f32) -> Vec<Vec<usize>> {
    let mut placed: Vec<Vec<usize>> = vec![Vec::new()];
    let mut filled = 0.0;
    for (i, &height) in heights.iter().enumerate() {
        if filled + height > limit && filled > 0.0 && placed.len() < columns {
            placed.push(Vec::new());
            filled = 0.0;
        }
        placed.last_mut().expect("starts with one").push(i);
        filled += height;
    }
    placed
}

fn list(app: &App) -> El {
    let k = kit::theme();
    let mut content = col().key("catalog-rows").w_full().px(PAD).pb(PAD);
    for (b, block) in blocks(&app.rows, app.browsing()).iter().enumerate() {
        content = content.child(block_el(app, b, block, k));
    }
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
        .on_layout(|app: &mut App, [_, _, width, _]| app.grid_width = width)
        .child(content)
}

fn block_el(app: &App, b: usize, block: &Block, k: &'static Kit) -> El {
    let columns = match block.shape {
        Shape::Single => 1,
        Shape::Settings => columns_for(app.grid_width, MIN_SETTINGS_COLUMN),
        Shape::Items => columns_for(app.grid_width, MIN_COLUMN),
    };
    let heights: Vec<f32> = block.groups.iter().map(group_height).collect();
    let mut grid = row()
        .key(format!("grid-{b}"))
        .w_full()
        .equal_columns(columns)
        .gap(COLUMN_GAP);
    for (c, groups) in place(&heights, columns).into_iter().enumerate() {
        let mut column = col().key(format!("grid-{b}-{c}")).w_full().min_w(0.0);
        for g in groups {
            column = column.child(group_el(app, &block.groups[g], k));
        }
        grid = grid.child(column);
    }
    let mut out = col().w_full();
    if let Some(title) = &block.title {
        out = out
            .child(
                row()
                    .w_full()
                    .pt(14.0)
                    .pb(6.0)
                    .child(kit::group(title.clone()).role("heading").level(2)),
            )
            .child(kit::hairline());
    }
    out.child(grid)
}

fn group_el(app: &App, group: &Group, k: &'static Kit) -> El {
    let mut out = col().w_full().min_w(0.0);
    if let Some(words) = &group.heading {
        out = out.child(heading(words));
    }
    for &i in &group.rows {
        out = out.child(build_row(app, i, k));
    }
    out
}

fn heading(words: &str) -> El {
    let mut r = row().w_full().h(HEADING).gap(7.0).items_end().pb(4.0);
    if let Some(brand) = brand_icons::for_group(words) {
        r = r.child(brand_tile(brand, HEADING_TILE, HEADING_GLYPH));
    }
    r.child(
        kit::group(words.to_string())
            .role("heading")
            .level(3)
            .shrink()
            .truncate(),
    )
}

fn brand_tile(brand: &Brand, size: f32, glyph: f32) -> El {
    let fill = hex(brand.tile).expect("a generated color");
    let tint = if brand.light_glyph {
        kit::paper()
    } else {
        kit::ink()
    };
    el().size(size, size)
        .radius(size / 4.0)
        .center()
        .fill(fill)
        .a11y_hidden()
        .child(icon(brand.icon, glyph).color(tint))
}

/// An Item's logo, or its first letter when it has no logo of its own.
pub(super) fn tile(id: &str, name: &str) -> El {
    if let Some(brand) = brand_icons::for_item(id) {
        return brand_tile(brand, TILE, TILE_GLYPH);
    }
    el().size(TILE, TILE)
        .radius(TILE / 4.0)
        .center()
        .fill(kit::forest().alpha(0.12))
        .a11y_hidden()
        .child(
            text(monogram(name))
                .font(kit::bold_style())
                .color(kit::forest()),
        )
}

fn monogram(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

/// A row in a column, with its rule underneath.
fn cell(key: String, content: El) -> El {
    col()
        .key(key)
        .w_full()
        .min_w(0.0)
        .child(content.w_full().min_h(CELL).px(2.0).gap(9.0).center_y())
        .child(kit::row_rule())
}

fn build_row(app: &App, i: usize, k: &'static Kit) -> El {
    match &app.rows[i] {
        CatRow::Base(b) => base_row(*b, k),
        CatRow::Item(x) => item_row(app, *x, k),
        CatRow::Setting(x) => setting_row(app, *x, k),
        CatRow::Package(p) => package_row(p, k),
        CatRow::AddPackage(q) => add_package_row(q),
        CatRow::Empty => row()
            .w_full()
            .h(kit::ROW)
            .center_y()
            .child(kit::label("Nothing selected")),
        // Headings are read into `Group`s before anything draws.
        CatRow::Section(_) | CatRow::Group(_) | CatRow::Surface(_) => el().h(0.0),
    }
}

fn base_row(b: usize, k: &Kit) -> El {
    let entry = view().catalog.base_items().nth(b).expect("a Base Item");
    let content = row()
        .child(kit::check_box(k, true, true))
        .child(tile(entry.id(), entry.name()))
        .child(kit::label(entry.name().to_string()).shrink().truncate());
    cell(format!("base-{}", entry.id()), content)
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
        .child(tile(entry.id(), entry.name()))
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
    cell(format!("item-{}", entry.id()), content)
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
        _ => kit::label("In your Machine Type"),
    }
}

/// A row can't reach the overlay layer, so the open Select builds here, against its trigger's key.
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
    cell(format!("setting-{id}"), content)
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
        .child(tile(package, package))
        .child(kit::label(package.to_string()).shrink().truncate());
    cell(format!("package-{package}"), content)
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
    cell("add-package-row".to_string(), content)
}

fn footer(app: &App) -> El {
    let words = format!("{} selected", app.selection_count());
    let left = kit::action(
        "selection-count".to_string(),
        &words,
        app.only_selected,
        |app: &mut App, tx| app.set_filter_selected(!app.only_selected, tx),
    );
    // What's being built is something to keep. Setting up this Mac once, with nothing saved, stays
    // available but isn't what the screen leads with.
    let right = row()
        .gap(14.0)
        .center_y()
        .child(kit::action(
            "go-review".to_string(),
            "Set up this Mac now",
            false,
            |app: &mut App, _| app.screen = Screen::Review,
        ))
        .child(kit::primary(
            "go-save",
            "Save Machine Type",
            true,
            |app: &mut App, _| app.screen = Screen::Save,
        ));
    kit::foot("catalog-footer", left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_count_follows_the_width() {
        assert_eq!(columns_for(911.0, MIN_COLUMN), 3, "the default window");
        assert_eq!(columns_for(630.0, MIN_COLUMN), 2, "the smallest window");
        assert_eq!(columns_for(100.0, MIN_COLUMN), 1);
        assert_eq!(columns_for(5000.0, MIN_COLUMN), MAX_COLUMNS);
        assert_eq!(columns_for(911.0, MIN_SETTINGS_COLUMN), 2);
    }

    #[test]
    fn groups_stay_in_reading_order_across_columns() {
        let placed = place(&[100.0, 60.0, 60.0, 40.0, 100.0, 40.0], 3);
        let order: Vec<usize> = placed.iter().flatten().copied().collect();
        assert_eq!(order, [0, 1, 2, 3, 4, 5]);
        assert!(placed.iter().all(|column| !column.is_empty()));
    }

    #[test]
    fn one_column_takes_everything_and_a_tall_group_is_never_split() {
        assert_eq!(place(&[10.0, 20.0], 1), vec![vec![0, 1]]);
        let placed = place(&[900.0, 10.0, 10.0], 2);
        assert_eq!(placed[0], vec![0]);
        assert_eq!(placed[1], vec![1, 2]);
    }

    #[test]
    fn the_tallest_column_is_as_short_as_reading_order_allows() {
        // The Languages page: filling each column to an even share left the middle one at 668.
        let heights = [
            215.0, 141.0, 141.0, 141.0, 104.0, 67.0, 104.0, 252.0, 67.0, 104.0, 67.0, 141.0, 104.0,
        ];
        let placed = place(&heights, 3);
        let tallest = placed
            .iter()
            .map(|column| column.iter().map(|&i| heights[i]).sum::<f32>())
            .fold(0.0, f32::max);
        assert_eq!(tallest, 638.0);
    }

    #[test]
    fn fewer_groups_than_columns_is_fine() {
        let placed = place(&[50.0], 3);
        assert_eq!(placed, vec![vec![0], vec![], vec![]]);
    }

    fn languages() -> Vec<Block> {
        let mut app = crate::state::init();
        app.show_section(0);
        blocks(&app.rows, true)
    }

    fn headings(block: &Block) -> Vec<&str> {
        block
            .groups
            .iter()
            .filter_map(|g| g.heading.as_deref())
            .collect()
    }

    #[test]
    fn a_language_stays_next_to_its_managers_and_empty_headings_go() {
        let page = languages();
        assert_eq!(page.len(), 1);
        let names = headings(&page[0]);
        let javascript = names.iter().position(|n| *n == "JavaScript").unwrap();
        assert_eq!(names[javascript + 1], "Node.js version managers");
        assert!(
            !names.contains(&"Version managers"),
            "its Items all sit under the choice's own heading: {names:?}"
        );
    }

    #[test]
    fn a_category_with_no_groups_spreads_across_the_columns() {
        let mut app = crate::state::init();
        app.show_section(1);
        let page = blocks(&app.rows, true);
        assert_eq!(page[0].groups.len(), 10, "Browsers, a group of one each");
        assert!(page[0].groups.iter().all(|g| g.heading.is_none()));
    }

    #[test]
    fn a_search_names_each_category_and_browsing_does_not() {
        assert_eq!(languages()[0].title, None);
        let mut app = crate::state::init();
        app.search.text = "ripgrep".to_string();
        app.rebuild_rows();
        let found = blocks(&app.rows, app.browsing());
        assert_eq!(found[0].title.as_deref(), Some("CLI tools"));
        assert_eq!(
            found.last().unwrap().shape,
            Shape::Single,
            "add from Homebrew"
        );
    }

    #[test]
    fn every_row_lands_in_a_group() {
        let mut app = crate::state::init();
        app.fill_typical();
        app.only_selected = true;
        app.rebuild_rows();
        let drawn: usize = blocks(&app.rows, app.browsing())
            .iter()
            .flat_map(|b| &b.groups)
            .map(|g| g.rows.len())
            .sum();
        let drawable = app
            .rows
            .iter()
            .filter(|r| {
                !matches!(
                    r,
                    CatRow::Section(_) | CatRow::Group(_) | CatRow::Surface(_)
                )
            })
            .count();
        assert_eq!(drawn, drawable);
    }

    #[test]
    fn a_monogram_is_the_first_letter_or_digit() {
        assert_eq!(monogram("fnm"), "F");
        assert_eq!(monogram(".dotter"), "D");
        assert_eq!(monogram("1thing"), "1");
        assert_eq!(monogram(""), "");
    }

    #[test]
    fn logos_are_real_and_unrelated_brands_stay_out() {
        assert!(brand_icons::for_item("python").is_some());
        assert!(brand_icons::for_item("homebrew").is_some(), "a Base Item");
        assert!(brand_icons::for_item("delta").is_none(), "not the airline");
        assert!(brand_icons::for_item("lazygit").is_none(), "not Git's mark");
        assert!(brand_icons::for_group("JavaScript").is_some());
        assert!(brand_icons::for_group("Other languages").is_none());
    }
}
