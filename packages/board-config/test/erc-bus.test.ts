import { describe, expect, it } from 'vitest';
import { erc, uartCrossover } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

// Base I2C fixture: two devices on the same bus with the same default address
const i2cPair = (extra: Partial<DiagramV2> = {}): DiagramV2 => ({
  version: 2,
  board: 'esp32-s3-zero',
  parts: [
    { id: 'mcu', type: 'esp32-s3-zero' },
    { id: 'a', type: 'bme280', attrs: { i2c_address: '0x76' } },
    { id: 'b', type: 'pca9685', attrs: { i2c_address: '0x76' } },
  ],
  nets: [
    { name: 'SDA', kind: 'signal', protocol: 'i2c_sda' },
    { name: 'SCL', kind: 'signal', protocol: 'i2c_scl' },
  ],
  connections: [
    ['a:SDA', 'SDA'], ['b:SDA', 'SDA'], ['mcu:GPIO8', 'SDA'],
    ['a:SCL', 'SCL'], ['b:SCL', 'SCL'], ['mcu:GPIO9', 'SCL'],
  ],
  wires: [],
  ...extra,
});

describe('bus rules', () => {
  describe('I2C_ADDR_CONFLICT', () => {
    it('fires when two devices on the same bus share an address', () => {
      expect(codesOf(i2cPair())).toContain('I2C_ADDR_CONFLICT');
    });

    it('does NOT fire when devices have distinct addresses', () => {
      const ok = i2cPair();
      ok.parts = ok.parts.map((p) =>
        p.id === 'b' ? { ...p, attrs: { i2c_address: '0x40' } } : p,
      );
      expect(codesOf(ok)).not.toContain('I2C_ADDR_CONFLICT');
    });
  });

  describe('I2C_NO_PULLUP', () => {
    it('fires when no pull path exists on an open-drain I2C net', () => {
      expect(codesOf(i2cPair())).toContain('I2C_NO_PULLUP');
    });

    it('is satisfied by MCU internal pullups declared in attrs.internal_pullups', () => {
      const internal = i2cPair();
      internal.parts = internal.parts.map((p) =>
        p.id === 'mcu' ? { ...p, attrs: { internal_pullups: 'GPIO8,GPIO9' } } : p,
      );
      expect(codesOf(internal)).not.toContain('I2C_NO_PULLUP');
    });

    it('accepts internal_pullups as an agent-supplied array', () => {
      const internal = i2cPair();
      internal.parts = internal.parts.map((p) =>
        p.id === 'mcu'
          ? { ...p, attrs: { internal_pullups: ['GPIO8', 'GPIO9'] } as unknown as Record<string, string> }
          : p,
      );
      expect(codesOf(internal)).not.toContain('I2C_NO_PULLUP');
    });

    it('matches STM32-style internal pullup pin aliases against GPIO pin names', () => {
      const internal = i2cPair();
      internal.parts = internal.parts.map((p) =>
        p.id === 'mcu'
          ? { ...p, attrs: { internal_pullups: ['PB8', 'PB9'] } as unknown as Record<string, string> }
          : p,
      );
      expect(codesOf(internal)).not.toContain('I2C_NO_PULLUP');
    });

    it('is satisfied by physical pull-up resistors bridging I2C nets to a power rail', () => {
      const resistored = i2cPair();
      resistored.parts.push({ id: 'r1', type: 'resistor' }, { id: 'r2', type: 'resistor' });
      resistored.nets.push({ name: 'V3', kind: 'power', voltage: 3.3 });
      resistored.connections.push(
        ['r1:1', 'SDA'], ['r1:2', 'V3'],
        ['r2:1', 'SCL'], ['r2:2', 'V3'],
        ['mcu:3V3', 'V3'],
      );
      expect(codesOf(resistored)).not.toContain('I2C_NO_PULLUP');
    });
  });

  describe('PIN_INPUT_FLOATING', () => {
    it('fires for a required input pin not connected to any net', () => {
      const bad: DiagramV2 = {
        version: 2,
        board: 'esp32-s3-zero',
        parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'u1', type: 'ultrasonic' }],
        nets: [],
        connections: [],
        wires: [],
      };
      expect(codesOf(bad)).toContain('PIN_INPUT_FLOATING');
    });

    it('does NOT fire when the required input is wired', () => {
      const good: DiagramV2 = {
        version: 2,
        board: 'esp32-s3-zero',
        parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'u1', type: 'ultrasonic' }],
        nets: [{ name: 'T', kind: 'signal' }],
        connections: [['u1:TRIG', 'T'], ['mcu:GPIO4', 'T']],
        wires: [],
      };
      expect(codesOf(good)).not.toContain('PIN_INPUT_FLOATING');
    });
  });

  describe('UART_CROSSOVER (unit-level helper)', () => {
    it('fires when two TX pins are wired to the same net', () => {
      const result = uartCrossover('UART_NET', [
        { key: 'mcu:GPIO43', role: 'uart_tx' },
        { key: 'dev:TX', role: 'uart_tx' },
      ]);
      expect(result.map((d) => d.code)).toContain('UART_CROSSOVER');
    });

    it('fires when two RX pins are wired to the same net', () => {
      const result = uartCrossover('UART_NET', [
        { key: 'mcu:GPIO44', role: 'uart_rx' },
        { key: 'dev:RX', role: 'uart_rx' },
      ]);
      expect(result.map((d) => d.code)).toContain('UART_CROSSOVER');
    });

    it('does NOT fire for a correct TX→RX crossover', () => {
      const result = uartCrossover('UART_NET', [
        { key: 'mcu:GPIO43', role: 'uart_tx' },
        { key: 'dev:RX', role: 'uart_rx' },
      ]);
      expect(result.map((d) => d.code)).not.toContain('UART_CROSSOVER');
    });
  });
});

// ---------------------------------------------------------------------------
// SpiceDispenser hero fixture
// ---------------------------------------------------------------------------
// Hardware: ESP32-S3-Zero MCU driving a PCA9685 via I2C; two servos on
// PCA9685 PWM channels 8 and 12; 3.3V and 5V rails from the MCU; pull-up
// resistors on SDA and SCL.
// Expected result: ZERO errors; the exact warning set is asserted below.
// ---------------------------------------------------------------------------

const dispenserDiagram: DiagramV2 = {
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

describe('SpiceDispenser hero fixture', () => {
  it('produces ZERO errors', () => {
    const diagnostics = erc(dispenserDiagram);
    const errors = diagnostics.filter((d) => d.severity === 'error');
    expect(errors).toEqual([]);
  });

  it('produces the exact expected warning set (empty — fully clean wiring)', () => {
    const diagnostics = erc(dispenserDiagram);
    const warnings = diagnostics.filter((d) => d.severity === 'warning');
    // All warnings resolved:
    //   I2C_NO_PULLUP:      resolved by r1/r2 physical pull-ups to V3
    //   I2C_ADDR_CONFLICT:  only one I2C device (pca9685 at 0x40)
    //   PWR_VOLTAGE_MISMATCH: pca9685 @ 3.3V ∈ [2.3, 5.5]; servo @ 5V ∈ [4.8, 6.0]
    //   PWR_NO_GROUND:      pca:GND and srv1:GND and srv2:GND all wired to GND (0V)
    //   PIN_INPUT_FLOATING: srv1:PWM and srv2:PWM both wired to PWM1/PWM2
    expect(warnings).toEqual([]);
  });
});
