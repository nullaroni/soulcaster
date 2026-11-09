mod common;

use common::{repo_root, stub};
use jsonschema::{Draft, PatternOptions};
use macflayer_core::catalog::{Catalog, Lookup};
use macflayer_core::lifecycle::Stage;
use macflayer_core::recipe::{Recipe, Step};
use serde_json::{Value, json};

fn recipe_schema() -> jsonschema::Validator {
    let schema: Value = serde_json::from_slice(
        &std::fs::read(repo_root().join("schema/recipe-1.schema.json")).unwrap(),
    )
    .unwrap();
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .with_pattern_options(PatternOptions::regex())
        .build(&schema)
        .expect("the recipe schema compiles under plain regex")
}

fn lint_of(mutate: impl Fn(&mut Value)) -> Vec<String> {
    let mut file: Value = serde_json::from_str(include_str!("fixtures/catalog-stub.json")).unwrap();
    mutate(&mut file);
    Catalog::parse(&file.to_string())
        .expect("the mutated stub still parses")
        .lint()
}

fn entry_mut<'a>(file: &'a mut Value, id: &str) -> &'a mut Value {
    file["recipes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["id"] == id)
        .unwrap()
}

#[test]
fn the_shipped_catalog_loads_with_243_items_in_15_sections() {
    let catalog = Catalog::shipped();
    assert_eq!(catalog.sections().len(), 15);
    assert_eq!(catalog.listed().count(), 243);
    for entry in catalog.listed() {
        let listing = entry.listing.as_ref().unwrap();
        assert!(
            catalog.sections().iter().any(|s| s.id == listing.section),
            "{}",
            entry.id()
        );
    }
}

#[test]
fn the_shipped_catalog_passes_catalog_lint() {
    assert_eq!(Catalog::shipped().lint(), Vec::<String>::new());
}

#[test]
fn the_stub_passes_catalog_lint() {
    assert_eq!(stub().lint(), Vec::<String>::new());
}

#[test]
fn base_items_are_exactly_the_three_and_are_never_listed() {
    let catalog = Catalog::shipped();
    let base: Vec<&str> = catalog.base_items().map(|e| e.id()).collect();
    assert_eq!(base, ["command-line-tools", "homebrew", "gh"]);
    assert!(catalog.listed().all(|e| !e.recipe.is_base()));
}

#[test]
fn every_item_gets_a_recipe_and_every_recipe_matches_the_schema() {
    let validator = recipe_schema();
    let catalog = Catalog::shipped();
    let recipes: Vec<&Recipe> = catalog.entries().map(|e| &e.recipe).collect();
    assert_eq!(
        recipes.len(),
        281,
        "243 listed Items, plus Base Items, settings and Items only a Recipe names"
    );
    let file = json!({"schemaVersion": 1, "recipes": recipes});
    let errors: Vec<String> = validator
        .iter_errors(&file)
        .take(5)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn a_plain_item_gets_its_recipe_from_its_install_data() {
    let catalog = Catalog::shipped();
    let node = catalog.entry("node").unwrap();
    assert_eq!(node.name(), "Node.js");
    assert_eq!(node.recipe.requires(), ["homebrew"]);
    assert!(matches!(&node.recipe.steps()[0], Step::BrewFormula { name, .. } if name == "node"));
    assert_eq!(node.covers_packages, ["formula:node"]);

    let cask = catalog.entry("firefox").unwrap();
    assert!(
        matches!(&cask.recipe.steps()[0], Step::BrewCask { name, admin: false, .. } if name == "firefox")
    );
}

#[test]
fn a_tap_item_installs_by_its_full_name_and_never_gets_admin() {
    let catalog = Catalog::shipped();
    let yabai = catalog.entry("yabai").unwrap();
    assert!(
        matches!(&yabai.recipe.steps()[0], Step::BrewFormula { name, .. } if name == "asmvik/formulae/yabai")
    );
    assert_eq!(yabai.recipe.taps(), ["asmvik/formulae"]);
    assert!(!yabai.recipe.has_admin_step());
}

#[test]
fn an_official_cask_with_a_pkg_is_admin_and_a_formula_never_is() {
    let catalog = Catalog::shipped();
    assert!(catalog.entry("virtualbox").unwrap().recipe.has_admin_step());
    let openjdk = catalog.entry("openjdk").unwrap();
    assert!(!openjdk.recipe.has_admin_step());
}

#[test]
fn explicit_recipes_win_over_install_data_and_keep_the_listing() {
    let catalog = Catalog::shipped();
    let xcode = catalog.entry("xcode").unwrap();
    assert_eq!(xcode.name(), "Xcode");
    assert!(xcode.listing.is_some());
    assert!(matches!(xcode.recipe.steps()[0], Step::NeedsYou { .. }));
    assert_eq!(xcode.recipe.stage(|_| false), Stage::User);
    assert_eq!(
        xcode.recipe.stage(|_| true),
        Stage::User,
        "every gated step is skipped once Xcode is ready"
    );
    // Xcode has appeared, but it isn't ready yet: it reaches its admin steps with no gate open.
    let appeared = |p: &macflayer_core::recipe::Probe| {
        matches!(p, macflayer_core::recipe::Probe::AppBundle { .. })
    };
    assert_eq!(xcode.recipe.stage(appeared), Stage::Admin);
}

#[test]
fn old_ids_resolve_forever_and_unknown_ids_get_a_suggestion() {
    let catalog = stub();
    match catalog.lookup("example-old-name") {
        Lookup::Found {
            entry,
            renamed_from,
        } => {
            assert_eq!(entry.id(), "ghostty");
            assert_eq!(renamed_from, Some("example-old-name"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        catalog.lookup("ghostty"),
        Lookup::Found {
            renamed_from: None,
            ..
        }
    ));
    match catalog.lookup("ghosty") {
        Lookup::Unknown { suggestion } => assert_eq!(suggestion.as_deref(), Some("ghostty")),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        catalog.lookup("nothing-like-it-at-all"),
        Lookup::Unknown { suggestion: None }
    ));
}

#[test]
fn a_rename_cycle_stops_instead_of_hanging() {
    let catalog = Catalog::parse(
        r#"{"recipes":[{"id":"loop-a","renamedTo":"loop-b"},{"id":"loop-b","renamedTo":"loop-a"}]}"#,
    )
    .unwrap();
    assert!(matches!(catalog.lookup("loop-a"), Lookup::Cycle));
    assert!(catalog.lint().iter().any(|p| p.contains("rename cycle")));
}

#[test]
fn a_recipe_field_outside_the_closed_set_is_refused_at_load() {
    let file = |mutate: &dyn Fn(&mut Value)| {
        let mut file: Value =
            serde_json::from_str(include_str!("fixtures/catalog-stub.json")).unwrap();
        mutate(&mut file);
        Catalog::parse(&file.to_string())
    };
    assert!(file(&|f| entry_mut(f, "ghostty")["postInstall"] = json!("curl | sh")).is_err());
    assert!(
        file(&|f| entry_mut(f, "ghostty")["steps"][0] = json!({"type": "shell", "command": "x"}))
            .is_err()
    );
}

#[test]
fn the_lifecycle_refuses_schema_breaking_recipes() {
    // Each case in the prototype changes one thing, so the change is the only reason it can fail.
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/recipes-examples.json")).unwrap();
    let validator = recipe_schema();
    assert!(
        validator.is_valid(&fixture["catalog"]),
        "the examples themselves pass"
    );
    let cases = fixture["mustFailSchema"].as_array().unwrap();
    assert_eq!(cases.len(), 30);
    for case in cases {
        let file = patched(&fixture["catalog"], case);
        assert!(
            !validator.is_valid(&file),
            "{} passed the schema",
            case["name"]
        );
    }
}

#[test]
fn the_lifecycle_lint_cases_are_each_caught() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/recipes-examples.json")).unwrap();
    assert_eq!(
        Catalog::parse(&fixture["catalog"].to_string())
            .unwrap()
            .lint(),
        Vec::<String>::new()
    );
    let cases = fixture["mustFailLint"].as_array().unwrap();
    assert_eq!(cases.len(), 9);
    for case in cases {
        let file = patched(&fixture["catalog"], case);
        let problems = Catalog::parse(&file.to_string()).unwrap().lint();
        assert!(!problems.is_empty(), "{} passed lint", case["name"]);
    }
}

/// Applies a case's small JSON Patch to the recipe it names.
fn patched(catalog: &Value, case: &Value) -> Value {
    let mut file = catalog.clone();
    let recipe = entry_mut(&mut file, case["from"].as_str().unwrap());
    for op in case["patch"].as_array().unwrap() {
        let path = op["path"].as_str().unwrap();
        let (parent, last) = path.rsplit_once('/').unwrap();
        let target = recipe
            .pointer_mut(parent)
            .unwrap_or_else(|| panic!("{path}"));
        match (op["op"].as_str().unwrap(), target) {
            ("remove", Value::Array(list)) => {
                list.remove(last.parse().unwrap());
            }
            ("remove", Value::Object(map)) => {
                map.remove(last);
            }
            ("add", Value::Array(list)) if last == "-" => list.push(op["value"].clone()),
            ("add", Value::Array(list)) => list.insert(last.parse().unwrap(), op["value"].clone()),
            ("replace", Value::Array(list)) => {
                list[last.parse::<usize>().unwrap()] = op["value"].clone()
            }
            (_, Value::Object(map)) => {
                map.insert(last.to_string(), op["value"].clone());
            }
            (name, target) => panic!("{name} on {target}"),
        }
    }
    file
}

#[test]
fn catalog_lint_catches_each_mutation_from_the_prototype() {
    type Mutation = fn(&mut Value);
    let cases: &[(&str, Mutation)] = &[
        ("renamedTo names an unknown ID", |f| {
            entry_mut(f, "example-old-name")["renamedTo"] = json!("gone")
        }),
        ("a rename can't change an Item into a setting", |f| {
            entry_mut(f, "example-old-name")["renamedTo"] = json!("dock.autohide")
        }),
        ("base Items must be exactly", |f| {
            entry_mut(f, "ghostty")["base"] = json!(true)
        }),
        ("an integer setting needs bounds", |f| {
            entry_mut(f, "keyboard.key-repeat")
                .as_object_mut()
                .unwrap()
                .remove("maximum");
        }),
        ("a setting needs a read path", |f| {
            entry_mut(f, "dock.autohide")["stores"] = json!([])
        }),
        ("a pattern can't use lookaround", |f| {
            entry_mut(f, "git.default-branch")["pattern"] = json!("^(?!-)[a-z]+$")
        }),
        ("a retired Item keeps its detection data", |f| {
            entry_mut(f, "example-retired-tool")["checks"] = json!([]);
        }),
        ("covered by more than one Item", |f| {
            entry_mut(f, "ghostty")["coversPackages"] = json!(["cask:ghostty", "formula:opencode"]);
        }),
        ("coversPackages must list cask:ghostty", |f| {
            entry_mut(f, "ghostty")["coversPackages"] = json!([])
        }),
        ("needs formula: or cask:", |f| {
            entry_mut(f, "tailscale")["conflictsWith"] = json!(["tailscale"])
        }),
        ("replacedBy names an unknown ID", |f| {
            entry_mut(f, "example-retired-tool")["retired"]["replacedBy"] = json!("gone")
        }),
        ("unknown requirement not-there", |f| {
            entry_mut(f, "dock.apps")["requires"] = json!(["not-there"])
        }),
    ];
    for (want, mutate) in cases {
        let problems = lint_of(mutate);
        assert!(
            problems.iter().any(|p| p.contains(want)),
            "{want}: {problems:?}"
        );
    }
}
