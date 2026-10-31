//! Checks a crate's integration tests are all built: with `autotests = false`, a `tests/*.rs` file is only compiled
//! as a module of `tests/main.rs` or as its own `[[test]]` in Cargo.toml, and one left out would silently not run.
//! Included by each `tests/main.rs` with `#[path = ".../test-support/all_listed.rs"] mod all_listed;`.

#[test]
fn every_test_file_is_built() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main = std::fs::read_to_string(dir.join("tests/main.rs")).unwrap();
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(dir.join("tests")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let Some(stem) = name.strip_suffix(".rs") else { continue };
        if stem == "main" || main.contains(&format!("mod {stem};")) || manifest.contains(&format!("\"tests/{name}\"")) {
            continue;
        }
        missing.push(name);
    }
    assert!(
        missing.is_empty(),
        "add these to tests/main.rs as `mod <name>;` (or as a [[test]] in Cargo.toml): {missing:?}"
    );
}
