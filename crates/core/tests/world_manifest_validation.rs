use labwired_config::EnvironmentManifest;
use labwired_core::world::World;
use std::path::Path;

#[test]
fn world_rejects_a_direct_manifest_that_bypasses_file_validation() {
    let manifest = EnvironmentManifest {
        schema_version: "2.0".to_string(),
        name: "invalid-world".to_string(),
        nodes: Vec::new(),
        interconnects: Vec::new(),
    };

    let error = match World::from_manifest(manifest, Path::new(".")) {
        Ok(_) => panic!("World::from_manifest accepted an invalid direct manifest"),
        Err(error) => format!("{error:#}"),
    };

    assert!(error.contains("schema_version '2.0'"), "{error}");
}
