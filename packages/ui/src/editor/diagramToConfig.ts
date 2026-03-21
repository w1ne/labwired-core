import type { Diagram } from './types';
import { COMPONENT_REGISTRY } from './components/index';
import { findPinFunction } from './pin-mapping';

/** Chip YAML templates keyed by board name. */
const CHIP_YAMLS: Record<string, string> = {
  stm32f103: `
name: "stm32f103c8"
arch: "arm"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x40010800
    size: "1KB"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x40010C00
    size: "1KB"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x40011000
    size: "1KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
  - id: "i2c1"
    type: "i2c"
    base_address: 0x40005400
    size: "1KB"
    irq: 31
  - id: "afio"
    type: "afio"
    base_address: 0x40010000
    size: "1KB"
  - id: "exti"
    type: "exti"
    base_address: 0x40010400
    size: "1KB"
  - id: "dma1"
    type: "dma"
    base_address: 0x40020000
    size: "1KB"
  - id: "adc1"
    type: "adc"
    base_address: 0x40012400
    size: "1KB"
    irq: 18
`,
  stm32f401: `
name: "stm32f401re"
arch: "arm"
flash:
  base: 0x08000000
  size: "512KB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40023800
    size: "1KB"
    config:
      profile: "stm32f4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x40020000
    size: "1KB"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x40020400
    size: "1KB"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x40020800
    size: "1KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
`,
};

/**
 * Parse an MCU pin label into { peripheral, pin } for various naming conventions.
 * Supports: PA5 (STM32), D0/A0 (Arduino), GPIO0 (ESP32), GP0 (RPi Pico), P0.00 (nRF).
 */
function parseMcuPin(pinLabel: string): { peripheral: string; pin: number } | null {
  // STM32: PA0, PB12, PC13
  const stm = pinLabel.match(/^P([A-C])(\d+)$/i);
  if (stm) return { peripheral: `gpio${stm[1].toLowerCase()}`, pin: parseInt(stm[2], 10) };

  // Arduino: D0-D13 → gpiod, A0-A5 → gpioa
  const ard = pinLabel.match(/^([DA])(\d+)$/i);
  if (ard) return { peripheral: ard[1].toLowerCase() === 'd' ? 'gpiod' : 'gpioa', pin: parseInt(ard[2], 10) };

  // ESP32/RPi Pico: GPIO0, GP0
  const gpio = pinLabel.match(/^(?:GPIO|GP)(\d+)$/i);
  if (gpio) return { peripheral: 'gpio0', pin: parseInt(gpio[1], 10) };

  // nRF52840: P0.00, P0.31
  const nrf = pinLabel.match(/^P(\d+)\.(\d+)$/);
  if (nrf) return { peripheral: `gpio${nrf[1]}`, pin: parseInt(nrf[2], 10) };

  return null;
}

/**
 * Convert a visual diagram into system YAML + chip YAML for the WASM simulator.
 */
export function diagramToConfig(diagram: Diagram): { systemYaml: string; chipYaml: string } {
  const chipYaml = CHIP_YAMLS[diagram.board];
  if (!chipYaml) {
    throw new Error(`Unknown board: ${diagram.board}`);
  }

  // Build board_io entries from wires that connect components to MCU pins
  const boardIoEntries: string[] = [];

  for (const wire of diagram.wires) {
    // Find which end is the MCU and which is a component
    let mcuEnd: typeof wire.from | null = null;
    let compEnd: typeof wire.from | null = null;

    if (wire.from.part === 'mcu') {
      mcuEnd = wire.from;
      compEnd = wire.to;
    } else if (wire.to.part === 'mcu') {
      mcuEnd = wire.to;
      compEnd = wire.from;
    } else {
      continue; // Wire between two non-MCU components — skip for board_io
    }

    // Look up the component to determine board_io kind
    const part = diagram.parts.find((p) => p.id === compEnd!.part);
    if (!part) continue;
    const def = COMPONENT_REGISTRY.get(part.type);
    if (!def?.boardIoKind) continue;

    // Parse MCU pin label to get GPIO peripheral + pin number
    const gpioPin = parseMcuPin(mcuEnd.pin);
    if (!gpioPin) continue;

    const kind = def.boardIoKind;

    // Determine signal direction based on boardIoKind
    const signal = (kind === 'button' || kind === 'adc_input') ? 'input' : 'output';

    // Use real kind now that Rust core supports all variants
    const rustKind = kind;

    // For ADC inputs, try to resolve the actual ADC peripheral from pin mapping
    let peripheral = gpioPin.peripheral;
    if (kind === 'adc_input') {
      const adcFunc = findPinFunction(diagram.board, mcuEnd.pin, 'adc');
      if (adcFunc) {
        peripheral = adcFunc.peripheral;
      }
    }

    boardIoEntries.push(`  - id: "${part.id}"
    kind: "${rustKind}"
    peripheral: "${peripheral}"
    pin: ${gpioPin.pin}
    signal: "${signal}"
    active_high: true`);
  }

  const systemYaml = `name: "playground-board"
chip: "inline"
board_io:
${boardIoEntries.length > 0 ? boardIoEntries.join('\n') : '  []'}
`;

  return { systemYaml, chipYaml };
}
