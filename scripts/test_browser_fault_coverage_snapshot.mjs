#!/usr/bin/env node
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end gate for the browser snapshot/restore, fault-experiment and
// coverage entry points, through the real wasm artifact (the wasm-bindgen
// `--target nodejs` package is the same .wasm the playground loads).
//
// Firmware: the committed nRF54L15 smart-ring probe (Cortex-M33, DWARF), which
// prints four I2C WHO_AM_I answers and "probe done" inside 20 000 cycles.
//
//   1. restore then run == straight run (cycles, PC, registers, console);
//   2. fault_experiment gives the lockstep verdict, identically twice;
//   3. coverage maps the run to functions and emits LCOV.
//
// Usage (after `wasm-pack build --target nodejs --dev --out-dir pkg` in
// crates/wasm/):  node scripts/test_browser_fault_coverage_snapshot.mjs

import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import assert from 'node:assert/strict';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');
const pkgPath = resolve(repoRoot, 'crates/wasm/pkg/labwired_wasm.js');
if (!existsSync(pkgPath)) {
  console.error(`[lab-tools] ${pkgPath} missing — run wasm-pack build --target nodejs --dev --out-dir pkg in crates/wasm first.`);
  process.exit(1);
}
const { WasmSimulator } = await import(pkgPath);

const systemYaml = readFileSync(resolve(repoRoot, 'examples/nrf54l15-smart-ring/system.yaml'), 'utf8');
const chipYaml = readFileSync(resolve(repoRoot, 'configs/chips/nrf54l15.yaml'), 'utf8');
const elf = readFileSync(resolve(repoRoot, 'tests/fixtures/nrf54l15-smart-ring.elf'));
const open = () => WasmSimulator.new_from_config(systemYaml, chipYaml, new Uint8Array(elf), {});
const decoder = new TextDecoder();

function observe(sim) {
  const regs = [];
  for (let i = 0; i < 16; i++) regs.push(sim.get_register(i));
  // snapshot_save doubles as a cycle reading.
  const cycles = JSON.parse(sim.snapshot_save('probe')).cycles;
  return { cycles, pc: sim.get_pc(), regs, console: decoder.decode(sim.drain_uart_output()) };
}

// 1. restore then run == straight run.
const straight = open();
straight.set_peripheral_tick_interval(1);
straight.step_batch(1_000);
straight.drain_uart_output();
straight.step_batch(9_000);
const expected = observe(straight);
assert.match(expected.console, /\[OK\]/, 'the probe prints its answers in this window');

const sim = open();
assert.equal(sim.snapshot_unavailable_reason(), undefined);
sim.set_peripheral_tick_interval(1);
sim.step_batch(1_000);
const saved = JSON.parse(sim.snapshot_save('after boot'));
sim.drain_uart_output();
sim.step_batch(12_345);
sim.feed_uart_input(new TextEncoder().encode('noise'));
const restored = JSON.parse(sim.snapshot_restore(saved.id));
assert.equal(restored.cycles, saved.cycles);
assert.equal(sim.drain_uart_output().length, 0, 'output before the point is not shown twice');
sim.step_batch(9_000);
assert.deepEqual(observe(sim), expected, 'restore then run must equal a straight run');
console.log(`[lab-tools] snapshot: restored to cycle ${saved.cycles}, then matched a straight run at cycle ${expected.cycles}`);

// 2. fault experiment.
const plan = JSON.stringify({
  until_cycle: 20_000,
  faults: [{ at_cycle: 500, kind: 'register_bit_flip', register: 'R0', bit: 3 }],
});
const a = sim.fault_experiment(plan, true);
const b = sim.fault_experiment(plan, true);
assert.equal(a, b, 'same inputs, same report');
const report = JSON.parse(a);
assert.equal(report.verdict, 'output_changed');
assert.equal(report.first_divergence.registers[0].register, 'R0');
const crash = JSON.parse(sim.fault_experiment(JSON.stringify({
  until_cycle: 20_000,
  faults: [{ at_cycle: 500, kind: 'register_bit_flip', register: 'pc', bit: 28 }],
}), true));
assert.equal(crash.verdict, 'crashed');
assert.throws(() => sim.fault_experiment('{"until_cycle":10,"faults":[{"at_cycle":1,"kind":"bus_nack"}]}', true), /unknown variant/);
console.log(`[lab-tools] faults: R0 flip -> ${report.verdict}; PC flip -> ${crash.verdict}`);

// 3. coverage.
const cov = open();
cov.enable_coverage();
cov.step_batch(20_000);
const r = JSON.parse(cov.coverage_report());
const fn = (name) => r.functions.find((f) => f.name === name);
assert.equal(fn('main').entered, true);
assert.equal(fn('HardFault_Handler').entered, false);
assert.ok(r.statement_percent > 50 && r.statement_percent < 100, String(r.statement_percent));
assert.match(r.lcov, /FNDA:1,main\n/);
console.log(`[lab-tools] coverage: ${r.covered_statements}/${r.total_statements} lines, ${r.covered_functions}/${r.total_functions} functions`);
console.log('[lab-tools] OK');
