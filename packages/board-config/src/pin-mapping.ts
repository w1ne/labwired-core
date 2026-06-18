/**
 * Maps MCU pin IDs to their alternate functions (ADC channels, I2C buses, SPI buses, timers, etc.)
 * Used by diagramToConfig to auto-detect connection types from wires.
 */

import type { PinEtype } from './catalog';

export interface PinFunction {
  type: 'gpio' | 'adc' | 'i2c' | 'spi' | 'timer' | 'uart' | 'can';
  peripheral: string;
  channel?: number;
  role?: string; // 'scl' | 'sda' | 'mosi' | 'miso' | 'sck' | 'nss' | 'tx' | 'rx'
}

export interface PinMapping {
  gpio: { peripheral: string; pin: number };
  functions: PinFunction[];
}

/** STM32F103 pin alternate functions. */
const STM32F103_PINS: Record<string, PinMapping> = {
  // Power rails — present on every STM32 dev board (Nucleo / Blue Pill / etc.).
  // Exposed here so power-rail wires (mcu:VCC → peripheral:VCC) are recognized
  // by the ERC and the name-based power_out rule fires correctly.
  VCC: { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  GND: { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  PA0: { gpio: { peripheral: 'gpioa', pin: 0 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 0 },
    { type: 'timer', peripheral: 'tim2', channel: 1 },
  ]},
  PA1: { gpio: { peripheral: 'gpioa', pin: 1 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 1 },
    { type: 'timer', peripheral: 'tim2', channel: 2 },
  ]},
  PA2: { gpio: { peripheral: 'gpioa', pin: 2 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 2 },
    { type: 'uart', peripheral: 'uart2', role: 'tx' },
    { type: 'timer', peripheral: 'tim2', channel: 3 },
  ]},
  PA3: { gpio: { peripheral: 'gpioa', pin: 3 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 3 },
    { type: 'uart', peripheral: 'uart2', role: 'rx' },
    { type: 'timer', peripheral: 'tim2', channel: 4 },
  ]},
  PA4: { gpio: { peripheral: 'gpioa', pin: 4 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 4 },
    { type: 'spi', peripheral: 'spi1', role: 'nss' },
  ]},
  PA5: { gpio: { peripheral: 'gpioa', pin: 5 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 5 },
    { type: 'spi', peripheral: 'spi1', role: 'sck' },
  ]},
  PA6: { gpio: { peripheral: 'gpioa', pin: 6 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 6 },
    { type: 'spi', peripheral: 'spi1', role: 'miso' },
    { type: 'timer', peripheral: 'tim3', channel: 1 },
  ]},
  PA7: { gpio: { peripheral: 'gpioa', pin: 7 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 7 },
    { type: 'spi', peripheral: 'spi1', role: 'mosi' },
    { type: 'timer', peripheral: 'tim3', channel: 2 },
  ]},
  PA8: { gpio: { peripheral: 'gpioa', pin: 8 }, functions: [
    { type: 'timer', peripheral: 'tim1', channel: 1 },
  ]},
  PA9: { gpio: { peripheral: 'gpioa', pin: 9 }, functions: [
    { type: 'uart', peripheral: 'uart1', role: 'tx' },
    { type: 'timer', peripheral: 'tim1', channel: 2 },
  ]},
  PA10: { gpio: { peripheral: 'gpioa', pin: 10 }, functions: [
    { type: 'uart', peripheral: 'uart1', role: 'rx' },
    { type: 'timer', peripheral: 'tim1', channel: 3 },
  ]},
  PA11: { gpio: { peripheral: 'gpioa', pin: 11 }, functions: [
    { type: 'timer', peripheral: 'tim1', channel: 4 },
    { type: 'can', peripheral: 'bxcan1', role: 'rx' },
  ]},
  PA12: { gpio: { peripheral: 'gpioa', pin: 12 }, functions: [
    { type: 'can', peripheral: 'bxcan1', role: 'tx' },
  ]},
  PA13: { gpio: { peripheral: 'gpioa', pin: 13 }, functions: [] },
  PA14: { gpio: { peripheral: 'gpioa', pin: 14 }, functions: [] },
  PA15: { gpio: { peripheral: 'gpioa', pin: 15 }, functions: [
    { type: 'timer', peripheral: 'tim2', channel: 1 },
    { type: 'spi', peripheral: 'spi1', role: 'nss' },
  ]},

  PB0: { gpio: { peripheral: 'gpiob', pin: 0 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 8 },
    { type: 'timer', peripheral: 'tim3', channel: 3 },
  ]},
  PB1: { gpio: { peripheral: 'gpiob', pin: 1 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 9 },
    { type: 'timer', peripheral: 'tim3', channel: 4 },
  ]},
  PB3: { gpio: { peripheral: 'gpiob', pin: 3 }, functions: [
    { type: 'spi', peripheral: 'spi1', role: 'sck' },
    { type: 'timer', peripheral: 'tim2', channel: 2 },
  ]},
  PB4: { gpio: { peripheral: 'gpiob', pin: 4 }, functions: [
    { type: 'spi', peripheral: 'spi1', role: 'miso' },
    { type: 'timer', peripheral: 'tim3', channel: 1 },
  ]},
  PB5: { gpio: { peripheral: 'gpiob', pin: 5 }, functions: [
    { type: 'spi', peripheral: 'spi1', role: 'mosi' },
  ]},
  PB6: { gpio: { peripheral: 'gpiob', pin: 6 }, functions: [
    { type: 'i2c', peripheral: 'i2c1', role: 'scl' },
    { type: 'timer', peripheral: 'tim4', channel: 1 },
  ]},
  PB7: { gpio: { peripheral: 'gpiob', pin: 7 }, functions: [
    { type: 'i2c', peripheral: 'i2c1', role: 'sda' },
    { type: 'timer', peripheral: 'tim4', channel: 2 },
  ]},
  PB8: { gpio: { peripheral: 'gpiob', pin: 8 }, functions: [
    { type: 'i2c', peripheral: 'i2c1', role: 'scl' },
    { type: 'timer', peripheral: 'tim4', channel: 3 },
  ]},
  PB9: { gpio: { peripheral: 'gpiob', pin: 9 }, functions: [
    { type: 'i2c', peripheral: 'i2c1', role: 'sda' },
    { type: 'timer', peripheral: 'tim4', channel: 4 },
  ]},
  PB10: { gpio: { peripheral: 'gpiob', pin: 10 }, functions: [
    { type: 'i2c', peripheral: 'i2c2', role: 'scl' },
    { type: 'uart', peripheral: 'uart3', role: 'tx' },
  ]},
  PB11: { gpio: { peripheral: 'gpiob', pin: 11 }, functions: [
    { type: 'i2c', peripheral: 'i2c2', role: 'sda' },
    { type: 'uart', peripheral: 'uart3', role: 'rx' },
  ]},
  PB12: { gpio: { peripheral: 'gpiob', pin: 12 }, functions: [
    { type: 'spi', peripheral: 'spi2', role: 'nss' },
  ]},
  PB13: { gpio: { peripheral: 'gpiob', pin: 13 }, functions: [
    { type: 'spi', peripheral: 'spi2', role: 'sck' },
  ]},
  PB14: { gpio: { peripheral: 'gpiob', pin: 14 }, functions: [
    { type: 'spi', peripheral: 'spi2', role: 'miso' },
  ]},
  PB15: { gpio: { peripheral: 'gpiob', pin: 15 }, functions: [
    { type: 'spi', peripheral: 'spi2', role: 'mosi' },
  ]},

  PC0: { gpio: { peripheral: 'gpioc', pin: 0 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 10 },
  ]},
  PC1: { gpio: { peripheral: 'gpioc', pin: 1 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 11 },
  ]},
  PC2: { gpio: { peripheral: 'gpioc', pin: 2 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 12 },
  ]},
  PC3: { gpio: { peripheral: 'gpioc', pin: 3 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 13 },
  ]},
  PC4: { gpio: { peripheral: 'gpioc', pin: 4 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 14 },
  ]},
  PC5: { gpio: { peripheral: 'gpioc', pin: 5 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 15 },
  ]},
  // PC6-PC12: present on LQFP-64 (Nucleo-F103RB) but not on LQFP-48 (Blue Pill).
  // Listed here so bundled labs authored for Nucleo (e.g. epaper-tricolor-lab,
  // which wires BUSY → PC7) don't trip PIN_NOT_ON_CHIP in the playground.
  PC6: { gpio: { peripheral: 'gpioc', pin: 6 }, functions: [
    { type: 'timer', peripheral: 'tim8', channel: 1 },
  ]},
  PC7: { gpio: { peripheral: 'gpioc', pin: 7 }, functions: [
    { type: 'timer', peripheral: 'tim8', channel: 2 },
  ]},
  PC8: { gpio: { peripheral: 'gpioc', pin: 8 }, functions: [
    { type: 'timer', peripheral: 'tim8', channel: 3 },
  ]},
  PC9: { gpio: { peripheral: 'gpioc', pin: 9 }, functions: [
    { type: 'timer', peripheral: 'tim8', channel: 4 },
  ]},
  PC10: { gpio: { peripheral: 'gpioc', pin: 10 }, functions: [
    { type: 'spi', peripheral: 'spi3', role: 'sck' },
    { type: 'uart', peripheral: 'uart3', role: 'tx' },
  ]},
  PC11: { gpio: { peripheral: 'gpioc', pin: 11 }, functions: [
    { type: 'spi', peripheral: 'spi3', role: 'miso' },
    { type: 'uart', peripheral: 'uart3', role: 'rx' },
  ]},
  PC12: { gpio: { peripheral: 'gpioc', pin: 12 }, functions: [
    { type: 'spi', peripheral: 'spi3', role: 'mosi' },
  ]},
  PC13: { gpio: { peripheral: 'gpioc', pin: 13 }, functions: [] },
  PC14: { gpio: { peripheral: 'gpioc', pin: 14 }, functions: [] },
  PC15: { gpio: { peripheral: 'gpioc', pin: 15 }, functions: [] },
};

/**
 * STM32L476 pin mappings (NUCLEO-L476RG LQFP-64).
 * PA0-PA7 carry ADC1 channels 5-12 per RM0351 Table 16.
 * Reuses the F103 baseline for GPIO structure; ADC channel numbers differ
 * (L476 uses ADC1 channels 5..12 for PA0..PA7 per RM0351 §16.4.1).
 */
const STM32L476_PINS: Record<string, PinMapping> = {
  ...STM32F103_PINS,
  // Override PA0-PA7 with L476-correct ADC1 channel numbers (RM0351 §16)
  PA0: { gpio: { peripheral: 'gpioa', pin: 0 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 5 },
    { type: 'timer', peripheral: 'tim2', channel: 1 },
  ]},
  PA1: { gpio: { peripheral: 'gpioa', pin: 1 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 6 },
    { type: 'timer', peripheral: 'tim2', channel: 2 },
  ]},
  PA2: { gpio: { peripheral: 'gpioa', pin: 2 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 7 },
    { type: 'uart', peripheral: 'uart2', role: 'tx' },
    { type: 'timer', peripheral: 'tim2', channel: 3 },
  ]},
  PA3: { gpio: { peripheral: 'gpioa', pin: 3 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 8 },
    { type: 'uart', peripheral: 'uart2', role: 'rx' },
    { type: 'timer', peripheral: 'tim2', channel: 4 },
  ]},
  PA4: { gpio: { peripheral: 'gpioa', pin: 4 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 9 },
    { type: 'spi', peripheral: 'spi1', role: 'nss' },
  ]},
  PA5: { gpio: { peripheral: 'gpioa', pin: 5 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 10 },
    { type: 'spi', peripheral: 'spi1', role: 'sck' },
  ]},
  PA6: { gpio: { peripheral: 'gpioa', pin: 6 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 11 },
    { type: 'spi', peripheral: 'spi1', role: 'miso' },
    { type: 'timer', peripheral: 'tim3', channel: 1 },
  ]},
  PA7: { gpio: { peripheral: 'gpioa', pin: 7 }, functions: [
    { type: 'adc', peripheral: 'adc1', channel: 12 },
    { type: 'spi', peripheral: 'spi1', role: 'mosi' },
    { type: 'timer', peripheral: 'tim3', channel: 2 },
  ]},
  // L476 has additional GPIO ports D/E/H present on LQFP-64
  PD0: { gpio: { peripheral: 'gpiod', pin: 0 }, functions: [] },
  PD1: { gpio: { peripheral: 'gpiod', pin: 1 }, functions: [] },
  PD2: { gpio: { peripheral: 'gpiod', pin: 2 }, functions: [] },
  PD3: { gpio: { peripheral: 'gpiod', pin: 3 }, functions: [] },
  PD4: { gpio: { peripheral: 'gpiod', pin: 4 }, functions: [] },
  PD5: { gpio: { peripheral: 'gpiod', pin: 5 }, functions: [
    { type: 'uart', peripheral: 'uart2', role: 'tx' },
  ]},
  PD6: { gpio: { peripheral: 'gpiod', pin: 6 }, functions: [
    { type: 'uart', peripheral: 'uart2', role: 'rx' },
  ]},
  PD7: { gpio: { peripheral: 'gpiod', pin: 7 }, functions: [] },
  PD8: { gpio: { peripheral: 'gpiod', pin: 8 }, functions: [] },
  PD9: { gpio: { peripheral: 'gpiod', pin: 9 }, functions: [] },
  PE0: { gpio: { peripheral: 'gpioe', pin: 0 }, functions: [] },
  PE1: { gpio: { peripheral: 'gpioe', pin: 1 }, functions: [] },
  PH0: { gpio: { peripheral: 'gpioh', pin: 0 }, functions: [] },
  PH1: { gpio: { peripheral: 'gpioh', pin: 1 }, functions: [] },
};

/** STM32H563 pin mappings (extends F103 with additional GPIO ports D-G). */
const STM32H563_PINS: Record<string, PinMapping> = {
  ...STM32F103_PINS,
  PD0: { gpio: { peripheral: 'gpiod', pin: 0 }, functions: [
    { type: 'can', peripheral: 'fdcan1', role: 'rx' },
  ] },
  PD1: { gpio: { peripheral: 'gpiod', pin: 1 }, functions: [
    { type: 'can', peripheral: 'fdcan1', role: 'tx' },
  ] },
  PE0: { gpio: { peripheral: 'gpioe', pin: 0 }, functions: [] },
  PF4: { gpio: { peripheral: 'gpiof', pin: 4 }, functions: [] },
  PG4: { gpio: { peripheral: 'gpiog', pin: 4 }, functions: [] },
};

/** RP2040 pin mappings (GP0-GP28). */
const RP2040_PINS: Record<string, PinMapping> = {
  // Power rails exposed on the Pico board header.
  '3V3': { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  GND:   { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  VBUS:  { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  ...Object.fromEntries(
    Array.from({ length: 29 }, (_, i) => [
      `GP${i}`,
      {
        gpio: { peripheral: 'gpio', pin: i },
        functions: i <= 3
          ? [{ type: 'uart' as const, peripheral: 'uart0', role: i % 2 === 0 ? 'tx' : 'rx' }]
          : [],
      },
    ]),
  ),
};

/** nRF52840 pin mappings (P0.00-P0.31, P1.00-P1.15). */
const NRF52840_PINS: Record<string, PinMapping> = {
  // Power rails — nRF52840 DK exposes VDD (3V3) and GND on its header.
  VDD: { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  GND: { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  ...Object.fromEntries(
    Array.from({ length: 32 }, (_, i) => [
      `P0.${String(i).padStart(2, '0')}`,
      { gpio: { peripheral: 'gpio0', pin: i }, functions: [] as PinFunction[] },
    ]),
  ),
  ...Object.fromEntries(
    Array.from({ length: 16 }, (_, i) => [
      `P1.${String(i).padStart(2, '0')}`,
      { gpio: { peripheral: 'gpio1', pin: i }, functions: [] as PinFunction[] },
    ]),
  ),
};

/** ESP32-C3 pin mappings (GPIO0-GPIO21). */
const ESP32C3_PINS: Record<string, PinMapping> = {
  // Power rails exposed on the Super Mini board header.
  '3V3': { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  GND:   { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  ...Object.fromEntries(
    Array.from({ length: 22 }, (_, i) => [
      `GPIO${i}`,
      {
        gpio: { peripheral: 'gpio', pin: i },
        functions: i <= 1
          ? [{ type: 'uart' as const, peripheral: 'uart0', role: i === 0 ? 'tx' : 'rx' }]
          : [],
      },
    ]),
  ),
};

/** ESP32-classic pin mappings (GPIO0-GPIO39 + power rails).
 *  VSPI default pinmux puts SCK on GPIO18, MOSI on GPIO23, MISO on GPIO19,
 *  CS on GPIO5 — surfaced as `spi` functions so the validator marks them
 *  SPI-capable for `epaper-tricolor-lab`-style components. */
const ESP32_PINS: Record<string, PinMapping> = {
  // Power rails (sim-only — physical 3V3 / GND don't have a GPIO bank).
  '3V3': { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  'GND': { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  ...Object.fromEntries(
    [
      0, 1, 2, 3, 4, 5, 12, 13, 14, 15, 16, 17, 18, 19, 21, 22, 23, 25, 26, 27,
      32, 33, 34, 35, 36, 39,
    ].map((n) => {
      const spi: PinFunction[] =
        n === 18 || n === 23 || n === 19 || n === 5
          ? [{ type: 'spi', peripheral: 'spi3', role: 'sck' }]
          : [];
      const uart: PinFunction[] =
        n === 1 || n === 3
          ? [{ type: 'uart', peripheral: 'uart0', role: n === 1 ? 'tx' : 'rx' }]
          : [];
      return [
        `GPIO${n}`,
        {
          gpio: { peripheral: 'gpio', pin: n },
          functions: [...spi, ...uart],
        },
      ];
    }),
  ),
};

/** ESP32-S3 pin mappings (GPIO0-GPIO21, GPIO26-GPIO48 + power rails).
 *  The S3 has a contiguous-ish GPIO bank: 0-21 and 26-48 (22-25 are strapping/
 *  USB/JTAG pins not brought out on most modules). UART0 is on GPIO43(TX)/GPIO44(RX)
 *  per the ESP32-S3 TRM; I2C and SPI defaults follow ESP-IDF v5 conventions. */
const ESP32S3_PINS: Record<string, PinMapping> = {
  '3V3': { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  '5V':  { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  'GND': { gpio: { peripheral: 'gpio', pin: 0 }, functions: [] },
  ...Object.fromEntries(
    [
      ...Array.from({ length: 22 }, (_, i) => i),          // 0-21
      ...Array.from({ length: 23 }, (_, i) => i + 26),     // 26-48
    ].map((n) => {
      const uart: PinFunction[] =
        n === 43 || n === 44
          ? [{ type: 'uart', peripheral: 'uart0', role: n === 43 ? 'tx' : 'rx' }]
          : [];
      const i2c: PinFunction[] =
        n === 8 || n === 9
          ? [{ type: 'i2c', peripheral: 'i2c0', role: n === 8 ? 'sda' : 'scl' }]
          : [];
      const spi: PinFunction[] =
        n === 11 || n === 12 || n === 13
          ? [{ type: 'spi', peripheral: 'spi2', role: n === 11 ? 'mosi' : n === 12 ? 'sck' : 'miso' }]
          : [];
      return [
        `GPIO${n}`,
        {
          gpio: { peripheral: 'gpio', pin: n },
          functions: [...uart, ...i2c, ...spi],
        },
      ];
    }),
  ),
};

export const PIN_MAPS: Record<string, Record<string, PinMapping>> = {
  stm32f103: STM32F103_PINS,
  stm32f401: STM32F103_PINS, // Similar enough for now
  stm32f401cdu6: STM32F103_PINS, // Black Pill F401CDU6 — same PA/PB/PC GPIO scheme; TODO: dedicated map
  stm32l476: STM32L476_PINS,
  stm32h563: STM32H563_PINS,
  rp2040: RP2040_PINS,
  nrf52840: NRF52840_PINS,
  'nrf52840-onboarding': NRF52840_PINS, // Full-peripheral onboarding variant — identical GPIO bank layout
  esp32: ESP32_PINS,
  esp32c3: ESP32C3_PINS,
  esp32s3: ESP32S3_PINS,        // Updated from ESP32_PINS to the correct S3 GPIO range
  'esp32-s3-zero': ESP32S3_PINS, // Waveshare ESP32-S3-Zero module — same S3 GPIO bank
};

/**
 * Look up a pin's mapping for a given board.
 */
export function getPinMapping(board: string, pinLabel: string): PinMapping | null {
  const map = PIN_MAPS[board];
  if (!map) return null;
  return map[pinLabel.toUpperCase()] ?? null;
}

/**
 * Find a specific alternate function for a pin.
 */
export function findPinFunction(
  board: string,
  pinLabel: string,
  type: PinFunction['type'],
): PinFunction | null {
  const mapping = getPinMapping(board, pinLabel);
  if (!mapping) return null;
  return mapping.functions.find((f) => f.type === type) ?? null;
}

/** Electrical characteristics of an MCU pin. */
export interface PinElectrical {
  etype: PinEtype;
  internalPullup: boolean;
}

/** Shared power-rail overrides for ESP32-S3 variants (includes 5 V rail). */
const ESP32S3_POWER_OVERRIDES: Record<string, PinElectrical> = {
  '3V3': { etype: 'power_out', internalPullup: false },
  '5V':  { etype: 'power_out', internalPullup: false },
  GND:   { etype: 'power_out', internalPullup: false },
};

/** Per-board pin-level overrides; pins absent here fall through to the
 *  name-based default rule in getPinEtype(). These take highest precedence. */
const PIN_ELECTRICAL_OVERRIDES: Record<string, Record<string, PinElectrical>> = {
  'esp32-s3-zero': ESP32S3_POWER_OVERRIDES,
  esp32s3:         ESP32S3_POWER_OVERRIDES,
  esp32: {
    '3V3': { etype: 'power_out', internalPullup: false },
    GND: { etype: 'power_out', internalPullup: false },
  },
  // Other boards use the name-based default rule for power pins.
};

/**
 * Name-based default rule for power-rail pins.
 *
 * Pin labels matching common power-supply patterns are resolved to
 * `power_out` regardless of board, so that STM32/nRF/RP2040 boards (which
 * lack explicit entries in PIN_ELECTRICAL_OVERRIDES) don't default to
 * `bidirectional` and falsely trigger PWR_RAIL_UNDRIVEN in the ERC.
 *
 * Rules (case-insensitive, optional ".N" multi-instance suffix stripped):
 *   - /^(3V3|5V|VCC|VDD|VBUS|3\.3V)(\.\d+)?$/i  → power_out
 *   - /^(GND|VSS|AGND)(\.\d+)?$/i                → power_out
 *
 * Returns null when the label doesn't match any power pattern (caller
 * falls through to the bidirectional+pullup default).
 */
function powerPinDefaultEtype(pinLabel: string): PinElectrical | null {
  const stripped = pinLabel.replace(/\.\d+$/, '');
  if (/^(3V3|5V|VCC|VDD|VBUS|3\.3V)$/i.test(stripped)) {
    return { etype: 'power_out', internalPullup: false };
  }
  if (/^(GND|VSS|AGND)$/i.test(stripped)) {
    return { etype: 'power_out', internalPullup: false };
  }
  return null;
}

/**
 * Electrical type of an MCU pin.
 *
 * Resolution order (first match wins):
 *   1. Per-board override table (PIN_ELECTRICAL_OVERRIDES) — escape hatch.
 *   2. Name-based default rule (powerPinDefaultEtype) — covers all boards.
 *   3. Mapped GPIO-capable pin → bidirectional + internalPullup:true.
 *
 * Returns null for unknown pin or board.
 */
export function getPinEtype(board: string, pinLabel: string): PinElectrical | null {
  // 1. Per-board override takes highest precedence.
  const override = PIN_ELECTRICAL_OVERRIDES[board]?.[pinLabel];
  if (override) return override;

  // 2. Name-based power-rail default (works across all boards).
  const powerDefault = powerPinDefaultEtype(pinLabel);
  if (powerDefault) {
    // Only apply if the pin actually exists in the board's map (or the board
    // has no map — treat it as known when the board itself is unknown and
    // we'd return null from getPinMapping anyway).
    const mapping = getPinMapping(board, pinLabel);
    if (mapping) return powerDefault;
    // Pin name looks like power but is not in this board's map → return null
    // so that getPinMapping(board, pinLabel) == null callers still get null.
    return null;
  }

  // 3. Generic GPIO pin default.
  const mapping = getPinMapping(board, pinLabel);
  if (!mapping) return null;
  return { etype: 'bidirectional', internalPullup: true };
}
