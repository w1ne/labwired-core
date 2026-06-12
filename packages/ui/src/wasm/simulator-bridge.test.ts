import { describe, expect, it } from 'vitest';
import { SimulatorBridge } from './simulator-bridge';

// serde_wasm_bindgen serializes `serde_json::Value::Object` as a JS `Map`, so
// the wasm methods that build their result with `serde_json::json!{…}`
// (`get_board_io_states`, `get_board_io_analog_states`) hand back arrays of
// Maps. Consumers read `entry.id` / `entry.active`, which is `undefined` on a
// Map — that's what left the STM32 blinky LED stuck OFF. The bridge normalizes
// those entries to plain objects; these tests lock that in.
function bridgeWith(sim: Record<string, unknown>): SimulatorBridge {
  // The constructor is private by design (factories own wasm init); reach it
  // directly with a fake `sim` so we can exercise the normalization in
  // isolation, no wasm load required.
  return new (SimulatorBridge as unknown as { new (sim: unknown): SimulatorBridge })(sim);
}

describe('SimulatorBridge board-IO normalization', () => {
  it('converts Map entries from get_board_io_states into plain objects', () => {
    const bridge = bridgeWith({
      get_board_io_states: () => [
        new Map<string, unknown>([
          ['id', 'led_pa5'],
          ['active', true],
        ]),
      ],
    });

    const states = bridge.getBoardIoStates();
    expect(states).toEqual([{ id: 'led_pa5', active: true }]);
    // Property access (what the playground does) must work, not yield undefined.
    expect(states[0].id).toBe('led_pa5');
    expect(states[0].active).toBe(true);
  });

  it('passes through plain-object entries unchanged', () => {
    const bridge = bridgeWith({
      get_board_io_states: () => [{ id: 'led_pa5', active: false }],
    });
    expect(bridge.getBoardIoStates()).toEqual([{ id: 'led_pa5', active: false }]);
  });

  it('treats a null/empty result as an empty array', () => {
    expect(bridgeWith({ get_board_io_states: () => null }).getBoardIoStates()).toEqual([]);
  });

  it('normalizes Map entries from get_board_io_analog_states too', () => {
    const bridge = bridgeWith({
      get_board_io_analog_states: () => [
        new Map<string, unknown>([
          ['id', 'ntc'],
          ['kind', 'adc_input'],
          ['value', 2048],
        ]),
      ],
    });
    const states = bridge.getAnalogStates();
    expect(states).toEqual([{ id: 'ntc', kind: 'adc_input', value: 2048 }]);
    expect(states[0].value).toBe(2048);
  });
});

describe('SimulatorBridge cycle accounting', () => {
  it('does not produce NaN cycles when the ESP32 aided step returns no count', () => {
    // The ESP32 dual-core aided step path can return undefined while still
    // advancing the batch; accumulating that raw turned totalCycles into NaN.
    const bridge = bridgeWith({
      install_arduino_esp32_quirks: () => {},
      step_with_esp32_aids: () => undefined,
      step_batch: () => 1000,
      get_pc: () => 0x400d0000,
    });
    bridge.installArduinoEsp32QuirksAutodiscover(new Uint8Array([0]));
    const executed = bridge.stepBatch(500_000);
    expect(Number.isFinite(executed)).toBe(true);
    expect(executed).toBe(500_000); // falls back to the requested batch size
    expect(Number.isFinite(bridge.totalCycles)).toBe(true);
    expect(bridge.totalCycles).toBe(500_000);
  });

  it('uses the real count when the step returns one (STM32 path)', () => {
    const bridge = bridgeWith({ step_batch: () => 250_000, get_pc: () => 0 });
    bridge.stepBatch(500_000);
    expect(bridge.totalCycles).toBe(250_000);
  });
});

describe('SimulatorBridge FDCAN trace normalization', () => {
  it('converts FDCAN trace entries into plain objects', () => {
    const bridge = bridgeWith({
      fdcan_trace_snapshot: () => [
        new Map<string, unknown>([
          ['seq', 1],
          ['peripheral', 'fdcan1'],
          ['direction', 'tx'],
          ['id', 0x7e0],
          ['data', [0x03, 0x22, 0xf1, 0x90]],
          ['extended', false],
          ['fd', false],
          ['bitrate_switch', false],
          ['remote', false],
        ]),
      ],
    });

    const frames = bridge.fdcanTraceSnapshot();
    expect(frames).toEqual([
      {
        seq: 1,
        peripheral: 'fdcan1',
        direction: 'tx',
        id: 0x7e0,
        data: [0x03, 0x22, 0xf1, 0x90],
        extended: false,
        fd: false,
        bitrate_switch: false,
        remote: false,
      },
    ]);
    expect(frames[0].data[1]).toBe(0x22);
  });

  it('returns an empty trace when the local wasm bundle is older than the bridge API', () => {
    expect(bridgeWith({}).fdcanTraceSnapshot()).toEqual([]);
  });
});
