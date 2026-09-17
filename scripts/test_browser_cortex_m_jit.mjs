#!/usr/bin/env node
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end gate for the opt-in browser Cortex-M wasm JIT.
//
// `crates/wasm/tests/motor_states.rs` boots the same firmware through the
// native binding surface, so the interpreter side of this board is already
// covered. What was not covered anywhere: the `set_jit_enabled(true)` +
// `step_batch` path a browser actually takes. This script drives that path in
// Node (the wasm-bindgen `--target nodejs` package is the same .wasm the
// browser loads, including `WebAssembly.Module`/`Instance` for every compiled
// Thumb block), twice per run:
//
//   1. JIT off — the interpreter reference.
//   2. JIT on  — must retire the SAME executed cycles, land on the same PC,
//      emit the same UART bytes and the same motor-plant state.
//
// and then asserts non-vacuity: the JIT-on run must have COMPILED a block and
// RUN one. Without that, a run where the emitter refuses everything would be
// byte-identical to the interpreter and this gate would silently prove
// nothing. If compilation cannot happen (e.g. `WebAssembly` missing), this
// FAILS — there is no skip path.
//
// Usage (after `wasm-pack build --target nodejs --dev` from `crates/wasm/`):
//
//   node scripts/test_browser_cortex_m_jit.mjs
//
// `browser-layer` in .github/workflows/core-ci.yml runs this with the dev
// profile: debug_assertions stay on, which is where the JIT's cycle-account
// assertions (`step_batch_cortex_m_jit`'s `debug_assert_eq!`) live.

import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');

const pkgPath = resolve(repoRoot, 'crates/wasm/pkg/labwired_wasm.js');
if (!existsSync(pkgPath)) {
  console.error(
    `[cortex-m-jit] ${pkgPath} missing — run \`wasm-pack build --target nodejs --dev\` from crates/wasm/ first.`,
  );
  process.exit(1);
}

const { WasmSimulator } = await import(pkgPath);

// The board and firmware the crate's own `motor_states.rs` test uses. The ELF
// is committed under crates/wasm/tests/fixtures/, so this gate needs no
// firmware build and no new board.
const chipYaml = readFileSync(resolve(repoRoot, 'configs/chips/stm32l476.yaml'), 'utf8');
const systemYaml = readFileSync(resolve(repoRoot, 'examples/nucleo-l476rg-bldc/system.yaml'), 'utf8');
const elfBytes = readFileSync(resolve(repoRoot, 'crates/wasm/tests/fixtures/firmware-l476-bldc-six-step.elf'));

// Long enough that the six-step control loop has executed its hot blocks many
// times over, small enough to stay well inside the browser-layer job's wall.
const BATCH_CYCLES = 25_000;
const BATCHES = 20;

function boot(jitEnabled) {
  const sim = WasmSimulator.new_from_config(systemYaml, chipYaml, elfBytes, null);
  sim.set_jit_enabled(jitEnabled);
  // The browser bridge applies the recommended tick interval at init
  // (simulator-bridge.ts). It is load-bearing for this gate: at the default
  // interval of 1 the advance planner clamps every window to one instruction,
  // a compiled block (4+ instructions) can never fit in a window, and the JIT
  // would look engaged (blocks compiled) while never running one.
  const interval = sim.recommended_tick_interval();
  if (interval > 1) {
    sim.set_peripheral_tick_interval(interval);
  }

  const executed = [];
  const uart = [];
  for (let i = 0; i < BATCHES; i++) {
    executed.push(Number(sim.step_batch(BATCH_CYCLES)));
    uart.push(...sim.drain_uart_output());
  }

  return {
    sim,
    executed,
    uart: Buffer.from(uart),
    pc: sim.get_pc(),
    actuators: JSON.stringify(sim.get_actuator_states()),
    // `peek`, not `read_memory`: this is observation, not a bus access, so it
    // cannot fire read side effects that would make the two runs diverge.
    ram: Buffer.from(sim.peek(0x2000_0000, 128)),
    compiled: sim.jit_compiled_blocks(),
    hits: sim.jit_hits(),
    refusals: sim.jit_refusals(),
  };
}

const failures = [];
function check(ok, message) {
  if (!ok) {
    failures.push(message);
  }
}

console.log(`[cortex-m-jit] L476 six-step, ${BATCHES} × ${BATCH_CYCLES} cycles per variant`);

const off = boot(false);
const on = boot(true);

check(
  off.executed.length === on.executed.length,
  `batch count differs: off=${off.executed.length} on=${on.executed.length}`,
);
const batches = Math.min(off.executed.length, on.executed.length);
for (let i = 0; i < batches; i++) {
  check(
    off.executed[i] === on.executed[i],
    `batch ${i}: executed cycles differ: interpreter=${off.executed[i]} JIT=${on.executed[i]}`,
  );
}
const offTotal = off.executed.reduce((a, b) => a + b, 0);
const onTotal = on.executed.reduce((a, b) => a + b, 0);
check(offTotal === onTotal, `total executed cycles differ: interpreter=${offTotal} JIT=${onTotal}`);
check(off.pc === on.pc, `final PC differs: interpreter=0x${off.pc.toString(16)} JIT=0x${on.pc.toString(16)}`);
check(
  off.uart.equals(on.uart),
  `UART bytes differ (${off.uart.length} vs ${on.uart.length} bytes)`,
);
check(
  off.actuators === on.actuators,
  `motor plant state differs:\n  interpreter: ${off.actuators}\n  JIT:         ${on.actuators}`,
);
check(off.ram.equals(on.ram), `RAM at 0x20000000 differs after ${offTotal} cycles`);

// The interpreter run must not have touched the browser cache at all — that is
// what makes "JIT on" the only variable between the two runs above.
check(off.compiled === 0n, `JIT-off run compiled ${off.compiled} blocks`);
check(off.hits === 0n, `JIT-off run had ${off.hits} JIT hits`);

// Anti-vacuity. A cache that compiled nothing has zero hits too, so both are
// required: without `compiled > 0` this gate would pass on a build where the
// browser JIT is dead code that always falls back to the interpreter.
check(
  on.compiled > 0n,
  `browser JIT compiled no blocks in ${onTotal} cycles — the Cortex-M emitter never engaged (refusals=${on.refusals})`,
);
check(
  on.hits > 0n,
  `browser JIT compiled ${on.compiled} block(s) but never ran one — every dispatch fell back (refusals=${on.refusals})`,
);

console.log(
  `[cortex-m-jit] interpreter: ${offTotal} cycles, pc=0x${off.pc.toString(16)}, uart=${off.uart.length}B`,
);
console.log(
  `[cortex-m-jit] JIT:         ${onTotal} cycles, pc=0x${on.pc.toString(16)}, uart=${on.uart.length}B, ` +
    `compiled=${on.compiled}, hits=${on.hits}, refusals=${on.refusals}`,
);

if (failures.length > 0) {
  console.error(`[cortex-m-jit] FAIL (${failures.length}):`);
  for (const failure of failures) {
    console.error(`  - ${failure}`);
  }
  process.exit(1);
}

console.log('[cortex-m-jit] PASS — JIT and interpreter observably equivalent, compiled blocks > 0, block runs > 0');
