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
    /// Explicit run seed. The interpreter has no surfaced RNG today; determinism
    /// comes from the absence of nondeterminism, asserted by the reproducibility
    /// test, so this is 0 with `nondeterminism: "none"`.
    pub seed: u64,
    pub nondeterminism: String,
    pub firmware: HashedFile,
    pub configs: Vec<HashedFile>,
    pub results: ManifestResults,
    pub coverage: Option<CoverageSummary>,
    /// Contract slot for fault-injection evidence (populated by a later plan).
    pub fault_injections: Vec<Value>,
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
    fn canonical_json_sorts_keys() {
        let v = serde_json::json!({ "b": 1, "a": 2, "c": { "z": 1, "y": 2 } });
        assert_eq!(canonical_json(&v), r#"{"a":2,"b":1,"c":{"y":2,"z":1}}"#);
    }
}
