import type { Diagram } from './types';
import { COMPONENT_META } from './component-meta';
import { findPinFunction } from './pin-mapping';
import { CHIP_YAMLS } from './chip-yamls';

const DEFAULT_CPU_HZ_BY_BOARD: Record<string, number> = {
  stm32l476: 250_000,
};

// I2C sensor/display devices the engine models, with their default bus address.
// Building one from a diagram needs an external_devices entry plus an
// i2c_device board_io binding (these attach by address, no chip-select line).
const I2C_DEVICE_ADDRESSES: Record<string, number> = {
  adxl345: 0x53,
  mpu6050: 0x68,
  bme280: 0x76,
  'oled-ssd1306': 0x3c,
};

// SPI display/sensor devices the engine models, addressed by a chip-select pin
// — same shape as the PCD8544 path below, minus the data/command line.
const SPI_DEVICE_TYPES = new Set(['ili9341', 'max31855', 'ssd1680_tricolor_290']);

/**
 * Parse an MCU pin label into { peripheral, pin } for various naming conventions.
 * Supports: PA5 (STM32), D0/A0 (Arduino), GPIO0 (ESP32), GP0 (RPi Pico), P0.00 (nRF).
 */
function parseMcuPin(pinLabel: string): { peripheral: string; pin: number } | null {
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

  const uartPeripheralForPart = (partId: string): string | null => {
    for (const pinId of ['RX', 'TX']) {
      const mcuPin = mcuPinForPartPin(partId, pinId);
      if (!mcuPin) continue;
      const uart = findPinFunction(diagram.board, mcuPin, 'uart');
      if (uart) return uart.peripheral;
    }
    return null;
  };

  const i2cPeripheralForPart = (partId: string): string | null => {
    for (const pinId of ['SDA', 'SCL']) {
      const mcuPin = mcuPinForPartPin(partId, pinId);
      if (!mcuPin) continue;
      const i2c = findPinFunction(diagram.board, mcuPin, 'i2c');
      if (i2c) return i2c.peripheral;
    }
    return null;
  };

  for (const part of diagram.parts) {
    if (part.type !== 'ultrasonic') continue;

    const trigPin = mcuPinForPartPin(part.id, 'TRIG');
    const echoPin = mcuPinForPartPin(part.id, 'ECHO');
    if (!trigPin || !echoPin) continue;

    const distance = Number.parseFloat(part.attrs?.distance ?? '');
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

  for (const part of diagram.parts) {
    if (part.type !== 'sn74hc165') continue;

    const connection = spiPeripheralForPart(part.id);
    const csPin = mcuPinForPartPin(part.id, 'SH_LD');
    if (!connection || !csPin) continue;

    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "sn74hc165"
    connection: "${connection}"
    config:
      cs_pin: "${csPin}"
      inputs: ${Number.parseInt(part.attrs?.inputs ?? '165', 10) || 165}`);
  }

  for (const part of diagram.parts) {
    if (part.type !== 'iolink-master') continue;

    const connection = uartPeripheralForPart(part.id);
    if (!connection) continue;

    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "iolink-master"
    connection: "${connection}"
    config:
      pd_in_len: 1
      m_seq_type: 1
      com: "COM2"`);
  }

  // I2C sensors/displays (ADXL345, MPU6050, BME280, SSD1306 OLED). Attach by
  // address on the resolved I2C bus; emit both the device model and its
  // i2c_device board_io binding so the engine wires it up.
  for (const part of diagram.parts) {
    const address = I2C_DEVICE_ADDRESSES[part.type];
    if (address === undefined) continue;
    const connection = i2cPeripheralForPart(part.id);
    if (!connection) continue;
    const addr = `0x${address.toString(16)}`;
    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "${part.type}"
    connection: "${connection}"
    config:
      i2c_address: ${addr}`);
    boardIoEntries.push(`  - id: "${part.id}"
    kind: "i2c_device"
    peripheral: "${connection}"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: ${addr}
    device_type: "${part.type}"`);
  }

  // SPI displays/sensors addressed by a chip-select pin (ILI9341 TFT, MAX31855
  // thermocouple, SSD1680 tri-color e-paper). Mirrors the PCD8544 path.
  for (const part of diagram.parts) {
    if (!SPI_DEVICE_TYPES.has(part.type)) continue;
    const connection = spiPeripheralForPart(part.id);
    const csPin = mcuPinForPartPin(part.id, 'CS');
    if (!connection || !csPin) continue;
    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "${part.type}"
    connection: "${connection}"
    config:
      cs_pin: "${csPin}"`);
    const csGpio = parseMcuPin(csPin);
    if (csGpio) {
      boardIoEntries.push(`  - id: "${part.id}"
    kind: "spi_device"
    peripheral: "${connection}"
    pin: ${csGpio.pin}
    signal: "input"
    active_high: true
    device_type: "${part.type}"`);
    }
  }

  // UART stream device: NEO-6M GPS. Emits NMEA on the resolved UART.
  for (const part of diagram.parts) {
    if (part.type !== 'neo6m-gps') continue;
    const connection = uartPeripheralForPart(part.id);
    if (!connection) continue;
    externalDeviceEntries.push(`  - id: "${part.id}"
    type: "neo6m-gps"
    connection: "${connection}"
    config: {}`);
    boardIoEntries.push(`  - id: "${part.id}"
    kind: "uart_device"
    peripheral: "${connection}"
    pin: 0
    signal: "input"
    active_high: true
    device_type: "neo6m-gps"`);
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
    if (part.type === 'sn74hc165') continue;
    if (part.type === 'iolink-master') continue;
    if (part.type === 'neo6m-gps') continue;
    if (I2C_DEVICE_ADDRESSES[part.type] !== undefined) continue;
    if (SPI_DEVICE_TYPES.has(part.type)) continue;
    const boardIoKind = COMPONENT_META[part.type]?.boardIoKind;
    if (!boardIoKind) continue;

    // Parse MCU pin label to get GPIO peripheral + pin number
    const gpioPin = parseMcuPin(mcuEnd.pin);
    if (!gpioPin) continue;

    // Determine signal direction based on boardIoKind
    const signal = (boardIoKind === 'button' || boardIoKind === 'adc_input') ? 'input' : 'output';

    // For ADC inputs, try to resolve the actual ADC peripheral from pin mapping
    let peripheral = gpioPin.peripheral;
    if (boardIoKind === 'adc_input') {
      const adcFunc = findPinFunction(diagram.board, mcuEnd.pin, 'adc');
      if (adcFunc) {
        peripheral = adcFunc.peripheral;
      }
    }

    boardIoEntries.push(`  - id: "${part.id}"
    kind: "${boardIoKind}"
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
