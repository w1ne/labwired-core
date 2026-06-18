/**
 * Legacy 14-code diagram diagnostics — extracted from packages/mcp/src/diagnostics.ts.
 * Shares pin-mapping and catalog with the rest of board-config. This is the
 * authoritative implementation; packages/mcp re-exports from here.
 */
import { getCatalogPart } from './catalog';
import type { PinEtype } from './catalog';
import { findPinFunction, getPinMapping } from './pin-mapping';

// ---------------------------------------------------------------------------
// Component labels — kept here because the catalog doesn't carry human-readable
// labels (only deviceClass + boardIoKind). These drive diagnostic messages.
// ---------------------------------------------------------------------------
const COMPONENT_LABELS: Record<string, string> = {
  // MCU boards
  mcu: 'MCU',
  'arduino-uno': 'Arduino Uno',
  'stm32-dev': 'STM32 Dev Board',
  'nucleo-h563zi': 'NUCLEO-H563ZI',
  'nucleo-f401re': 'NUCLEO-F401RE',
  'stm32-blackpill': 'STM32 Black Pill',
  esp32: 'ESP32',
  'esp32-c3-supermini': 'ESP32-C3 Super Mini',
  'esp32-s3-zero': 'ESP32-S3-Zero',
  'rpi-pico': 'RPi Pico',
  'nrf52840-dk': 'nRF52840 DK',
  // Output
  led: 'LED',
  'rgb-led': 'RGB LED',
  buzzer: 'Buzzer',
  servo: 'Servo Motor',
  neopixel: 'NeoPixel Strip',
  // Input
  button: 'Push Button',
  potentiometer: 'Potentiometer',
  'slide-switch': 'Slide Switch',
  'dip-switch': 'DIP Switch',
  'rotary-encoder': 'Rotary Encoder',
  keypad: '4x4 Keypad',
  // Sensors
  dht22: 'DHT22 Sensor',
  'pir-sensor': 'PIR Sensor',
  ultrasonic: 'HC-SR04',
  ldr: 'Photoresistor',
  adxl345: 'ADXL345',
  bme280: 'BME280',
  max31855: 'MAX31855',
  mpu6050: 'MPU6050',
  'neo6m-gps': 'NEO-6M GPS',
  'ntc-thermistor': 'NTC Thermistor',
  // Displays
  'seven-segment': '7-Segment',
  lcd1602: 'LCD 16x2',
  'oled-ssd1306': 'OLED 128x64',
  'led-matrix': '8x8 LED Matrix',
  ili9341: 'ILI9341 TFT 240x320',
  ssd1680_tricolor_290: 'E-Paper 2.9" tri-color (SSD1680)',
  uc8151d_tricolor_290: 'E-Paper 2.9" tri-color (UC8151D)',
  pcd8544: 'PCD8544',
  // Passives + ICs
  resistor: 'Resistor',
  capacitor: 'Capacitor',
  diode: 'Diode',
  transistor: 'Transistor',
  '74hc595': '74HC595',
  sn74hc165: '74HC165',
  'iolink-master': 'IO-Link Master',
  l293d: 'L293D',
  pca9685: 'PCA9685',
  ir: 'IR Transceiver',
};

// ---------------------------------------------------------------------------
// Public types (re-exported from index.ts)
// ---------------------------------------------------------------------------

export type DiagnosticSeverity = 'error' | 'warning';
export type DiagnosticCode =
  | 'PIN_NOT_ON_CHIP'
  | 'PIN_NOT_ON_COMPONENT'
  | 'PIN_LACKS_ADC'
  | 'PIN_LACKS_PWM'
  | 'PIN_LACKS_I2C'
  | 'PIN_LACKS_SPI'
  | 'WIRE_INVALID_PART'
  | 'WIRE_SELF_LOOP'
  | 'WIRE_DUPLICATE'
  | 'BOARDIO_NOT_TO_MCU'
  | 'BOARDIO_MULTIPLE_WIRES'
  | 'PIN_OVERLOADED'
  | 'NO_MCU'
  | 'COMPONENT_DANGLING'
  | 'UNKNOWN_COMPONENT';

export interface Diagnostic {
  severity: DiagnosticSeverity;
  code: DiagnosticCode;
  message: string;
  location?: { part_id?: string; pin?: string };
  fix?: string;
}

// Diagram shape (subset used by validation) — duck-typed against the UI's full Diagram.
export interface DiagramPart {
  id: string;
  type: string;
}
export interface WireEndpoint {
  part: string;
  pin: string;
}
export interface DiagramWire {
  from: WireEndpoint;
  to: WireEndpoint;
}
export interface ValidateDiagram {
  board: string;
  parts: DiagramPart[];
  wires: DiagramWire[];
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

interface PartMeta {
  label: string | null;
  boardIoKind: string | null;
  isMcu: boolean;
}

function getPartMeta(type: string): PartMeta | null {
  const catalog = getCatalogPart(type);
  if (!catalog) return null;
  const label = COMPONENT_LABELS[type] ?? type;
  const isMcu = catalog.deviceClass === 'mcu';
  return {
    label,
    boardIoKind: catalog.boardIoKind ?? null,
    isMcu,
  };
}

interface Role {
  part: DiagramPart | null;
  meta: PartMeta | null;
  isMcu: boolean;
  boardIoKind: string | null;
}

function getPart(diagram: ValidateDiagram, endpoint: WireEndpoint): DiagramPart | null {
  return diagram.parts.find((p) => p.id === endpoint.part) ?? null;
}

function getRole(diagram: ValidateDiagram, endpoint: WireEndpoint): Role {
  const part = getPart(diagram, endpoint);
  if (!part) return { part: null, meta: null, isMcu: false, boardIoKind: null };
  const meta = getPartMeta(part.type);
  return {
    part,
    meta,
    isMcu: meta?.isMcu ?? part.id === 'mcu',
    boardIoKind: meta?.boardIoKind ?? null,
  };
}

/** Power-rail pin names that every board has. Decorative power wires for
 *  these bypass the alt-function check — they're not signal pins. */
const POWER_PINS = new Set(['VCC', 'GND', '3V3', '5V', 'VIN', 'VBUS', 'VDD', 'VSS']);

function pinCompatibilityDiag(
  board: string,
  mcuPin: string,
  kind: string,
  partId?: string,
): Diagnostic | null {
  if (POWER_PINS.has(mcuPin.toUpperCase())) return null;
  const pin = getPinMapping(board, mcuPin);
  if (!pin) {
    return {
      severity: 'error',
      code: 'PIN_NOT_ON_CHIP',
      message: `Pin ${mcuPin} is not available on this board model.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'Pick a pin that exists on the selected board.',
    };
  }
  // Alt-function "lacks" diagnostics are WARNINGS, not errors.
  if (kind === 'adc_input' && !findPinFunction(board, mcuPin, 'adc')) {
    return {
      severity: 'warning',
      code: 'PIN_LACKS_ADC',
      message: `${mcuPin} doesn't expose ADC input — fine if this is a digital control wire.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'For the analog signal, route to an ADC-capable pin (PA0-PA7 on STM32F1).',
    };
  }
  if (kind === 'pwm_output' && !findPinFunction(board, mcuPin, 'timer')) {
    return {
      severity: 'warning',
      code: 'PIN_LACKS_PWM',
      message: `${mcuPin} doesn't expose a timer/PWM output — fine if this is a digital control wire.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'For the PWM signal, route to a pin with timer alternate function.',
    };
  }
  if (kind === 'i2c_device' && !findPinFunction(board, mcuPin, 'i2c')) {
    return {
      severity: 'warning',
      code: 'PIN_LACKS_I2C',
      message: `${mcuPin} isn't I2C-capable — fine if this is a control line (RST/INT/etc.).`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'I2C SDA/SCL signals must go to I2C-capable pins.',
    };
  }
  if (kind === 'spi_device' && !findPinFunction(board, mcuPin, 'spi')) {
    return {
      severity: 'warning',
      code: 'PIN_LACKS_SPI',
      message: `${mcuPin} isn't SPI-capable — fine if this is a control line (DC/RST/BUSY/etc.).`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'SPI MOSI/MISO/SCK signals must go to SPI-capable pins.',
    };
  }
  return null;
}

function diagnoseWireEndpoints(
  diagram: ValidateDiagram,
  from: WireEndpoint,
  to: WireEndpoint,
): Diagnostic | null {
  const a = getRole(diagram, from);
  const b = getRole(diagram, to);

  if (!a.part || !b.part) {
    return {
      severity: 'error',
      code: 'WIRE_INVALID_PART',
      message: `Wire endpoint references unknown part: ${!a.part ? from.part : to.part}.`,
    };
  }
  if (!a.meta || !b.meta) {
    return {
      severity: 'error',
      code: 'UNKNOWN_COMPONENT',
      message: `Component type "${(!a.meta ? a.part : b.part).type}" not in registry. Did you misspell it?`,
      location: { part_id: !a.meta ? a.part.id : b.part.id },
    };
  }
  if (a.part.id === b.part.id) {
    return {
      severity: 'error',
      code: 'WIRE_SELF_LOOP',
      message: 'A component cannot be wired to itself.',
      location: { part_id: a.part.id },
    };
  }

  const boardIoEnd = a.boardIoKind ? a : b.boardIoKind ? b : null;
  const otherEnd = boardIoEnd === a ? b : a;
  if (!boardIoEnd) return null;

  if (!otherEnd.isMcu) {
    return {
      severity: 'error',
      code: 'BOARDIO_NOT_TO_MCU',
      message: `${boardIoEnd.meta?.label ?? 'This component'} must connect directly to the MCU.`,
      location: { part_id: boardIoEnd.part!.id },
      fix: 'Route the wire directly to an MCU pin — no intermediate components.',
    };
  }

  const mcuPin = otherEnd === a ? from.pin : to.pin;
  return pinCompatibilityDiag(diagram.board, mcuPin, boardIoEnd.boardIoKind!, boardIoEnd.part!.id);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export function diagnoseDiagram(diagram: ValidateDiagram): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seenWireKey = new Set<string>();
  // Default wires to [] so diagrams without wires don't throw.
  const wires = diagram.wires ?? [];

  for (const wire of wires) {
    const key = `${wire.from.part}:${wire.from.pin}->${wire.to.part}:${wire.to.pin}`;
    const reverseKey = `${wire.to.part}:${wire.to.pin}->${wire.from.part}:${wire.from.pin}`;
    if (seenWireKey.has(key) || seenWireKey.has(reverseKey)) {
      out.push({
        severity: 'error',
        code: 'WIRE_DUPLICATE',
        message: `Duplicate wire ${wire.from.part}.${wire.from.pin} ↔ ${wire.to.part}.${wire.to.pin}.`,
      });
      continue;
    }
    seenWireKey.add(key);
    const d = diagnoseWireEndpoints(diagram, wire.from, wire.to);
    if (d) out.push(d);
  }

  // Pin-existence: a wire endpoint must reference a real pin on the part. This
  // is what stops "looks wired but isn't" diagrams — a wire to a hallucinated
  // pin (rgb-led.DIN, buzzer.PWM, button.OUT) resolves to nothing in the
  // renderer, so the part shows as disconnected. Only parts with TYPED pins are
  // checked here; MCU pins come from PIN_MAPS and are validated via
  // PIN_NOT_ON_CHIP, and pin-less legacy parts are skipped (can't check yet).
  for (const wire of wires) {
    for (const ep of [wire.from, wire.to] as const) {
      const part = diagram.parts.find((p) => p.id === ep.part);
      if (!part) continue; // unknown part → WIRE_INVALID_PART already covers it
      const cat = getCatalogPart(part.type);
      if (!cat?.pins?.length) continue; // MCU / legacy pin-less part
      if (cat.pins.some((pin) => pin.name === ep.pin)) continue;
      const valid = cat.pins.map((pin) => pin.name).join(', ');
      out.push({
        severity: 'error',
        code: 'PIN_NOT_ON_COMPONENT',
        message: `Pin "${ep.pin}" does not exist on ${COMPONENT_LABELS[part.type] ?? part.type} (${part.id}). Valid pins: ${valid}.`,
        location: { part_id: part.id, pin: ep.pin },
        fix: `Wire to one of this component's actual pins: ${valid}.`,
      });
    }
  }

  const mcuPinAssignments = new Map<string, string>();
  const componentMcuWireCount = new Map<string, number>();
  for (const wire of wires) {
    const mcuEndpoint = getRole(diagram, wire.from).isMcu
      ? wire.from
      : getRole(diagram, wire.to).isMcu
        ? wire.to
        : null;
    const otherEndpoint = mcuEndpoint === wire.from ? wire.to : mcuEndpoint === wire.to ? wire.from : null;
    if (!mcuEndpoint || !otherEndpoint) continue;
    const otherRole = getRole(diagram, otherEndpoint);
    if (!otherRole.boardIoKind) continue;
    const partId = otherEndpoint.part;
    componentMcuWireCount.set(partId, (componentMcuWireCount.get(partId) ?? 0) + 1);
    const existingPart = mcuPinAssignments.get(mcuEndpoint.pin);
    if (existingPart && existingPart !== partId) {
      out.push({
        severity: 'error',
        code: 'PIN_OVERLOADED',
        message: `MCU pin ${mcuEndpoint.pin} is assigned to multiple functional components.`,
        location: { pin: mcuEndpoint.pin },
        fix: `Route ${partId} to a different MCU pin, or disconnect ${existingPart}.`,
      });
    }
    mcuPinAssignments.set(mcuEndpoint.pin, partId);
  }

  // Only flag multiple MCU wires for simple-GPIO components — SPI/I2C/UART
  // devices legitimately need multiple wires (MOSI/SCK/CS + control + power).
  const SINGLE_WIRE_KINDS = new Set(['led', 'button', 'adc_input', 'pwm_output']);
  // Active signal etypes: a pin that drives or is driven by an MCU GPIO. A part
  // that declares more than one of these (e.g. HC-SR04 with TRIG + ECHO) is
  // inherently multi-wire, so the single-wire rule must not apply to it — even
  // though its legacy boardIoKind is 'button'. Without this, the hard
  // validation gate would reject every legitimate ultrasonic/multi-signal board.
  const SIGNAL_ETYPES = new Set<PinEtype>(['input', 'output', 'bidirectional', 'open_drain', 'tri_state']);
  for (const [partId, count] of componentMcuWireCount) {
    if (count <= 1) continue;
    const part = diagram.parts.find((p) => p.id === partId);
    const meta = part ? getPartMeta(part.type) : null;
    if (!meta?.boardIoKind || !SINGLE_WIRE_KINDS.has(meta.boardIoKind)) continue;
    const cat = part ? getCatalogPart(part.type) : undefined;
    const signalPins = cat?.pins?.filter((pin) => SIGNAL_ETYPES.has(pin.etype)) ?? [];
    if (signalPins.length > 1) continue;
    out.push({
      severity: 'error',
      code: 'BOARDIO_MULTIPLE_WIRES',
      message: `${meta.label ?? partId} has ${count} MCU connections; expected exactly one for board_io.`,
      location: { part_id: partId },
    });
  }

  const hasMcu = diagram.parts.some((p) => {
    const meta = getPartMeta(p.type);
    return meta?.isMcu ?? p.id === 'mcu';
  });
  if (!hasMcu) {
    out.push({
      severity: 'error',
      code: 'NO_MCU',
      message: 'Diagram has no MCU. Add a board before simulating.',
      fix: 'Add an MCU component (e.g. stm32-dev) and wire peripherals to its pins.',
    });
  }

  for (const part of diagram.parts) {
    const meta = getPartMeta(part.type);
    if (!meta?.boardIoKind) continue;
    if ((componentMcuWireCount.get(part.id) ?? 0) === 0) {
      out.push({
        severity: 'warning',
        code: 'COMPONENT_DANGLING',
        message: `${meta.label ?? part.id} has no MCU connection — it won't be simulated.`,
        location: { part_id: part.id },
        fix: 'Wire one of its pins to an MCU pin, or remove the component.',
      });
    }
  }

  const seen = new Set<string>();
  return out.filter((d) => {
    const k = `${d.code}|${d.message}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}
