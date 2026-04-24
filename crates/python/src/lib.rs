// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::{
    bus::SystemBus,
    system::{cortex_m, riscv},
    Arch, DebugControl, SimulationError, StopReason,
};
use pyo3::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn map_sim_err(e: SimulationError) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(e.to_string())
}

fn map_anyhow(e: anyhow::Error) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(e.to_string())
}

fn map_io(e: anyhow::Error) -> PyErr {
    pyo3::exceptions::PyIOError::new_err(e.to_string())
}

/// Python-facing stop reason. Flattened to a simple struct so it works
/// under pyo3 0.20 (which does not yet support complex enum variants).
#[pyclass(name = "StopReason", get_all)]
#[derive(Clone, Debug)]
pub struct PyStopReason {
    pub kind: String,
    pub pc: Option<u32>,
}

impl From<StopReason> for PyStopReason {
    fn from(r: StopReason) -> Self {
        match r {
            StopReason::Breakpoint(pc) => PyStopReason {
                kind: "Breakpoint".into(),
                pc: Some(pc),
            },
            StopReason::StepDone => PyStopReason {
                kind: "StepDone".into(),
                pc: None,
            },
            StopReason::MaxStepsReached => PyStopReason {
                kind: "MaxStepsReached".into(),
                pc: None,
            },
            StopReason::ManualStop => PyStopReason {
                kind: "ManualStop".into(),
                pc: None,
            },
        }
    }
}

#[pymethods]
impl PyStopReason {
    fn __repr__(&self) -> String {
        match self.pc {
            Some(pc) => format!("StopReason({}, pc={:#010x})", self.kind, pc),
            None => format!("StopReason({})", self.kind),
        }
    }
}

fn build_bus(system_path: Option<PathBuf>) -> anyhow::Result<SystemBus> {
    let Some(sys_path) = system_path else {
        // No system config: fall back to the default bus. Note this currently
        // carries an STM32F103-ish peripheral layout; prefer passing an
        // explicit system config in production use.
        return Ok(SystemBus::new());
    };

    let manifest = labwired_config::SystemManifest::from_file(&sys_path)?;
    let chip_path = sys_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&manifest.chip);
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;
    SystemBus::from_config(&chip, &manifest)
}

/// Python binding for a running simulation.
///
/// Exposed to Python as `labwired.Machine`.
#[pyclass(name = "Machine")]
pub struct PyMachine {
    inner: Arc<Mutex<Box<dyn DebugControl + Send>>>,
}

#[pymethods]
impl PyMachine {
    #[new]
    #[pyo3(signature = (firmware, system_config=None))]
    fn new(firmware: String, system_config: Option<String>) -> PyResult<Self> {
        let firmware_path = PathBuf::from(firmware);
        let system_path = system_config.map(PathBuf::from);

        let program = labwired_loader::load_elf(&firmware_path).map_err(map_io)?;
        let mut bus = build_bus(system_path).map_err(map_anyhow)?;

        let machine: Box<dyn DebugControl + Send> = match program.arch {
            Arch::Arm => {
                let (cpu, _nvic) = cortex_m::configure_cortex_m(&mut bus);
                let mut m = labwired_core::Machine::new(cpu, bus);
                m.load_firmware(&program).map_err(map_sim_err)?;
                Box::new(m)
            }
            Arch::RiscV => {
                let cpu = riscv::configure_riscv(&mut bus);
                let mut m = labwired_core::Machine::new(cpu, bus);
                m.load_firmware(&program).map_err(map_sim_err)?;
                Box::new(m)
            }
            Arch::Unknown => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Unsupported architecture in ELF",
                ));
            }
        };

        Ok(PyMachine {
            inner: Arc::new(Mutex::new(machine)),
        })
    }

    #[pyo3(signature = (max_steps=None))]
    fn run(&self, max_steps: Option<u32>) -> PyResult<PyStopReason> {
        let mut guard = self.inner.lock().unwrap();
        let reason = guard.run(max_steps).map_err(map_sim_err)?;
        Ok(reason.into())
    }

    fn step(&self) -> PyResult<PyStopReason> {
        let mut guard = self.inner.lock().unwrap();
        let reason = guard.step_single().map_err(map_sim_err)?;
        Ok(reason.into())
    }

    fn read_register(&self, id: u8) -> u32 {
        self.inner.lock().unwrap().read_core_reg(id)
    }

    fn write_register(&self, id: u8, val: u32) {
        self.inner.lock().unwrap().write_core_reg(id, val);
    }

    fn read_memory(&self, addr: u32, len: usize) -> PyResult<Vec<u8>> {
        self.inner
            .lock()
            .unwrap()
            .read_memory(addr, len)
            .map_err(map_sim_err)
    }

    fn write_memory(&self, addr: u32, data: Vec<u8>) -> PyResult<()> {
        self.inner
            .lock()
            .unwrap()
            .write_memory(addr, &data)
            .map_err(map_sim_err)
    }

    fn add_breakpoint(&self, addr: u32) {
        self.inner.lock().unwrap().add_breakpoint(addr);
    }

    fn remove_breakpoint(&self, addr: u32) {
        self.inner.lock().unwrap().remove_breakpoint(addr);
    }

    fn clear_breakpoints(&self) {
        self.inner.lock().unwrap().clear_breakpoints();
    }

    fn reset(&self) -> PyResult<()> {
        self.inner.lock().unwrap().reset().map_err(map_sim_err)
    }

    fn get_pc(&self) -> u32 {
        self.inner.lock().unwrap().get_pc()
    }

    fn set_pc(&self, addr: u32) {
        self.inner.lock().unwrap().set_pc(addr);
    }

    fn get_cycle_count(&self) -> u64 {
        self.inner.lock().unwrap().get_cycle_count()
    }

    /// Serialize current machine state to a JSON string.
    fn snapshot(&self) -> PyResult<String> {
        let guard = self.inner.lock().unwrap();
        let snap = guard.snapshot();
        serde_json::to_string(&snap)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Restore machine state from a JSON string produced by `snapshot()`.
    fn restore(&self, json_snapshot: String) -> PyResult<()> {
        let snap: labwired_core::snapshot::MachineSnapshot =
            serde_json::from_str(&json_snapshot)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        self.inner
            .lock()
            .unwrap()
            .restore(&snap)
            .map_err(map_sim_err)
    }
}

#[pymodule]
fn labwired(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_class::<PyMachine>()?;
    m.add_class::<PyStopReason>()?;
    Ok(())
}
