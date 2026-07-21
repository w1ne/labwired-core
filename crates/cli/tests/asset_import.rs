use std::fs;
use std::process::Command;

#[test]
fn test_asset_import_fixtures() {
    // Determine workspace root.
    // Tests run in `core/crates/cli`, so workspace root is `../../`
    let workspace_root = std::env::current_dir()
        .unwrap()
        .parent()
        .unwrap() // crates
        .parent()
        .unwrap() // core
        .to_path_buf();

    let fixtures_dir = workspace_root.join("tests/fixtures");
    let real_world_dir = fixtures_dir.join("real_world");

    let mut svd_files = Vec::new();

    // Add the manual fixture
    svd_files.push(fixtures_dir.join("advanced_stm32.svd"));

    // Add all real-world fixtures if they exist
    if real_world_dir.exists() {
        for entry in fs::read_dir(&real_world_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "svd") {
                svd_files.push(path);
            }
        }
    }

    assert!(!svd_files.is_empty(), "No SVD fixtures found to test!");

    for svd_path in svd_files {
        println!("Testing SVD Import: {:?}", svd_path);

        // Output file will be side-by-side with .json extension
        let output_path = svd_path.with_extension("test_output.json");

        let status = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .arg("asset")
            .arg("import-svd")
            .arg("--input")
            .arg(&svd_path)
            .arg("--output")
            .arg(&output_path)
            .current_dir(&workspace_root) // Run from workspace root so paths are relative to it if needed
            .status()
            .expect("Failed to execute labwired-cli");

        assert!(status.success(), "Failed to import {:?}", svd_path);
        assert!(
            output_path.exists(),
            "Output JSON not created for {:?}",
            svd_path
        );

        // Parse the output and assert structural correctness.
        let json_str = fs::read_to_string(&output_path)
            .unwrap_or_else(|e| panic!("read output JSON for {svd_path:?}: {e}"));
        let json: serde_json::Value =
            serde_json::from_str(&json_str).expect("output JSON must parse");

        let peripherals = json["peripherals"]
            .as_object()
            .expect("peripherals must be an object");
        assert!(
            !peripherals.is_empty(),
            "imported SVD {svd_path:?} has 0 peripherals in output JSON"
        );

        // Every peripheral must have >= 1 register with a sane (>0) address.
        let mut found_register_with_address = false;
        for (pname, peripheral) in peripherals {
            let regs = peripheral["registers"]
                .as_array()
                .unwrap_or_else(|| panic!("peripheral {pname} missing registers array"));
            if regs.is_empty() {
                continue;
            }
            for reg in regs {
                // base_address is on the peripheral; offset is on the register.
                let base = peripheral["base_address"].as_u64().unwrap_or(0);
                let offset = reg["offset"].as_u64().unwrap_or(0);
                if base + offset > 0 {
                    found_register_with_address = true;
                    break;
                }
            }
            if found_register_with_address {
                break;
            }
        }
        assert!(
            found_register_with_address,
            "no peripheral register with a sane (>0) address found in imported {svd_path:?}"
        );

        // Basic clean up
        let _ = fs::remove_file(output_path);
    }
}
