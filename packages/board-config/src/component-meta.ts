import type { BoardIoKind } from './types';

export interface ComponentMeta { boardIoKind?: BoardIoKind; }

export const COMPONENT_META: Record<string, ComponentMeta> = {
  // MCU boards (no boardIoKind — they are the MCU, not a board_io peripheral)
  mcu: {},
  'arduino-uno': {},
  'stm32-dev': {},
  'nucleo-h563zi': {},
  'nucleo-f401re': {},
  'stm32-blackpill': {},
  esp32: {},
  'esp32-c3-supermini': {},
  'esp32-s3-zero': {},
  'rpi-pico': {},
  'nrf52840-dk': {},

  // Output
  led: { boardIoKind: 'led' },
  'rgb-led': { boardIoKind: 'led' },
  buzzer: { boardIoKind: 'pwm_output' },
  servo: { boardIoKind: 'pwm_output' },
  neopixel: { boardIoKind: 'spi_device' },

  // Input
  button: { boardIoKind: 'button' },
  potentiometer: { boardIoKind: 'adc_input' },
  'slide-switch': { boardIoKind: 'button' },
  'dip-switch': { boardIoKind: 'button' },
  'rotary-encoder': { boardIoKind: 'button' },
  keypad: { boardIoKind: 'button' },

  // Sensors
  dht22: { boardIoKind: 'button' },
  'pir-sensor': { boardIoKind: 'button' },
  ultrasonic: { boardIoKind: 'button' },
  ldr: { boardIoKind: 'adc_input' },
  adxl345: { boardIoKind: 'i2c_device' },
  bme280: { boardIoKind: 'i2c_device' },
  max31855: { boardIoKind: 'spi_device' },
  mpu6050: { boardIoKind: 'i2c_device' },
  // neo6m-gps has boardIoKind: 'uart_device' in the UI but uart_device is
  // outside BoardIoKind; omit to avoid a type error.
  'neo6m-gps': {},
  'ntc-thermistor': { boardIoKind: 'adc_input' },

  // Displays
  'seven-segment': { boardIoKind: 'spi_device' },
  lcd1602: { boardIoKind: 'i2c_device' },
  'oled-ssd1306': { boardIoKind: 'i2c_device' },
  pcd8544: { boardIoKind: 'spi_device' },
  'led-matrix': { boardIoKind: 'spi_device' },
  ili9341: { boardIoKind: 'spi_device' },
  ssd1680_tricolor_290: { boardIoKind: 'spi_device' },
  uc8151d_tricolor_290: { boardIoKind: 'spi_device' },

  // Passives (no boardIoKind)
  resistor: {},
  capacitor: {},
  diode: {},
  transistor: {},

  // ICs
  '74hc595': { boardIoKind: 'spi_device' },
  l293d: { boardIoKind: 'pwm_output' },
};
