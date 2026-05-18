/**
 * Pure-data diagram diagnostics for `labwired_validate_diagram`.
 *
 * Mirror of packages/ui/src/editor/circuitDiagnostics.ts but with NO React
 * imports — the MCP server is a node stdio process and shouldn't pull in
 * UI deps. Component metadata lives in ./component-meta, pin alternate
 * functions in ./pin-mapping (copied from @labwired/ui at v0.13).
 *
 * If a finding's wording or code changes here, sync the UI version.
 */
import { getComponentMeta } from './component-meta.js';
import { findPinFunction, getPinMapping } from './pin-mapping.js';

export type DiagnosticSeverity = 'error' | 'warning';
export type DiagnosticCode =
  | 'PIN_NOT_ON_CHIP'
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

interface Role {
  part: DiagramPart | null;
  meta: ReturnType<typeof getComponentMeta>;
  isMcu: boolean;
  boardIoKind: string | null;
}

function getPart(diagram: ValidateDiagram, endpoint: WireEndpoint): DiagramPart | null {
  return diagram.parts.find((p) => p.id === endpoint.part) ?? null;
}

function getRole(diagram: ValidateDiagram, endpoint: WireEndpoint): Role {
  const part = getPart(diagram, endpoint);
  if (!part) return { part: null, meta: null, isMcu: false, boardIoKind: null };
  const meta = getComponentMeta(part.type);
  return {
    part,
    meta,
    isMcu: meta?.category === 'mcu' || part.id === 'mcu',
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
  if (kind === 'adc_input' && !findPinFunction(board, mcuPin, 'adc')) {
    return {
      severity: 'error',
      code: 'PIN_LACKS_ADC',
      message: `${mcuPin} does not expose ADC input on this board.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'Route the analog sensor to an ADC-capable pin (commonly PA0-PA7 on STM32F1).',
    };
  }
  if (kind === 'pwm_output' && !findPinFunction(board, mcuPin, 'timer')) {
    return {
      severity: 'error',
      code: 'PIN_LACKS_PWM',
      message: `${mcuPin} does not expose a timer/PWM output on this board.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'Route to a pin with timer alternate function.',
    };
  }
  if (kind === 'i2c_device' && !findPinFunction(board, mcuPin, 'i2c')) {
    return {
      severity: 'error',
      code: 'PIN_LACKS_I2C',
      message: `${mcuPin} is not an I2C-capable pin on this board.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'On STM32F1, I2C1 SDA=PB7, SCL=PB6; I2C2 SDA=PB11, SCL=PB10.',
    };
  }
  if (kind === 'spi_device' && !findPinFunction(board, mcuPin, 'spi')) {
    return {
      severity: 'error',
      code: 'PIN_LACKS_SPI',
      message: `${mcuPin} is not an SPI-capable pin on this board.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'On STM32F1, SPI1 SCK=PA5 MISO=PA6 MOSI=PA7.',
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
      fix: "Route the wire directly to an MCU pin — no intermediate components.",
    };
  }

  const mcuPin = otherEnd === a ? from.pin : to.pin;
  return pinCompatibilityDiag(diagram.board, mcuPin, boardIoEnd.boardIoKind!, boardIoEnd.part!.id);
}

export function diagnoseDiagram(diagram: ValidateDiagram): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seenWireKey = new Set<string>();

  for (const wire of diagram.wires) {
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

  const mcuPinAssignments = new Map<string, string>();
  const componentMcuWireCount = new Map<string, number>();
  for (const wire of diagram.wires) {
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
  for (const [partId, count] of componentMcuWireCount) {
    if (count > 1) {
      const part = diagram.parts.find((p) => p.id === partId);
      const meta = part ? getComponentMeta(part.type) : null;
      out.push({
        severity: 'error',
        code: 'BOARDIO_MULTIPLE_WIRES',
        message: `${meta?.label ?? partId} has ${count} MCU connections; expected exactly one for board_io.`,
        location: { part_id: partId },
      });
    }
  }

  const hasMcu = diagram.parts.some((p) => {
    const m = getComponentMeta(p.type);
    return m?.category === 'mcu' || p.id === 'mcu';
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
    const meta = getComponentMeta(part.type);
    if (!meta?.boardIoKind) continue;
    if ((componentMcuWireCount.get(part.id) ?? 0) === 0) {
      out.push({
        severity: 'warning',
        code: 'COMPONENT_DANGLING',
        message: `${meta.label} has no MCU connection — it won't be simulated.`,
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
