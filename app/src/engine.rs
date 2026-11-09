//! The one thing the app knows about an engine. Everything a screen shows comes from the events
//! `poll` returns, so wiring in the real engine means implementing this trait and changing the
//! single line in `state::init` that builds it.

use soulcaster_core::ask::{Password, PasswordRequest};
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Choice, Row};
use soulcaster_core::event::Event;
use soulcaster_core::plan::Plan;

use crate::answers::Answers;
use crate::drift_flow::Checked;
use crate::fleet_flow::FleetNews;

/// What came back from Drift's work on the engine's thread.
pub enum DriftNews {
    Checked(Checked, Vec<String>),
    /// A Run began for what the person chose to Apply.
    Applying(Vec<String>),
    Failed(Vec<String>),
}

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
    /// Whether a Run here really changes this Mac. A real engine also asks during the Run, when
    /// it knows what's needed, so Review doesn't ask first.
    fn is_real(&self) -> bool {
        false
    }
    /// The Run is waiting for the password, and why.
    fn password_request(&self) -> Option<PasswordRequest> {
        None
    }
    /// The person chose not to give it. Admin work goes to the queue.
    fn decline_password(&mut self) {}
    /// This Mac now follows this Config, for Drift to compare with.
    fn follow(&mut self, _config: &Config, _saved: bool) {}
    /// Looks at this Mac. The answer arrives through `poll_drift`.
    fn check_drift(&mut self) {}
    /// Carries out what the person chose. Only the Drift screen's confirmations reach this.
    fn carry_out_drift(&mut self, _decisions: Vec<(Row, Choice)>) {}
    fn send_waiting_saves(&mut self) {}
    fn discard_waiting_saves(&mut self) {}
    fn poll_drift(&mut self) -> Option<DriftNews> {
        None
    }
    /// Reads the Fleet. The answer arrives through `poll_fleet`.
    fn refresh_fleet(&mut self) {}
    /// Sends this Mac's Machine Entry, with how many differences Drift last found if it looked.
    fn publish_self(&mut self, _differences: Option<u32>) {}
    /// Looks at this Mac to start a Machine Type. The answer arrives through `poll_fleet`.
    fn survey_this_mac(&mut self) {}
    fn poll_fleet(&mut self) -> Option<FleetNews> {
        None
    }
    /// GitHub's device code and address, while sign-in is waiting on the person.
    fn github_code(&self) -> Option<(String, String)> {
        None
    }
}
