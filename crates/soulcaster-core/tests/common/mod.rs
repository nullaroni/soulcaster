#![allow(dead_code)]

use std::path::{Path, PathBuf};

use soulcaster_core::catalog::Catalog;

/// The Config format's stub Catalog. It keeps the renamed, retired and deprecated examples the
/// shipped Catalog has no reason to hold, so the rules for them have something to run on.
pub fn stub() -> Catalog {
    Catalog::parse(include_str!("../fixtures/catalog-stub.json")).expect("the stub parses")
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

pub fn example(path: &str) -> Vec<u8> {
    std::fs::read(repo_root().join("schema/examples").join(path)).unwrap()
}
