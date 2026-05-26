#!/usr/bin/env node
// Minimal verification that the browser JIT prototype loads + exposes
// its JS API correctly. Doesn't run firmware — just compiles the JIT
// hot block via `WebAssembly.Module` and verifies behaviour against the
// known-correct test vector from `bb_multi.rs::hot_bb_arithmetic_matches_interp`.
//
// Usage: node scripts/bench_browser_jit_minimal.mjs

import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const wasmPath = resolve(__dirname, '../crates/wasm/pkg/labwired_wasm.js');
if (!existsSync(wasmPath)) {
  console.error(`run \`wasm-pack build --target nodejs --release\` from crates/wasm/ first.`);
  process.exit(1);
}

// Find the baked hot-block wasm bytes — they live in target/.../out/...
import { execSync } from 'node:child_process';
const outDir = execSync('find target -path "*/build/labwired-core-*/out/xtensa_jit_hot_bb.wasm" -print -quit', {
  cwd: resolve(__dirname, '..'),
}).toString().trim();
if (!outDir) {
  console.error('xtensa_jit_hot_bb.wasm not found — run cargo build -p labwired-core first.');
  process.exit(1);
}
const fullPath = resolve(__dirname, '..', outDir);
const wasmBytes = readFileSync(fullPath);
console.log(`hot_bb.wasm: ${wasmBytes.length} bytes from ${outDir}`);
console.log(`magic: ${[...wasmBytes.slice(0, 4)].map(b => b.toString(16).padStart(2, '0')).join(' ')}`);

// Validate + compile + instantiate with a JS host import.
console.log(`\nWebAssembly.validate: ${WebAssembly.validate(wasmBytes)}`);

// Use a typed-array ring with an index pointer — Array.shift() is O(N)
// which would dominate the benchmark with N=1M.
let pending = new Uint8Array(0);
let pendingIdx = 0;
const importObj = {
  host: {
    read_u8: () => {
      if (pendingIdx >= pending.length) return -1;
      return pending[pendingIdx++];
    },
  },
};
function stage(bytes) {
  pending = Uint8Array.from(bytes);
  pendingIdx = 0;
}

const module = await WebAssembly.compile(wasmBytes);
const instance = await WebAssembly.instantiate(module, importObj);
const run = instance.exports.run;
console.log(`exports: ${Object.keys(instance.exports).join(', ')}`);

// Test vector from bb_multi.rs::hot_bb_arithmetic_matches_interp:
//   stage [0xAB, 0xCD], a3=0x3FFB_0000, a5=0x1234, l32r=0x40008534
//   expected exit=0, a10=0x1234, a6=0xAB, a2=0xCD&0xAB=0x89, a8=0x40008534
stage([0xAB, 0xCD]);
const ret = run(0x3FFB_0000 | 0, 0x1234, 0x40008534 | 0);
console.log(`run(...) returned: [${ret}]`);
const [exit, a2, a6, a8, a10] = ret;
const ok = exit === 0
  && (a10 >>> 0) === 0x1234
  && (a6 >>> 0)  === 0xAB
  && (a2 >>> 0)  === (0xCD & 0xAB)
  && (a8 >>> 0)  === 0x40008534;
console.log(`expected exit=0 a10=0x1234 a6=0xAB a2=0x${(0xCD & 0xAB).toString(16)} a8=0x40008534`);
console.log(`got      exit=${exit} a10=0x${(a10>>>0).toString(16)} a6=0x${(a6>>>0).toString(16)} a2=0x${(a2>>>0).toString(16)} a8=0x${(a8>>>0).toString(16)}`);
console.log(ok ? 'PASS browser JIT arithmetic matches interpreter' : 'FAIL');

// Bus-error path: stage 0 bytes; should return exit=5 + zeroed regs.
stage([]);
const ret2 = run(0x3FFB_0000 | 0, 0x1234, 0x40008534 | 0);
console.log(`\nbus-error run: [${ret2}]`);
console.log(ret2[0] === 5 ? 'PASS bus-error path returns exit=5' : 'FAIL bus-error path');

// Bench: tight loop calling `run` to estimate wasm-call overhead from
// JS. This bounds how fast the hot path can possibly be.
const N = 1_000_000;
// Re-stage every call (cheap with the indexed ring) so we exercise the
// realistic host-import path: stage two bytes per call, then call run.
pending = new Uint8Array([0xAA, 0xBB]);
const t0 = performance.now();
for (let i = 0; i < N; i++) {
  pendingIdx = 0;
  run(0x3FFB_0000 | 0, 0, 0x40008534 | 0);
}
const t1 = performance.now();
const dt = t1 - t0;
console.log(`\n${N.toLocaleString()} JIT calls: ${dt.toFixed(0)} ms = ${(dt * 1000 / N).toFixed(1)} ns/call`);
console.log(`(this is the per-call overhead the browser JIT adds vs interpreter dispatch)`);
