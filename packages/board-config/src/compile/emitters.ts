/**
 * Shared YAML-fragment emitters extracted from diagram-to-config.ts.
 * Both compile() and the (now-delegating) diagramToConfig import from here
 * so the YAML output is string-identical for back-compat.
 */

import { COMPONENT_META } from '../component-meta';
import { findPinFunction } from '../pin-mapping';
import type { Diagram } from '../types';

const DEFAULT_CPU_HZ_BY_BOARD: Record<string, number> = {
  stm32l476: 250_000,
};

/** I2C sensor/display devices with their default bus address (legacy table). */
export const I2C_DEVICE_ADDRESSES: Record<string, number> = {
  adxl345: 0x53,
  mpu6050: 0x68,
  bme280: 0x76,
  'oled-ssd1306': 0x3c,
};

/** SPI display/sensor devices addressed by chip-select pin (legacy set). */
export const SPI_DEVICE_TYPES = new Set([
  'ili9341',
  'max31855',
  'ssd1680_tricolor_290',
]);

/**
 * Parse an MCU pin label into { peripheral, pin } for various naming conventions.
 * Supports: PA5 (STM32), D0/A0 (Arduino), GPIO0 (ESP32), GP0 (RPi Pico), P0.00 (nRF).
 */
export function parseMcuPin(pinLabel: string): { peripheral: string; pin: number } | null {
  // STM32: PA0, PB12, PC13, PD0, PE1, PH0 (covers all L476 ports A-H)
  const stm = pinLabel.match(/^P([A-H])(\d+)$/i);
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

// ---------------------------------------------------------------------------
// Wire-based helpers (used by legacy path and by SPI/UART in compile())
// ---------------------------------------------------------------------------

export function mcuPinForPartPin(
  diagram: Diagram,
  partId: string,
  pinId: string,
): string | null {
  for (const wire of diagram.wires) {
    if (wire.from.part === 'mcu' && wire.to.part === partId && wire.to.pin === pinId) {
      return wire.from.pin;
    }
    if (wire.to.part === 'mcu' && wire.from.part === partId && wire.from.pin === pinId) {
      return wire.to.pin;
    }
  }
  return null;
}

export function spiPeripheralForPart(diagram: Diagram, partId: string): string | null {
  const spiPins = ['CLK', 'SCK', 'DIN', 'MOSI'];
  for (const pinId of spiPins) {
    const mcuPin = mcuPinForPartPin(diagram, partId, pinId);
    if (!mcuPin) continue;
    const spi = findPinFunction(diagram.board, mcuPin, 'spi');
    if (spi) return spi.peripheral;
  }
  return null;
}

export function uartPeripheralForPart(diagram: Diagram, partId: string): string | null {
  for (const pinId of ['RX', 'TX']) {
    const mcuPin = mcuPinForPartPin(diagram, partId, pinId);
    if (!mcuPin) continue;
    const uart = findPinFunction(diagram.board, mcuPin, 'uart');
    if (uart) return uart.peripheral;
  }
  return null;
}

export function i2cPeripheralForPartWire(diagram: Diagram, partId: string): string | null {
  for (const pinId of ['SDA', 'SCL']) {
    const mcuPin = mcuPinForPartPin(diagram, partId, pinId);
    if (!mcuPin) continue;
    const i2c = findPinFunction(diagram.board, mcuPin, 'i2c');
    if (i2c) return i2c.peripheral;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Fragment builders (pure string; used by both legacy and compile paths)
// ---------------------------------------------------------------------------

/** Emit the external_devices + board_io fragments for an ultrasonic part. */
export function emitUltrasonic(
  diagram: Diagram,
  partId: string,
): { externalDevice?: string; boardIo?: string } {
  const part = diagram.parts.find((p) => p.id === partId);
  if (!part) return {};
  const trigPin = mcuPinForPartPin(diagram, partId, 'TRIG');
  const echoPin = mcuPinForPartPin(diagram, partId, 'ECHO');
  if (!trigPin || !echoPin) return {};
  const distance = Number.parseFloat(part.attrs?.distance ?? '');
  const distanceCm = Number.isFinite(distance) ? distance : 100;
  return {
    externalDevice: `  - id: "${partId}"
    type: "hc-sr04"
    connection: "gpio"
    config:
      trig_pin: "${trigPin}"
      echo_pin: "${echoPin}"
      distance_cm: ${distanceCm}
      cpu_hz: ${DEFAULT_CPU_HZ_BY_BOARD[diagram.board] ?? 80_000_000}`,
  };
}

/** Emit the external_devices + board_io fragments for a pcd8544 part. */
export function emitPcd8544(
  diagram: Diagram,
  partId: string,
): { externalDevice?: string; boardIo?: string } {
  const connection = spiPeripheralForPart(diagram, partId);
  const csPin = mcuPinForPartPin(diagram, partId, 'CE');
  const dcPin = mcuPinForPartPin(diagram, partId, 'DC');
  if (!connection || !csPin || !dcPin) return {};
  const csGpio = parseMcuPin(csPin);
  return {
    externalDevice: `  - id: "${partId}"
    type: "pcd8544"
    connection: "${connection}"
    config:
      cs_pin: "${csPin}"
      dc_pin: "${dcPin}"`,
    boardIo: csGpio
      ? `  - id: "${partId}"
    kind: "spi_device"
    peripheral: "${connection}"
    pin: ${csGpio.pin}
    signal: "input"
    active_high: true
    device_type: "pcd8544"`
      : undefined,
  };
}

/** Emit the external_devices fragment for a sn74hc165 part. */
export function emitSn74hc165(
  diagram: Diagram,
  partId: string,
): { externalDevice?: string; boardIo?: string } {
  const part = diagram.parts.find((p) => p.id === partId);
  if (!part) return {};
  const connection = spiPeripheralForPart(diagram, partId);
  const csPin = mcuPinForPartPin(diagram, partId, 'SH_LD');
  if (!connection || !csPin) return {};
  return {
    externalDevice: `  - id: "${partId}"
    type: "sn74hc165"
    connection: "${connection}"
    config:
      cs_pin: "${csPin}"
      inputs: ${Number.parseInt(part.attrs?.inputs ?? '165', 10) || 165}`,
  };
}

/** Emit the external_devices fragment for an iolink-master part. */
export function emitIolinkMaster(
  diagram: Diagram,
  partId: string,
): { externalDevice?: string; boardIo?: string } {
  const connection = uartPeripheralForPart(diagram, partId);
  if (!connection) return {};
  return {
    externalDevice: `  - id: "${partId}"
    type: "iolink-master"
    connection: "${connection}"
    config:
      pd_in_len: 1
      m_seq_type: 1
      com: "COM2"`,
  };
}

/**
 * Emit the external_devices + board_io fragments for a legacy I2C device
 * (adxl345, mpu6050, bme280, oled-ssd1306).
 * `connection` is the resolved i2c peripheral name.
 */
export function emitLegacyI2cDevice(
  partId: string,
  partType: string,
  connection: string,
  address: number,
): { externalDevice: string; boardIo: string } {
  const addr = `0x${address.toString(16)}`;
  return {
    externalDevice: `  - id: "${partId}"
    type: "${partType}"
    connection: "${connection}"
    config:
      i2c_address: ${addr}`,
    boardIo: `  - id: "${partId}"
    kind: "i2c_device"
    peripheral: "${connection}"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: ${addr}
    device_type: "${partType}"`,
  };
}

/** Emit the external_devices + board_io fragments for an SPI device from the legacy SPI_DEVICE_TYPES set. */
export function emitSpiDevice(
  diagram: Diagram,
  partId: string,
  partType: string,
): { externalDevice?: string; boardIo?: string } {
  const connection = spiPeripheralForPart(diagram, partId);
  const csPin = mcuPinForPartPin(diagram, partId, 'CS');
  if (!connection || !csPin) return {};
  const csGpio = parseMcuPin(csPin);
  return {
    externalDevice: `  - id: "${partId}"
    type: "${partType}"
    connection: "${connection}"
    config:
      cs_pin: "${csPin}"`,
    boardIo: csGpio
      ? `  - id: "${partId}"
    kind: "spi_device"
    peripheral: "${connection}"
    pin: ${csGpio.pin}
    signal: "input"
    active_high: true
    device_type: "${partType}"`
      : undefined,
  };
}

/** Emit the external_devices + board_io fragments for a neo6m-gps part. */
export function emitNeo6mGps(
  diagram: Diagram,
  partId: string,
): { externalDevice?: string; boardIo?: string } {
  const connection = uartPeripheralForPart(diagram, partId);
  if (!connection) return {};
  return {
    externalDevice: `  - id: "${partId}"
    type: "neo6m-gps"
    connection: "${connection}"
    config: {}`,
    boardIo: `  - id: "${partId}"
    kind: "uart_device"
    peripheral: "${connection}"
    pin: 0
    signal: "input"
    active_high: true
    device_type: "neo6m-gps"`,
  };
}

/**
 * Emit board_io entries for point-to-point wires from legacy diagrams
 * (handles led, button, pwm_output, adc_input etc. via COMPONENT_META).
 * Skips parts that have dedicated emitters (ultrasonic, pcd8544, etc.).
 */
export function emitBoardIoFromWires(diagram: Diagram): string[] {
  const entries: string[] = [];
  const skipTypes = new Set([
    'ultrasonic', 'pcd8544', 'sn74hc165', 'iolink-master', 'neo6m-gps',
  ]);

  for (const wire of diagram.wires) {
    let mcuEnd: typeof wire.from | null = null;
    let compEnd: typeof wire.from | null = null;

    if (wire.from.part === 'mcu') {
      mcuEnd = wire.from;
      compEnd = wire.to;
    } else if (wire.to.part === 'mcu') {
      mcuEnd = wire.to;
      compEnd = wire.from;
    } else {
      continue;
    }

    const part = diagram.parts.find((p) => p.id === compEnd!.part);
    if (!part) continue;
    if (skipTypes.has(part.type)) continue;
    if (I2C_DEVICE_ADDRESSES[part.type] !== undefined) continue;
    if (SPI_DEVICE_TYPES.has(part.type)) continue;
    const boardIoKind = COMPONENT_META[part.type]?.boardIoKind;
    if (!boardIoKind) continue;

    const gpioPin = parseMcuPin(mcuEnd.pin);
    if (!gpioPin) continue;

    const signal = (boardIoKind === 'button' || boardIoKind === 'adc_input') ? 'input' : 'output';

    let peripheral = gpioPin.peripheral;
    if (boardIoKind === 'adc_input') {
      const adcFunc = findPinFunction(diagram.board, mcuEnd.pin, 'adc');
      if (adcFunc) {
        peripheral = adcFunc.peripheral;
      }
    }

    entries.push(`  - id: "${part.id}"
    kind: "${boardIoKind}"
    peripheral: "${peripheral}"
    pin: ${gpioPin.pin}
    signal: "${signal}"
    active_high: true`);
  }

  return entries;
}

/** Assemble the system YAML string from collected fragment arrays. */
export function buildSystemYaml(
  externalDeviceEntries: string[],
  boardIoEntries: string[],
): string {
  return `name: "playground-board"
chip: "inline"
external_devices:
${externalDeviceEntries.length > 0 ? externalDeviceEntries.join('\n') : '  []'}
board_io:
${boardIoEntries.length > 0 ? boardIoEntries.join('\n') : '  []'}
`;
}
