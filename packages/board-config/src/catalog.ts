// The single declarative part catalog: every part type's device class,
// board_io mapping, and (incrementally) typed pin declarations. Replaces
// the metadata previously split across component-meta.ts here, the copy in
// packages/mcp, and the UI registry. Parts without `pins` are legacy:
// existing diagnostics still apply to them, pin-pair ERC does not (yet).

import type { BoardIoKind } from './types';
import type { NetProtocol } from './schema';

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
  mcu: { type: 'mcu' },
  'arduino-uno': { type: 'arduino-uno' },
  'stm32-dev': { type: 'stm32-dev' },
  'nucleo-h563zi': { type: 'nucleo-h563zi' },
  'nucleo-f401re': { type: 'nucleo-f401re' },
  'stm32-blackpill': { type: 'stm32-blackpill' },
  esp32: { type: 'esp32' },
  'esp32-c3-supermini': { type: 'esp32-c3-supermini' },
  'esp32-s3-zero': { type: 'esp32-s3-zero' },
  'rpi-pico': { type: 'rpi-pico' },
  'nrf52840-dk': { type: 'nrf52840-dk' },

  // --- Typed parts (initial set; grows incrementally) ---
  led: {
    type: 'led',
    boardIoKind: 'led',
    pins: [p('A', 'passive'), p('C', 'passive')],
  },
  button: {
    type: 'button',
    boardIoKind: 'button',
    pins: [p('1', 'passive'), p('2', 'passive')],
  },
  resistor: {
    type: 'resistor',
    pins: [p('1', 'passive'), p('2', 'passive')],
  },
  servo: {
    type: 'servo',
    boardIoKind: 'pwm_output',
    pins: [p('PWM', 'input', 'pwm', true), p('VCC', 'power_in'), p('GND', 'power_in')],
    operatingVoltage: { min: 4.8, max: 6.0 },
  },
  pca9685: {
    type: 'pca9685',
    boardIoKind: 'i2c_device',
    pins: pca9685Pins,
    operatingVoltage: { min: 2.3, max: 5.5 },
  },
  bme280: {
    type: 'bme280',
    boardIoKind: 'i2c_device',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('SDA', 'open_drain', 'i2c_sda'),
      p('SCL', 'open_drain', 'i2c_scl'),
    ],
    operatingVoltage: { min: 1.71, max: 3.6 },
  },
  ultrasonic: {
    type: 'ultrasonic',
    boardIoKind: 'button',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('TRIG', 'input', 'gpio', true),
      p('ECHO', 'output', 'gpio'),
    ],
    operatingVoltage: { min: 4.5, max: 5.5 },
  },

  // --- Legacy parts: boardIoKind carried over verbatim, no pins yet ---
  'rgb-led': { type: 'rgb-led', boardIoKind: 'led' },
  buzzer: { type: 'buzzer', boardIoKind: 'pwm_output' },
  neopixel: { type: 'neopixel', boardIoKind: 'spi_device' },
  potentiometer: { type: 'potentiometer', boardIoKind: 'adc_input' },
  'slide-switch': { type: 'slide-switch', boardIoKind: 'button' },
  'dip-switch': { type: 'dip-switch', boardIoKind: 'button' },
  'rotary-encoder': { type: 'rotary-encoder', boardIoKind: 'button' },
  keypad: { type: 'keypad', boardIoKind: 'button' },
  dht22: { type: 'dht22', boardIoKind: 'button' },
  'pir-sensor': { type: 'pir-sensor', boardIoKind: 'button' },
  ldr: { type: 'ldr', boardIoKind: 'adc_input' },
  adxl345: { type: 'adxl345', boardIoKind: 'i2c_device' },
  max31855: { type: 'max31855', boardIoKind: 'spi_device' },
  mpu6050: { type: 'mpu6050', boardIoKind: 'i2c_device' },
  'neo6m-gps': { type: 'neo6m-gps' },
  'ntc-thermistor': { type: 'ntc-thermistor', boardIoKind: 'adc_input' },
  'seven-segment': { type: 'seven-segment', boardIoKind: 'spi_device' },
  lcd1602: { type: 'lcd1602', boardIoKind: 'i2c_device' },
  'oled-ssd1306': { type: 'oled-ssd1306', boardIoKind: 'i2c_device' },
  pcd8544: { type: 'pcd8544', boardIoKind: 'spi_device' },
  'led-matrix': { type: 'led-matrix', boardIoKind: 'spi_device' },
  ili9341: { type: 'ili9341', boardIoKind: 'spi_device' },
  ssd1680_tricolor_290: { type: 'ssd1680_tricolor_290', boardIoKind: 'spi_device' },
  uc8151d_tricolor_290: { type: 'uc8151d_tricolor_290', boardIoKind: 'spi_device' },
  capacitor: { type: 'capacitor' },
  diode: { type: 'diode' },
  transistor: { type: 'transistor' },
  '74hc595': { type: '74hc595', boardIoKind: 'spi_device' },
  l293d: { type: 'l293d', boardIoKind: 'pwm_output' },
};

/** Look up a catalog part by diagram part type. */
export function getCatalogPart(type: string): CatalogPart | undefined {
  return CATALOG[type];
}
