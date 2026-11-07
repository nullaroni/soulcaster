//! `plan` in the lifecycle design: what a selection needs, computed without touching anything.
//! The Review screen shows it, and it re-derives on every answer, so it never asks for something
//! the plan doesn't need.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{BASE, catalog};
use crate::lifecycle::Stage;

/// Which sample Machine the simulated `check` reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Machine {
    Bare,
    BareTouchId,
    HasHomebrew,
}

impl Machine {
    pub fn base_present(self) -> bool {
        self == Machine::HasHomebrew
    }
    pub fn github_signed_in(self) -> bool {
        self == Machine::HasHomebrew
    }
    pub fn touch_id(self) -> bool {
        self == Machine::BareTouchId
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Why {
    Selected,
    Base,
    PulledIn { by: String },
}

#[derive(Clone, Debug)]
pub struct PlanEntry {
    pub id: String,
    pub name: String,
    pub stage: Stage,
    pub why: Why,
    pub is_setting: bool,
    pub already_ok: bool,
    /// It stops at a person's step or an open approval. It holds nothing open while it waits.
    pub waits_for: Option<Waits>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Waits {
    Person { title: String, detail: String, action: String, apple_account: bool },
    License,
    Tap(String),
}

#[derive(Clone, Debug)]
pub struct ConflictAsk {
    pub a: String,
    pub b: String,
    pub reason: String,
}

#[derive(Default, Clone)]
pub struct Answers {
    pub licenses: BTreeSet<String>,
    pub taps: BTreeSet<String>,
    /// Which side of a conflict won, keyed by both Items in sorted order.
    pub dropped: BTreeSet<String>,
}

pub struct Plan {
    pub entries: Vec<PlanEntry>,
    pub needs_password: bool,
    pub admin_names: Vec<String>,
    pub needs_github: bool,
    pub licenses: Vec<(String, String)>,
    pub taps: Vec<(String, Vec<String>)>,
    pub conflicts: Vec<ConflictAsk>,
    pub first_open: Vec<(String, String)>,
}

pub fn make_plan(selected: &BTreeSet<String>, settings: &BTreeMap<String, String>, machine: Machine, answers: &Answers) -> Plan {
    let cat = catalog();
    let mut conflicts = Vec::new();
    let chosen: BTreeSet<&String> = selected.iter().filter(|id| !answers.dropped.contains(*id)).collect();
    for id in &chosen {
        let Some(item) = cat.item(id) else { continue };
        for (other, reason) in cat.conflicts_of(item) {
            if chosen.contains(&other.id) && item.id < other.id {
                conflicts.push(ConflictAsk { a: item.id.clone(), b: other.id.clone(), reason: reason.to_string() });
            }
        }
    }

    // Base Items first, then what's selected, then whatever those require.
    let mut entries: Vec<PlanEntry> = BASE
        .iter()
        .map(|b| PlanEntry {
            id: b.id.to_string(),
            name: b.name.to_string(),
            stage: if b.id == "gh" || machine.base_present() { Stage::User } else { Stage::Admin },
            why: Why::Base,
            is_setting: false,
            already_ok: machine.base_present(),
            waits_for: None,
        })
        .collect();
    let mut seen: BTreeSet<String> = BASE.iter().map(|b| b.id.to_string()).collect();

    let mut queue: Vec<(String, Why)> = chosen.iter().map(|id| ((*id).clone(), Why::Selected)).collect();
    let mut at = 0;
    while at < queue.len() {
        let (id, why) = queue[at].clone();
        at += 1;
        if !seen.insert(id.clone()) {
            continue;
        }
        for need in cat.requires(&id) {
            if !seen.contains(need) {
                queue.push((need.clone(), Why::PulledIn { by: cat.name_of(&id) }));
            }
        }
        let Some(item) = cat.item(&id) else {
            entries.push(PlanEntry { name: id.clone(), id, stage: Stage::User, why, is_setting: false, already_ok: false, waits_for: None });
            continue;
        };
        let wants_admin = item.needs_from_person.iter().any(|n| n.kind == "admin-password");
        let tap_ok = item.tap.as_ref().is_none_or(|t| answers.taps.contains(t));
        let waits_for = if !tap_ok {
            item.tap.clone().map(Waits::Tap)
        } else if let Some(step) = cat.person_step(&id) {
            Some(Waits::Person {
                title: step.title.clone(),
                detail: step.detail.clone(),
                action: step.action.clone(),
                apple_account: step.kind == "apple-account",
            })
        } else if item.needs_from_person.iter().any(|n| n.kind == "license") && !answers.licenses.contains(&id) {
            Some(Waits::License)
        } else {
            None
        };
        // Rule 2 in the lifecycle: an Item is in the Admin Stage only if it would reach an admin
        // step with no open person-gate before it, and third-party tap code never is.
        let admin = wants_admin && item.tap.is_none() && waits_for.is_none();
        entries.push(PlanEntry {
            name: item.name.clone(),
            id,
            stage: if admin { Stage::Admin } else { Stage::User },
            why,
            is_setting: false,
            already_ok: false,
            waits_for,
        });
    }

    for (id, _) in settings {
        if let Some(setting) = cat.setting(id) {
            for need in &setting.requires {
                if seen.insert(need.clone()) && cat.item(need).is_some() {
                    entries.push(PlanEntry {
                        id: need.clone(),
                        name: cat.name_of(need),
                        stage: Stage::User,
                        why: Why::PulledIn { by: setting.title.clone() },
                        is_setting: false,
                        already_ok: false,
                        waits_for: None,
                    });
                }
            }
            entries.push(PlanEntry {
                id: id.clone(),
                name: setting.title.clone(),
                stage: Stage::User,
                why: Why::Selected,
                is_setting: true,
                already_ok: false,
                waits_for: None,
            });
        }
    }

    // The lifecycle's User Stage order: official installs, settings, then third-party tap Items.
    let order = |e: &PlanEntry| -> (u8, u8) {
        let third_party = cat.item(&e.id).and_then(|i| i.tap.as_ref()).is_some();
        let stage = if e.stage == Stage::Admin { 0 } else { 1 };
        let within = if e.why == Why::Base { 0 } else if e.is_setting { 2 } else if third_party { 3 } else { 1 };
        (stage, within)
    };
    entries.sort_by_key(order);

    let admin_names: Vec<String> =
        entries.iter().filter(|e| e.stage == Stage::Admin && !e.already_ok).map(|e| e.name.clone()).collect();

    let licenses: Vec<(String, String)> = entries
        .iter()
        .filter_map(|e| cat.item(&e.id))
        .filter(|i| i.needs_from_person.iter().any(|n| n.kind == "license"))
        .map(|i| (i.id.clone(), i.name.clone()))
        .collect();
    let mut tap_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in entries.iter().filter_map(|e| cat.item(&e.id)) {
        if let Some(tap) = &item.tap {
            tap_map.entry(tap.clone()).or_default().push(item.name.clone());
        }
    }
    let first_open = entries
        .iter()
        .filter_map(|e| cat.item(&e.id))
        .flat_map(|i| i.opens_after.iter().map(move |n| (i.name.clone(), n.detail.clone())))
        .collect();

    Plan {
        needs_password: !admin_names.is_empty(),
        admin_names,
        needs_github: !machine.github_signed_in(),
        licenses,
        taps: tap_map.into_iter().collect(),
        conflicts,
        first_open,
        entries,
    }
}

impl Plan {
    pub fn waiting(&self) -> impl Iterator<Item = &PlanEntry> {
        self.entries.iter().filter(|e| e.waits_for.is_some())
    }
    pub fn count_where(&self, f: impl Fn(&PlanEntry) -> bool) -> usize {
        self.entries.iter().filter(|e| f(e)).count()
    }
}
