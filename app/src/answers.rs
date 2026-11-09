//! What a person answered on Review, and what that means for each Item.

use std::collections::BTreeSet;

use macflayer_core::plan::{NeedKind, PlanEntry};

#[derive(Default, Clone, Debug)]
pub struct Answers {
    pub licenses: BTreeSet<String>,
    pub taps: BTreeSet<String>,
}

/// Why an Item would stop and wait for a person. It holds nothing open while it waits.
#[derive(Clone, Debug, PartialEq)]
pub enum Waits {
    Tap(String),
    Person { text: String, link: Option<String> },
    License,
}

pub fn waits_for(entry: &PlanEntry, answers: &Answers) -> Option<Waits> {
    if let Some(tap) = entry
        .recipe
        .taps()
        .into_iter()
        .find(|tap| !answers.taps.contains(tap))
    {
        return Some(Waits::Tap(tap));
    }
    if let Some(need) = entry
        .needs
        .iter()
        .find(|need| need.kind == NeedKind::Person)
    {
        return Some(Waits::Person {
            text: need.text.clone(),
            link: need.link.clone(),
        });
    }
    let has_license = entry
        .needs
        .iter()
        .any(|need| need.kind == NeedKind::License);
    (has_license && !answers.licenses.contains(&entry.id)).then_some(Waits::License)
}
