import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

// ---------------------------------------------------------------------------
// SpiceDispenser hero fixture
// ---------------------------------------------------------------------------
// Duplicated from erc-bus.test.ts so this file is self-contained and mutation
// cases never share state with the bus-rules suite.
// ---------------------------------------------------------------------------

function baseFixture(): DiagramV2 {
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
      // MCU rails
      ['mcu:GND',   'GND'],
      ['mcu:3V3',   'V3'],
      ['mcu:5V',    'V5'],
      // PCA9685 power
      ['pca:VCC',   'V3'],
      ['pca:GND',   'GND'],
      // PCA9685 I2C bus
      ['pca:SDA',   'SDA'],
      ['pca:SCL',   'SCL'],
      // MCU I2C GPIO
      ['mcu:GPIO8', 'SDA'],
      ['mcu:GPIO9', 'SCL'],
      // Pull-up resistors: one end on the bus net, other end on V3
      ['r1:1', 'SDA'],
      ['r1:2', 'V3'],
      ['r2:1', 'SCL'],
      ['r2:2', 'V3'],
      // Servo 1: PWM from PCA9685 LED8
      ['pca:LED8',  'PWM1'],
      ['srv1:PWM',  'PWM1'],
      ['srv1:VCC',  'V5'],
      ['srv1:GND',  'GND'],
      // Servo 2: PWM from PCA9685 LED12
      ['pca:LED12', 'PWM2'],
      ['srv2:PWM',  'PWM2'],
      ['srv2:VCC',  'V5'],
      ['srv2:GND',  'GND'],
    ],
    wires: [],
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Deep-clone so each mutation operates on a fresh copy of the fixture. */
function clone(d: DiagramV2): DiagramV2 {
  return structuredClone(d);
}

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

// ---------------------------------------------------------------------------
// Baseline
// ---------------------------------------------------------------------------

describe('SpiceDispenser mutation sweep', () => {
  it('baseline fixture is clean (zero diagnostics)', () => {
    expect(erc(baseFixture())).toEqual([]);
  });

  // -------------------------------------------------------------------------
  // Mutation cases
  // -------------------------------------------------------------------------

  const cases: [
    name: string,
    mutate: (d: DiagramV2) => DiagramV2,
    expectedCode: string,
  ][] = [
    // 1. Remove the pull-up resistors → I2C_NO_PULLUP
    [
      'remove pull-up resistors',
      (d) => {
        const m = clone(d);
        m.parts = m.parts.filter((p) => p.id !== 'r1' && p.id !== 'r2');
        m.connections = m.connections.filter(
          ([ref]) => !ref.startsWith('r1:') && !ref.startsWith('r2:'),
        );
        return m;
      },
      'I2C_NO_PULLUP',
    ],

    // 2. Add a second pca9685 at 0x40 on the same bus → I2C_ADDR_CONFLICT
    [
      'second pca9685 at same address 0x40',
      (d) => {
        const m = clone(d);
        m.parts.push({ id: 'pca2', type: 'pca9685', attrs: { i2c_address: '0x40' } });
        m.connections.push(
          ['pca2:SDA', 'SDA'],
          ['pca2:SCL', 'SCL'],
          ['pca2:VCC', 'V3'],
          ['pca2:GND', 'GND'],
        );
        return m;
      },
      'I2C_ADDR_CONFLICT',
    ],

    // 3. Move servo VCC from V5 to V3 → PWR_VOLTAGE_MISMATCH
    //    servo operatingVoltage: { min: 4.8, max: 6.0 }; 3.3 V is out of range
    [
      'servo VCC on 3.3V rail',
      (d) => {
        const m = clone(d);
        m.connections = m.connections.map(([ref, net]) =>
          (ref === 'srv1:VCC' || ref === 'srv2:VCC') && net === 'V5'
            ? [ref, 'V3']
            : [ref, net],
        );
        return m;
      },
      'PWR_VOLTAGE_MISMATCH',
    ],

    // 4. Remove pca9685's GND connection → PWR_NO_GROUND
    [
      'pca9685 GND disconnected',
      (d) => {
        const m = clone(d);
        m.connections = m.connections.filter(([ref]) => ref !== 'pca:GND');
        return m;
      },
      'PWR_NO_GROUND',
    ],

    // 5. Remove one servo's PWM connection → PIN_INPUT_FLOATING
    //    srv1:PWM has required === true in the catalog
    [
      'srv1 PWM pin disconnected',
      (d) => {
        const m = clone(d);
        m.connections = m.connections.filter(([ref]) => ref !== 'srv1:PWM');
        return m;
      },
      'PIN_INPUT_FLOATING',
    ],

    // 6. Connect pca LED0 to V3 (output onto power_out rail) → NET_DRIVER_CONFLICT
    //    pca:LED0 is etype 'output'; mcu:3V3 is etype 'power_out'
    [
      'pca LED0 output driven onto V3 rail',
      (d) => {
        const m = clone(d);
        m.connections.push(['pca:LED0', 'V3']);
        return m;
      },
      'NET_DRIVER_CONFLICT',
    ],

    // 7. Connect one MCU pin to both V3 and V5 → NET_RAIL_SHORT
    //    Bridge clause in matrix-rules: one pin sits on two declared power nets
    //    at different voltages (3.3 V vs 5 V).
    [
      'MCU pin bridging V3 and V5',
      (d) => {
        const m = clone(d);
        // GPIO4 is an ordinary bidirectional MCU pin; put it on both power nets.
        m.connections.push(['mcu:GPIO4', 'V3'], ['mcu:GPIO4', 'V5']);
        return m;
      },
      'NET_RAIL_SHORT',
    ],

    // 8. Misspell a net name in one connection → SCHEMA_NET_UNDECLARED
    [
      'connection references undeclared net name',
      (d) => {
        const m = clone(d);
        // Replace the 'GND' net reference in mcu:GND with a typo
        m.connections = m.connections.map(([ref, net]) =>
          ref === 'mcu:GND' ? [ref, 'GNDD'] : [ref, net],
        );
        return m;
      },
      'SCHEMA_NET_UNDECLARED',
    ],
  ];

  it.each(cases)('%s → %s fires', (_name, mutate, expectedCode) => {
    const mutated = mutate(baseFixture());
    expect(codesOf(mutated)).toContain(expectedCode);
  });
});
