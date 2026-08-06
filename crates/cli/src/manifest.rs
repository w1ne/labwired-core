// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Signable, reproducible run manifest.
//!
//! A `RunManifest` records the inputs and deterministic outputs of a single
//! `labwired test` run — firmware and config hashes, engine version, the
//! result subset, and a coverage summary — together with a `digest`: a SHA-256
//! over the canonical JSON of every other field. Wall-clock time is excluded, so
//! two runs of the same inputs on different machines produce a byte-identical
//! digest. The digest is the stable artifact a buyer signs.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MANIFEST_SCHEMA_VERSION: &str = "1.0";

/// A file referenced by the run, with the SHA-256 of its contents.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HashedFile {
    pub path: String,
    pub sha256: String,
}

/// One assertion and whether it passed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AssertionOutcome {
    pub assertion: String,
    pub passed: bool,
}

/// The deterministic subset of the run result.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManifestResults {
    pub status: String,
    pub stop_reason: String,
    pub steps_executed: u64,
    pub cycles: u64,
    pub instructions: u64,
    pub assertions: Vec<AssertionOutcome>,
    pub cpu_state_digest: String,
}

/// Rolled-up coverage counts.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CoverageSummary {
    pub statements_total: usize,
    pub statements_covered: usize,
    pub branches_total: usize,
    pub branches_covered: usize,
}

/// The full run manifest. Serialised to `run-manifest.json`.
#[derive(Debug, Clone, Serialize)]
pub struct RunManifest {
    pub manifest_schema_version: String,
    pub engine_version: String,
    /// Explicit run seed. Seeded sensor noise derives per-channel PRNGs from
    /// this seed plus the component id, so a noisy run still replays
    /// bit-identically (asserted by the reproducibility test).
    pub seed: u64,
    /// `"none"` — no modeled randomness anywhere in the run.
    /// `"seeded(sensor-noise)"` — at least one attached device applies seeded
    /// Gaussian noise/bias/thermal-lag on reads; the run is stochastic in
    /// content but identical across replays of the same seed.
    pub nondeterminism: String,
    pub firmware: HashedFile,
    pub configs: Vec<HashedFile>,
    pub results: ManifestResults,
    pub coverage: Option<CoverageSummary>,
    /// Per-fault evidence for fault-injection runs; empty for ordinary runs.
    /// Included in the digest so a faulted run's verdict is signed and reproducible.
    pub fault_injections: Vec<crate::faults::FaultEvidence>,
    /// SHA-256 over the canonical JSON of every field above. Filled by
    /// [`RunManifest::finalize_digest`].
    pub digest: String,
}

impl RunManifest {
    /// Compute and store `digest` as the SHA-256 of the canonical JSON of this
    /// manifest with the `digest` field itself removed. Idempotent.
    pub fn finalize_digest(&mut self) {
        self.digest = String::new();
        let mut value = serde_json::to_value(&*self).expect("manifest serialises");
        if let Some(obj) = value.as_object_mut() {
            obj.remove("digest");
        }
        self.digest = sha256_hex(canonical_json(&value).as_bytes());
    }
}

/// SHA-256 of bytes as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Config keys that mark a device as noise-enabled, on either side of the
/// kit divide: Rust kits take them in `config:`, declarative descriptors
/// declare them on `metadata.inputs`.
const NOISE_KEYS: [&str; 3] = ["noise_sigma", "bias", "thermal_tau_s"];

/// True when the system manifest wires at least one noise-enabled device —
/// either an `external_devices` entry carrying noise config keys (Rust kits),
/// or a device whose embedded declarative descriptor declares noise inputs.
/// Used to stamp `nondeterminism` honestly: seeded noise is reproducible,
/// but it is not the absence of variation.
pub fn any_noise_enabled(system_yaml: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Probe {
        #[serde(default)]
        external_devices: Vec<Device>,
    }
    #[derive(serde::Deserialize)]
    struct Device {
        r#type: String,
        #[serde(default)]
        config: std::collections::HashMap<String, serde_yaml::Value>,
    }
    let Ok(probe) = serde_yaml::from_str::<Probe>(system_yaml) else {
        return false;
    };
    probe.external_devices.iter().any(|d| {
        NOISE_KEYS.iter().any(|k| d.config.contains_key(*k))
            || labwired_config::embedded_device_yaml(&d.r#type)
                .and_then(|y| labwired_config::DeviceDescriptor::from_yaml(y).ok())
                .and_then(|desc| desc.metadata)
                .map(|m| {
                    m.inputs.iter().any(|i| {
                        i.noise_sigma.is_some() || i.bias.is_some() || i.thermal_tau_s.is_some()
                    })
                })
                .unwrap_or(false)
    })
}

#[cfg(test)]
mod noise_tests {
    #[test]
    fn noise_free_system_is_none() {
        let yaml = "name: x\nexternal_devices: []\n";
        assert!(!super::any_noise_enabled(yaml));
    }

    #[test]
    fn kit_config_noise_key_is_detected() {
        let yaml = "name: x\nexternal_devices:\n  - id: imu\n    type: mpu6050\n    connection: i2c1\n    config:\n      noise_sigma: 0.02\n";
        assert!(super::any_noise_enabled(yaml));
    }

    #[test]
    fn declarative_descriptor_noise_is_detected() {
        // mcp9808's embedded descriptor declares noise_sigma on its input.
        let yaml =
            "name: x\nexternal_devices:\n  - id: temp\n    type: mcp9808\n    connection: i2c1\n";
        assert!(super::any_noise_enabled(yaml));
    }

    #[test]
    fn plain_device_is_not_flagged() {
        let yaml =
            "name: x\nexternal_devices:\n  - id: imu\n    type: mpu6050\n    connection: i2c1\n";
        assert!(!super::any_noise_enabled(yaml));
    }

    #[test]
    fn malformed_yaml_is_not_flagged() {
        assert!(!super::any_noise_enabled("not: [valid"));
    }
}

/// SHA-256 of a serialisable value's canonical JSON. Used for the CPU-state
/// digest so the manifest carries a stable fingerprint, not the whole snapshot.
pub fn digest_value<T: Serialize>(value: &T) -> String {
    let v = serde_json::to_value(value).unwrap_or(Value::Null);
    sha256_hex(canonical_json(&v).as_bytes())
}

/// Serialise a JSON value with object keys sorted recursively, so logically
/// equal values always produce identical bytes. Assumes a float-free value
/// (the digested region is integers, strings and bools only).
fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string()),
                        canonical_json(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RunManifest {
        RunManifest {
            manifest_schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            engine_version: "1.2.3".to_string(),
            seed: 0,
            nondeterminism: "none".to_string(),
            firmware: HashedFile {
                path: "fw.elf".to_string(),
                sha256: "aa".to_string(),
            },
            configs: vec![HashedFile {
                path: "sys.yaml".to_string(),
                sha256: "bb".to_string(),
            }],
            results: ManifestResults {
                status: "passed".to_string(),
                stop_reason: "MaxStepsReached".to_string(),
                steps_executed: 100,
                cycles: 100,
                instructions: 100,
                assertions: vec![AssertionOutcome {
                    assertion: "uart_contains(OK)".to_string(),
                    passed: true,
                }],
                cpu_state_digest: "cc".to_string(),
            },
            coverage: Some(CoverageSummary {
                statements_total: 10,
                statements_covered: 5,
                branches_total: 4,
                branches_covered: 2,
            }),
            fault_injections: Vec::new(),
            digest: String::new(),
        }
    }

    #[test]
    fn digest_is_stable_and_excludes_itself() {
        let mut a = sample();
        a.finalize_digest();
        assert!(!a.digest.is_empty());

        // Re-finalizing is idempotent (digest excludes the digest field).
        let first = a.digest.clone();
        a.finalize_digest();
        assert_eq!(a.digest, first);

        // A fresh manifest with the same inputs digests identically.
        let mut b = sample();
        b.finalize_digest();
        assert_eq!(a.digest, b.digest);
    }

    #[test]
    fn digest_changes_when_an_input_changes() {
        let mut a = sample();
        a.finalize_digest();

        let mut b = sample();
        b.firmware.sha256 = "different".to_string();
        b.finalize_digest();

        assert_ne!(
            a.digest, b.digest,
            "a changed firmware hash must move the digest"
        );
    }

    #[test]
    fn digest_includes_fault_injections() {
        let mut a = sample();
        a.finalize_digest();

        let mut b = sample();
        b.fault_injections.push(crate::faults::FaultEvidence {
            id: "f1".to_string(),
            kind: "WrongResetValue".to_string(),
            fired: true,
            error: None,
        });
        b.finalize_digest();

        assert_ne!(
            a.digest, b.digest,
            "fault evidence must be part of the signed digest"
        );
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let v = serde_json::json!({ "b": 1, "a": 2, "c": { "z": 1, "y": 2 } });
        assert_eq!(canonical_json(&v), r#"{"a":2,"b":1,"c":{"y":2,"z":1}}"#);
    }
}
