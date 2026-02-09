use assert_cmd::Command;
use std::fs;
use tempfile::tempdir;

#[test]
#[allow(deprecated)]
fn test_cli_integration() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("svd-ingestor")?;
    let temp_dir = tempdir()?;
    let output_dir = temp_dir.path();
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/dummy_stm32.svd");

    // Ensure fixture exists
    if !fixture_path.exists() {
        // Fallback or skip if running in environment without fixtures (though we checked it exists)
        eprintln!("Skipping test: fixture not found at {:?}", fixture_path);
        return Ok(());
    }

    cmd.arg("--input")
        .arg(&fixture_path)
        .arg("--output-dir")
        .arg(output_dir);

    cmd.assert().success();

    // Verify output file exists
    let expected_output = output_dir.join("usart1.yaml");
    assert!(
        expected_output.exists(),
        "Output file usart1.yaml should be created"
    );

    // Basic content check
    let content = fs::read_to_string(&expected_output)?;
    assert!(content.contains("peripheral: USART1"));
    assert!(content.contains("registers:"));
    assert!(content.contains("id: SR"));

    Ok(())
}
