// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Browser snapshot/restore, fault experiments and firmware coverage.
//!
//! # Snapshot and restore
//!
//! The machine's own snapshot types do not carry all of RAM, peripheral and
//! device state, or the event scheduler, so restoring from one would report
//! success over a machine that never existed. The browser uses the same method
//! as `labwired_core::session::Session::restore`: a simulator built by
//! [`WasmSimulator::new_from_config`] keeps its build inputs and a journal of
//! every call that changes the machine (steps and stimulus). A snapshot is a
//! position in that journal plus a digest of the machine (cycles, core
//! registers, main RAM). A restore builds a fresh machine from the same inputs,
//! replays the journal up to that position and compares the digest. The
//! simulator is deterministic, so it is the same machine; if the digest differs
//! the restore is refused and the current machine is left as it was.
//!
//! A simulator wired to another MCU (a UART wire or the shared air bus) is fed
//! by a second machine the journal cannot replay, so it refuses to snapshot.
//!
//! # Fault experiments
//!
//! Two fresh copies are built and brought to the same point by the same
//! replay; `labwired_core::vfi::run_lockstep` then fires the faults on one copy
//! and compares them instruction by instruction.
//!
//! # Coverage
//!
//! A `PcCoverageObserver` on the machine, mapped through the firmware ELF's
//! DWARF by `labwired_loader::coverage` — the same report as the CLI's
//! `labwired test --coverage`.

use crate::WasmSimulator;
use labwired_core::pc_coverage::PcCoverageObserver;
use labwired_core::DebugControl;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

/// The inputs [`WasmSimulator::new_from_config`] was called with.
pub(crate) struct CtorInputs {
    pub system_yaml: String,
    pub chip_yaml: String,
    pub firmware: Vec<u8>,
    pub blobs: HashMap<String, Vec<u8>>,
}

/// One call that changes the machine, in the order JS made it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Op {
    Step(u32),
    StepSingle,
    StepBatch(u32),
    StepEsp32Aids(u32),
    FeedUart(Vec<u8>),
    FeedRtt(Vec<u8>),
    WriteRttDown(Vec<u8>),
    WriteSemihosting(Vec<u8>),
    SetInput(String, f64),
    SetInputs(serde_json::Value),
    ListInputs,
    SetBoardIo(String, bool),
    SetAdcValue(String, u16),
    SetAdcMillivolts(String, u8, u16),
    ClearAdc(String, u8),
    SetNtc(String, f32),
    SetPot(String, f32),
    SetMotorInput(String, String, f64),
    SetMotorFault(String, String, bool),
    SetCosim(String, f64),
    SetIdleFastForward(bool),
    SetTickInterval(u32),
    SetJit(bool),
    InstallEsp32Quirks(Vec<u8>),
    ApplyRuntimeSnapshot(Vec<u8>),
    WatchLogic(serde_json::Value),
    ReadLogicEdges(f64),
}

impl Op {
    fn is_step(&self) -> bool {
        matches!(
            self,
            Op::Step(_) | Op::StepSingle | Op::StepBatch(_) | Op::StepEsp32Aids(_)
        )
    }
}

/// A position in the journal. Identical consecutive steps share one entry
/// (the playground steps every frame, so an hour is hundreds of thousands of
/// them); `last_times` pins how many repeats of the last entry the position
/// includes, because that entry keeps counting after the position is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mark {
    entries: usize,
    last_times: u32,
}

/// A saved point: a journal position and what the machine looked like there.
#[derive(Debug, Clone)]
struct SavedPoint {
    id: u32,
    label: String,
    ops: Mark,
    cycles: u64,
    digest: u64,
}

#[derive(Default)]
pub(crate) struct SimTools {
    pub(crate) ctor: Option<Rc<CtorInputs>>,
    /// Calls with a repeat count; see [`Mark`].
    journal: Vec<(Op, u32)>,
    /// The first change the journal cannot replay; snapshots are refused
    /// from then on.
    untracked: Option<String>,
    points: Vec<SavedPoint>,
    next_id: u32,
    coverage: Option<Arc<PcCoverageObserver>>,
    /// The application ELF handed to `install_arduino_esp32_quirks`, used for
    /// coverage when the machine was built from flash images.
    elf: Option<Vec<u8>>,
}

fn js(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

impl WasmSimulator {
    pub(crate) fn record(&self, op: Op) {
        let mut tools = self.tools.borrow_mut();
        if let Op::InstallEsp32Quirks(elf) = &op {
            tools.elf = Some(elf.clone());
        }
        match tools.journal.last_mut() {
            Some((last, times)) if op.is_step() && *last == op && *times < u32::MAX => *times += 1,
            _ => tools.journal.push((op, 1)),
        }
    }

    fn current_mark(&self) -> Mark {
        let tools = self.tools.borrow();
        Mark {
            entries: tools.journal.len(),
            last_times: tools.journal.last().map_or(0, |(_, t)| *t),
        }
    }

    /// The journal up to `mark`, with the last entry's repeats pinned.
    fn journal_prefix(&self, mark: Mark) -> Vec<(Op, u32)> {
        let tools = self.tools.borrow();
        let mut prefix = tools.journal[..mark.entries].to_vec();
        if let Some((_, times)) = prefix.last_mut() {
            *times = mark.last_times;
        }
        prefix
    }

    /// Note a change the journal cannot replay.
    pub(crate) fn untracked(&self, what: &str) {
        let mut tools = self.tools.borrow_mut();
        if tools.untracked.is_none() {
            tools.untracked = Some(what.to_string());
        }
    }

    /// Whether a coverage observer is attached (live recording).
    pub(crate) fn coverage_recording(&self) -> bool {
        self.tools.borrow().coverage.is_some()
    }

    /// Why this simulator cannot be snapshotted, if it cannot.
    fn snapshot_refusal(&self) -> Option<String> {
        let tools = self.tools.borrow();
        if tools.ctor.is_none() {
            return Some(
                "this simulator was not built by new_from_config, so it cannot be rebuilt".into(),
            );
        }
        tools.untracked.as_ref().map(|what| {
            format!("this simulator is {what}; a replay cannot reproduce another machine's input")
        })
    }

    /// Re-make one journaled call. The call records itself again, so the
    /// rebuilt simulator ends with the same journal. Its result is dropped:
    /// the original call already reported it, and replaying an error
    /// reproduces its effect exactly.
    fn replay(&mut self, op: &Op) {
        let _ = match op {
            Op::Step(n) => self.step(*n),
            Op::StepSingle => self.step_single(),
            Op::StepBatch(n) => self.step_batch(*n).map(|_| ()),
            Op::StepEsp32Aids(n) => self.step_with_esp32_aids(*n),
            Op::FeedUart(b) => {
                self.feed_uart_input(b);
                Ok(())
            }
            Op::FeedRtt(b) => self.feed_rtt_input(b),
            Op::WriteRttDown(b) => self.write_rtt_down(b).map(|_| ()),
            Op::WriteSemihosting(b) => self.write_semihosting_input(b),
            Op::SetInput(c, v) => self.set_input(c, *v),
            Op::SetInputs(v) => match serde_wasm_bindgen::to_value(v) {
                Ok(v) => self.set_inputs(v),
                Err(e) => Err(js(e)),
            },
            Op::ListInputs => self.list_inputs().map(|_| ()),
            Op::SetBoardIo(id, a) => self.set_board_io_input(id, *a),
            Op::SetAdcValue(p, v) => self.set_adc_value(p, *v),
            Op::SetAdcMillivolts(p, c, mv) => self.set_adc_channel_millivolts(p, *c, *mv),
            Op::ClearAdc(p, c) => self.clear_adc_channel(p, *c),
            Op::SetNtc(d, t) => self.set_ntc_temperature(d, *t),
            Op::SetPot(d, p) => self.set_potentiometer(d, *p),
            Op::SetMotorInput(i, n, v) => self.set_motor_input(i, n, *v),
            Op::SetMotorFault(i, f, a) => self.set_motor_fault(i, f, *a),
            Op::SetCosim(p, v) => self.set_cosim_signal(p, *v),
            Op::SetIdleFastForward(on) => {
                self.set_idle_fast_forward_enabled(*on);
                Ok(())
            }
            Op::SetTickInterval(n) => {
                self.set_peripheral_tick_interval(*n);
                Ok(())
            }
            Op::SetJit(on) => {
                self.set_jit_enabled(*on);
                Ok(())
            }
            Op::InstallEsp32Quirks(elf) => self.install_arduino_esp32_quirks(elf),
            Op::ApplyRuntimeSnapshot(b) => self.apply_runtime_snapshot(b),
            Op::WatchLogic(v) => match serde_wasm_bindgen::to_value(v) {
                Ok(v) => {
                    self.watch_logic_signals(v);
                    Ok(())
                }
                Err(e) => Err(js(e)),
            },
            Op::ReadLogicEdges(c) => {
                self.read_logic_edges(*c);
                Ok(())
            }
        };
    }

    /// The position before the first step: the configuration a run starts
    /// from.
    fn boot_mark(&self) -> Mark {
        let tools = self.tools.borrow();
        let entries = tools
            .journal
            .iter()
            .position(|(op, _)| op.is_step())
            .unwrap_or(tools.journal.len());
        Mark {
            entries,
            last_times: entries.checked_sub(1).map_or(0, |i| tools.journal[i].1),
        }
    }

    fn firmware_elf(&self) -> Option<Vec<u8>> {
        let tools = self.tools.borrow();
        if let Some(elf) = &tools.elf {
            return Some(elf.clone());
        }
        tools
            .ctor
            .as_ref()
            .map(|c| c.firmware.clone())
            .filter(|f| !f.is_empty())
    }
}

/// A copy the lockstep engine drives: the machine and its console sink.
struct Side<'a>(&'a mut WasmSimulator);

impl labwired_core::vfi::LockstepTarget for Side<'_> {
    fn machine(&mut self) -> &mut dyn DebugControl {
        self.0
            .machine
            .as_mut()
            .expect("rebuilt simulators have a machine")
    }
    fn machine_ref(&self) -> &dyn DebugControl {
        self.0
            .machine
            .as_ref()
            .expect("rebuilt simulators have a machine")
    }
    fn console(&self) -> Vec<u8> {
        self.0
            .uart_sink
            .lock()
            .map(|b| b.clone())
            .unwrap_or_default()
    }
}

impl WasmSimulator {
    /// A machine for the logic below, with a `String` error so the logic can
    /// be tested natively (a `JsValue` cannot be built off wasm32).
    fn machine_s(&self) -> Result<&labwired_core::Machine<Box<dyn labwired_core::Cpu>>, String> {
        self.machine
            .as_ref()
            .ok_or_else(|| "simulator has no machine".to_string())
    }

    pub(crate) fn snapshot_save_s(&mut self, label: Option<String>) -> Result<String, String> {
        if let Some(why) = self.snapshot_refusal() {
            return Err(format!("cannot snapshot: {why}"));
        }
        let digest = self.digest_s()?;
        let cycles = self.machine_s()?.total_cycles;
        let mark = self.current_mark();
        let mut tools = self.tools.borrow_mut();
        tools.next_id += 1;
        let point = SavedPoint {
            id: tools.next_id,
            label: label.unwrap_or_else(|| format!("state {}", tools.next_id)),
            ops: mark,
            cycles,
            digest,
        };
        let out = serde_json::json!({"id": point.id, "label": point.label, "cycles": point.cycles});
        tools.points.push(point);
        Ok(out.to_string())
    }

    fn digest_s(&self) -> Result<u64, String> {
        let m = self.machine_s()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        m.total_cycles.hash(&mut h);
        let n = DebugControl::get_register_names(m).len();
        for i in 0..n {
            DebugControl::read_core_reg(m, i as u8).hash(&mut h);
        }
        DebugControl::get_pc(m).hash(&mut h);
        m.bus.ram.data.hash(&mut h);
        Ok(h.finish())
    }

    pub(crate) fn snapshot_restore_s(&mut self, id: u32) -> Result<String, String> {
        if let Some(why) = self.snapshot_refusal() {
            return Err(format!("cannot restore: {why}"));
        }
        let point = self
            .tools
            .borrow()
            .points
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| format!("no saved state {id}"))?;
        let mut fresh = self.rebuild_s(point.ops, None)?;
        let digest = fresh.digest_s()?;
        let cycles = fresh.machine_s()?.total_cycles;
        if digest != point.digest || cycles != point.cycles {
            return Err(format!(
                "restore refused: the replay reached cycle {cycles}, the saved state is at cycle \
                 {}, and their registers or RAM differ. Something changed the machine that the \
                 journal does not record (for example a memory read with side effects). The \
                 current state is unchanged.",
                point.cycles
            ));
        }
        // Carry over what belongs to the tab, not to the machine.
        {
            let mut old = self.tools.borrow_mut();
            let mut new = fresh.tools.borrow_mut();
            new.points = std::mem::take(&mut old.points);
            new.next_id = old.next_id;
            if old.elf.is_some() {
                new.elf = old.elf.take();
            }
            new.coverage = old.coverage.take();
        }
        let coverage = fresh.tools.borrow().coverage.clone();
        if let Some(obs) = coverage {
            fresh.jit_browser_enabled = false;
            fresh.jit_browser_cache = None;
            if let Some(m) = fresh.machine.as_mut() {
                m.add_observer(obs);
            }
        }
        // Output up to the saved point was shown before; do not repeat it.
        if let Ok(mut sink) = fresh.uart_sink.lock() {
            sink.clear();
        }
        *self = fresh;
        Ok(serde_json::json!({"id": point.id, "cycles": point.cycles}).to_string())
    }

    /// A fresh simulator from this one's build inputs with the journal replayed
    /// up to `ops`. With `coverage`, the observer is attached
    /// before the first replayed call, so it sees the run from power-on.
    fn rebuild_s(
        &self,
        ops: Mark,
        coverage: Option<Arc<PcCoverageObserver>>,
    ) -> Result<WasmSimulator, String> {
        let ctor = self
            .tools
            .borrow()
            .ctor
            .clone()
            .ok_or("this simulator was not built by new_from_config")?;
        let journal = self.journal_prefix(ops);
        // The inputs built a machine once already, so they build one again;
        // an error here is not reachable from a well-formed simulator.
        let mut fresh = WasmSimulator::new_from_config_parts(ctor)
            .map_err(|_| "rebuilding the machine from its own inputs failed".to_string())?;
        if let Some(obs) = coverage {
            fresh.jit_browser_enabled = false;
            if let Some(m) = fresh.machine.as_mut() {
                m.add_observer(obs.clone());
            }
            fresh.tools.get_mut().coverage = Some(obs);
        }
        for (op, times) in &journal {
            for _ in 0..*times {
                fresh.replay(op);
            }
        }
        Ok(fresh)
    }

    pub(crate) fn fault_experiment_s(
        &self,
        plan_json: &str,
        from_boot: bool,
    ) -> Result<String, String> {
        let plan: labwired_core::vfi::FaultPlan =
            serde_json::from_str(plan_json).map_err(|e| format!("fault plan: {e}"))?;
        if let Some(why) = self.snapshot_refusal() {
            return Err(format!("cannot run a fault experiment: {why}"));
        }
        let ops = if from_boot {
            self.boot_mark()
        } else {
            self.current_mark()
        };
        let mut golden = self.rebuild_s(ops, None)?;
        let mut faulted = self.rebuild_s(ops, None)?;
        let isa = labwired_core::vfi::Isa::from_family(self.arch);
        let report = labwired_core::vfi::run_lockstep(
            &mut Side(&mut golden),
            &mut Side(&mut faulted),
            &plan,
            isa,
        )?;
        serde_json::to_string(&report).map_err(|e| e.to_string())
    }

    pub(crate) fn enable_coverage_s(&mut self) -> Result<(), String> {
        if self.tools.borrow().coverage.is_some() {
            return Ok(());
        }
        let obs = Arc::new(PcCoverageObserver::new());
        self.jit_browser_enabled = false;
        self.jit_browser_cache = None;
        self.machine
            .as_mut()
            .ok_or("simulator has no machine")?
            .add_observer(obs.clone());
        self.tools.borrow_mut().coverage = Some(obs);
        Ok(())
    }

    /// Firmware coverage as JSON. With live recording on
    /// ([`Self::enable_coverage_s`]) it is what ran since recording started
    /// (`"method": "live"`). Otherwise it is measured from power-on: a fresh
    /// copy replays every recorded call with an observer attached from the
    /// first instruction (`"method": "replay"`), which is exact because the
    /// simulator is deterministic, and costs re-simulating the run.
    pub(crate) fn coverage_report_s(&self) -> Result<String, String> {
        let live = self.tools.borrow().coverage.clone();
        let (obs, method, cycles) = match live {
            Some(obs) => (obs, "live", self.machine_s()?.total_cycles),
            None => {
                if let Some(why) = self.snapshot_refusal() {
                    return Err(format!("cannot measure coverage by replay: {why}"));
                }
                let obs = Arc::new(PcCoverageObserver::new());
                let ops = self.current_mark();
                let fresh = self.rebuild_s(ops, Some(obs.clone()))?;
                let cycles = fresh.machine_s()?.total_cycles;
                if cycles != self.machine_s()?.total_cycles {
                    return Err(format!(
                        "coverage replay reached cycle {cycles}, the machine is at cycle {}; \
                         something changed it that the journal does not record",
                        self.machine_s()?.total_cycles
                    ));
                }
                (obs, "replay", cycles)
            }
        };
        let elf = self.firmware_elf().ok_or(
            "coverage needs the firmware ELF with debug info; this machine was built from flash \
             images only",
        )?;
        let symbols = labwired_loader::SymbolProvider::from_bytes(elf)
            .map_err(|e| format!("firmware symbols: {e:#}"))?;
        let report = labwired_loader::coverage::CoverageReport::from_run(&symbols, &obs);
        let mut value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
        value["lcov"] = serde_json::Value::String(report.to_lcov());
        value["statement_percent"] = serde_json::json!(report.statement_percent());
        value["function_percent"] = serde_json::json!(report.function_percent());
        value["branch_percent"] = serde_json::json!(report.branch_percent());
        value["method"] = serde_json::json!(method);
        value["cycles"] = serde_json::json!(cycles);
        Ok(value.to_string())
    }
}

#[wasm_bindgen]
impl WasmSimulator {
    /// `null` when this simulator can be snapshotted, else the reason it
    /// cannot (built by the legacy constructor, or wired to another MCU).
    #[wasm_bindgen]
    pub fn snapshot_unavailable_reason(&self) -> Result<Option<String>, JsValue> {
        Ok(self.snapshot_refusal())
    }

    /// Save the current point. Returns JSON `{"id","label","cycles"}`.
    #[wasm_bindgen]
    pub fn snapshot_save(&mut self, label: Option<String>) -> Result<String, JsValue> {
        self.snapshot_save_s(label).map_err(js)
    }

    /// Saved points, oldest first, as JSON `[{"id","label","cycles"}]`.
    #[wasm_bindgen]
    pub fn snapshot_list(&self) -> Result<String, JsValue> {
        let tools = self.tools.borrow();
        Ok(serde_json::Value::Array(
            tools
                .points
                .iter()
                .map(|p| serde_json::json!({"id": p.id, "label": p.label, "cycles": p.cycles}))
                .collect(),
        )
        .to_string())
    }

    /// Return the machine to saved point `id`: a fresh machine is built from
    /// the same inputs and the recorded calls are replayed up to that point.
    /// Console output already shown is not shown again. Refused, with the
    /// machine untouched, when the replay does not reproduce the saved state.
    #[wasm_bindgen]
    pub fn snapshot_restore(&mut self, id: u32) -> Result<String, JsValue> {
        self.snapshot_restore_s(id).map_err(js)
    }

    /// Run a lockstep fault experiment and return the report as JSON (see
    /// `labwired_core::vfi::FaultReport`). `plan_json` is
    /// `{"until_cycle": N, "faults": [{"at_cycle": C, "kind": ..., ...}]}`.
    ///
    /// Two fresh copies of this machine are built and brought to the same
    /// point: the current one, or the start of the run when `from_boot` is
    /// true. This simulator is not changed.
    #[wasm_bindgen]
    pub fn fault_experiment(&self, plan_json: &str, from_boot: bool) -> Result<String, JsValue> {
        self.fault_experiment_s(plan_json, from_boot).map_err(js)
    }

    /// Start recording which instructions execute. The browser JIT is turned
    /// off while recording: compiled blocks do not report each instruction.
    #[wasm_bindgen]
    pub fn enable_coverage(&mut self) -> Result<(), JsValue> {
        self.enable_coverage_s().map_err(js)
    }

    #[wasm_bindgen]
    pub fn coverage_enabled(&self) -> Result<bool, JsValue> {
        Ok(self.tools.borrow().coverage.is_some())
    }

    /// The firmware coverage report as JSON: per-file lines, per-function
    /// summary, branches, percentages, the LCOV text under `lcov`, and
    /// `method`: `"live"` (recording since `enable_coverage`) or `"replay"`
    /// (measured from power-on by replaying the run on a fresh copy).
    #[wasm_bindgen]
    pub fn coverage_report(&self) -> Result<String, JsValue> {
        self.coverage_report_s().map_err(js)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    //! The browser entry points, driven natively on the committed nRF54L15
    //! smart-ring probe (a Cortex-M33 image with DWARF that prints four I²C
    //! WHO_AM_I answers and "probe done" inside 20 000 cycles).

    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn ring() -> WasmSimulator {
        let sys = root().join("examples/nrf54l15-smart-ring/system.yaml");
        let system_yaml = std::fs::read_to_string(&sys).unwrap();
        let manifest: labwired_config::SystemManifest = serde_yaml::from_str(&system_yaml).unwrap();
        let chip_yaml =
            std::fs::read_to_string(sys.parent().unwrap().join(&manifest.chip)).unwrap();
        let firmware =
            std::fs::read(root().join("tests/fixtures/nrf54l15-smart-ring.elf")).unwrap();
        WasmSimulator::new_from_config_parts(Rc::new(CtorInputs {
            system_yaml,
            chip_yaml,
            firmware,
            blobs: HashMap::new(),
        }))
        .unwrap()
    }

    /// Everything a user can observe: cycles, registers, RAM (the digest) and
    /// the console printed since the last drain.
    fn trace(sim: &WasmSimulator) -> (u64, u64, Vec<u8>) {
        (
            sim.machine_s().unwrap().total_cycles,
            sim.digest_s().unwrap(),
            sim.drain_uart_output(),
        )
    }

    #[test]
    fn restore_then_run_matches_a_straight_run() {
        let mut straight = ring();
        straight.set_peripheral_tick_interval(1);
        straight.step_batch(1_000).unwrap();
        let _ = straight.drain_uart_output();
        straight.step_batch(9_000).unwrap();
        let expected = trace(&straight);
        assert!(
            String::from_utf8_lossy(&expected.2).contains("OK"),
            "the probe prints its answers in this window"
        );

        let mut sim = ring();
        sim.set_peripheral_tick_interval(1);
        sim.step_batch(1_000).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&sim.snapshot_save_s(Some("after boot".into())).unwrap()).unwrap();
        let _ = sim.drain_uart_output();
        sim.step_batch(12_345).unwrap();
        sim.feed_uart_input(b"noise");
        let restored: serde_json::Value = serde_json::from_str(
            &sim.snapshot_restore_s(saved["id"].as_u64().unwrap() as u32)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(restored["cycles"], saved["cycles"]);
        assert!(
            sim.drain_uart_output().is_empty(),
            "output before the point is not re-shown"
        );
        sim.step_batch(9_000).unwrap();
        assert_eq!(trace(&sim), expected, "restore then run == straight run");

        // The saved list survives the restore and a second restore works.
        let list: serde_json::Value = serde_json::from_str(&sim.snapshot_list().unwrap()).unwrap();
        assert_eq!(list[0]["label"], "after boot");
        sim.snapshot_restore_s(saved["id"].as_u64().unwrap() as u32)
            .unwrap();
        assert_eq!(sim.machine_s().unwrap().total_cycles, 1_000);
    }

    #[test]
    fn identical_steps_share_one_entry_and_a_mid_run_snapshot_still_restores() {
        let mut sim = ring();
        for _ in 0..50 {
            sim.step_batch(100).unwrap();
        }
        let saved: serde_json::Value =
            serde_json::from_str(&sim.snapshot_save_s(None).unwrap()).unwrap();
        for _ in 0..70 {
            sim.step_batch(100).unwrap();
        }
        // 120 identical calls, one journal entry (the constructor recorded none).
        assert_eq!(sim.tools.borrow().journal.len(), 1);
        assert_eq!(sim.tools.borrow().journal[0].1, 120);
        sim.snapshot_restore_s(saved["id"].as_u64().unwrap() as u32)
            .unwrap();
        assert_eq!(sim.machine_s().unwrap().total_cycles, 5_000);
    }

    #[test]
    fn a_change_the_journal_cannot_see_refuses_the_restore() {
        let mut sim = ring();
        sim.step_batch(1_000).unwrap();
        // Poke RAM behind the journal's back, as an unrecorded call would.
        let ram = &mut sim.machine.as_mut().unwrap().bus.ram.data;
        ram[64] ^= 0xFF;
        let saved: serde_json::Value =
            serde_json::from_str(&sim.snapshot_save_s(None).unwrap()).unwrap();
        sim.step_batch(500).unwrap();
        let before = sim.digest_s().unwrap();
        let err = sim
            .snapshot_restore_s(saved["id"].as_u64().unwrap() as u32)
            .unwrap_err();
        assert!(err.contains("restore refused"), "{err}");
        assert_eq!(sim.digest_s().unwrap(), before, "the machine is untouched");
    }

    #[test]
    fn a_wired_simulator_refuses_to_snapshot() {
        let mut sim = ring();
        assert!(sim.snapshot_refusal().is_none());
        sim.untracked("wired to another MCU over a UART link");
        let reason = sim.snapshot_refusal().unwrap();
        assert!(reason.contains("UART link"), "{reason}");
        assert!(sim.snapshot_save_s(None).is_err());
        assert!(sim
            .fault_experiment_s(r#"{"until_cycle":10,"faults":[]}"#, true)
            .is_err());
    }

    #[test]
    fn fault_experiment_matches_the_core_engine_and_repeats() {
        let mut sim = ring();
        sim.step_batch(200).unwrap();
        let plan = r#"{"until_cycle": 20000, "faults": [
            {"at_cycle": 500, "kind": "register_bit_flip", "register": "R0", "bit": 3}]}"#;
        let a = sim.fault_experiment_s(plan, true).unwrap();
        let b = sim.fault_experiment_s(plan, true).unwrap();
        assert_eq!(a, b, "same inputs, same report");
        let r: serde_json::Value = serde_json::from_str(&a).unwrap();
        assert_eq!(r["verdict"], "output_changed");
        assert_eq!(r["start_cycle"], 0, "from_boot starts at power-on");
        assert_eq!(r["first_divergence"]["registers"][0]["register"], "R0");

        // From the current point instead.
        let r: serde_json::Value =
            serde_json::from_str(&sim.fault_experiment_s(plan, false).unwrap()).unwrap();
        assert_eq!(r["start_cycle"], sim.machine_s().unwrap().total_cycles);

        let pc = r#"{"until_cycle": 20000, "faults": [
            {"at_cycle": 500, "kind": "register_bit_flip", "register": "pc", "bit": 28}]}"#;
        let r: serde_json::Value =
            serde_json::from_str(&sim.fault_experiment_s(pc, true).unwrap()).unwrap();
        assert_eq!(r["verdict"], "crashed");
        assert_eq!(
            sim.machine_s().unwrap().total_cycles,
            200,
            "this sim is unchanged"
        );
    }

    #[test]
    fn coverage_by_replay_covers_the_run_from_power_on() {
        // No recording: the report replays the run on a fresh copy with an
        // observer from the first instruction, and matches live recording.
        let mut replayed = ring();
        replayed.step_batch(20_000).unwrap();
        let r: serde_json::Value =
            serde_json::from_str(&replayed.coverage_report_s().unwrap()).unwrap();
        assert_eq!(r["method"], "replay");
        assert_eq!(r["cycles"], replayed.machine_s().unwrap().total_cycles);

        let mut live = ring();
        live.enable_coverage_s().unwrap();
        live.step_batch(20_000).unwrap();
        let l: serde_json::Value =
            serde_json::from_str(&live.coverage_report_s().unwrap()).unwrap();
        assert_eq!(l["method"], "live");
        assert_eq!(r["files"], l["files"], "replay == live recording from boot");
        assert_eq!(r["functions"], l["functions"]);
    }

    #[test]
    fn coverage_maps_the_run_to_lines_and_functions() {
        let mut sim = ring();
        sim.enable_coverage_s().unwrap();
        sim.step_batch(20_000).unwrap();
        let r: serde_json::Value = serde_json::from_str(&sim.coverage_report_s().unwrap()).unwrap();
        let func = |name: &str| {
            r["functions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["name"] == name)
                .unwrap()
                .clone()
        };
        assert_eq!(func("main")["entered"], true);
        assert_eq!(func("HardFault_Handler")["entered"], false);
        let pct = r["statement_percent"].as_f64().unwrap();
        assert!(pct > 50.0 && pct < 100.0, "{pct}");
        assert!(r["lcov"].as_str().unwrap().contains("FNDA:1,main\n"));

        // Coverage survives a restore and keeps accumulating.
        let mut sim = ring();
        sim.enable_coverage_s().unwrap();
        let id: serde_json::Value =
            serde_json::from_str(&sim.snapshot_save_s(None).unwrap()).unwrap();
        sim.step_batch(20_000).unwrap();
        sim.snapshot_restore_s(id["id"].as_u64().unwrap() as u32)
            .unwrap();
        assert!(sim.coverage_enabled().unwrap());
        let again: serde_json::Value =
            serde_json::from_str(&sim.coverage_report_s().unwrap()).unwrap();
        assert_eq!(again["files"], r["files"]);
    }
}
