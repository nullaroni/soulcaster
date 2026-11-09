//! The prototype's check cases, as Rust tests against the real parser.

mod common;

use common::{example, json_files, repo_root};
use macflayer_core::config::{Config, ConfigError};
use serde_json::{Map, Value, json};

fn conformance() -> Value {
    serde_json::from_slice(&std::fs::read(repo_root().join("schema/conformance.json")).unwrap())
        .unwrap()
}

#[test]
fn every_valid_example_is_accepted() {
    let files = json_files(&repo_root().join("schema/examples"));
    assert_eq!(files.len(), 5);
    for path in files {
        Config::parse(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

#[test]
fn every_invalid_example_is_refused() {
    let files = json_files(&repo_root().join("schema/examples/invalid"));
    assert_eq!(files.len(), 30);
    for path in files {
        assert!(
            Config::parse(&std::fs::read(&path).unwrap()).is_err(),
            "{} was accepted",
            path.display()
        );
    }
}

#[test]
fn lint_examples_pass_the_schema_because_lint_only_skips_an_entry() {
    let files = json_files(&repo_root().join("schema/examples/lint"));
    assert_eq!(files.len(), 16);
    for path in files {
        Config::parse(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

fn config_with(fields: &Map<String, Value>) -> Vec<u8> {
    let mut config = Map::new();
    config.insert("schemaVersion".to_string(), json!(1));
    config.insert("name".to_string(), json!("probe"));
    config.extend(fields.clone());
    serde_json::to_vec(&Value::Object(config)).unwrap()
}

#[test]
fn schema_cases_behave_as_expected() {
    let conformance = conformance();
    let cases = conformance["schemaCases"].as_array().unwrap();
    assert_eq!(cases.len(), 108);
    for case in cases {
        let bytes = config_with(case["fields"].as_object().unwrap());
        let got = Config::parse(&bytes);
        assert_eq!(
            got.is_ok(),
            case["valid"] == json!(true),
            "{}: {got:?}",
            case["label"]
        );
    }
}

#[test]
fn size_cases_behave_as_expected() {
    let conformance = conformance();
    let cases = conformance["sizeCases"].as_array().unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        let count = case["count"].as_u64().unwrap();
        let template = case["template"].as_str().unwrap();
        let names: Vec<String> = (0..count)
            .map(|i| template.replace("{}", &i.to_string()))
            .collect();
        let is_map = case["map"] == json!(true);
        let in_repo = case["repo"] == json!(true);
        let mut leaf = if is_map {
            let filler = if in_repo { json!("x") } else { json!(true) };
            Value::Object(names.iter().map(|n| (n.clone(), filler.clone())).collect())
        } else {
            json!(names)
        };
        let path: Vec<&str> = case["path"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        for key in path[1..].iter().rev() {
            leaf = json!({ *key: leaf });
        }
        let mut fields = Map::new();
        fields.insert(path[0].to_string(), leaf);
        if in_repo {
            fields
                .get_mut("dotfiles")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("repo".to_string(), json!("example/dotfiles"));
        }
        let got = Config::parse(&config_with(&fields));
        assert_eq!(
            got.is_ok(),
            case["valid"] == json!(true),
            "{}: {:?}",
            case["label"],
            got.err()
        );
    }
}

#[test]
fn parser_cases_behave_as_expected() {
    let conformance = conformance();
    let cases = conformance["parserCases"].as_array().unwrap();
    assert_eq!(cases.len(), 18);
    for case in cases {
        let mut text = case["text"]
            .as_str()
            .unwrap()
            .repeat(case["repeat"].as_u64().unwrap_or(1) as usize);
        let pad = case["padTo"].as_u64().unwrap_or(0) as usize;
        if pad > text.len() {
            text.push_str(&" ".repeat(pad - text.len()));
        }
        let got = Config::parse(text.as_bytes());
        assert_eq!(
            got.is_ok(),
            case["valid"] == json!(true),
            "{}: {:?}",
            case["label"],
            got.err()
        );
    }
}

#[test]
fn a_duplicate_key_is_named_at_any_depth() {
    let err = Config::parse(br#"{"schemaVersion":1,"name":"x","settings":{"a.b":1,"a.b":2}}"#)
        .unwrap_err();
    assert!(
        matches!(&err, ConfigError::Parse(text) if text.contains("duplicate key a.b")),
        "{err}"
    );
    // Stock serde_json would keep the last value and report 1 here.
    let err = Config::parse(br#"{"schemaVersion":99,"schemaVersion":1,"name":"x"}"#).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Parse(text) if text.contains("schemaVersion")),
        "{err}"
    );
}

#[test]
fn an_older_app_refuses_a_newer_config_by_reading_only_its_version() {
    // Fields this app has never heard of don't matter: the version answer comes first.
    let err = Config::parse(br#"{"schemaVersion":2,"name":"x","fromTheFuture":true}"#).unwrap_err();
    assert!(
        matches!(err, ConfigError::NewerVersion { found: 2, known: 1 }),
        "{err}"
    );
    assert!(err.to_string().contains("update MacFlayer"), "{err}");
}

#[test]
fn a_missing_or_non_integer_version_rejects_the_file() {
    for text in [
        r#"{"name":"x"}"#,
        r#"{"schemaVersion":"1","name":"x"}"#,
        r#"{"schemaVersion":0,"name":"x"}"#,
        r#"{"schemaVersion":1.5,"name":"x"}"#,
    ] {
        assert!(
            matches!(
                Config::parse(text.as_bytes()),
                Err(ConfigError::Invalid { .. })
            ),
            "{text}"
        );
    }
}

#[test]
fn an_unknown_field_rejects_the_file_and_the_error_names_it() {
    let err = Config::parse(br#"{"schemaVersion":1,"name":"x","setings":{}}"#).unwrap_err();
    assert!(err.to_string().contains("setings"), "{err}");
    let err = Config::parse(br#"{"schemaVersion":1,"name":"x","packages":{"taps":["a/b"]}}"#)
        .unwrap_err();
    assert!(err.to_string().contains("packages"), "{err}");
}

#[test]
fn the_parsed_config_holds_the_file_typed() {
    let config = Config::parse(&example("owner-personal.json")).unwrap();
    assert!(config.items.iter().any(|i| i == "ghostty"));
    assert_eq!(
        config.settings["dock.autohide"],
        macflayer_core::config::SettingValue::Bool(true)
    );
    assert!(
        config
            .dotfiles
            .as_ref()
            .unwrap()
            .links
            .contains_key("~/.zshrc")
    );
}

#[test]
fn canonical_examples_write_back_byte_for_byte() {
    for name in [
        "developer.json",
        "lifecycle.json",
        "minimal.json",
        "owner-personal.json",
        "owner-work.json",
    ] {
        let bytes = example(name);
        let config = Config::parse(&bytes).unwrap();
        assert_eq!(
            config.to_canonical_json(),
            String::from_utf8(bytes).unwrap(),
            "{name}"
        );
    }
}
