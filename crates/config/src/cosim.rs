// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CosimAdapter {
    ExternalProcess,
    Fmi,
    Mock,
    /// `labwired_core::analog` — the in-core MNA engine. No `model` path: the
    /// circuit is a SPICE netlist under `config.netlist` / `config.netlist_text`.
    /// The only adapter the browser can run, since it spawns no process.
    Analog,
}

pub(crate) fn default_cosim_step_ns() -> u64 {
    1_000
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CosimModelConfig {
    pub id: String,
    pub adapter: CosimAdapter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default = "default_cosim_step_ns")]
    pub step_ns: u64,
    #[serde(default)]
    pub inputs: HashMap<String, String>,
    #[serde(default)]
    pub outputs: HashMap<String, String>,
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}
