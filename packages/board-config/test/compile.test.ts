import { describe, expect, it } from 'vitest';
import { compile } from '../src/compile';
import { diagramToConfig } from '../src/diagram-to-config';
import type { Diagram } from '../src/types';
import type { DiagramV2 } from '../src/schema';

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/** SpiceDispenser hero fixture (v2, clean baseline). */
function dispenserFixture(): DiagramV2 {
  return {
    version: 2,
    board: 'esp32-s3-zero',
    parts: [
      { id: 'mcu',  type: 'esp32-s3-zero' },
      { id: 'pca',  type: 'pca9685',  attrs: { i2c_address: '0x40' } },
      { id: 'srv1', type: 'servo' },
      { id: 'srv2', type: 'servo' },
      { id: 'r1',   type: 'resistor' },
      { id: 'r2',   type: 'resistor' },
    ],
    nets: [
      { name: 'GND',  kind: 'power',  voltage: 0   },
      { name: 'V3',   kind: 'power',  voltage: 3.3 },
      { name: 'V5',   kind: 'power',  voltage: 5.0 },
      { name: 'SDA',  kind: 'signal', protocol: 'i2c_sda' },
      { name: 'SCL',  kind: 'signal', protocol: 'i2c_scl' },
      { name: 'PWM1', kind: 'signal' },
      { name: 'PWM2', kind: 'signal' },
    ],
    connections: [
      ['mcu:GND',   'GND'],
      ['mcu:3V3',   'V3'],
      ['mcu:5V',    'V5'],
      ['pca:VCC',   'V3'],
      ['pca:GND',   'GND'],
      ['pca:SDA',   'SDA'],
      ['pca:SCL',   'SCL'],
      ['mcu:GPIO8', 'SDA'],
      ['mcu:GPIO9', 'SCL'],
      ['r1:1', 'SDA'],
      ['r1:2', 'V3'],
      ['r2:1', 'SCL'],
      ['r2:2', 'V3'],
      ['pca:LED8',  'PWM1'],
      ['srv1:PWM',  'PWM1'],
      ['srv1:VCC',  'V5'],
      ['srv1:GND',  'GND'],
      ['pca:LED12', 'PWM2'],
      ['srv2:PWM',  'PWM2'],
      ['srv2:VCC',  'V5'],
      ['srv2:GND',  'GND'],
    ],
    wires: [],
  };
}

/** ERC-erroring fixture: misspell GND to trigger SCHEMA_NET_UNDECLARED. */
function ercErrorFixture(): DiagramV2 {
  const d = dispenserFixture();
  // Misspell GND → GNDD on one connection to trigger SCHEMA_NET_UNDECLARED
  d.connections = d.connections.map(([ref, net]) =>
    ref === 'mcu:GND' ? [ref, 'GNDD'] : [ref, net],
  );
  return d;
}

// ---------------------------------------------------------------------------
// Task 1 tests
// ---------------------------------------------------------------------------

describe('compile()', () => {
  describe('ERC gate', () => {
    it('returns ok:false when ERC has errors', () => {
      const result = compile(ercErrorFixture());
      expect(result.ok).toBe(false);
    });

    it('returns no YAML when ERC has errors', () => {
      const result = compile(ercErrorFixture());
      expect(result.systemYaml).toBeUndefined();
      expect(result.chipYaml).toBeUndefined();
    });

    it('returns diagnostics containing the SCHEMA code when ERC has errors', () => {
      const result = compile(ercErrorFixture());
      const codes = result.diagnostics.map((d) => d.code);
      expect(codes).toContain('SCHEMA_NET_UNDECLARED');
    });
  });

  describe('clean dispenser fixture', () => {
    it('returns ok:true', () => {
      const result = compile(dispenserFixture());
      expect(result.ok).toBe(true);
    });

    it('systemYaml contains a pca9685 external_devices entry', () => {
      const result = compile(dispenserFixture());
      expect(result.systemYaml).toContain('type: "pca9685"');
    });

    it('systemYaml external_devices entry for pca9685 has connection i2c0 (net-derived from GPIO8)', () => {
      // GPIO8 on esp32-s3-zero is i2c0 SDA (per pin-mapping.ts ESP32S3_PINS)
      const result = compile(dispenserFixture());
      expect(result.systemYaml).toContain('connection: "i2c0"');
    });

    it('systemYaml pca9685 entry has address 0x40', () => {
      const result = compile(dispenserFixture());
      expect(result.systemYaml).toContain('i2c_address: 0x40');
    });

    it('returns no diagnostics (clean diagram)', () => {
      const result = compile(dispenserFixture());
      expect(result.diagnostics).toHaveLength(0);
    });

    it('chipYaml is undefined (esp32-s3-zero has no CHIP_YAMLS entry)', () => {
      const result = compile(dispenserFixture());
      // esp32-s3-zero is not in the inline CHIP_YAMLS table — chipYaml should be absent
      expect(result.chipYaml).toBeUndefined();
    });
  });

  describe('COMPILE_BUS_UNBOUND', () => {
    it('fires when SDA net has no MCU i2c-capable pin', () => {
      // Wire SDA to a plain GPIO that has no i2c function (GPIO0)
      const d = dispenserFixture();
      d.connections = d.connections.map(([ref, net]) =>
        ref === 'mcu:GPIO8' ? ['mcu:GPIO0', net] : [ref, net],
      );
      const result = compile(d);
      expect(result.ok).toBe(false);
      const codes = result.diagnostics.map((x) => x.code);
      expect(codes).toContain('COMPILE_BUS_UNBOUND');
    });

    it('hint references SDA pin', () => {
      const d = dispenserFixture();
      d.connections = d.connections.map(([ref, net]) =>
        ref === 'mcu:GPIO8' ? ['mcu:GPIO0', net] : [ref, net],
      );
      const result = compile(d);
      const busUnbound = result.diagnostics.find((x) => x.code === 'COMPILE_BUS_UNBOUND');
      expect(busUnbound).toBeDefined();
      expect(busUnbound?.subjects).toContain('pca:SDA');
    });
  });

  describe('back-compat: compile() output == diagramToConfig() for v1 LED diagram', () => {
    // V1 LED blinky on stm32l476 — same fixture as diagram-to-config.test.ts
    const ledDiagram: Diagram = {
      board: 'stm32l476',
      parts: [{ id: 'led1', type: 'led' }],
      wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } }],
    };

    it('compile() returns ok:true for a clean v1 diagram', () => {
      expect(compile(ledDiagram).ok).toBe(true);
    });

    it('compile() systemYaml is string-identical to diagramToConfig() systemYaml', () => {
      const legacyResult = diagramToConfig(ledDiagram);
      const compileResult = compile(ledDiagram);
      expect(compileResult.systemYaml).toBe(legacyResult.systemYaml);
    });

    it('compile() chipYaml is string-identical to diagramToConfig() chipYaml', () => {
      const legacyResult = diagramToConfig(ledDiagram);
      const compileResult = compile(ledDiagram);
      expect(compileResult.chipYaml).toBe(legacyResult.chipYaml);
    });
  });

  describe('back-compat: compile() output == diagramToConfig() for v1 ADC (potentiometer) diagram', () => {
    // V1 ADC input on stm32l476 — same fixture as diagram-to-config.test.ts
    const adcDiagram: Diagram = {
      board: 'stm32l476',
      parts: [{ id: 'pot1', type: 'potentiometer' }],
      wires: [{ from: { part: 'mcu', pin: 'PA0' }, to: { part: 'pot1', pin: 'out' } }],
    };

    it('compile() systemYaml is string-identical to diagramToConfig() systemYaml', () => {
      const legacyResult = diagramToConfig(adcDiagram);
      const compileResult = compile(adcDiagram);
      expect(compileResult.systemYaml).toBe(legacyResult.systemYaml);
    });
  });

  describe('diagramToConfig() still works (delegation)', () => {
    it('emits a system.yaml with the LED as a board_io led on gpioa pin 5', () => {
      const diagram: Diagram = {
        board: 'stm32l476',
        parts: [{ id: 'led1', type: 'led' }],
        wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } }],
      };
      const { systemYaml, chipYaml } = diagramToConfig(diagram);
      expect(systemYaml).toContain('id: "led1"');
      expect(systemYaml).toContain('gpioa');
      expect(systemYaml).toContain('pin: 5');
      expect(chipYaml).toContain('0x08000000');
    });

    it('throws on an unknown board', () => {
      expect(() => diagramToConfig({ board: 'nope', parts: [], wires: [] })).toThrow(/Unknown board/);
    });
  });
});
