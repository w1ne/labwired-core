#!/usr/bin/env node
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// #124 Phase 4 browser-side JIT benchmark harness.
//
// Loads the labwired-ereader Arduino-ESP32 ELF through the same path the
// playground uses, then runs `step_with_esp32_aids` twice: once with the
// browser JIT off (baseline) and once with it on. Reports elapsed time +
// cyc/sec for each variant + speedup percentage.
//
// Usage (after `wasm-pack build --target nodejs --release` from
// `crates/wasm/`):
//
//   node scripts/bench_browser_jit.mjs [cycles] [warmup]
//
// Defaults: cycles=10000000, warmup=200000. Both are integers. The
// harness expects the labwired-ereader ELF at
// `/tmp/labwired-ereader/build/labwired-ereader.ino.elf` (override via
// `LABWIRED_EREADER_ELF`).

import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');

const pkgPath = resolve(repoRoot, 'crates/wasm/pkg/labwired_wasm.js');
if (!existsSync(pkgPath)) {
  console.error(`[bench] ${pkgPath} missing — run \`wasm-pack build --target nodejs --release\` from crates/wasm/ first.`);
  process.exit(1);
}
const wasmModule = await import(pkgPath);
const { WasmSimulator } = wasmModule;

const elfPath = process.env.LABWIRED_EREADER_ELF
  ?? '/tmp/labwired-ereader/build/labwired-ereader.ino.elf';
if (!existsSync(elfPath)) {
  console.error(`[bench] ELF not found at ${elfPath}. Build labwired-ereader or set LABWIRED_EREADER_ELF.`);
  process.exit(1);
}
const elfBytes = readFileSync(elfPath);

const sysYaml = readFileSync(resolve(repoRoot, 'configs/systems/esp32-wroom-epaper.yaml'), 'utf8');
// `chip:` ref in the system YAML is relative to the YAML's location; the
// playground resolves it client-side. Mimic that by inlining the chip YAML.
const chipYaml = readFileSync(resolve(repoRoot, 'configs/chips/esp32.yaml'), 'utf8');

const cycles = parseInt(process.argv[2] ?? '10000000', 10);
const warmup = parseInt(process.argv[3] ?? '200000', 10);

function makeSim() {
  const sim = WasmSimulator.new_from_config(sysYaml, chipYaml, elfBytes);
  sim.install_arduino_esp32_quirks(elfBytes);
  return sim;
}

function fmt(n) {
  return n.toLocaleString('en-US', { maximumFractionDigits: 0 });
}

function bench(name, jitOn) {
  const sim = makeSim();
  sim.set_jit_enabled(jitOn);
  // Warmup gets the firmware past BROM and into the hot block.
  sim.step_with_esp32_aids(warmup);
  // Measured run.
  const t = sim.bench_jit(cycles);
  const cps = (cycles / (t / 1000));
  const hits = jitOn ? sim.jit_hits() : 0n;
  const refusals = jitOn ? sim.jit_refusals() : 0n;
  console.log(`  ${name.padEnd(20)} ${t.toFixed(1).padStart(8)} ms  ${fmt(cps).padStart(14)} cyc/s   hits=${hits}  refusals=${refusals}`);
  return { t, cps };
}

console.log(`bench: ereader, ${fmt(cycles)} cycles + ${fmt(warmup)} warmup, ELF=${elfPath}`);

console.log('\nRun 1:');
const a0 = bench('baseline (JIT off)', false);
const a1 = bench('with browser JIT',  true);

console.log('\nRun 2 (for variance):');
const b0 = bench('baseline (JIT off)', false);
const b1 = bench('with browser JIT',  true);

const baseMs = (a0.t + b0.t) / 2;
const jitMs  = (a1.t + b1.t) / 2;
const speedup = ((baseMs - jitMs) / baseMs) * 100;

console.log(`\nMean over 2 runs:`);
console.log(`  baseline:   ${baseMs.toFixed(0)} ms  (${fmt(cycles / (baseMs / 1000))} cyc/s)`);
console.log(`  with JIT:   ${jitMs.toFixed(0)} ms  (${fmt(cycles / (jitMs / 1000))} cyc/s)`);
console.log(`  speedup:    ${speedup.toFixed(2)}%`);
