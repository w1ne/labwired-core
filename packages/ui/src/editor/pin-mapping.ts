/**
 * Maps MCU pin IDs to their alternate functions (ADC channels, I2C buses, SPI buses, timers, etc.)
 * Used by diagramToConfig to auto-detect connection types from wires.
 */

export interface PinFunction {
  type: 'gpio' | 'adc' | 'i2c' | 'spi' | 'timer' | 'uart';
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
  ]},
  PA12: { gpio: { peripheral: 'gpioa', pin: 12 }, functions: [] },
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
  PC13: { gpio: { peripheral: 'gpioc', pin: 13 }, functions: [] },
  PC14: { gpio: { peripheral: 'gpioc', pin: 14 }, functions: [] },
  PC15: { gpio: { peripheral: 'gpioc', pin: 15 }, functions: [] },
};

const PIN_MAPS: Record<string, Record<string, PinMapping>> = {
  stm32f103: STM32F103_PINS,
  stm32f401: STM32F103_PINS, // Similar enough for now
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
