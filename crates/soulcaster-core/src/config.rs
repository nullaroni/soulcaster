//! Reading a Config. The order matters: size cap, strict JSON, the version on its own, then the
//! closed schema. A rule that can change between releases (an unknown ID, a value the Catalog
//! refuses, a lint finding) isn't here, because those skip one entry instead of refusing the file.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use jsonschema::{Draft, PatternOptions, Validator};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

pub const SCHEMA_VERSION: u64 = 1;
pub const MAX_BYTES: usize = 1024 * 1024;

const SCHEMA: &str = include_str!("../../../schema/config-1.schema.json");

/// A Config, as read. Lists and maps hold what the file says, including IDs this app doesn't know.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub schema_version: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(default, skip_serializing_if = "Packages::is_empty")]
    pub packages: Packages,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, SettingValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dotfiles: Option<Dotfiles>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formulae: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub casks: Vec<String>,
}

impl Packages {
    pub fn is_empty(&self) -> bool {
        self.formulae.is_empty() && self.casks.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(Number),
    String(String),
    List(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Dotfiles {
    pub repo: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub links: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_script: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    TooLarge {
        bytes: usize,
    },
    /// Not strict RFC 8259: a syntax error, a duplicate key, `NaN`, or a number too large.
    Parse(String),
    /// This Config needs a newer Soulcaster. Nothing else in the file was read.
    NewerVersion {
        found: u64,
        known: u64,
    },
    /// The file breaks the closed format. `path` is where, such as `/packages/formulae/0`.
    Invalid {
        path: String,
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "this Machine Type is {bytes} bytes, over the {MAX_BYTES} limit"
            ),
            Self::Parse(message) => write!(f, "this Machine Type isn't strict JSON: {message}"),
            Self::NewerVersion { found, known } => write!(
                f,
                "this Machine Type uses schema version {found} and this app knows up to {known}, so update Soulcaster to read it"
            ),
            Self::Invalid { path, message } => {
                let at = if path.is_empty() { "top level" } else { path };
                write!(f, "this Machine Type is invalid at {at}: {message}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        if bytes.len() > MAX_BYTES {
            return Err(ConfigError::TooLarge { bytes: bytes.len() });
        }
        let value = strict_parse(bytes).map_err(ConfigError::Parse)?;
        if let Some(found) = value.get("schemaVersion").and_then(Value::as_u64)
            && found > SCHEMA_VERSION
        {
            return Err(ConfigError::NewerVersion {
                found,
                known: SCHEMA_VERSION,
            });
        }
        if let Some(error) = validator().iter_errors(&value).next() {
            return Err(ConfigError::Invalid {
                path: error.instance_path().to_string(),
                message: error.to_string(),
            });
        }
        serde_json::from_value(value).map_err(|e| ConfigError::Invalid {
            path: String::new(),
            message: e.to_string(),
        })
    }

    /// The one form the app writes: 2-space indent, final newline, fixed key order, sorted
    /// entries. A list that is a setting's value keeps its order, because the order means something.
    pub fn to_canonical_json(&self) -> String {
        let mut sorted = self.clone();
        sorted.items.sort();
        sorted.packages.formulae.sort();
        sorted.packages.casks.sort();
        let mut text =
            serde_json::to_string_pretty(&sorted).expect("a Machine Type always serializes");
        text.push('\n');
        text
    }
}

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(SCHEMA).expect("the shipped schema is JSON");
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .with_pattern_options(PatternOptions::regex())
            .build(&schema)
            .expect("the shipped schema compiles")
    })
}

/// serde_json keeps the last duplicate key, so a preview could show a value the engine doesn't
/// use. This visitor refuses duplicates at every depth. serde_json itself refuses `NaN`,
/// `Infinity`, `1e999`, lone surrogates, a byte order mark and nesting past 128 levels.
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;

        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = Strict;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Strict, E> {
                Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("number out of range"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_unit<E>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
                let mut out = Vec::new();
                while let Some(Strict(value)) = seq.next_element()? {
                    out.push(value);
                }
                Ok(Strict(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
                let mut out = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    let Strict(value) = map.next_value()?;
                    if out.insert(key.clone(), value).is_some() {
                        return Err(de::Error::custom(format!("duplicate key {key}")));
                    }
                }
                Ok(Strict(Value::Object(out)))
            }
        }

        deserializer.deserialize_any(StrictVisitor)
    }
}

/// JSON with no duplicate key at any depth. serde_json would keep the last one, so anything that
/// decides from a file reads it through this.
pub fn strict_parse(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice::<Strict>(bytes)
        .map(|strict| strict.0)
        .map_err(|e| e.to_string())
}
