/// Golden file tests for error messages.
///
/// Each file in tests/fixtures/errors/ (rules-NN.yaml or rules-NN.json) is parsed
/// through the same pipeline as the server. The error/warning output is compared
/// against the corresponding .error.txt file.
///
/// To regenerate expected files after implementation changes:
///   UPDATE_ERRORS=1 cargo test error_fixtures -- --nocapture

use std::path::Path;

fn process_fixture(content: &str) -> String {
    let val = match yttp::parse(content) {
        Ok(v) => v,
        Err(e) => return format!("{e}"),
    };

    match mockinx::rule::parse_rules(&val) {
        Err(e) => format!("{e}"),
        Ok(rules) => {
            let warnings = mockinx::validate::validate_rules(&rules);
            if warnings.is_empty() {
                String::new()
            } else {
                warnings
                    .iter()
                    .map(|w| format!("{w}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

#[test]
fn error_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/errors");
    let update = std::env::var("UPDATE_ERRORS").is_ok();

    let mut entries: Vec<_> = std::fs::read_dir(&fixtures_dir)
        .expect("fixtures/errors/ directory not found")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.ends_with(".yaml") || name.ends_with(".json")) && name.starts_with("rules-")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    assert!(!entries.is_empty(), "no fixture files found");

    let mut failures = Vec::new();

    for entry in &entries {
        let input_path = entry.path();
        let stem = input_path.file_stem().unwrap().to_string_lossy();
        let error_path = fixtures_dir.join(format!("{stem}.error.txt"));
        let content = std::fs::read_to_string(&input_path).unwrap();

        let actual = process_fixture(&content);

        if update {
            std::fs::write(&error_path, &actual).unwrap();
            println!("updated: {}", error_path.display());
        } else {
            let expected = std::fs::read_to_string(&error_path).unwrap_or_else(|_| {
                panic!(
                    "missing {}\nrun: UPDATE_ERRORS=1 cargo test error_fixtures -- --nocapture",
                    error_path.display()
                )
            });

            if actual != expected {
                failures.push(format!(
                    "MISMATCH: {}\n  expected: {expected:?}\n  actual:   {actual:?}",
                    input_path.display()
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} fixture(s) failed:\n{}\n\nTo update: UPDATE_ERRORS=1 cargo test error_fixtures -- --nocapture",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
