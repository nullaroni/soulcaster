//! The look, from the approved brand sheet: cream paper, ink, forest titles, one crest-yellow
//! button, mandarin for failures only. A row is a name and an action.

use std::sync::OnceLock;

use rune::color::hex;
use rune::{Color, El, Family, Icon, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Kit, PaletteSeed, Type};

pub fn paper() -> Color {
    hex("#FBF6EA").expect("a fixed color")
}
pub fn ink() -> Color {
    hex("#14130F").expect("a fixed color")
}
pub fn forest() -> Color {
    hex("#015D47").expect("a fixed color")
}
pub fn crest() -> Color {
    hex("#E0F354").expect("a fixed color")
}
pub fn mandarin() -> Color {
    hex("#FD550A").expect("a fixed color")
}
pub fn dim() -> Color {
    hex("#80AEA3").expect("a fixed color")
}
fn rule_color() -> Color {
    ink().alpha(0.22)
}
fn row_rule_color() -> Color {
    ink().alpha(0.14)
}

/// Row height, without its rule.
pub const ROW: f32 = 31.0;
const UI_SIZE: f32 = 12.0;
const UI_LINE: f32 = 16.0;
pub const BOLD: f32 = 700.0;

pub fn ui_style() -> TextStyle {
    TextStyle::new(UI_SIZE, 400.0).lh(UI_LINE)
}

pub fn bold_style() -> TextStyle {
    TextStyle::new(UI_SIZE, BOLD).lh(UI_LINE)
}

pub fn log_style() -> TextStyle {
    TextStyle::new(11.5, 400.0).lh(17.0)
}

/// Light only: the brand sheet is cream paper, and it has no dark version.
pub fn theme() -> &'static Kit {
    static KIT: OnceLock<Kit> = OnceLock::new();
    KIT.get_or_init(|| {
        let mut k = Kit::from_palette(PaletteSeed {
            bg: paper(),
            fg: ink(),
            accent: forest(),
        });
        let ui = ui_style();
        let bold = bold_style();
        k.ty = Type {
            ui,
            ui_medium: bold,
            ui_compact: ui,
            small: ui,
            small_medium: bold,
            body: ui,
            body_medium: bold,
            text_small: ui,
            text_small_medium: bold,
            text_xsmall: ui,
            text_xsmall_medium: bold,
            mono: log_style(),
        };
        k.c.page = paper();
        k.c.bg_base = paper();
        k.c.bg_deep = paper();
        k.c.layer_01 = paper();
        k.c.surface_raised = paper();
        k.c.surface_raised_base = paper();
        k.c.text_base = ink();
        k.c.text_strong = ink();
        k.c.text_muted = ink();
        k.c.text_weak = ink();
        k.c.icon_base = ink();
        k.c.border_focus = forest();
        k.c.hover = ink().alpha(0.05);
        k.c.pressed = ink().alpha(0.1);
        k
    })
}

/// The mark in the title bar: an oval with a slash through it.
pub static MARK: Icon = Icon {
    name: "sc-mark",
    view: 64.0,
    body: r#"<path fill="currentColor" fill-rule="evenodd" d="M32 3C19.85 3 10 15.98 10 32s9.85 29 22 29 22-12.98 22-29S44.15 3 32 3zm0 6c4.42 0 8 10.3 8 23s-3.58 23-8 23-8-10.3-8-23 3.58-23 8-23z"/><path fill="currentColor" d="M17 54L12 48 43 10 48 16z"/>"#,
};

pub static PLUS: Icon = Icon {
    name: "sc-plus",
    view: 16.0,
    body: r#"<path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};

pub fn hairline() -> El {
    el().w_full().h(1.0).fill(rule_color())
}

fn row_rule() -> El {
    el().w_full().h(1.0).fill(row_rule_color())
}

/// A row with its rule underneath, as every list on every screen has.
pub fn ruled(key: String, content: El) -> El {
    col()
        .key(key)
        .w_full()
        .child(content.w_full().min_h(ROW).px(14.0).gap(10.0).center_y())
        .child(row_rule())
}

/// The screen's Playfair title, the only place that face appears.
pub fn title(words: &str) -> El {
    text(words.to_string())
        .font(title_style())
        .color(forest())
        .role("heading")
        .level(1)
}

pub fn title_style() -> TextStyle {
    TextStyle {
        family: Family::Named("Playfair Display"),
        line_height: 28.0,
        ..TextStyle::new(28.0, 900.0)
    }
}

/// The padded area a title sits in.
pub fn title_pad(content: El) -> El {
    row()
        .w_full()
        .px(16.0)
        .pt(16.0)
        .gap(10.0)
        .items_end()
        .child(content)
}

pub fn label(words: impl Into<String>) -> El {
    text(words.into()).font(ui_style()).color(ink())
}

pub fn group(words: impl Into<String>) -> El {
    text(words.into()).font(bold_style()).color(forest())
}

/// A word that does something, and turns forest and bold when it's the current choice.
pub fn action<S: 'static>(
    key: String,
    words: &str,
    on: bool,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    action_colored(key, words, if on { forest() } else { ink() }, on, f)
}

/// The same word in mandarin, for the one thing that failed.
pub fn failed_action<S: 'static>(
    key: String,
    words: &str,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    action_colored(key, words, mandarin(), true, f)
}

fn action_colored<S: 'static>(
    key: String,
    words: &str,
    color: Color,
    strong: bool,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(words.to_string())
        .focusable(true)
        .focus_ring(1.5, 1.0, forest())
        .hover(ink().alpha(0.05))
        .radius(3.0)
        .px(2.0)
        .on_press_tx(f)
        .child(
            text(words.to_string())
                .font(if strong { bold_style() } else { ui_style() })
                .color(color),
        )
}

/// The one crest-yellow button on a screen, or its outlined form when there's nothing to do yet.
pub fn primary<S: 'static>(
    key: &str,
    words: &str,
    enabled: bool,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(words.to_string())
        .h(28.0)
        .px(14.0)
        .radius(999.0)
        .center()
        .child(text(words.to_string()).font(bold_style()).color(ink()));
    if enabled {
        b = b
            .fill(crest())
            .focusable(true)
            .focus_ring(2.0, 2.0, forest())
            .hover(ink().alpha(0.06))
            .on_press_tx(f);
    } else {
        b = b.border(1.0, ink()).disabled(true);
    }
    b
}

/// The 48-point bar at the bottom of a screen.
pub fn foot(key: &str, left: El, right: El) -> El {
    col().key(key.to_string()).w_full().child(hairline()).child(
        row()
            .w_full()
            .h(47.0)
            .px(14.0)
            .center_y()
            .child(left)
            .child(el().grow(1.0))
            .child(right),
    )
}

/// A checkbox mark. The row around it carries the role, label and handler, so the whole row presses
/// and a screen reader hears one control.
pub fn check_box(k: &Kit, on: bool, locked: bool) -> El {
    let fill = match (on, locked) {
        (true, true) => dim(),
        (true, false) => forest(),
        _ => Color::TRANSPARENT,
    };
    let edge = if !on { ink() } else { fill };
    el().size(13.0, 13.0)
        .center()
        .fill(fill)
        .border(1.0, edge)
        .a11y_hidden()
        .child(
            icon(k.glyph.check, 9.0)
                .color(paper())
                .opacity(if on { 1.0 } else { 0.0 }),
        )
}
