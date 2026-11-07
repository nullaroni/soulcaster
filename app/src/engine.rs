//! The one thing the app knows about an engine. Everything a screen shows comes from the events
//! `poll` returns, so wiring in the real engine means implementing this trait and changing the
//! single line in `state::init` that builds it.

use soulcaster_core::ask::Password;
use soulcaster_core::event::Event;
use soulcaster_core::plan::Plan;

use crate::answers::Answers;

pub trait Engine {
    /// Starts a full Run of an approved plan.
    fn start_run(&mut self, plan: &Plan, answers: &Answers);
    /// A retry, or a finished queue entry: a smaller Run over one Item and what waits on it.
    fn start_scoped(&mut self, item: &str, waiting: &[String], queue_id: Option<&str>);
    /// Validates the password once. The result comes back as `ask.state` events: `done` when it
    /// worked, `waiting` when it should be asked for again. Passwords are never events.
    fn submit_password(&mut self, password: Password);
    /// Starts GitHub's device flow and returns the code to show.
    fn start_github_sign_in(&mut self) -> String;
    /// Whatever has happened since the last call. `now` is seconds on the window's clock.
    fn poll(&mut self, now: f64) -> Vec<Event>;
    /// Seconds until the next event is expected, for engines that know.
    fn next_due(&self, now: f64) -> Option<f64>;
}
