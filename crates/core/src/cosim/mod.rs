// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

mod external_process;
mod registry;
pub mod shm;

pub use external_process::ExternalProcessCosimAdapter;
pub use registry::{
    build_cosim_adapter, CosimModelStep, CosimRoutedModelStep, CosimRunner, CosimRunnerModel,
};

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::collections::BTreeMap;

/// Scalar value exchanged at a co-simulation boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum CosimSignalValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
}

pub type CosimSignals = BTreeMap<String, CosimSignalValue>;

/// One deterministic handoff from LabWired into an external model.
#[derive(Debug, Clone, PartialEq)]
pub struct CosimStep {
    pub time_ns: u64,
    pub dt_ns: u64,
    pub inputs: CosimSignals,
}

/// Outputs produced by an external model after a co-simulation step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CosimStepResult {
    pub outputs: CosimSignals,
}

/// Runtime contract implemented by external-process, FMI, or in-process
/// adapters. LabWired owns time advancement; adapters consume one bounded
/// step and return observable values.
pub trait CosimAdapter: Send {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult>;
}

/// Deterministic adapter used by tests and manifest dry-runs before a real
/// external simulator is wired in.
#[derive(Debug, Clone)]
pub struct StaticCosimAdapter {
    outputs: CosimSignals,
}

impl StaticCosimAdapter {
    pub fn new(outputs: CosimSignals) -> Self {
        Self { outputs }
    }
}

impl CosimAdapter for StaticCosimAdapter {
    fn step(&mut self, _step: CosimStep) -> SimResult<CosimStepResult> {
        Ok(CosimStepResult {
            outputs: self.outputs.clone(),
        })
    }
}

/// A peripheral that proxies its operations to an external process via IPC.
/// This is used for high-performance co-simulation with RTL models (e.g. Verilator).
#[derive(Debug)]
pub struct CosimPeripheral {
    pub name: String,
    // Add IPC transport (e.g. SharedMemory)
}

impl Peripheral for CosimPeripheral {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // TODO: Propose transaction over IPC and wait for result
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // TODO: Propose transaction over IPC
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // TODO: Sync simulation time with external process
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
