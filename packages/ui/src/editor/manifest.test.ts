import { describe, it, expect } from 'vitest';
import { canRenderInEditor, manifestToEditorState } from './manifest';
import { COMPONENT_REGISTRY } from './components';

// A read-only "schematic manifest" for the thermal-fingerprint IO-Link sensor:
// ESP32-C3 SuperMini + MLX90640 thermal camera (I2C) + M12 IO-Link connector.
// (proto.cat emits this for rendering; the sim manifest is a separate, leaner one.)
const THERMAL_MANIFEST = `
chip: "esp32c3"
external_devices:
  - id: "cam"
    type: "mlx90640"
    connection: "i2c0"
    config:
      sda_pin: "8"
      scl_pin: "9"
      i2c_addr: "0x33"
  - id: "iolink"
    type: "m12-iolink"
    connection: "uart1"
    config:
      cq_pin: "21"
`;

describe('manifestToEditorState — I2C sensor + IO-Link connector', () => {
  it('every new component type is registered', () => {
    expect(COMPONENT_REGISTRY.has('mlx90640')).toBe(true);
    expect(COMPONENT_REGISTRY.has('m12-iolink')).toBe(true);
    expect(COMPONENT_REGISTRY.has('esp32-c3-supermini')).toBe(true);
  });

  it('renders the full thermal device (chip + camera + connector)', () => {
    expect(canRenderInEditor(THERMAL_MANIFEST)).toBe(true);
    const state = manifestToEditorState(THERMAL_MANIFEST);
    expect(state).not.toBeNull();
    const parts = state!.diagram.parts;
    const byId = Object.fromEntries(parts.map((p) => [p.id, p.type]));
    expect(byId['mcu']).toBe('esp32-c3-supermini');
    expect(byId['cam']).toBe('mlx90640');
    expect(byId['iolink']).toBe('m12-iolink');
    expect(parts).toHaveLength(3);
  });

  it('wires I2C (SDA/SCL) to the camera and C/Q to the connector', () => {
    const state = manifestToEditorState(THERMAL_MANIFEST)!;
    const wires = state.diagram.wires;
    const has = (fromPin: string, toPart: string, toPin: string) =>
      wires.some(
        (w) => w.from.part === 'mcu' && w.from.pin === fromPin && w.to.part === toPart && w.to.pin === toPin,
      );
    expect(has('GPIO8', 'cam', 'SDA')).toBe(true);
    expect(has('GPIO9', 'cam', 'SCL')).toBe(true);
    expect(has('3V3', 'cam', 'VCC')).toBe(true);
    expect(has('GPIO21', 'iolink', 'CQ')).toBe(true);
    expect(has('GND', 'iolink', 'L-')).toBe(true);
  });

  it('falls back (null) when a device type has no ComponentDef', () => {
    const unknown = `
chip: "esp32c3"
external_devices:
  - id: "x"
    type: "totally-unknown-widget"
    connection: "i2c0"
    config: {}
`;
    expect(canRenderInEditor(unknown)).toBe(false);
    expect(manifestToEditorState(unknown)).toBeNull();
  });
});
