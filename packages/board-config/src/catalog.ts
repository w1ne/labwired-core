// The single declarative part catalog: every part type's device class,
// board_io mapping, and (incrementally) typed pin declarations. Replaces
// the metadata previously split across component-meta.ts here, the copy in
// packages/mcp, and the UI registry. Parts without `pins` are legacy:
// existing diagnostics still apply to them, pin-pair ERC does not (yet).

import type { BoardIoKind } from './types';
import type { NetProtocol } from './schema';

/** Compiler/ERC dispatch class. */
export type DeviceClass =
  | 'mcu' | 'board_io' | 'i2c_device' | 'spi_device' | 'uart_device' | 'passive' | 'tool'
  // Non-electrical canvas annotation (sticky note). No pins, no wires; the
  // compiler skips it (see compile/index.ts "Unknown device class: skip") and
  // every ERC rule ignores it (no boardIoKind, no typed pins).
  | 'annotation';

/** KiCad-vocabulary pin electrical types. */
export type PinEtype =
  | 'input' | 'output' | 'bidirectional' | 'tri_state' | 'passive'
  | 'open_drain' | 'open_emitter' | 'power_in' | 'power_out'
  | 'nc' | 'unspecified' | 'not_internally_connected';

/** A declared part pin. */
export interface PinDecl {
  name: string;
  etype: PinEtype;
  /** Protocol meaning, when the pin has one. */
  role?: NetProtocol;
  /** Pin must be on a net (floating-input ERC, Plan B). */
  required?: boolean;
}

/** A catalog entry for one part type. */
export interface CatalogPart {
  type: string;
  /** ERC/compiler dispatch class. */
  deviceClass: DeviceClass;
  /** Legacy board_io mapping (same meaning as COMPONENT_META). */
  boardIoKind?: BoardIoKind;
  /** Typed pins; undefined = legacy part, pin-level ERC skipped. */
  pins?: PinDecl[];
  /** Supply range in volts for PWR_VOLTAGE_MISMATCH (Plan B). */
  operatingVoltage?: { min: number; max: number };
}

const p = (name: string, etype: PinEtype, role?: NetProtocol, required?: boolean): PinDecl =>
  required ? { name, etype, role, required } : role ? { name, etype, role } : { name, etype };

const pca9685Pins: PinDecl[] = [
  p('VCC', 'power_in'),
  p('GND', 'power_in'),
  p('SDA', 'open_drain', 'i2c_sda'),
  p('SCL', 'open_drain', 'i2c_scl'),
  p('OE', 'input'),
  ...Array.from({ length: 16 }, (_, i) => p(`LED${i}`, 'output', 'pwm')),
];

export const CATALOG: Record<string, CatalogPart> = {
  // --- MCU boards: pins come from PIN_MAPS, not the catalog ---
  mcu: { type: 'mcu', deviceClass: 'mcu' },
  'arduino-uno': { type: 'arduino-uno', deviceClass: 'mcu' },
  'stm32-dev': { type: 'stm32-dev', deviceClass: 'mcu' },
  'nucleo-h563zi': { type: 'nucleo-h563zi', deviceClass: 'mcu' },
  'nucleo-l476rg': { type: 'nucleo-l476rg', deviceClass: 'mcu' },
  'nucleo-f401re': { type: 'nucleo-f401re', deviceClass: 'mcu' },
  'stm32-blackpill': { type: 'stm32-blackpill', deviceClass: 'mcu' },
  esp32: { type: 'esp32', deviceClass: 'mcu' },
  'esp32-c3-supermini': { type: 'esp32-c3-supermini', deviceClass: 'mcu' },
  'esp32-s3-zero': { type: 'esp32-s3-zero', deviceClass: 'mcu' },
  'rpi-pico': { type: 'rpi-pico', deviceClass: 'mcu' },
  'nrf52840-dk': { type: 'nrf52840-dk', deviceClass: 'mcu' },

  // --- Typed parts (initial set; grows incrementally) ---
  led: {
    type: 'led',
    deviceClass: 'board_io',
    boardIoKind: 'led',
    pins: [p('A', 'passive'), p('C', 'passive')],
  },
  button: {
    type: 'button',
    deviceClass: 'board_io',
    boardIoKind: 'button',
    pins: [p('1', 'passive'), p('2', 'passive')],
  },
  resistor: {
    type: 'resistor',
    deviceClass: 'passive',
    pins: [p('1', 'passive'), p('2', 'passive')],
  },
  'can-transceiver': {
    type: 'can-transceiver',
    deviceClass: 'passive',
    pins: [
      p('TXD', 'input'),
      p('RXD', 'output'),
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('CAN_H', 'passive'),
      p('CAN_L', 'passive'),
    ],
  },
  'iolink-transceiver': {
    type: 'iolink-transceiver',
    deviceClass: 'passive',
    pins: [
      p('TXD', 'input'),
      p('RXD', 'output'),
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('CQ', 'bidirectional'),
      p('L+', 'power_in'),
    ],
  },
  'can-diagnostic-tool': {
    type: 'can-diagnostic-tool',
    deviceClass: 'tool',
    pins: [
      p('CAN_H', 'passive'),
      p('CAN_L', 'passive'),
      p('GND', 'power_in'),
    ],
  },
  // Logic analyzer: a high-impedance probe instrument. Channels observe a net
  // (passive), GND is its reference. Mirrors can-diagnostic-tool so the F103/H5
  // UDS labs and the IO-Link lab validate. Pins match logic-analyzer.tsx.
  'logic-analyzer': {
    type: 'logic-analyzer',
    deviceClass: 'tool',
    pins: [
      p('CH0', 'passive'),
      p('CH1', 'passive'),
      p('CH2', 'passive'),
      p('CH3', 'passive'),
      p('GND', 'power_in'),
    ],
  },
  // Quectel BG770A cellular modem over UART. Pins match bg770a-cellular.tsx.
  'bg770a-cellular': {
    type: 'bg770a-cellular',
    deviceClass: 'uart_device',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('TX', 'output', 'uart_tx'),
      p('RX', 'input', 'uart_rx'),
    ],
    operatingVoltage: { min: 3.3, max: 4.3 },
  },
  // Free-form canvas annotation (sticky note). Not a circuit element: no pins,
  // no wires. Registered so it is not flagged as an unknown component; the
  // 'annotation' device class makes the compiler and every ERC rule skip it.
  note: { type: 'note', deviceClass: 'annotation' },
  servo: {
    type: 'servo',
    deviceClass: 'board_io',
    boardIoKind: 'pwm_output',
    pins: [p('PWM', 'input', 'pwm', true), p('VCC', 'power_in'), p('GND', 'power_in')],
    operatingVoltage: { min: 4.8, max: 6.0 },
  },
  pca9685: {
    type: 'pca9685',
    deviceClass: 'i2c_device',
    boardIoKind: 'i2c_device',
    pins: pca9685Pins,
    operatingVoltage: { min: 2.3, max: 5.5 },
  },
  bme280: {
    type: 'bme280',
    deviceClass: 'i2c_device',
    boardIoKind: 'i2c_device',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('SDA', 'open_drain', 'i2c_sda'),
      p('SCL', 'open_drain', 'i2c_scl'),
      // 4-wire SPI mode pins (also drawn by the renderer): chip-select and
      // data-out. Passive so they add no power/bus ERC — they only need to
      // exist for pin-name validation to accept a wire the renderer can draw.
      p('CSB', 'passive'),
      p('SDO', 'passive'),
    ],
    operatingVoltage: { min: 1.71, max: 3.6 },
  },
  ultrasonic: {
    type: 'ultrasonic',
    deviceClass: 'board_io',
    boardIoKind: 'button',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('TRIG', 'input', 'gpio', true),
      p('ECHO', 'output', 'gpio'),
    ],
    operatingVoltage: { min: 4.5, max: 5.5 },
  },

  // --- IR transceivers (I2C interface; SPI-interface variants are future work) ---
  ir: {
    type: 'ir',
    deviceClass: 'i2c_device',
    boardIoKind: 'i2c_device',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('SDA', 'open_drain', 'i2c_sda'),
      p('SCL', 'open_drain', 'i2c_scl'),
    ],
  },

  // --- Legacy parts: boardIoKind carried over verbatim, no pins yet ---
  // Pins below MUST match the @labwired/ui renderer pin ids exactly — a wire
  // that validates here has to be one the renderer can actually draw. The
  // catalog↔renderer parity is locked by a test in packages/ui. Pins are
  // 'passive' (mirroring led/button) so adding them introduces no power/ERC
  // rules — this layer only enforces that the pin NAME exists on the part.
  'rgb-led': {
    type: 'rgb-led',
    deviceClass: 'board_io',
    boardIoKind: 'led',
    pins: [p('R', 'passive'), p('G', 'passive'), p('B', 'passive'), p('GND', 'passive')],
  },
  buzzer: {
    type: 'buzzer',
    deviceClass: 'board_io',
    boardIoKind: 'pwm_output',
    pins: [p('+', 'passive'), p('-', 'passive')],
  },
  neopixel: { type: 'neopixel', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  potentiometer: {
    type: 'potentiometer',
    deviceClass: 'passive',
    boardIoKind: 'adc_input',
    pins: [p('1', 'passive'), p('W', 'passive'), p('2', 'passive')],
  },
  'slide-switch': { type: 'slide-switch', deviceClass: 'board_io', boardIoKind: 'button' },
  'dip-switch': { type: 'dip-switch', deviceClass: 'board_io', boardIoKind: 'button' },
  'rotary-encoder': { type: 'rotary-encoder', deviceClass: 'board_io', boardIoKind: 'button' },
  keypad: { type: 'keypad', deviceClass: 'board_io', boardIoKind: 'button' },
  dht22: { type: 'dht22', deviceClass: 'board_io', boardIoKind: 'button' },
  'pir-sensor': { type: 'pir-sensor', deviceClass: 'board_io', boardIoKind: 'button' },
  ldr: { type: 'ldr', deviceClass: 'passive', boardIoKind: 'adc_input' },
  adxl345: { type: 'adxl345', deviceClass: 'i2c_device', boardIoKind: 'i2c_device' },
  max31855: { type: 'max31855', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  mpu6050: { type: 'mpu6050', deviceClass: 'i2c_device', boardIoKind: 'i2c_device' },
  'neo6m-gps': { type: 'neo6m-gps', deviceClass: 'uart_device', boardIoKind: 'uart_device' },
  'ntc-thermistor': { type: 'ntc-thermistor', deviceClass: 'passive', boardIoKind: 'adc_input' },
  'seven-segment': { type: 'seven-segment', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  lcd1602: { type: 'lcd1602', deviceClass: 'i2c_device', boardIoKind: 'i2c_device' },
  'oled-ssd1306': { type: 'oled-ssd1306', deviceClass: 'i2c_device', boardIoKind: 'i2c_device' },
  pcd8544: { type: 'pcd8544', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  'led-matrix': { type: 'led-matrix', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  ili9341: { type: 'ili9341', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  ssd1680_tricolor_290: { type: 'ssd1680_tricolor_290', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  uc8151d_tricolor_290: { type: 'uc8151d_tricolor_290', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  capacitor: { type: 'capacitor', deviceClass: 'passive' },
  diode: { type: 'diode', deviceClass: 'passive' },
  transistor: { type: 'transistor', deviceClass: 'passive' },
  '74hc595': { type: '74hc595', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  sn74hc165: { type: 'sn74hc165', deviceClass: 'spi_device', boardIoKind: 'spi_device' },
  'iolink-master': { type: 'iolink-master', deviceClass: 'uart_device', boardIoKind: 'uart_device' },
  l293d: { type: 'l293d', deviceClass: 'board_io', boardIoKind: 'pwm_output' },
};

/** Look up a catalog part by diagram part type. */
export function getCatalogPart(type: string): CatalogPart | undefined {
  return CATALOG[type];
}
