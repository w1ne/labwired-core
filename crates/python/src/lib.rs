#![allow(non_local_definitions)]

use labwired_core::{
    bus::SystemBus,
    system::{cortex_m, riscv},
    Arch, DebugControl, SimulationError, StopReason,
};
use pyo3::prelude::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// Wrapper for StopReason to expose to Python
#[derive(Clone, Debug, PartialEq)]
#[pyclass(name = "StopReason")]
struct PyStopReason {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    pc: Option<u32>,
}

impl From<StopReason> for PyStopReason {
    fn from(r: StopReason) -> Self {
        match r {
            StopReason::Breakpoint(pc) => PyStopReason {
                kind: "breakpoint".to_string(),
                pc: Some(pc),
            },
            StopReason::StepDone => PyStopReason {
                kind: "step_done".to_string(),
                pc: None,
            },
            StopReason::MaxStepsReached => PyStopReason {
                kind: "max_steps_reached".to_string(),
                pc: None,
            },
            StopReason::ManualStop => PyStopReason {
                kind: "manual_stop".to_string(),
                pc: None,
            },
        }
    }
}

// Wrapper for exceptions
struct PySimulationError(SimulationError);

impl From<PySimulationError> for PyErr {
    fn from(err: PySimulationError) -> PyErr {
        pyo3::exceptions::PyRuntimeError::new_err(err.0.to_string())
    }
}

fn build_bus(system_path: Option<PathBuf>) -> anyhow::Result<SystemBus> {
    use labwired_core::memory::LinearMemory;

    // Default memory map if no config
    if system_path.is_none() {
        let mut bus = SystemBus::new();
        // Standard Cortex-M layout
        bus.flash = LinearMemory::new(1024 * 1024, 0x0800_0000); // 1MB Flash
        bus.ram = LinearMemory::new(128 * 1024, 0x2000_0000); // 128KB RAM
        return Ok(bus);
    }

    // Load from config
    let sys_path = system_path.unwrap();
    let manifest = labwired_config::SystemManifest::from_file(&sys_path)?;

    let chip_path = sys_path
        .parent()
        .unwrap_or(&PathBuf::from("."))
        .join(&manifest.chip);
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;

    let bus = SystemBus::from_config(&chip, &manifest)?;

    Ok(bus)
}

#[pyclass]
/// The core LabWired machine simulator.
///
/// This class provides a direct interface to the Rust simulation core, allowing
/// for loading firmware, stepping execution, inspecting state, and time-travel debugging.
struct Machine {
    inner: Arc<Mutex<Box<dyn DebugControl + Send>>>,
}

#[allow(non_local_definitions)]
#[pymethods]
impl Machine {
    #[new]
    #[pyo3(signature = (firmware, system_config=None))]
    /// Create a new Machine instance.
    ///
    /// Args:
    ///     firmware (str): Path to the ELF firmware file to load.
    ///     system_config (Optional[str]): Path to the system configuration YAML.
    ///         If None, defaults to a standard Cortex-M layout (1MB Flash @ 0x08000000, 128KB RAM @ 0x20000000).
    fn new(firmware: String, system_config: Option<String>) -> PyResult<Self> {
        let firmware_path = PathBuf::from(firmware);
        let system_path = system_config.map(PathBuf::from);

        // Load firmware to check arch
        let program = labwired_loader::load_elf(&firmware_path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;

        // Config Bus
        let mut bus = build_bus(system_path)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        // Create Machine based on Architecture
        let machine: Box<dyn DebugControl + Send> = match program.arch {
            Arch::Arm => {
                let (cpu, _nvic) = cortex_m::configure_cortex_m(&mut bus);
                let mut m = labwired_core::Machine::new(cpu, bus);
                m.load_firmware(&program).map_err(PySimulationError)?;
                Box::new(m)
            }
            Arch::RiscV => {
                let cpu = riscv::configure_riscv(&mut bus);
                let mut m = labwired_core::Machine::new(cpu, bus);
                m.load_firmware(&program).map_err(PySimulationError)?;
                Box::new(m)
            }
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Unsupported architecture",
                ))
            }
        };

        Ok(Machine {
            inner: Arc::new(Mutex::new(machine)),
        })
    }

    /// Run the simulation for a specified number of steps.
    ///
    /// Args:
    ///     max_steps (Optional[int]): Maximum number of instructions to execute.
    ///         If None, runs indefinitely until a breakpoint or manual stop.
    ///
    /// Returns:
    ///     StopReason: The reason why the simulation stopped (e.g., Breakpoint, MaxStepsReached).
    fn step(&mut self, max_steps: Option<u32>) -> PyResult<PyStopReason> {
        let mut guard = self.inner.lock().unwrap();
        let reason = guard.run(max_steps).map_err(PySimulationError)?;
        Ok(reason.into())
    }

    /// Read a core register by its ID.
    ///
    /// Args:
    ///     id (int): The register ID (e.g., 0-15 for ARM Cortex-M R0-PC).
    ///
    /// Returns:
    ///     int: The 32-bit value of the register.
    fn read_register(&self, id: u8) -> u32 {
        let guard = self.inner.lock().unwrap();
        guard.read_core_reg(id)
    }

    /// Write a value to a core register.
    ///
    /// Args:
    ///     id (int): The register ID.
    ///     val (int): The 32-bit value to write.
    fn write_register(&mut self, id: u8, val: u32) {
        let mut guard = self.inner.lock().unwrap();
        guard.write_core_reg(id, val);
    }

    /// Read a block of memory.
    ///
    /// Args:
    ///     addr (int): The start address.
    ///     len (int): The number of bytes to read.
    ///
    /// Returns:
    ///     List[int]: The bytes read from memory.
    fn read_memory(&self, addr: u32, len: usize) -> PyResult<Vec<u8>> {
        let guard = self.inner.lock().unwrap();
        guard
            .read_memory(addr, len)
            .map_err(PySimulationError)
            .map_err(Into::into)
    }

    /// Write a block of memory.
    ///
    /// Args:
    ///     addr (int): The start address.
    ///     data (List[int]): The bytes to write.
    fn write_memory(&mut self, addr: u32, data: Vec<u8>) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap();
        guard
            .write_memory(addr, &data)
            .map_err(PySimulationError)
            .map_err(Into::into)
    }

    /// Take a snapshot of the current machine state.
    ///
    /// Returns:
    ///     str: A JSON string containing the full state of the CPU and peripherals.
    fn snapshot(&self) -> PyResult<String> {
        let guard = self.inner.lock().unwrap();
        let snap = guard.snapshot();
        serde_json::to_string(&snap)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Restore the machine state from a snapshot.
    ///
    /// Args:
    ///     json_snapshot (str): The JSON string from a previous `snapshot()` call.
    fn restore(&mut self, json_snapshot: String) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap();
        let snap: labwired_core::snapshot::MachineSnapshot =
            serde_json::from_str(&json_snapshot)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        guard
            .restore(&snap)
            .map_err(PySimulationError)
            .map_err(Into::into)
    }

    /// Get the current Program Counter (PC).
    fn get_pc(&self) -> u32 {
        let guard = self.inner.lock().unwrap();
        guard.get_pc()
    }
}

#[pymodule]
fn labwired(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Machine>()?;
    m.add_class::<PyStopReason>()?;
    Ok(())
}
