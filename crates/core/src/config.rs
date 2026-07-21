// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// Enable the instruction decode cache for the CPU core.
    pub decode_cache_enabled: bool,
    /// Interval in instructions for ticking peripherals (1 = every instruction).
    pub peripheral_tick_interval: u32,
    /// Enable optimized multi-byte memory access paths in the SystemBus.
    pub optimized_bus_access: bool,
    /// Enable high-throughput batch execution for the CPU.
    pub batch_mode_enabled: bool,
    /// Enable scheduler-safe CPU idle fast-forwarding.
    ///
    /// Off by default so existing runners keep instruction-for-instruction
    /// behavior unless they explicitly opt in.
    pub idle_fast_forward_enabled: bool,

    /// Opt into the RISC-V (RV32IMC) wasm-JIT fast path for `Machine<RiscV>`
    /// (chunk H). Off by default: with it `false` the interpreter runs every
    /// instruction and behavior is bit-identical to a build without the
    /// `jit` feature. When `true` *and* the `jit` feature is compiled in
    /// *and* the correctness SafetyGate allows (no observers / breakpoints /
    /// logic probes / cycle-accurate peripheral), hot basic blocks are
    /// compiled to wasm and retired atomically; the interpreter remains the
    /// oracle for everything the JIT does not model. Has no effect without
    /// the `jit` feature.
    #[serde(default)]
    pub riscv_jit_enabled: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            decode_cache_enabled: true,
            peripheral_tick_interval: 1, // Correctness by default
            optimized_bus_access: true,
            batch_mode_enabled: true,
            idle_fast_forward_enabled: false,
            riscv_jit_enabled: false,
        }
    }
}
