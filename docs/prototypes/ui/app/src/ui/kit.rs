//! The look: the neutral kit with one accent, plus the small parts every screen shares.

use std::sync::OnceLock;

use rune::{Color, El, Icon, Tx, el, icon, oklch, row, text};
use rune_kit::Kit;

use crate::catalog::Price;

/// Colors the kit has no token for.
#[derive(Clone, Copy)]
pub struct Tones {
    pub accent: Color,
    pub accent_deep: Color,
    pub accent_soft: Color,
    pub accent_ink: Color,
    pub on_accent: Color,
    pub good: Color,
    pub good_soft: Color,
    pub warn: Color,
    pub warn_soft: Color,
    pub warn_ink: Color,
    pub bad: Color,
    pub bad_soft: Color,
}

fn tones(dark: bool) -> Tones {
    if dark {
        Tones {
            accent: oklch(0.66, 0.17, 275.0),
            accent_deep: oklch(0.52, 0.2, 290.0),
            accent_soft: oklch(0.32, 0.07, 275.0),
            accent_ink: oklch(0.82, 0.1, 275.0),
            on_accent: oklch(0.99, 0.0, 0.0),
            good: oklch(0.76, 0.15, 155.0),
            good_soft: oklch(0.3, 0.05, 155.0),
            warn: oklch(0.8, 0.14, 80.0),
            warn_soft: oklch(0.32, 0.05, 80.0),
            warn_ink: oklch(0.86, 0.12, 85.0),
            bad: oklch(0.72, 0.17, 25.0),
            bad_soft: oklch(0.3, 0.06, 25.0),
        }
    } else {
        Tones {
            accent: oklch(0.52, 0.2, 275.0),
            accent_deep: oklch(0.42, 0.21, 290.0),
            accent_soft: oklch(0.94, 0.03, 275.0),
            accent_ink: oklch(0.42, 0.17, 275.0),
            on_accent: oklch(0.99, 0.0, 0.0),
            good: oklch(0.55, 0.15, 155.0),
            good_soft: oklch(0.95, 0.04, 155.0),
            warn: oklch(0.72, 0.16, 75.0),
            warn_soft: oklch(0.96, 0.05, 85.0),
            warn_ink: oklch(0.46, 0.11, 70.0),
            bad: oklch(0.55, 0.2, 25.0),
            bad_soft: oklch(0.95, 0.03, 25.0),
        }
    }
}

/// The theme for this frame. Virtual rows build outside `view`, so they read the same value.
pub fn theme(dark: bool) -> (&'static Kit, Tones) {
    static DARK: OnceLock<Kit> = OnceLock::new();
    static LIGHT: OnceLock<Kit> = OnceLock::new();
    DARK_NOW.store(dark, std::sync::atomic::Ordering::Relaxed);
    let build = |dark: bool| Kit::neutral(dark);
    let k = if dark { DARK.get_or_init(|| build(true)) } else { LIGHT.get_or_init(|| build(false)) };
    (k, tones(dark))
}

static DARK_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn theme_now() -> (&'static Kit, Tones) {
    theme(DARK_NOW.load(std::sync::atomic::Ordering::Relaxed))
}

pub mod icons {
    use rune::Icon;

    macro_rules! glyph {
        ($name:ident, $id:literal, $body:literal) => {
            pub static $name: Icon = Icon { name: $id, view: 16.0, body: $body };
        };
    }
    const _: () = ();

    glyph!(SPARK, "sc-spark", r#"<path d="M8 1.5l1.6 4.4 4.9 2.1-4.9 2.1L8 14.5l-1.6-4.4L1.5 8l4.9-2.1z" fill="currentColor"/>"#);
    glyph!(CHECK_CIRCLE, "sc-check-circle", r#"<circle cx="8" cy="8" r="6.5" fill="currentColor"/><path d="M5 8.2l2.1 2.1L11 6.2" stroke="white" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#);
    glyph!(X_CIRCLE, "sc-x-circle", r#"<circle cx="8" cy="8" r="6.5" fill="currentColor"/><path d="M5.7 5.7l4.6 4.6M10.3 5.7l-4.6 4.6" stroke="white" stroke-width="1.5" stroke-linecap="round"/>"#);
    glyph!(PERSON, "sc-person", r#"<circle cx="8" cy="8" r="6.5" fill="currentColor"/><circle cx="8" cy="6.2" r="1.9" fill="white"/><path d="M4.6 11.6c.6-1.7 1.9-2.4 3.4-2.4s2.8.7 3.4 2.4" stroke="white" stroke-width="1.4" fill="none" stroke-linecap="round"/>"#);
    glyph!(DOT, "sc-dot", r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
    glyph!(LINK, "sc-link", r#"<circle cx="8" cy="8" r="6.5" fill="currentColor"/><path d="M5 8h6" stroke="white" stroke-width="1.5" stroke-linecap="round"/>"#);
    glyph!(QUESTION, "sc-question", r#"<circle cx="8" cy="8" r="5.8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M6.3 6.4c.2-.9.9-1.4 1.8-1.4 1 0 1.7.6 1.7 1.5 0 1.3-1.7 1.3-1.7 2.6" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linecap="round"/><circle cx="8.1" cy="11" r=".8" fill="currentColor"/>"#);
    glyph!(LOCK_CLOSED, "sc-lock", r#"<rect x="3.5" y="7" width="9" height="6.5" rx="1.5" fill="currentColor"/><path d="M5.5 7V5a2.5 2.5 0 015 0v2" stroke="currentColor" stroke-width="1.3" fill="none"/>"#);
    glyph!(LOCK_OPEN, "sc-lock-open", r#"<rect x="3.5" y="7" width="9" height="6.5" rx="1.5" fill="currentColor"/><path d="M5.5 7V5a2.5 2.5 0 014.8-.9" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linecap="round"/>"#);
    glyph!(ARROWS_SWAP, "sc-swap", r#"<path d="M3 5.5h9M9.5 3l2.5 2.5L9.5 8M13 10.5H4M6.5 8L4 10.5 6.5 13" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#);
    glyph!(WARN, "sc-warn", r#"<path d="M8 2.2l6.3 11.1H1.7z" fill="currentColor"/><path d="M8 6.4v3.2" stroke="white" stroke-width="1.4" stroke-linecap="round"/><circle cx="8" cy="11.4" r=".8" fill="white"/>"#);
    glyph!(INFO, "sc-info", r#"<circle cx="8" cy="8" r="5.8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 7.3v3.4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="5.2" r=".8" fill="currentColor"/>"#);
    glyph!(MOON, "sc-moon", r#"<path d="M13 9.8A5.6 5.6 0 016.2 3a5.6 5.6 0 106.8 6.8z" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linejoin="round"/>"#);
    glyph!(GLOBE, "sc-globe", r#"<circle cx="8" cy="8" r="5.8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M2.4 8h11.2M8 2.2c1.8 1.7 2.6 3.6 2.6 5.8S9.8 12.100 8 13.800C6.200 12.100 5.400 10.200 5.400 8S6.200 3.900 8 2.200z" stroke="currentColor" stroke-width="1.100" fill="none"/>"#);
    glyph!(CODE, "sc-code", r#"<path d="M5.500 4.500L2 8l3.500 3.500M10.500 4.500L14 8l-3.500 3.500" stroke="currentColor" stroke-width="1.300" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#);
    glyph!(PENCIL, "sc-pencil", r#"<path d="M3 13l.7-3L10.500 3.200a1.100 1.100 0 011.600 0l.7.7a1.100 1.100 0 010 1.600L6 12.300z" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(TERMINAL, "sc-terminal", r#"<rect x="2" y="3" width="12" height="10" rx="2" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M4.800 6.300L6.800 8l-2 1.700M8.300 10h3" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#);
    glyph!(WRENCH, "sc-wrench", r#"<path d="M10.300 2.400a3.300 3.300 0 00-3.900 4.400L2.600 10.600a1.300 1.300 0 001.800 1.800l3.800-3.800a3.300 3.300 0 004.400-3.900l-2 2-1.800-.5-.5-1.800z" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(BOX, "sc-box", r#"<path d="M8 2l5.500 3v6L8 14l-5.500-3V5zM2.500 5L8 8l5.500-3M8 8v6" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(DATABASE, "sc-db", r#"<ellipse cx="8" cy="4" rx="5" ry="2" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M3 4v8c0 1.100 2.200 2 5 2s5-.9 5-2V4M3 8c0 1.100 2.200 2 5 2s5-.9 5-2" stroke="currentColor" stroke-width="1.200" fill="none"/>"#);
    glyph!(BRANCH, "sc-branch", r#"<circle cx="4.500" cy="3.500" r="1.500" stroke="currentColor" stroke-width="1.200" fill="none"/><circle cx="4.500" cy="12.500" r="1.500" stroke="currentColor" stroke-width="1.200" fill="none"/><circle cx="11.500" cy="5" r="1.500" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M4.500 5v6M11.500 6.500c0 2.500-3 2.500-7 3.500" stroke="currentColor" stroke-width="1.200" fill="none"/>"#);
    glyph!(PHONE, "sc-phone", r#"<rect x="4.500" y="1.800" width="7" height="12.400" rx="1.600" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M7 12.300h2" stroke="currentColor" stroke-width="1.200" stroke-linecap="round"/>"#);
    glyph!(CLOUD, "sc-cloud", r#"<path d="M4.500 12.500a3 3 0 01-.4-6A4 4 0 0112 6.300a3.100 3.100 0 01-.5 6.200z" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(NETWORK, "sc-network", r#"<circle cx="8" cy="3.500" r="1.700" stroke="currentColor" stroke-width="1.200" fill="none"/><circle cx="3.500" cy="12.300" r="1.700" stroke="currentColor" stroke-width="1.200" fill="none"/><circle cx="12.500" cy="12.300" r="1.700" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M7.200 5l-3 5.800M8.800 5l3 5.800M5.200 12.300h5.600" stroke="currentColor" stroke-width="1.200" fill="none"/>"#);
    glyph!(WINDOW, "sc-window", r#"<rect x="2" y="3" width="12" height="10" rx="2" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M2 6.200h12M8 6.200V13" stroke="currentColor" stroke-width="1.200"/>"#);
    glyph!(CHAT, "sc-chat", r#"<path d="M2.500 4.500a2 2 0 012-2h7a2 2 0 012 2v5a2 2 0 01-2 2H7l-3 2.500v-2.500a2 2 0 01-1.500-2z" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(FONT, "sc-font", r#"<path d="M3 13L7 3h1l4 10M4.500 9.500h6" stroke="currentColor" stroke-width="1.300" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#);
    glyph!(SLIDERS, "sc-sliders", r#"<path d="M3 5h10M3 11h10" stroke="currentColor" stroke-width="1.200" stroke-linecap="round"/><circle cx="6" cy="5" r="1.600" fill="currentColor"/><circle cx="10.500" cy="11" r="1.600" fill="currentColor"/>"#);
    glyph!(LIST, "sc-list", r#"<path d="M5.500 4.500H13M5.500 8H13M5.500 11.500H13" stroke="currentColor" stroke-width="1.200" stroke-linecap="round"/><circle cx="3" cy="4.500" r=".9" fill="currentColor"/><circle cx="3" cy="8" r=".9" fill="currentColor"/><circle cx="3" cy="11.500" r=".9" fill="currentColor"/>"#);
    glyph!(GITHUB, "sc-github", r#"<path d="M8 1.500a6.500 6.500 0 00-2 12.700c.3.100.4-.1.400-.3v-1.200c-1.800.4-2.200-.8-2.200-.8-.3-.700-.7-.9-.7-.9-.6-.4 0-.4 0-.4.600 0 1 .7 1 .7.600 1 1.500.700 1.900.5.100-.4.200-.700.400-.900-1.400-.200-3-.7-3-3.200 0-.7.300-1.300.700-1.700-.100-.2-.3-.9.100-1.800 0 0 .5-.2 1.800.7a6 6 0 013.200 0c1.200-.9 1.800-.7 1.800-.7.400.9.100 1.600.100 1.800.4.400.7 1 .7 1.700 0 2.500-1.500 3-3 3.200.2.200.5.600.5 1.200v1.800c0 .2.100.4.400.300A6.500 6.500 0 008 1.500z" fill="currentColor"/>"#);
    glyph!(KEY, "sc-key", r#"<circle cx="5.300" cy="10.700" r="2.800" stroke="currentColor" stroke-width="1.200" fill="none"/><path d="M7.300 8.700L13 3M10.800 5.200l1.700 1.700" stroke="currentColor" stroke-width="1.200" stroke-linecap="round"/>"#);
    glyph!(DOC, "sc-doc", r#"<path d="M4 2h5.500L12.500 5v9H4zM9.500 2v3h3" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(SHIELD, "sc-shield", r#"<path d="M8 1.800l5 1.900v4.100c0 3-2.100 5.300-5 6.400-2.900-1.100-5-3.400-5-6.400V3.700z" stroke="currentColor" stroke-width="1.200" fill="none" stroke-linejoin="round"/>"#);
    glyph!(APPLE, "sc-apple", r#"<path d="M11 8.500c0-1.500 1.200-2.200 1.300-2.300-.7-1-1.800-1.200-2.200-1.200-.9-.1-1.800.6-2.300.6s-1.200-.5-2-.5C4.800 5.100 3.500 6 3.500 8.500c0 2 1.500 5 2.800 5 .6 0 1-.5 1.900-.5s1.200.5 1.900.5c1.200 0 2.400-2.500 2.700-3.500-.1 0-1.800-.700-1.800-1.500zM9.500 3.800c.4-.5.700-1.200.6-1.900-.6 0-1.300.4-1.700.9-.4.400-.7 1.100-.6 1.800.7.100 1.300-.3 1.700-.8z" fill="currentColor"/>"#);
    glyph!(CHEVRON, "sc-chevron", r#"<path d="M6 3.500L10.500 8 6 12.500" stroke="currentColor" stroke-width="1.400" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#);
}

pub fn section_icon(id: &str) -> &'static Icon {
    match id {
        "languages" => &icons::CODE,
        "browsers" => &icons::GLOBE,
        "editors" => &icons::PENCIL,
        "agents" => &icons::SPARK,
        "terminals" => &icons::TERMINAL,
        "cli" => &icons::WRENCH,
        "containers" => &icons::BOX,
        "databases" => &icons::DATABASE,
        "git" => &icons::BRANCH,
        "mobile" => &icons::PHONE,
        "cloud" => &icons::CLOUD,
        "networking" => &icons::NETWORK,
        "window-management" => &icons::WINDOW,
        "communication" => &icons::CHAT,
        "fonts" => &icons::FONT,
        _ => &icons::LIST,
    }
}

/// A tile with an Item's first letter, tinted by its name, until the Catalog has real icons.
pub fn tile(k: &Kit, name: &str, size: f32) -> El {
    let hash = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let hue = (hash % 360) as f32;
    let initial = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_ascii_uppercase().to_string();
    let (top, bottom) = if k.dark { (oklch(0.46, 0.09, hue), oklch(0.36, 0.09, hue + 24.0)) } else { (oklch(0.68, 0.12, hue), oklch(0.57, 0.13, hue + 24.0)) };
    el().size(size, size)
        .radius(size * 0.26)
        .center()
        .linear_gradient(160.0, [(0.0, top), (1.0, bottom)])
        .a11y_hidden()
        .child(text(initial).font(rune::TextStyle::new(size * 0.5, 600.0)).color(Color::WHITE))
}

/// A checkbox mark. The row around it carries the role, label and handler, so the whole row
/// presses and a screen reader hears one control.
pub fn check_mark(k: &Kit, tones: Tones, on: bool, locked: bool) -> El {
    let fill = if locked { k.c.layer_03 } else if on { tones.accent } else { Color::TRANSPARENT };
    let border = if on { Color::TRANSPARENT } else { k.c.border_strong };
    el().size(18.0, 18.0)
        .radius(5.0)
        .center()
        .fill(fill)
        .border(1.0, border)
        .a11y_hidden()
        .child(icon(k.glyph.check, 11.0).color(if locked { k.c.icon_muted } else { tones.on_accent }).opacity(if on { 1.0 } else { 0.0 }))
}

pub fn tag(k: &Kit, label: &str, fill: Color, ink: Color) -> El {
    row().h(20.0).px(7.0).radius(5.0).center().fill(fill).child(text(label).font(k.ty.text_xsmall_medium).color(ink))
}

pub fn price_tag(k: &Kit, tones: Tones, price: Price) -> Option<El> {
    match price {
        Price::Paid => Some(tag(k, "Paid", tones.warn_soft, tones.warn_ink)),
        Price::Subscription => Some(tag(k, "Needs a subscription", tones.warn_soft, tones.warn_ink)),
        Price::PaidAtWork => Some(tag(k, "Paid for work use", tones.warn_soft, tones.warn_ink)),
        Price::FreePlan => Some(tag(k, "Free plan", k.c.layer_03, k.c.text_muted)),
        Price::Free | Price::NotChecked => None,
    }
}

pub fn count_badge(k: &Kit, tones: Tones, n: usize) -> El {
    row().h(18.0).min_w(18.0).px(5.0).radius(9.0).center().fill(tones.accent).child(text(n.to_string()).font(k.ty.text_xsmall_medium).color(tones.on_accent))
}

pub fn mini_seg<S: 'static>(k: &Kit, tones: Tones, key: String, label: &str, on: bool, enabled: bool, f: impl Fn(&mut S) + 'static) -> El {
    let mut chip = row()
        .key(key)
        .role("radio")
        .label(label.to_string())
        .checked(on)
        .h(20.0)
        .px(7.0)
        .radius(4.0)
        .center()
        .fill(if on { k.c.layer_03 } else { Color::TRANSPARENT })
        .child(text(label.to_string()).font(k.ty.text_xsmall).color(if on { k.c.text_base } else if enabled { k.c.text_muted } else { k.c.text_faint }));
    let _ = tones;
    if enabled {
        chip = chip.focusable(true).hover(k.c.hover).on_press(f);
    } else {
        chip = chip.disabled(true);
    }
    chip
}

pub fn card(k: &Kit) -> El {
    rune::col().w_full().radius(10.0).fill(k.c.layer_01).border(1.0, k.c.border_weak)
}

pub fn heading(k: &Kit, title: &str, sub: &str) -> El {
    let mut c = rune::col().gap(3.0).w_full();
    c = c.child(text(title).font(rune::TextStyle::new(20.0, 600.0)).color(k.c.text_strong).level(1).role("heading"));
    if !sub.is_empty() {
        c = c.child(text(sub).font(k.ty.text_small).color(k.c.text_muted).w_full().wrap());
    }
    c
}

/// A button-styled row for the one main action on a screen.
pub fn primary<S: 'static>(k: &Kit, tones: Tones, key: &str, label: &str, enabled: bool, f: impl Fn(&mut S, &mut Tx) + 'static) -> El {
    let mut b = row()
        .key(key)
        .role("button")
        .label(label.to_string())
        .h(36.0)
        .px(16.0)
        .gap(6.0)
        .radius(8.0)
        .center()
        .fill(if enabled { tones.accent } else { k.c.layer_03 })
        .child(text(label.to_string()).font(k.ty.text_small_medium).color(if enabled { tones.on_accent } else { k.c.text_faint }));
    if enabled {
        b = b.focusable(true).focus_ring(2.0, 2.0, tones.accent).hover(tones.accent_deep).on_press_tx(f);
    } else {
        b = b.disabled(true);
    }
    b
}

pub fn status_line(k: &Kit, glyph: &'static Icon, color: Color, words: &str) -> El {
    row().gap(6.0).center_y()
        .child(icon(glyph, 14.0).color(color))
        .child(text(words.to_string()).font(k.ty.text_small).color(k.c.text_base))
}
