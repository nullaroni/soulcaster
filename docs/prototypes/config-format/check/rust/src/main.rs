//! PROTOTYPE: the engine's language, with the real crates. It proves the schema compiles under plain `regex`,
//! gives the same answers as Python and Ajv, and that a strict parser refuses what stock serde_json accepts.
//!
//!     config-check <docs/prototypes/config-format> <corpus from check.py> [other-schema.json ...]
use jsonschema::{Draft, PatternOptions};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::Path, process};

const MAX_BYTES: usize = 1024 * 1024;

/// serde_json keeps the last duplicate key, so the preview could show a value the engine doesn't use.
/// This visitor refuses duplicates at every depth. serde_json already refuses NaN, Infinity, 1e999,
/// lone surrogates, a byte order mark and nesting past 128 levels.
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("number out of range"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_unit<E>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut out = Vec::new();
                while let Some(Strict(v)) = a.next_element()? {
                    out.push(v);
                }
                Ok(Strict(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut out = Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    let Strict(v) = a.next_value()?;
                    if out.insert(k.clone(), v).is_some() {
                        return Err(de::Error::custom(format!("duplicate key {k}")));
                    }
                }
                Ok(Strict(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}

fn strict_parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("file is over {MAX_BYTES} bytes"));
    }
    serde_json::from_slice::<Strict>(bytes).map(|s| s.0).map_err(|e| e.to_string())
}

fn patterns<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x) {
                    ("pattern", Value::String(s)) => out.push(s),
                    _ => patterns(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| patterns(x, out)),
        _ => {}
    }
}

fn json_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let schema = strict_parse(&fs::read(root.join("config.schema.json")).unwrap()).expect("schema parses");
    let corpus: Value = serde_json::from_slice(&fs::read(&args[2]).unwrap()).unwrap();
    let mut failures = 0;

    // 1. Every pattern compiles under the plain regex crate, including the other prototype's and the stub's.
    let stub = strict_parse(&fs::read(root.join("catalog-stub.json")).unwrap()).expect("stub parses");
    let mut sources = vec![("config.schema.json".to_string(), schema.clone()), ("catalog-stub.json".to_string(), stub)];
    for extra in &args[3..] {
        sources.push((extra.rsplit('/').next().unwrap().to_string(), strict_parse(&fs::read(extra).unwrap()).expect("extra schema parses")));
    }
    for (name, doc) in &sources {
        let mut found = Vec::new();
        patterns(doc, &mut found);
        let bad: Vec<_> = found.iter().filter(|p| regex::Regex::new(p).is_err()).collect();
        failures += bad.len();
        println!("plain regex: {}/{} patterns in {name} compile", found.len() - bad.len(), found.len());
        for b in bad {
            println!("  DOES NOT COMPILE: {b}");
        }
    }

    // 2. The validator itself, using the plain regex engine the app would use.
    let plain = jsonschema::options().with_draft(Draft::Draft202012).with_pattern_options(PatternOptions::regex()).build(&schema).expect("schema compiles with plain regex");
    let fancy = jsonschema::options().with_draft(Draft::Draft202012).with_pattern_options(PatternOptions::fancy_regex()).build(&schema).expect("schema compiles with fancy-regex");

    // 3. Examples go through the strict parser first, as the engine's would.
    for (dir, want_valid) in [("examples", true), ("examples/invalid", false), ("examples/lint", true)] {
        let (mut pass, mut total) = (0, 0);
        for path in json_files(&root.join(dir)) {
            total += 1;
            let ok = strict_parse(&fs::read(&path).unwrap()).map(|v| plain.is_valid(&v)).unwrap_or(false);
            if ok == want_valid {
                pass += 1;
            } else {
                failures += 1;
                println!("UNEXPECTED {}", path.display());
            }
        }
        println!("{dir}: {pass}/{total} behaved as expected ({})", if want_valid { "accepted" } else { "refused" });
    }

    let cases = corpus["cases"].as_array().unwrap();
    let mut right = 0;
    for c in cases {
        if plain.is_valid(&c["config"]) == c["valid"].as_bool().unwrap() {
            right += 1;
        } else {
            failures += 1;
            println!("UNEXPECTED {}", c["label"]);
        }
    }
    println!("conformance: {right}/{} behaved as expected", cases.len());

    let parser = corpus["parser"].as_array().unwrap();
    let mut right = 0;
    for c in parser {
        let got = strict_parse(c["text"].as_str().unwrap().as_bytes()).map(|v| plain.is_valid(&v)).unwrap_or(false);
        if got == c["valid"].as_bool().unwrap() {
            right += 1;
        } else {
            failures += 1;
            println!("UNEXPECTED parser case: {}", c["label"]);
        }
    }
    println!("parser cases: {right}/{} behaved as expected", parser.len());

    let dupes = br#"{"schemaVersion":99,"schemaVersion":1,"name":"x"}"#;
    let stock = serde_json::from_slice::<Value>(dupes).is_ok();
    println!("duplicate schemaVersion: stock serde_json {}, strict parser {}", if stock { "accepts" } else { "refuses" }, if strict_parse(dupes).is_err() { "refuses" } else { failures += 1; "ACCEPTS" });

    let fuzz = corpus["fuzz"].as_array().unwrap();
    let (mut verdicts, mut differs) = (String::with_capacity(fuzz.len()), 0);
    for c in fuzz {
        let v = plain.is_valid(c);
        differs += usize::from(v != fancy.is_valid(c));
        verdicts.push(if v { '1' } else { '0' });
    }
    let accepted = verdicts.matches('1').count();
    let hash: String = Sha256::digest(verdicts.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    println!("fuzz: {} cases, {accepted} accepted, sha256 {hash}", verdicts.len());
    println!("fuzz: plain regex and fancy-regex differ on {differs} cases");
    failures += differs;
    process::exit(i32::from(failures > 0));
}
