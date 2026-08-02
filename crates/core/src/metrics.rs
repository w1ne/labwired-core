// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimulationObserver;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug)]
pub struct PerformanceMetrics {
    instruction_count: AtomicU64,
    cycle_count: AtomicU64,
    peripheral_cycle_count: AtomicU64,
    peripheral_cycles_by_name: Mutex<HashMap<String, u64>>,
    start_time: Instant,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceMetrics {
    pub fn new() -> Self {
        Self {
            instruction_count: AtomicU64::new(0),
            cycle_count: AtomicU64::new(0),
            peripheral_cycle_count: AtomicU64::new(0),
            peripheral_cycles_by_name: Mutex::new(HashMap::new()),
            start_time: Instant::now(),
        }
    }

    pub fn reset(&self) {
        self.instruction_count.store(0, Ordering::SeqCst);
        self.cycle_count.store(0, Ordering::SeqCst);
        self.peripheral_cycle_count.store(0, Ordering::SeqCst);
        if let Ok(mut m) = self.peripheral_cycles_by_name.lock() {
            m.clear();
        }
    }

    pub fn get_instructions(&self) -> u64 {
        self.instruction_count.load(Ordering::SeqCst)
    }

    /// Overwrite the instruction counter from an authoritative external source.
    ///
    /// Used by the JIT-eligible CLI test path, which does NOT install this
    /// object as a per-step observer (the observer presence gates the RV32IMC
    /// JIT off). Instead it sources the retired-instruction count directly from
    /// the machine's own batch return values, so the count stays exact whether
    /// a batch was interpreted or compiled. See `execute_test_loop`.
    pub fn set_instructions(&self, instructions: u64) {
        self.instruction_count.store(instructions, Ordering::SeqCst);
    }

    /// Overwrite the cycle counter from an authoritative external source
    /// (`Machine::total_cycles`). Peer to [`Self::set_instructions`]: the
    /// JIT-eligible test path can't rely on the per-step `on_step_end` tap
    /// (compiled blocks retire without firing it), so it mirrors the machine's
    /// canonical cycle counter here before each stimulus/limit check.
    pub fn set_cycles(&self, cycles: u64) {
        self.cycle_count.store(cycles, Ordering::SeqCst);
    }

    pub fn get_cycles(&self) -> u64 {
        self.cycle_count.load(Ordering::SeqCst)
    }

    pub fn get_peripheral_cycles_total(&self) -> u64 {
        self.peripheral_cycle_count.load(Ordering::SeqCst)
    }

    pub fn get_peripheral_cycles(&self, name: &str) -> u64 {
        self.peripheral_cycles_by_name
            .lock()
            .ok()
            .and_then(|m| m.get(name).copied())
            .unwrap_or(0)
    }

    pub fn get_ips(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.get_instructions() as f64 / elapsed
        } else {
            0.0
        }
    }
}

impl SimulationObserver for PerformanceMetrics {
    fn on_simulation_start(&self) {
        // Reset counters on each start if needed, or just keep them cumulative
    }

    fn on_step_start(&self, _pc: u32, _opcode: u32) {
        self.instruction_count.fetch_add(1, Ordering::SeqCst);
    }

    fn on_step_end(&self, cycles: u32, _registers: &[u32]) {
        self.cycle_count.fetch_add(cycles as u64, Ordering::SeqCst);
    }

    fn on_peripheral_tick(&self, name: &str, cycles: u32) {
        if cycles == 0 {
            return;
        }
        self.cycle_count.fetch_add(cycles as u64, Ordering::SeqCst);
        self.peripheral_cycle_count
            .fetch_add(cycles as u64, Ordering::SeqCst);
        if let Ok(mut m) = self.peripheral_cycles_by_name.lock() {
            *m.entry(name.to_string()).or_insert(0) += cycles as u64;
        }
    }
}
