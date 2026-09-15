use labwired_core::session::{
    AddrOrSymbol, OpenOptions, Session, SessionError, SessionSnapshot, StopReason,
};
use labwired_core::system::builder::{
    BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use pyo3::{
    create_exception,
    exceptions::{PyAssertionError, PyRuntimeError, PyValueError},
    prelude::*,
};
use std::{path::PathBuf, time::Duration};

create_exception!(_native, ExpectTimeout, PyAssertionError);
create_exception!(_native, NotSupported, PyRuntimeError);

fn error(e: SessionError) -> PyErr {
    match e {
        SessionError::ExpectTimeout { .. } => ExpectTimeout::new_err(e.to_string()),
        SessionError::NotSupported { .. } => NotSupported::new_err(e.to_string()),
        SessionError::Sim(_) | SessionError::Other(_) => PyRuntimeError::new_err(e.to_string()),
        _ => PyValueError::new_err(e.to_string()),
    }
}
fn build_error(e: anyhow::Error) -> PyErr {
    let text = format!("{e:#}");
    if text.starts_with("not supported:") || text.starts_with("UART receive source ") {
        NotSupported::new_err(text)
    } else {
        PyValueError::new_err(text)
    }
}
#[pyclass(unsendable)]
struct Snapshot {
    inner: SessionSnapshot,
}
#[pyclass(unsendable)]
struct NativeSession {
    inner: Option<Session>,
}
impl NativeSession {
    fn get(&self) -> PyResult<&Session> {
        self.inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Sim is closed"))
    }
    fn get_mut(&mut self) -> PyResult<&mut Session> {
        self.inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("Sim is closed"))
    }
}
#[pymethods]
impl NativeSession {
    #[new]
    #[pyo3(signature=(elf, chip_path, system_path=None, uart=None, catalog_root=None))]
    fn new(
        elf: PathBuf,
        chip_path: Option<PathBuf>,
        system_path: Option<PathBuf>,
        uart: Option<String>,
        catalog_root: Option<PathBuf>,
    ) -> PyResult<Self> {
        let loaded = system_path
            .as_ref()
            .map(|path| labwired_config::SystemManifest::from_file(path).map_err(build_error))
            .transpose()?;
        let path = if let Some(path) = chip_path {
            path
        } else if let (Some(system), Some(path)) = (loaded.as_ref(), system_path.as_ref()) {
            let reference = std::path::Path::new(&system.chip);
            if labwired_config::is_builtin_chip_spec(&system.chip) {
                catalog_root
                    .as_ref()
                    .ok_or_else(|| {
                        PyValueError::new_err("a packaged catalog is required for a bare chip name")
                    })?
                    .join("chips")
                    .join(format!("{}.yaml", system.chip))
            } else {
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(reference)
            }
        } else {
            return Err(PyValueError::new_err("provide chip or system"));
        };
        let chip_path = path
            .canonicalize()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let chip = labwired_config::ChipDescriptor::from_file(&chip_path).map_err(build_error)?;
        let mut system: labwired_config::SystemManifest = if let Some(system) = loaded {
            system
        } else {
            serde_json::from_value(serde_json::json!({"name":chip.name,"chip":chip_path}))
                .map_err(|e| PyValueError::new_err(e.to_string()))?
        };
        system.chip = chip_path.to_string_lossy().to_string();
        if uart.is_some() {
            system.debug_uart = uart;
        }
        let firmware =
            std::fs::read(elf).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        let session = Session::open(
            BuildRequest {
                chip: &chip,
                system: &system,
                firmware: FirmwareSource::Elf(&firmware),
                boot: BootMode::FastBoot,
                blobs: &BlobMap::new(),
                options: BuildOptions {
                    uart_rx: system.debug_uart.clone(),
                    ..Default::default()
                },
            },
            OpenOptions::default(),
        )
        .map_err(build_error)?;
        Ok(Self {
            inner: Some(session),
        })
    }
    fn close(&mut self) {
        self.inner = None;
    }
    #[getter]
    fn time(&self) -> PyResult<f64> {
        Ok(self.get()?.time().as_secs_f64())
    }
    #[getter]
    fn cycles(&self) -> PyResult<u64> {
        Ok(self.get()?.cycles())
    }
    fn run_for(&mut self, ns: u64) -> PyResult<super::PyStopReason> {
        let session = self.get_mut()?;
        let reason = if ns == 0 {
            session.run_cycles(0)
        } else {
            session.run_for(Duration::from_nanos(ns))
        }
        .map_err(error)?;
        let (kind, pc) = match reason {
            StopReason::Reached => ("reached", None),
            StopReason::Halted => ("halted", None),
            StopReason::Breakpoint(pc) => ("breakpoint", Some(pc)),
            StopReason::Error => ("error", None),
        };
        Ok(super::PyStopReason {
            kind: kind.into(),
            pc,
        })
    }
    fn expect(&mut self, pattern: &str, ns: u64) -> PyResult<(String, Vec<Option<String>>, f64)> {
        let m = self
            .get_mut()?
            .expect(pattern, Duration::from_nanos(ns))
            .map_err(error)?;
        Ok((m.text, m.captures, m.at.as_secs_f64()))
    }
    fn send(&mut self, data: &[u8]) -> PyResult<()> {
        self.get_mut()?.send(data);
        Ok(())
    }
    fn read_uart(&mut self) -> PyResult<Vec<u8>> {
        Ok(self.get_mut()?.read_uart())
    }
    fn uart_transcript(&self) -> PyResult<String> {
        Ok(self.get()?.uart_transcript())
    }
    fn set_input(&mut self, channel: &str, value: f64) -> PyResult<()> {
        self.get_mut()?.set_input(channel, value).map_err(error)
    }
    fn set_inputs(&mut self, values: Vec<(String, f64)>) -> PyResult<()> {
        self.get_mut()?.set_inputs(&values).map_err(error)
    }
    fn list_inputs(&mut self) -> PyResult<String> {
        serde_json::to_string(&self.get_mut()?.list_inputs())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
    fn set_pin(&mut self, binding: &str, active: bool) -> PyResult<()> {
        self.get_mut()?.set_pin(binding, active).map_err(error)
    }
    fn read_memory(&self, address: u64, length: usize) -> PyResult<Vec<u8>> {
        self.get()?.read_memory(address, length).map_err(error)
    }
    fn read_u32(&self, address: &PyAny) -> PyResult<u32> {
        let at = if let Ok(n) = address.extract::<u64>() {
            AddrOrSymbol::Addr(n)
        } else {
            AddrOrSymbol::Symbol(address.extract::<&str>()?)
        };
        self.get()?.read_u32(at).map_err(error)
    }
    fn write_u32(&mut self, address: &PyAny, value: u32) -> PyResult<()> {
        let at = if let Ok(n) = address.extract::<u64>() {
            AddrOrSymbol::Addr(n)
        } else {
            AddrOrSymbol::Symbol(address.extract::<&str>()?)
        };
        self.get_mut()?.write_u32(at, value).map_err(error)
    }
    fn symbol(&self, name: &str) -> PyResult<Option<u64>> {
        Ok(self.get()?.symbol(name))
    }
    fn frames(&mut self) -> PyResult<String> {
        serde_json::to_string(&self.get_mut()?.frames())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
    fn snapshot(&self) -> PyResult<Snapshot> {
        Ok(Snapshot {
            inner: self.get()?.snapshot(),
        })
    }
    fn restore(&mut self, snapshot: &Snapshot) -> PyResult<()> {
        self.get_mut()?.restore(&snapshot.inner).map_err(error)
    }
    #[allow(clippy::too_many_arguments)]
    fn inject_can(
        &mut self,
        bus: &str,
        id: u32,
        data: Vec<u8>,
        extended: bool,
        fd: bool,
        bitrate_switch: bool,
        remote: bool,
    ) -> PyResult<()> {
        self.get_mut()?
            .inject_can(
                bus,
                labwired_core::session::CanFrame {
                    id,
                    data,
                    extended,
                    fd,
                    bitrate_switch,
                    remote,
                },
            )
            .map_err(error)
    }
}
pub fn register(py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<NativeSession>()?;
    m.add_class::<Snapshot>()?;
    m.add("ExpectTimeout", py.get_type::<ExpectTimeout>())?;
    m.add("NotSupported", py.get_type::<NotSupported>())?;
    Ok(())
}
