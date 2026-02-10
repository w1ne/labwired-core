use std::process::Command;
use std::path::Path;

#[test]
fn test_asset_import_e2e() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    
    // Paths relative to workspace root (where we run cargo test)
    let fixture_svd = Path::new("tests/fixtures/advanced_stm32.svd");
    let output_json = Path::new("tests/fixtures/integration_test_output.json");

    assert!(fixture_svd.exists(), "Fixture SVD not found at {:?}", fixture_svd);

    // Run the CLI via cargo run (slow but realistic)
    // Note: We need to use labwired-cli binary
    let status = Command::new(cargo)
        .arg("run")
        .arg("-p")
        .arg("labwired-cli")
        .arg("--")
        .arg("asset")
        .arg("import-svd")
        .arg("--input")
        .arg(fixture_svd)
        .arg("--output")
        .arg(output_json)
        .current_dir("../../") // Go up from crates/cli/tests/ to workspace root? 
        // When running `cargo test` from core/crates/cli, CWD is crates/cli.
        // Wait, normally integration tests run with CWD = crate root.
        // Let's assume we run from workspace root to simplify paths, or adjust.
        // If we run `cargo test -p labwired-cli` from workspace root, CWD is workspace root.
        .status()
        .expect("Failed to execute labwired-cli");

    assert!(status.success(), "labwired-cli failed");

    // Verify Output
    // We need to resolve path relative to where we are.
    // Making this robust to CWD is tricky without `CARGO_MANIFEST_DIR`.
    // Let's try to read the file.
    
    // Assuming CWD of this test execution is `core/crates/cli` or `core`?
    // Cargo sets CWD to the crate root for integration tests in `tests/`.
    // So `core/crates/cli`.
    
    // Wait, the `Command` above spawned a subprocess.
    // If we rely on absolute paths it's safer.
    
    let root_dir = std::env::current_dir().unwrap();
    println!("Test CWD: {:?}", root_dir);
    
    // Check if the output file exists
    // The CLI above was run with `current_dir("../..")` which puts it at `core` (if we are in `core/crates/cli`).
    // So `tests/fixtures/...` would be `core/tests/fixtures/...`.
    
    // Let's rely on the file system.
    let expected_output_path = root_dir
        .parent().unwrap() // crates
        .parent().unwrap() // core
        .join("tests/fixtures/integration_test_output.json");

    assert!(expected_output_path.exists(), "Output JSON not created at {:?}", expected_output_path);

    let content = std::fs::read_to_string(&expected_output_path).unwrap();
    assert!(content.contains("\"name\": \"ADVANCED_DEVICE\""));
    assert!(content.contains("\"name\": \"TIMER1\""));
    
    // Cleanup
    let _ = std::fs::remove_file(expected_output_path);
}
