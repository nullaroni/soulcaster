//! The data behind the screens. check/check.py writes plans.json from the sample Configs, using the Config format's
//! own parser, resolve() and lint, so what renders here is what those rules produced. Nothing is fetched.

use serde_json::Value;
use std::sync::OnceLock;

pub fn all() -> &'static Value {
    static PLANS: OnceLock<Value> = OnceLock::new();
    PLANS.get_or_init(|| serde_json::from_str(include_str!("../plans.json")).expect("plans.json"))
}

pub fn plan(id: &str) -> &'static Value {
    &all()["plans"][id]
}

pub fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

pub fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

pub fn strs<'a>(v: &'a Value, key: &str) -> Vec<&'a str> {
    list(v, key).iter().filter_map(|x| x.as_str()).collect()
}
