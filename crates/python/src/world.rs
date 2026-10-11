//! A multi-chip world for Python: the machines of a lowered diagram, the UART
//! links and GPIO nets between them, stepped together in simulated time.
//!
//! This is `labwired_core::world::World` built the way `labwired test` builds it
//! from an environment manifest (`World::from_manifest`), with each node's
//! console captured the way the CLI captures it. Nothing here models anything.

use labwired_core::machine::world_hooks::ObserverRef;
use labwired_core::world::{StepResults, World};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// One node's console: everything the node wrote, and how much was handed out.
#[derive(Default)]
struct Console {
    sink: Arc<Mutex<Vec<u8>>>,
    transcript: Vec<u8>,
    cursor: usize,
}

impl Console {
    /// Move what the node has written since the last call into the transcript.
    fn sync(&mut self) {
        let mut sink = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        self.transcript.append(&mut sink);
    }
}

#[pyclass(unsendable)]
pub struct NativeWorld {
    world: Option<World>,
    consoles: BTreeMap<String, Console>,
    /// One round's results, reused from round to round.
    step: StepResults,
}

impl NativeWorld {
    fn world(&self) -> PyResult<&World> {
        self.world
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("World is closed"))
    }
    fn world_mut(&mut self) -> PyResult<&mut World> {
        self.world
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("World is closed"))
    }
    fn console(&mut self, node: &str) -> PyResult<&mut Console> {
        self.consoles
            .get_mut(node)
            .ok_or_else(|| PyValueError::new_err(format!("unknown world node '{node}'")))
    }
    fn sync_consoles(&mut self) {
        for console in self.consoles.values_mut() {
            console.sync();
        }
    }
    fn now_ns(&self) -> PyResult<u64> {
        let world = self.world()?;
        Ok(world
            .machines
            .keys()
            .filter_map(|id| world.node_time_ns(id))
            .min()
            .unwrap_or(0))
    }
    fn step_once(&mut self) -> PyResult<()> {
        let world = self
            .world
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("World is closed"))?;
        world.step_all_into(&mut self.step);
        match self.step.first_error() {
            Some((id, e)) => Err(PyRuntimeError::new_err(format!("node '{id}' step: {e:?}"))),
            None => Ok(()),
        }
    }
    fn machine(
        &mut self,
        node: &str,
    ) -> PyResult<&mut Box<dyn labwired_core::world::MachineTrait>> {
        self.world_mut()?
            .machines
            .get_mut(node)
            .ok_or_else(|| PyValueError::new_err(format!("unknown world node '{node}'")))
    }
}

#[pymethods]
impl NativeWorld {
    #[new]
    fn new(environment: PathBuf) -> PyResult<Self> {
        let manifest = labwired_config::EnvironmentManifest::from_file(&environment)
            .map_err(|e| PyValueError::new_err(format!("{e:#}")))?;
        let root = environment
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut world = World::from_manifest(manifest, &root)
            .map_err(|e| PyValueError::new_err(format!("{e:#}")))?;
        let mut consoles = BTreeMap::new();
        let mut ids: Vec<String> = world.machines.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let console = Console::default();
            world
                .machines
                .get_mut(&id)
                .expect("id was read from this world")
                .attach_uart_tx_sink(console.sink.clone(), false)
                .map_err(|e| PyRuntimeError::new_err(format!("node '{id}': {e:#}")))?;
            consoles.insert(id, console);
        }
        Ok(Self {
            world: Some(world),
            consoles,
            step: StepResults::new(),
        })
    }
    fn close(&mut self) {
        self.sync_consoles();
        self.world = None;
    }
    fn node_ids(&self) -> Vec<String> {
        self.consoles.keys().cloned().collect()
    }
    /// World time: the slowest node's clock, in ns.
    fn time_ns(&self) -> PyResult<u64> {
        self.now_ns()
    }
    fn node_time_ns(&self, node: &str) -> PyResult<u64> {
        self.world()?
            .node_time_ns(node)
            .ok_or_else(|| PyValueError::new_err(format!("unknown world node '{node}'")))
    }
    fn cpu_hz(&self, node: &str) -> PyResult<u64> {
        self.world()?
            .node_hz(node)
            .ok_or_else(|| PyValueError::new_err(format!("unknown world node '{node}'")))
    }
    fn cycles(&mut self, node: &str) -> PyResult<u64> {
        Ok(self.machine(node)?.total_cycles())
    }
    /// Advance every node by `ns` of simulated time.
    fn run_for(&mut self, ns: u64) -> PyResult<()> {
        let target = self.now_ns()?.saturating_add(ns);
        let mut stalled = 0u32;
        while self.now_ns()? < target {
            let before = self.now_ns()?;
            self.step_once()?;
            // Every node halted: time cannot move, so the run is over.
            if self.now_ns()? == before {
                stalled += 1;
                if stalled > 1_000_000 {
                    break;
                }
            } else {
                stalled = 0;
            }
        }
        self.sync_consoles();
        Ok(())
    }
    /// Run until `pattern` appears in `node`'s unread console output, or `ns`
    /// of simulated time pass. Returns `(text, captures, seconds)`.
    fn expect(
        &mut self,
        node: &str,
        pattern: &str,
        ns: u64,
    ) -> PyResult<(String, Vec<Option<String>>, f64)> {
        let re = regex::bytes::Regex::new(pattern).map_err(|e| {
            PyValueError::new_err(format!("invalid expect pattern /{pattern}/: {e}"))
        })?;
        let deadline = self.now_ns()?.saturating_add(ns);
        loop {
            self.sync_consoles();
            let at = self.now_ns()?;
            let console = self.console(node)?;
            if let Some(caps) = re.captures(&console.transcript[console.cursor..]) {
                let whole = caps.get(0).expect("group 0 always participates");
                let text = String::from_utf8_lossy(whole.as_bytes()).into_owned();
                let captures = (1..caps.len())
                    .map(|i| {
                        caps.get(i)
                            .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
                    })
                    .collect();
                console.cursor += whole.end();
                return Ok((text, captures, at as f64 / 1e9));
            }
            if at >= deadline {
                let seen =
                    String::from_utf8_lossy(&console.transcript[console.cursor..]).into_owned();
                return Err(crate::session::ExpectTimeout::new_err(format!(
                    "node '{node}': /{pattern}/ did not appear within {ns} ns of simulated time; unread output: {seen:?}"
                )));
            }
            self.step_once()?;
        }
    }
    /// Unread console output of `node`, drained.
    fn read_uart(&mut self, node: &str) -> PyResult<Vec<u8>> {
        self.sync_consoles();
        let console = self.console(node)?;
        let out = console.transcript[console.cursor..].to_vec();
        console.cursor = console.transcript.len();
        Ok(out)
    }
    fn uart_transcript(&mut self, node: &str) -> PyResult<String> {
        self.sync_consoles();
        Ok(String::from_utf8_lossy(&self.console(node)?.transcript).into_owned())
    }
    fn set_input(&mut self, node: &str, channel: &str, value: f64) -> PyResult<()> {
        self.machine(node)?
            .set_input_channel(channel, value)
            .map_err(PyValueError::new_err)
    }
    fn list_inputs(&mut self, node: &str) -> PyResult<String> {
        serde_json::to_string(&self.machine(node)?.list_input_channels())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
    /// Hold GPIO pad `(peripheral, pin)` of `node` at `level`, as an external
    /// contact would.
    fn set_pin(&mut self, node: &str, peripheral: &str, pin: u8, level: bool) -> PyResult<()> {
        if self.machine(node)?.drive_gpio_input(peripheral, pin, level) {
            Ok(())
        } else {
            Err(PyValueError::new_err(format!(
                "node '{node}': pad {peripheral}:{pin} cannot be driven from outside"
            )))
        }
    }
    /// Arm `node`'s logic analyzer on GPIO pads and return each level now.
    fn watch_logic(&mut self, node: &str, pads: Vec<(String, u8)>) -> PyResult<Vec<Option<bool>>> {
        let refs: Vec<ObserverRef> = pads
            .into_iter()
            .map(|(peripheral, pin)| ObserverRef {
                kind: "gpio".to_string(),
                peripheral,
                pin,
                line: None,
            })
            .collect();
        let rows = self.machine(node)?.observer_watch(&refs);
        if let Some(why) = rows.iter().find_map(|r| r.error.clone()) {
            return Err(PyValueError::new_err(format!("node '{node}': {why}")));
        }
        Ok(rows.into_iter().map(|r| r.initial).collect())
    }
    fn logic_edges(&mut self, node: &str, cursor: u64) -> PyResult<String> {
        let machine = self.machine(node)?;
        let batch = machine.observer_edges(cursor);
        let edges: Vec<_> = batch
            .edges
            .iter()
            .map(|e| serde_json::json!({"ch": e.ch, "cycle": e.cycle, "value": e.value}))
            .collect();
        Ok(serde_json::json!({
            "cursor": batch.cursor,
            "dropped": batch.dropped,
            "now_cycle": machine.total_cycles(),
            "edges": edges,
        })
        .to_string())
    }
    fn read_memory(&mut self, node: &str, address: u32, length: usize) -> PyResult<Vec<u8>> {
        self.machine(node)?
            .read_memory(address, length)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
    /// Every GPIO net as JSON: level, edge count, members, and diagnostics
    /// (`GPIO_NET_CONTENTION`, `GPIO_NET_FLOATING`).
    fn gpio_nets(&self) -> PyResult<String> {
        serde_json::to_string(&self.world()?.gpio_net_reports())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
}

pub fn register(m: &PyModule) -> PyResult<()> {
    m.add_class::<NativeWorld>()
}
