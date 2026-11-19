// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CPU-free firmware ports connected to the existing co-simulation blocks.
//! This is a checked authoring contract, not a second peripheral or solver
//! runtime. Adapter construction still validates the actual model semantics.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};

use crate::{CosimAdapter, CosimModelConfig};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexManifest {
    pub schema_version: u32,
    pub name: String,
    /// User-visible assumptions that scope this functional model. Empty for
    /// older manifests; callers must surface that as incomplete metadata.
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Checks the model cannot establish, even when its modeled behavior runs.
    #[serde(default)]
    pub unsupported_checks: Vec<String>,
    pub ports: Vec<FlexPort>,
    pub cosim_models: Vec<CosimModelConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexPort {
    pub name: String,
    pub signal: String,
    pub access: FlexPortAccess,
    pub kind: FlexPortKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlexPortAccess {
    Read,
    Write,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlexPortKind {
    Bool,
    Number,
}

impl FlexManifest {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let value: Value = serde_yaml::from_str(yaml).context("invalid Flex manifest YAML")?;
        check_cosim_keys(&value)?;
        let manifest: Self =
            serde_yaml::from_value(value).context("invalid Flex manifest schema")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate port ownership and model connectivity before execution.
    ///
    /// Signal names are flat identities. They do not implicitly create a CPU,
    /// GPIO bank, ADC or other board endpoint. Every signal has a declared
    /// producer in this manifest. Adapter construction is responsible for
    /// netlist parsing and the behavioral model's own schema and type checks.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported Flex schema_version {}; expected 1",
            self.schema_version
        );
        ensure!(
            !self.name.trim().is_empty() && self.name == self.name.trim(),
            "Flex name must be a nonempty, trimmed string"
        );
        for (field, notes) in [
            ("assumptions", &self.assumptions),
            ("unsupported_checks", &self.unsupported_checks),
        ] {
            ensure!(
                notes
                    .iter()
                    .all(|note| !note.trim().is_empty() && note == note.trim()),
                "Flex {field} entries must be nonempty and trimmed"
            );
        }

        let mut port_names = HashSet::new();
        let mut write_signals = HashMap::new();
        for port in &self.ports {
            token(&port.name, "Flex port name")?;
            token(&port.signal, &format!("port '{}'.signal", port.name))?;
            ensure!(
                port_names.insert(port.name.as_str()),
                "duplicate Flex port name '{}'",
                port.name
            );
            match port.kind {
                FlexPortKind::Bool => {
                    ensure!(
                        port.unit.is_none(),
                        "boolean port '{}' must not declare a unit",
                        port.name
                    );
                }
                FlexPortKind::Number => {
                    let unit = port.unit.as_deref().with_context(|| {
                        format!("numeric port '{}' requires an explicit unit", port.name)
                    })?;
                    token(unit, &format!("port '{}'.unit", port.name))?;
                }
            }
            match port.access {
                FlexPortAccess::Write => {
                    let initial = port.initial.with_context(|| {
                        format!(
                            "write port '{}' requires an explicit initial value",
                            port.name
                        )
                    })?;
                    ensure!(
                        initial.is_finite(),
                        "write port '{}' initial value must be finite",
                        port.name
                    );
                    ensure!(
                        port.kind != FlexPortKind::Bool || initial == 0.0 || initial == 1.0,
                        "boolean write port '{}' initial value must be 0 or 1",
                        port.name
                    );
                    ensure!(
                        write_signals
                            .insert(port.signal.as_str(), port.name.as_str())
                            .is_none(),
                        "signal '{}' has multiple firmware writers",
                        port.signal
                    );
                }
                FlexPortAccess::Read => ensure!(
                    port.initial.is_none(),
                    "read port '{}' must not declare an initial value",
                    port.name
                ),
            }
        }

        let mut model_ids = HashSet::new();
        let mut outputs = HashMap::new();
        let mut consumed = HashSet::new();
        for model in &self.cosim_models {
            token(&model.id, "co-simulation model id")?;
            ensure!(
                model_ids.insert(model.id.as_str()),
                "duplicate co-simulation model id '{}'",
                model.id
            );
            ensure!(
                model.step_ns > 0,
                "model '{}' step_ns must be nonzero",
                model.id
            );
            ensure!(
                matches!(model.adapter, CosimAdapter::Analog | CosimAdapter::Behavior),
                "model '{}' adapter {:?} is unsupported in CPU-free Flex",
                model.id,
                model.adapter
            );
            ensure!(
                model.model.is_none(),
                "model '{}' must not declare an external model path",
                model.id
            );
            for (name, signal) in &model.inputs {
                token(name, &format!("model '{}' input name", model.id))?;
                token(
                    signal,
                    &format!("model '{}' input '{name}' signal", model.id),
                )?;
                consumed.insert(signal.as_str());
            }
            for (name, signal) in &model.outputs {
                token(name, &format!("model '{}' output name", model.id))?;
                token(
                    signal,
                    &format!("model '{}' output '{name}' signal", model.id),
                )?;
                ensure!(
                    !write_signals.contains_key(signal.as_str()),
                    "signal '{signal}' is driven by both firmware and model '{}'",
                    model.id
                );
                if let Some(previous) = outputs.insert(signal.as_str(), model.id.as_str()) {
                    bail!(
                        "signal '{signal}' has multiple model outputs ('{previous}' and '{}')",
                        model.id
                    );
                }
            }
            if model.adapter == CosimAdapter::Analog {
                validate_analog(model)?;
            }
        }

        for model in &self.cosim_models {
            for signal in model.inputs.values() {
                ensure!(
                    write_signals.contains_key(signal.as_str())
                        || outputs.contains_key(signal.as_str()),
                    "model '{}' input signal '{signal}' has no declared producer",
                    model.id
                );
            }
        }
        for port in &self.ports {
            match port.access {
                FlexPortAccess::Read => ensure!(
                    outputs.contains_key(port.signal.as_str()),
                    "read port '{}' signal '{}' is not a model output",
                    port.name,
                    port.signal
                ),
                FlexPortAccess::Write => ensure!(
                    consumed.contains(port.signal.as_str()),
                    "write port '{}' signal '{}' is not consumed by any model",
                    port.name,
                    port.signal
                ),
            }
        }
        Ok(())
    }
}

fn token(value: &str, path: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control()),
        "{path} must be nonempty and contain no whitespace or control characters"
    );
    Ok(())
}

fn check_cosim_keys(value: &Value) -> Result<()> {
    let Value::Mapping(root) = value else {
        bail!("Flex manifest must be a mapping");
    };
    let Some(models) = root.get("cosim_models") else {
        return Ok(()); // Canonical deserialization reports the missing field.
    };
    let Value::Sequence(models) = models else {
        bail!("cosim_models must be a sequence");
    };
    for (index, model) in models.iter().enumerate() {
        let Value::Mapping(model) = model else {
            bail!("cosim_models[{index}] must be a mapping");
        };
        for key in model.keys() {
            let Value::String(key) = key else {
                bail!("cosim_models[{index}] contains a non-string key");
            };
            ensure!(
                ["id", "adapter", "model", "step_ns", "inputs", "outputs", "config"]
                    .contains(&key.as_str()),
                "cosim_models[{index}] contains unknown field '{key}'"
            );
        }
    }
    Ok(())
}

fn validate_analog(model: &CosimModelConfig) -> Result<()> {
    let path = format!("analog model '{}'", model.id);
    for key in model.config.keys() {
        ensure!(
            [
                "netlist_text",
                "vdd",
                "probes",
                "sources",
                "substeps",
                "integration",
                "input_sampling",
                "photoresistors"
            ]
            .contains(&key.as_str()),
            "{path} has unsupported config key '{key}'; this Flex path requires an inline netlist_text"
        );
    }
    ensure!(
        model.config.get("input_sampling").and_then(Value::as_str)
            == Some("previous_boundary"),
        "{path} requires config.input_sampling: previous_boundary; Flex must integrate using the input held over the preceding interval"
    );
    let netlist = model
        .config
        .get("netlist_text")
        .and_then(Value::as_str)
        .with_context(|| format!("{path} requires inline config.netlist_text"))?;
    ensure!(
        !netlist.trim().is_empty(),
        "{path} config.netlist_text must not be empty"
    );
    if let Some(value) = model.config.get("vdd") {
        let volts = value
            .as_f64()
            .with_context(|| format!("{path} config.vdd must be a finite positive number"))?;
        ensure!(
            volts.is_finite() && volts > 0.0,
            "{path} config.vdd must be a finite positive number"
        );
    }
    if let Some(value) = model.config.get("substeps") {
        let steps = value
            .as_u64()
            .with_context(|| format!("{path} config.substeps must be a positive u32"))?;
        ensure!(
            steps > 0 && steps <= u64::from(u32::MAX),
            "{path} config.substeps must be a positive u32"
        );
    }
    if let Some(value) = model.config.get("integration") {
        let integration = value
            .as_str()
            .with_context(|| format!("{path} config.integration must be be or trap"))?;
        ensure!(
            matches!(
                integration.trim().to_ascii_lowercase().as_str(),
                "be" | "backward_euler" | "euler" | "trap" | "trapezoidal"
            ),
            "{path} config.integration must be be or trap"
        );
    }
    // A photoresistor entry binds its input by key, in place of a `sources`
    // entry. The analog adapter validates the model parameters themselves.
    let mut source_routes = model.inputs.clone();
    if let Some(value) = model.config.get("photoresistors") {
        let Value::Mapping(models) = value else {
            bail!("{path} config.photoresistors must be a mapping");
        };
        for name in models.keys() {
            let Value::String(name) = name else {
                bail!("{path} config.photoresistors names must be strings");
            };
            token(name, &format!("{path} config.photoresistors"))?;
            ensure!(
                source_routes.remove(name).is_some(),
                "{path} config.photoresistors.{name} has no declared model route"
            );
        }
    }
    check_analog_routes(model, "sources", &source_routes)?;
    check_analog_routes(model, "probes", &model.outputs)?;
    Ok(())
}

fn check_analog_routes(
    model: &CosimModelConfig,
    key: &str,
    routes: &HashMap<String, String>,
) -> Result<()> {
    let path = format!("analog model '{}' config.{key}", model.id);
    let Some(value) = model.config.get(key) else {
        ensure!(
            routes.is_empty(),
            "{path} is required for the declared model routes"
        );
        return Ok(());
    };
    let Value::Mapping(bindings) = value else {
        bail!("{path} must be a mapping");
    };
    for (name, target) in bindings {
        let Value::String(name) = name else {
            bail!("{path} names must be strings");
        };
        token(name, &path)?;
        ensure!(
            routes.contains_key(name),
            "{path}.{name} has no declared model route"
        );
        let target = target
            .as_str()
            .with_context(|| format!("{path}.{name} must be a string"))?;
        ensure!(
            !target.trim().is_empty() && target == target.trim(),
            "{path}.{name} must be nonempty and trimmed"
        );
    }
    for name in routes.keys() {
        ensure!(
            bindings.contains_key(name.as_str()),
            "{path} has no binding for model route '{name}'"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RC: &str = r#"
schema_version: 1
name: RC firmware
ports:
  - { name: drive, signal: firmware.drive, access: write, kind: bool, initial: 0 }
  - { name: sample, signal: circuit.voltage, access: read, kind: number, unit: V }
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 100000
    inputs: { drive: firmware.drive }
    outputs: { voltage: circuit.voltage }
    config:
      netlist_text: |
        Vdrive in 0 dc 0
        R1 in out 10k
        C1 out 0 100n
      vdd: 3.3
      substeps: 10
      input_sampling: previous_boundary
      integration: be
      sources: { drive: Vdrive }
      probes: { voltage: "v(out)" }
"#;

    fn manifest() -> FlexManifest {
        FlexManifest::from_yaml(RC).unwrap()
    }

    #[test]
    fn cpu_free_rc_manifest_round_trips_and_reuses_parameterized_models() {
        let mut manifest = manifest();
        assert_eq!(manifest.ports[0].kind, FlexPortKind::Bool);
        assert_eq!(manifest.ports[1].access, FlexPortAccess::Read);
        let mut second = manifest.cosim_models[0].clone();
        second.id = "rc_slow".into();
        second
            .outputs
            .insert("voltage".into(), "circuit.slow".into());
        second.config.insert(
            "netlist_text".into(),
            "Vdrive in 0 dc 0\nR1 in out 20k\nC1 out 0 100n\n".into(),
        );
        manifest.cosim_models.push(second);
        let mut read = manifest.ports[1].clone();
        read.name = "slow_sample".into();
        read.signal = "circuit.slow".into();
        manifest.ports.push(read);
        // One firmware writer may fan out to several models.
        manifest.validate().unwrap();
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        let loaded = FlexManifest::from_yaml(&yaml).unwrap();
        assert_eq!(loaded.cosim_models.len(), 2);
        assert_eq!(loaded.ports.len(), 3);
    }

    #[test]
    fn unknown_schema_fields_are_not_discarded() {
        for yaml in [
            format!("{RC}\nchip: stm32f401\n"),
            RC.replace("name: drive,", "name: drive, optional: true,"),
            RC.replace(
                "    adapter: analog",
                "    adapter: analog\n    latency_ns: 4",
            ),
            RC.replace("      substeps: 10", "      substep: 10"),
        ] {
            assert!(FlexManifest::from_yaml(&yaml).is_err(), "{yaml}");
        }
    }

    #[test]
    fn scope_notes_round_trip_and_reject_blank_entries() {
        let mut manifest = manifest();
        manifest.assumptions = vec!["portable firmware uses named ports".into()];
        manifest.unsupported_checks = vec!["target instruction fidelity".into()];
        let loaded = FlexManifest::from_yaml(&serde_yaml::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(loaded.assumptions, manifest.assumptions);
        assert_eq!(loaded.unsupported_checks, manifest.unsupported_checks);

        manifest.assumptions.push("  ".into());
        assert!(manifest
            .validate()
            .unwrap_err()
            .to_string()
            .contains("assumptions entries"));
        manifest.assumptions.pop();
        manifest.unsupported_checks.push("not trimmed ".into());
        assert!(manifest
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unsupported_checks entries"));
    }

    #[test]
    fn analog_sampling_contract_is_required_with_a_specific_error() {
        for yaml in [
            RC.replace("      input_sampling: previous_boundary\n", ""),
            RC.replace(
                "input_sampling: previous_boundary",
                "input_sampling: current_boundary",
            ),
            RC.replace("input_sampling: previous_boundary", "input_sampling: true"),
        ] {
            let error = format!("{:#}", FlexManifest::from_yaml(&yaml).unwrap_err());
            assert!(
                error.contains("requires config.input_sampling: previous_boundary"),
                "{error}"
            );
        }
    }

    #[test]
    fn photoresistor_entries_bind_analog_inputs_in_place_of_sources() {
        let bound = RC
            .replace(
                "kind: bool, initial: 0",
                "kind: number, unit: lx, initial: 10",
            )
            .replace(
                "sources: { drive: Vdrive }",
                "photoresistors: { drive: { resistor: R1 } }",
            );
        assert_ne!(bound, RC);
        FlexManifest::from_yaml(&bound).unwrap();
        for (name, yaml, expected) in [
            (
                "unbound input",
                bound.replace("photoresistors: { drive: { resistor: R1 } }", ""),
                "config.sources is required",
            ),
            (
                "undeclared route",
                bound.replace("photoresistors: { drive:", "photoresistors: { typo:"),
                "config.photoresistors.typo has no declared model route",
            ),
            (
                "two owners",
                bound.replace(
                    "photoresistors:",
                    "sources: { drive: Vdrive }\n      photoresistors:",
                ),
                "config.sources.drive has no declared model route",
            ),
        ] {
            let error = format!("{:#}", FlexManifest::from_yaml(&yaml).unwrap_err());
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn ports_require_real_initial_state_and_explicit_numeric_units() {
        type Change = fn(&mut FlexManifest);
        let cases: &[(&str, Change)] = &[
            ("wrong version", |m| m.schema_version = 2),
            ("blank name", |m| m.name = " ".into()),
            ("blank port name", |m| m.ports[0].name.clear()),
            ("blank signal", |m| m.ports[0].signal.clear()),
            ("duplicate name", |m| m.ports[1].name = "drive".into()),
            ("missing write initial", |m| m.ports[0].initial = None),
            ("nonfinite write initial", |m| {
                m.ports[0].initial = Some(f64::NAN)
            }),
            ("nonboolean initial", |m| m.ports[0].initial = Some(2.0)),
            ("read initial", |m| m.ports[1].initial = Some(0.0)),
            ("missing numeric unit", |m| m.ports[1].unit = None),
            ("blank numeric unit", |m| m.ports[1].unit = Some(" ".into())),
            ("boolean unit", |m| m.ports[0].unit = Some("V".into())),
        ];
        for (label, change) in cases {
            let mut m = manifest();
            change(&mut m);
            assert!(m.validate().is_err(), "accepted {label}");
        }
        let mut m = manifest();
        m.ports[0].kind = FlexPortKind::Number;
        m.ports[0].unit = Some("V".into());
        m.ports[0].initial = Some(-0.5);
        m.validate().unwrap();
    }

    #[test]
    fn topology_rejects_missing_producers_and_conflicting_writers() {
        type Change = fn(&mut FlexManifest);
        let cases: &[(&str, Change)] = &[
            ("duplicate model id", |m| {
                let second = m.cosim_models[0].clone();
                m.cosim_models.push(second);
            }),
            ("duplicate firmware writer", |m| {
                let mut port = m.ports[0].clone();
                port.name = "second_drive".into();
                m.ports.push(port);
            }),
            ("firmware/model conflict", |m| {
                m.cosim_models[0]
                    .outputs
                    .insert("voltage".into(), "firmware.drive".into());
            }),
            ("multiple model outputs", |m| {
                let mut model = m.cosim_models[0].clone();
                model.id = "second".into();
                m.cosim_models.push(model);
            }),
            ("missing model input", |m| {
                m.cosim_models[0]
                    .inputs
                    .insert("drive".into(), "undeclared".into());
            }),
            ("unproduced read", |m| {
                m.ports[1].signal = "undeclared".into()
            }),
            ("unconsumed write", |m| {
                let mut port = m.ports[0].clone();
                port.name = "unused".into();
                port.signal = "firmware.unused".into();
                m.ports.push(port);
            }),
        ];
        for (label, change) in cases {
            let mut m = manifest();
            change(&mut m);
            assert!(m.validate().is_err(), "accepted {label}");
        }
    }

    #[test]
    fn analog_contract_rejects_external_or_ignored_configuration() {
        type Change = fn(&mut FlexManifest);
        let cases: &[(&str, Change)] = &[
            ("zero step", |m| m.cosim_models[0].step_ns = 0),
            ("external process", |m| {
                m.cosim_models[0].adapter = CosimAdapter::ExternalProcess
            }),
            ("mock adapter", |m| {
                m.cosim_models[0].adapter = CosimAdapter::Mock
            }),
            ("model path", |m| {
                m.cosim_models[0].model = Some("rc.py".into())
            }),
            ("missing sampling contract", |m| {
                m.cosim_models[0].config.remove("input_sampling");
            }),
            ("legacy sampling contract", |m| {
                m.cosim_models[0]
                    .config
                    .insert("input_sampling".into(), "current_boundary".into());
            }),
            ("file netlist", |m| {
                m.cosim_models[0].config.remove("netlist_text");
                m.cosim_models[0]
                    .config
                    .insert("netlist".into(), "rc.cir".into());
            }),
            ("empty netlist", |m| {
                m.cosim_models[0]
                    .config
                    .insert("netlist_text".into(), " ".into());
            }),
            ("nonfinite voltage", |m| {
                m.cosim_models[0]
                    .config
                    .insert("vdd".into(), f64::INFINITY.into());
            }),
            ("zero substeps", |m| {
                m.cosim_models[0].config.insert("substeps".into(), 0.into());
            }),
            ("wide substeps", |m| {
                m.cosim_models[0]
                    .config
                    .insert("substeps".into(), (u64::from(u32::MAX) + 1).into());
            }),
            ("unsupported integration", |m| {
                m.cosim_models[0]
                    .config
                    .insert("integration".into(), "guess".into());
            }),
            ("missing probe", |m| {
                m.cosim_models[0].config.remove("probes");
            }),
            ("unmapped source", |m| {
                m.cosim_models[0].config.get_mut("sources").unwrap()["unused"] = "Vextra".into();
            }),
        ];
        for (label, change) in cases {
            let mut m = manifest();
            change(&mut m);
            assert!(m.validate().is_err(), "accepted {label}");
        }
    }
}
