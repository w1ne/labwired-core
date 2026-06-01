/**
 * Slim pure-data mirror of the playground's COMPONENT_REGISTRY metadata.
 * Used by ./diagnostics for validate_diagram. Kept in lockstep with
 * packages/ui/src/editor/components/*.tsx — when a component is added there,
 * add a row here.
 *
 * Only the fields the diagnostics care about: label, category, boardIoKind.
 * No React, no render functions.
 */

export type ComponentCategory =
  | 'output'
  | 'input'
  | 'passive'
  | 'mcu'
  | 'sensor'
  | 'display'
  | 'ic';

export type BoardIoKind =
  | 'led'
  | 'button'
  | 'adc_input'
  | 'pwm_output'
  | 'i2c_device'
  | 'spi_device'
  | 'uart_device';

export interface ComponentMeta {
  label: string;
  category: ComponentCategory;
  /** Absent → passive/decoration; present → board_io kind for wiring. */
  boardIoKind?: BoardIoKind;
}

export const COMPONENT_META: Record<string, ComponentMeta> = {
  // MCU boards (label = product name)
  mcu: { label: 'MCU', category: 'mcu' },
  'arduino-uno': { label: 'Arduino Uno', category: 'mcu' },
  'stm32-dev': { label: 'STM32 Dev Board', category: 'mcu' },
  esp32: { label: 'ESP32', category: 'mcu' },
  'esp32-c3-supermini': { label: 'ESP32-C3 Super Mini', category: 'mcu' },
  'esp32-s3-zero': { label: 'ESP32-S3-Zero', category: 'mcu' },
  'rpi-pico': { label: 'RPi Pico', category: 'mcu' },
  'nrf52840-dk': { label: 'nRF52840 DK', category: 'mcu' },

  // Output
  led: { label: 'LED', category: 'output', boardIoKind: 'led' },
  'rgb-led': { label: 'RGB LED', category: 'output', boardIoKind: 'led' },
  buzzer: { label: 'Buzzer', category: 'output', boardIoKind: 'pwm_output' },
  servo: { label: 'Servo Motor', category: 'output', boardIoKind: 'pwm_output' },
  neopixel: { label: 'NeoPixel Strip', category: 'output', boardIoKind: 'spi_device' },

  // Input
  button: { label: 'Push Button', category: 'input', boardIoKind: 'button' },
  potentiometer: { label: 'Potentiometer', category: 'input', boardIoKind: 'adc_input' },
  'slide-switch': { label: 'Slide Switch', category: 'input', boardIoKind: 'button' },
  'dip-switch': { label: 'DIP Switch', category: 'input', boardIoKind: 'button' },
  'rotary-encoder': { label: 'Rotary Encoder', category: 'input', boardIoKind: 'button' },
  keypad: { label: '4x4 Keypad', category: 'input', boardIoKind: 'button' },

  // Sensors
  dht22: { label: 'DHT22 Sensor', category: 'sensor', boardIoKind: 'button' },
  'pir-sensor': { label: 'PIR Sensor', category: 'sensor', boardIoKind: 'button' },
  ultrasonic: { label: 'HC-SR04', category: 'sensor', boardIoKind: 'button' },
  ldr: { label: 'Photoresistor', category: 'sensor', boardIoKind: 'adc_input' },
  adxl345: { label: 'ADXL345', category: 'sensor', boardIoKind: 'i2c_device' },
  bme280: { label: 'BME280', category: 'sensor', boardIoKind: 'i2c_device' },
  max31855: { label: 'MAX31855', category: 'sensor', boardIoKind: 'spi_device' },
  mpu6050: { label: 'MPU6050', category: 'sensor', boardIoKind: 'i2c_device' },
  'neo6m-gps': { label: 'NEO-6M GPS', category: 'sensor', boardIoKind: 'uart_device' },
  'ntc-thermistor': { label: 'NTC Thermistor', category: 'sensor', boardIoKind: 'adc_input' },

  // Displays
  'seven-segment': { label: '7-Segment', category: 'display', boardIoKind: 'spi_device' },
  lcd1602: { label: 'LCD 16x2', category: 'display', boardIoKind: 'i2c_device' },
  'oled-ssd1306': { label: 'OLED 128x64', category: 'display', boardIoKind: 'i2c_device' },
  'led-matrix': { label: '8x8 LED Matrix', category: 'display', boardIoKind: 'spi_device' },
  ili9341: { label: 'ILI9341 TFT 240x320', category: 'display', boardIoKind: 'spi_device' },
  ssd1680_tricolor_290: {
    label: 'E-Paper 2.9" tri-color (SSD1680)',
    category: 'display',
    boardIoKind: 'spi_device',
  },

  // Passives + ICs (no boardIoKind)
  resistor: { label: 'Resistor', category: 'passive' },
  capacitor: { label: 'Capacitor', category: 'passive' },
  diode: { label: 'Diode', category: 'passive' },
  transistor: { label: 'Transistor', category: 'passive' },
  '74hc595': { label: '74HC595', category: 'ic', boardIoKind: 'spi_device' },
  sn74hc165: { label: '74HC165', category: 'ic', boardIoKind: 'spi_device' },
  'iolink-master': { label: 'IO-Link Master', category: 'ic', boardIoKind: 'uart_device' },
  l293d: { label: 'L293D', category: 'ic', boardIoKind: 'pwm_output' },
};

export function getComponentMeta(type: string): ComponentMeta | null {
  return COMPONENT_META[type] ?? null;
}
