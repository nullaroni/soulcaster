//! A throwaway prototype of Soulcaster's main screens, answering ticket #16. Everything is
//! simulated: it has no code path that installs, runs `sudo`, changes a setting or provisions
//! over the network.
//!
//! ```sh
//! cargo run                                      # a real window
//! cargo run -- --snap out.png --scene catalog    # one screen, headless
//! cargo run -- --script scripts/full-flow.json --snap out.png --at mark:finish
//! ```

mod catalog;
mod drift;
mod lifecycle;
mod plan;
mod scenes;
mod sim;
mod state;
mod ui;

use std::time::Duration;

use rune::{AppSpec, Key, KeyEvent, Titlebar, Tx};

use crate::state::{App, Screen};

fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let changed = app.tick(clock).is_some();
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
    let stress = std::env::args().any(|a| a == "stress");
    let init: fn() -> App = if stress { scenes::stress_init } else { state::init };
    rune::App::new(AppSpec {
        min_size: Some((960.0, 640.0)),
        scenes: scenes::SCENES,
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Soulcaster", (1180.0, 800.0), init, ui::view)
    })
    .titlebar(Titlebar::new(ui::TITLEBAR))
    .standard_menus()
    .run();
}
