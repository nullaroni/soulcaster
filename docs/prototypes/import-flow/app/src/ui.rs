//! Small shared pieces. Every string that came from a Config goes through `data` or `quoted`, so it never
//! shares a style with Soulcaster's own sentences.

use rune::{Color, El, col, el, row, text};
use rune_kit::Kit;

pub const WIDTH: f32 = 860.0;

pub fn para(k: &Kit, s: impl Into<String>) -> El {
    text(s.into()).font(k.ty.text_small).color(k.c.text_base).w_full().wrap()
}
pub fn muted(k: &Kit, s: impl Into<String>) -> El {
    text(s.into()).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap()
}
pub fn strong(k: &Kit, s: impl Into<String>) -> El {
    text(s.into()).font(k.ty.text_small_medium).color(k.c.text_base).w_full().wrap()
}
pub fn colored(k: &Kit, s: impl Into<String>, c: Color) -> El {
    text(s.into()).font(k.ty.text_xsmall_medium).color(c).w_full().wrap()
}

/// Text from the file: monospace, and allowed to break anywhere, so a long line can't push anything off screen.
pub fn data(k: &Kit, s: impl Into<String>) -> El {
    text(s.into()).font(k.ty.mono).color(k.c.text_base).w_full().wrap_anywhere()
}

/// A fixed-width label and a wrapping value on one line.
pub fn kv(k: &Kit, label: &str, value: El) -> El {
    row()
        .w_full()
        .items_start()
        .gap(12.0)
        .child(el().w(116.0).child(text(label.to_string()).font(k.ty.text_xsmall_medium).color(k.c.text_muted)))
        .child(col().grow(1.0).min_w(0.0).child(value))
}

pub fn quoted(k: &Kit, s: &str) -> El {
    el().w_full()
        .padding(8.0)
        .radius(k.radius.sm)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_weak)
        .child(data(k, format!("\"{s}\"")))
}

pub fn card(k: &Kit, inner: El) -> El {
    el().w_full().padding(14.0).radius(k.radius.md).fill(k.c.layer_01).border(1.0, k.c.border_weak).child(inner)
}

/// The one mark a risky part gets: a coloured rule on its left edge. No icon, no score, no red until something is wrong.
pub fn risky(k: &Kit, inner: El, on: bool) -> El {
    let rule = if on { k.c.warning.icon } else { k.c.border_strong };
    row()
        .w_full()
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .child(el().w(4.0).h_full().fill(rule))
        .child(col().grow(1.0).min_w(0.0).padding(14.0).child(inner))
}

pub fn section(k: &Kit, title: &str, lede: &str, body: El) -> El {
    let mut c = col().w_full().gap(10.0).child(text(title.to_string()).font(k.ty.body_medium).color(k.c.text_base));
    if !lede.is_empty() {
        c = c.child(muted(k, lede));
    }
    c.child(body)
}

pub fn page(k: &Kit, title: &str, crumb: &str, body: El, footer: Option<El>) -> El {
    let mut root = col()
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(
            row()
                .w_full()
                .center_y()
                .px(20.0)
                .py(12.0)
                .gap(8.0)
                .child(text("Soulcaster").font(k.ty.text_small_medium).color(k.c.text_muted))
                .child(text("/").font(k.ty.text_small).color(k.c.text_faint))
                .child(text(crumb.to_string()).font(k.ty.text_small_medium).color(k.c.text_base)),
        )
        .child(
            col().key("body").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(
                col().w_full().items_center().px(20.0).pb(24.0).child(
                    col().key("content").w_full().max_w(WIDTH).gap(18.0)
                        .child(text(title.to_string()).font(rune::tok::type_::TITLE).color(k.c.text_base).w_full().wrap())
                        .child(body),
                ),
            ),
        );
    if let Some(f) = footer {
        root = root.child(
            col().w_full().items_center().fill(k.c.layer_01).child(el().w_full().h(1.0).fill(k.c.border_base)).child(
                col().w_full().max_w(WIDTH + 40.0).px(20.0).py(12.0).child(f),
            ),
        );
    }
    root
}
