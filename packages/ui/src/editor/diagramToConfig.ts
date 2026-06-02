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
  // v1 subset — see core/configs/chips/stm32l476.yaml for the full peripheral list
  stm32l476: `
name: "stm32l476rg"
arch: "arm"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
    config:
      profile: "stm32l4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x48000000
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x48000400
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x48000800
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiod"
    type: "gpio"
    base_address: 0x48000C00
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioe"
    type: "gpio"
    base_address: 0x48001000
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioh"
    type: "gpio"
    base_address: 0x48001C00
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
    config:
      profile: "stm32v2"
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
    config:
      profile: "stm32v2"
  - id: "spi1"
    type: "spi"
    base_address: 0x40013000
    size: "1KB"
    irq: 35
    config:
      profile: "stm32_fifo"
  - id: "i2c1"
    type: "i2c"
    base_address: 0x40005400
    size: "1KB"
    irq: 31
    config:
      profile: "stm32l4"
  - id: "adc1"
    type: "adc"
    base_address: 0x50040000
    size: "1KB"
    irq: 18
    config:
      profile: "stm32l4"
  - id: "dma1"
    type: "dma"
    base_address: 0x40020000
    size: "1KB"
    irq: 11
`,
};

const DEFAULT_CPU_HZ_BY_BOARD: Record<string, number> = {
  stm32l476: 4_000_000,
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
 * If chipYamlOverride is provided, it is used instead of the built-in CHIP_YAMLS lookup.
 */
export function diagramToConfig(
  diagram: Diagram,
  chipYamlOverride?: string,
): { systemYaml: string; chipYaml: string } {
  const chipYaml = chipYamlOverride ?? CHIP_YAMLS[diagram.board];
  if (!chipYaml) {
    throw new Error(`Unknown board: ${diagram.board}. Provide a chipYamlOverride or add it to CHIP_YAMLS.`);
  }

  // Build board_io entries from wires that connect components to MCU pins
  const boardIoEntries: string[] = [];
  const externalDeviceEntries: string[] = [];

  const mcuPinForPartPin = (partId: string, pinId: string): string | null => {
    for (const wire of diagram.wires) {
      if (wire.from.part === 'mcu' && wire.to.part === partId && wire.to.pin === pinId) {
        return wire.from.pin;
      }
      if (wire.to.part === 'mcu' && wire.from.part === partId && wire.from.pin === pinId) {
        return wire.to.pin;
      }
    }
    return null;
  };

  const spiPeripheralForPart = (partId: string): string | null => {
    const spiPins = ['CLK', 'SCK', 'DIN', 'MOSI'];
    for (const pinId of spiPins) {
      const mcuPin = mcuPinForPartPin(partId, pinId);
      if (!mcuPin) continue;
      const spi = findPinFunction(diagram.board, mcuPin, 'spi');
      if (spi) return spi.peripheral;
    }
    return null;
  };

  for (const part of diagram.parts) {
    if (part.type !== 'ultrasonic') continue;

    const trigPin = mcuPinForPartPin(part.id, 'TRIG');
    const echoPin = mcuPinForPartPin(part.id, 'ECHO');
    if (!trigPin || !echoPin) continue;

    const distance = Number.parseFloat(part.attrs.distance ?? '');
    const distanceCm = Number.isFinite(distance) ? distance : 100;
    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "hc-sr04"
    connection: "gpio"
    config:
      trig_pin: "${trigPin}"
      echo_pin: "${echoPin}"
      distance_cm: ${distanceCm}
      cpu_hz: ${DEFAULT_CPU_HZ_BY_BOARD[diagram.board] ?? 80_000_000}`);
  }

  for (const part of diagram.parts) {
    if (part.type !== 'pcd8544') continue;

    const connection = spiPeripheralForPart(part.id);
    const csPin = mcuPinForPartPin(part.id, 'CE');
    const dcPin = mcuPinForPartPin(part.id, 'DC');
    if (!connection || !csPin || !dcPin) continue;

    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "pcd8544"
    connection: "${connection}"
    config:
      cs_pin: "${csPin}"
      dc_pin: "${dcPin}"`);

    const csGpio = parseMcuPin(csPin);
    if (csGpio) {
      boardIoEntries.push(`  - id: "${part.id}"
    kind: "spi_device"
    peripheral: "${connection}"
    pin: ${csGpio.pin}
    signal: "input"
    active_high: true
    device_type: "pcd8544"`);
    }
  }

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
    if (part.type === 'ultrasonic') continue;
    if (part.type === 'pcd8544') continue;
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
external_devices:
${externalDeviceEntries.length > 0 ? externalDeviceEntries.join('\n') : '  []'}
board_io:
${boardIoEntries.length > 0 ? boardIoEntries.join('\n') : '  []'}
`;

  return { systemYaml, chipYaml };
}
