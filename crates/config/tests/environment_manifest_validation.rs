use labwired_config::EnvironmentManifest;

fn parse_environment(yaml: &str) -> anyhow::Result<EnvironmentManifest> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("environment.yaml");
    std::fs::write(&path, yaml).unwrap();
    EnvironmentManifest::from_file(path)
}

#[test]
fn environment_manifest_from_file_rejects_unknown_top_level_keys() {
    let error = parse_environment(
        r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
interconnect: []
"#,
    )
    .unwrap_err();
    let error = format!("{error:#}");

    assert!(error.contains("unknown field `interconnect`"), "{error}");
}

#[test]
fn environment_manifest_from_file_rejects_an_unsupported_schema_version() {
    let error = parse_environment(
        r#"
schema_version: "2.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
"#,
    )
    .unwrap_err();
    let error = format!("{error:#}");

    assert!(error.contains("schema_version '2.0'"), "{error}");
    assert!(error.contains("'1.0'"), "{error}");
}

#[test]
fn environment_manifest_from_file_rejects_blank_and_duplicate_node_ids() {
    for (name, yaml, expected) in [
        (
            "blank-id",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: "   "
    system: tester.yaml
    firmware: tester.elf
"#,
            "nodes[0].id",
        ),
        (
            "duplicate-id",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
  - id: tester
    system: ecu.yaml
    firmware: ecu.elf
"#,
            "duplicate node id 'tester'",
        ),
    ] {
        let error = parse_environment(yaml).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn environment_manifest_from_file_rejects_missing_world_and_node_fields() {
    for (name, yaml, expected) in [
        (
            "blank-name",
            r#"
schema_version: "1.0"
name: "  "
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
"#,
            "non-empty name",
        ),
        (
            "no-nodes",
            r#"
schema_version: "1.0"
name: two-node
nodes: []
"#,
            "at least one node",
        ),
        (
            "blank-system",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: " "
    firmware: tester.elf
"#,
            "nodes[0].system",
        ),
        (
            "blank-firmware",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: " "
"#,
            "nodes[0].firmware",
        ),
    ] {
        let error = parse_environment(yaml).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn environment_manifest_from_file_rejects_unknown_node_and_interconnect_keys() {
    for (name, yaml, expected) in [
        (
            "node-key",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
    systm: typo.yaml
"#,
            "unknown field `systm`",
        ),
        (
            "interconnect-key",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
interconnects:
  - type: egress
    nodes: [tester]
    confg: {}
"#,
            "unknown field `confg`",
        ),
    ] {
        let error = parse_environment(yaml).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn environment_manifest_from_file_rejects_every_node_config_overrides_occurrence() {
    for (name, overrides) in [
        ("nonempty", "config_overrides:\n      uart2: disabled"),
        ("empty", "config_overrides: {}"),
        ("null", "config_overrides: null"),
    ] {
        let error = parse_environment(&format!(
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
    {overrides}
"#
        ))
        .unwrap_err();
        let error = format!("{error:#}");

        assert!(
            error.contains("nodes[0].config_overrides"),
            "{name}: {error}"
        );
        assert!(
            error.contains("unsupported in environment schema 1.0"),
            "{name}: {error}"
        );
    }
}

#[test]
fn environment_manifest_from_file_rejects_unknown_and_untyped_interconnect_config() {
    for (name, yaml, expected) in [
        (
            "uart-typo",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
  - id: ecu
    system: ecu.yaml
    firmware: ecu.elf
interconnects:
  - type: uart_cross_link
    nodes: [tester, ecu]
    config:
      node_a_urat: uart2
"#,
            "interconnects[0].config.node_a_urat is not supported for uart_cross_link",
        ),
        (
            "uart-nonstring",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
  - id: ecu
    system: ecu.yaml
    firmware: ecu.elf
interconnects:
  - type: uart_cross_link
    nodes: [tester, ecu]
    config:
      node_a_uart: 2
"#,
            "interconnects[0].config.node_a_uart must be a non-empty string",
        ),
        (
            "can-unknown-key",
            r#"
schema_version: "1.0"
name: two-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
  - id: ecu
    system: ecu.yaml
    firmware: ecu.elf
interconnects:
  - type: can_bus
    nodes: [tester, ecu]
    config:
      peripheral: fdcan1
      bitrate: 500000
"#,
            "interconnects[0].config.bitrate is not supported for can_bus",
        ),
        (
            "egress-unknown-key",
            r#"
schema_version: "1.0"
name: one-node
nodes:
  - id: tester
    system: tester.yaml
    firmware: tester.elf
interconnects:
  - type: egress
    nodes: [tester]
    config:
      url: 127.0.0.1:9000
      protcol: tcp
"#,
            "interconnects[0].config.protcol is not supported for egress",
        ),
    ] {
        let error = parse_environment(yaml).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(expected), "{name}: {error}");
    }
}
