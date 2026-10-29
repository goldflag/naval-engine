//! Every integration test of this crate in one test program: each file in this directory is a module of it.
//! `bun run rust:test -- --native -p naval-land-bake <file stem>` runs one file (its module).

mod release;
mod support;

/// With `autotests = false`, a file in tests/ that is neither declared above nor its own `[[test]]` in Cargo.toml would
/// never be compiled or run.
#[test]
fn every_file_in_tests_is_a_module_here() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("tests");
    let all = std::fs::read_to_string(dir.join("all.rs")).unwrap();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if let Some(stem) = name.strip_suffix(".rs").filter(|stem| *stem != "all") {
            assert!(
                all.contains(&format!("mod {stem};")),
                "tests/{name} is not a module of tests/all.rs"
            );
        }
    }
}
