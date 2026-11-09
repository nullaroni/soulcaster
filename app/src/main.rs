//! The Soulcaster window. The engine behind it is simulated for now, so nothing here installs,
//! runs `sudo`, changes a setting or provisions over the network.
//!
//! ```sh
//! cargo run -p soulcaster-app                                      # a real window
//! cargo run -p soulcaster-app -- --snap out.png --scene catalog    # one screen, headless
//! cargo run -p soulcaster-app -- --list-scenes
//! ```

mod answers;
#[rustfmt::skip]
mod brand_icons;
mod catalog_view;
mod drift;
mod drift_flow;
mod engine;
mod fleet_flow;
mod real;
mod repo_flow;
mod scenes;
mod sim;
mod state;
mod ui;

use std::time::Duration;

use rune::{AppSpec, Key, KeyEvent, Titlebar, Tx};

use crate::state::{App, Screen};

static FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf"),
    include_bytes!("../assets/fonts/PlayfairDisplay-wght.ttf"),
];

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let changed = app.tick(clock);
    if let Some(next) = app.next_due() {
        tx.wake_after(Duration::from_secs_f64(next.max(0.016)));
    }
    changed
}

fn on_key(app: &mut App, key: &KeyEvent, _tx: &mut Tx) -> bool {
    if key.key == Key::Escape {
        if app.open_select.take().is_some() {
            return true;
        }
        if app.screen == Screen::Review {
            app.screen = Screen::Catalog;
            return true;
        }
    }
    false
}

fn main() {
    rune::App::new(AppSpec {
        min_size: Some((820.0, 560.0)),
        scenes: scenes::SCENES,
        poll: Some(poll),
        on_key: Some(on_key),
        fonts: FONTS,
        families: ("IBM Plex Mono", "IBM Plex Mono"),
        ..AppSpec::new("Soulcaster", (1100.0, 720.0), state::init, ui::view)
    })
    .titlebar(Titlebar::new(ui::TITLEBAR))
    .standard_menus()
    .run();
}
