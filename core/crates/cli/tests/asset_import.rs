use std::process::Command;
use std::path::Path;
use std::fs;

#[test]
fn test_asset_import_fixtures() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    
    // Determine workspace root. 
    // Tests run in `core/crates/cli`, so workspace root is `../../`
    let workspace_root = std::env::current_dir().unwrap()
        .parent().unwrap() // crates
        .parent().unwrap() // core
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
            if path.extension().map_or(false, |ext| ext == "svd") {
                svd_files.push(path);
            }
        }
    }

    assert!(!svd_files.is_empty(), "No SVD fixtures found to test!");

    for svd_path in svd_files {
        println!("Testing SVD Import: {:?}", svd_path);
        
        // Output file will be side-by-side with .json extension
        let output_path = svd_path.with_extension("test_output.json");

        let status = Command::new(&cargo)
            .arg("run")
            .arg("-p")
            .arg("labwired-cli")
            .arg("--")
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
        assert!(output_path.exists(), "Output JSON not created for {:?}", svd_path);

        // Basic clean up
        let _ = fs::remove_file(output_path);
    }
}
