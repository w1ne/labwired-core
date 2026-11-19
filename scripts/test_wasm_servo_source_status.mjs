#!/usr/bin/env node
// Verify modeled source-status serialization through a real WASM package.
// This is a debugger-driven model test, not evidence of a physical signal.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const artifact = resolve(process.argv[2] ?? 'crates/wasm/pkg');
const module = await import(pathToFileURL(resolve(artifact, 'labwired_wasm.js')).href);
const WasmSimulator = module.WasmSimulator ?? module.default?.WasmSimulator;
if (module.initSync) module.initSync({ module: readFileSync(resolve(artifact, 'labwired_wasm_bg.wasm')) });
assert.ok(WasmSimulator, 'generated WASM package must export WasmSimulator');
const read = path => readFileSync(resolve(root, path));
const system = `name: status-serialization
chip: inline
cpu_hz: 160000000
external_devices:
  - id: pca
    type: pca9685
    connection: i2c0
    route: { sda: GPIO8, scl: GPIO9 }
    config: { i2c_address: 0x40 }
  - id: servo1
    type: servo
    connection: i2c0
    config: { source_address: 64, source_channel: 0, source_oe_pin: GPIO4 }
`;
const chip = read('configs/chips/esp32c3.yaml').toString();
const blobs = {
  esp32c3_irom: read('crates/core/roms/esp32c3/esp32c3_rom.bin'),
  esp32c3_drom: read('crates/core/roms/esp32c3/esp32c3_drom.bin'),
  esp32c3_flash: read('crates/wasm/tests/fixtures/esp32c3-hello-world-flash.bin'),
  labwired_esp32c3_flash_fast_start: new Uint8Array([1]),
};
const sim = WasmSimulator.new_from_config(system, chip, new Uint8Array(), blobs);
try {
  const sample = () => {
    const state = sim.get_actuator_states().find(entry => entry.id === 'servo1');
    assert.ok(state, 'servo must be visible');
    return state;
  };
  const write = (offset, value) => {
    const bytes = new Uint8Array(4);
    new DataView(bytes.buffer).setUint32(0, value, true);
    sim.write_memory(0x60004000 + offset, bytes);
  };
  assert.equal(sample().source_status, 'unknown');
  assert.equal(sample().active, false);
  write(0x08, 1 << 4); // OUT_W1TS
  write(0x24, 1 << 4); // ENABLE_W1TS
  assert.equal(sample().source_status, 'disabled');
  assert.equal(sample().active, false);
  write(0x0c, 1 << 4); // OUT_W1TC: no programmed PCA pulse
  assert.equal(sample().source_status, 'unknown');
  assert.equal(sample().active, false);
  console.log('servo source status WASM serialization: unknown → disabled → unknown');
} finally {
  sim.free();
}
