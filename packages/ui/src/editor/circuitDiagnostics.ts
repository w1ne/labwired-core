/**
 * Structured diagram diagnostics shared by the playground UI and MCP server.
 *
 * Each finding has a stable `code` an agent can branch on. The message is for
 * humans; the optional `fix` is a one-line suggestion.
 *
 * The legacy `validateDiagram(diagram): string[]` in ./circuitValidation is now
 * a thin adapter over `diagnoseDiagram` — single source of truth.
 */

import type { Diagram, WireEndpoint } from './types';
import { COMPONENT_REGISTRY } from './components/index';
import { findPinFunction, getPinMapping } from './pin-mapping';

export type DiagnosticSeverity = 'error' | 'warning';

/** Stable, machine-readable error codes. New codes are additive. */
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
  | 'BOARDIO_ALREADY_WIRED'
  | 'BOARDIO_MULTIPLE_WIRES'
  | 'PIN_OVERLOADED'
  | 'NO_MCU'
  | 'COMPONENT_DANGLING';

export interface Diagnostic {
  severity: DiagnosticSeverity;
  code: DiagnosticCode;
  message: string;
  /** Where it applies — useful for highlighting in the UI / pointing the agent. */
  location?: { part_id?: string; pin?: string };
  /** One-line suggestion the agent (or human) can act on. */
  fix?: string;
}

function getPart(diagram: Diagram, endpoint: WireEndpoint) {
  return diagram.parts.find((part) => part.id === endpoint.part) ?? null;
}

function getRole(diagram: Diagram, endpoint: WireEndpoint) {
  const part = getPart(diagram, endpoint);
  if (!part) return { part: null, def: null, isMcu: false, boardIoKind: null as string | null };
  const def = COMPONENT_REGISTRY.get(part.type) ?? null;
  return {
    part,
    def,
    isMcu: def?.category === 'mcu' || part.id === 'mcu',
    boardIoKind: def?.boardIoKind ?? null,
  };
}

/** Check a single MCU pin / boardIoKind compatibility. */
/** Power-rail pins are universal — every chip has VCC + GND. Decorative wires
 *  for these don't carry signal that needs alt-function support, so they
 *  bypass pin compatibility checks. */
const POWER_PINS = new Set(['VCC', 'GND', '3V3', '5V', 'VIN', 'VBUS', 'VDD', 'VSS']);

function pinCompatibilityDiag(board: string, mcuPin: string, kind: string, partId?: string): Diagnostic | null {
  if (POWER_PINS.has(mcuPin.toUpperCase())) return null;
  const pin = getPinMapping(board, mcuPin);
  if (!pin) {
    return {
      severity: 'error',
      code: 'PIN_NOT_ON_CHIP',
      message: `Pin ${mcuPin} is not available on this board model.`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'Pick a pin that exists on the selected board — check labwired_list_boards or the board datasheet.',
    };
  }
  // Alt-function "lacks" diagnostics are warnings, not errors. A wire from an
  // MCU pin to e.g. a 'spi_device' may be either a real SPI signal (MOSI/SCK/CS,
  // which need spi alt-function) or a plain GPIO control line (DC/RST/BUSY on
  // an SSD1680 e-paper, which don't). We can't tell from the boardIoKind alone,
  // so we hint rather than block.
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
      fix: 'I2C SDA/SCL signals must go to I2C-capable pins (STM32F1: I2C1 SDA=PB7 SCL=PB6).',
    };
  }
  if (kind === 'spi_device' && !findPinFunction(board, mcuPin, 'spi')) {
    return {
      severity: 'warning',
      code: 'PIN_LACKS_SPI',
      message: `${mcuPin} isn't SPI-capable — fine if this is a control line (DC/RST/BUSY/etc.).`,
      location: { pin: mcuPin, part_id: partId },
      fix: 'SPI MOSI/MISO/SCK signals must go to SPI-capable pins (STM32F1: SPI1 SCK=PA5 MOSI=PA7).',
    };
  }
  return null;
}

/** Per-wire structural validation. */
function diagnoseWireEndpoints(diagram: Diagram, from: WireEndpoint, to: WireEndpoint): Diagnostic | null {
  const a = getRole(diagram, from);
  const b = getRole(diagram, to);

  if (!a.part || !b.part || !a.def || !b.def) {
    return {
      severity: 'error',
      code: 'WIRE_INVALID_PART',
      message: 'Both ends of a wire must connect to known components.',
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
      message: `${boardIoEnd.def?.label ?? 'This component'} must connect directly to the MCU.`,
      location: { part_id: boardIoEnd.part!.id },
      fix: 'Route the wire from this component\'s pin directly to an MCU pin (no intermediate components).',
    };
  }

  const mcuPin = otherEnd === a ? from.pin : to.pin;
  return pinCompatibilityDiag(diagram.board, mcuPin, boardIoEnd.boardIoKind!, boardIoEnd.part!.id);
}

/**
 * Full-diagram diagnosis. Returns all findings (errors + warnings) the agent
 * or UI should know about. Empty array = clean.
 */
export function diagnoseDiagram(diagram: Diagram): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seenWireKey = new Set<string>();

  // 1. Per-wire endpoint validation + duplicate detection.
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

  // 2. MCU-pin double-assignment + component multiple-wires.
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
  // BOARDIO_MULTIPLE_WIRES only meaningful for simple GPIO components
  // (LED, button). SPI/I2C/UART devices need multiple wires by design
  // (e.g. an SPI display has MOSI/SCK/CS plus DC/RST + power).
  const SINGLE_WIRE_KINDS = new Set(['led', 'button', 'adc_input', 'pwm_output']);
  for (const [partId, count] of componentMcuWireCount) {
    if (count <= 1) continue;
    const part = diagram.parts.find((p) => p.id === partId);
    const def = part ? COMPONENT_REGISTRY.get(part.type) : null;
    if (!def?.boardIoKind || !SINGLE_WIRE_KINDS.has(def.boardIoKind)) continue;
    out.push({
      severity: 'error',
      code: 'BOARDIO_MULTIPLE_WIRES',
      message: `${def?.label ?? partId} has ${count} MCU connections; expected exactly one for board_io.`,
      location: { part_id: partId },
    });
  }

  // 3. Diagram-level warnings.
  const hasMcu = diagram.parts.some((p) => {
    const def = COMPONENT_REGISTRY.get(p.type);
    return def?.category === 'mcu' || p.id === 'mcu';
  });
  if (!hasMcu) {
    out.push({
      severity: 'error',
      code: 'NO_MCU',
      message: 'Diagram has no MCU. Add a board (e.g. STM32 Dev Board) before simulating.',
      fix: 'Drag an MCU component from the palette, then wire peripherals to its pins.',
    });
  }

  // 4. Dangling components — warning, not error (lets the agent ignore decoration parts).
  for (const part of diagram.parts) {
    const def = COMPONENT_REGISTRY.get(part.type);
    if (!def?.boardIoKind) continue;
    if ((componentMcuWireCount.get(part.id) ?? 0) === 0) {
      out.push({
        severity: 'warning',
        code: 'COMPONENT_DANGLING',
        message: `${def.label ?? part.id} has no MCU connection — it won't be simulated.`,
        location: { part_id: part.id },
        fix: 'Wire one of its pins to an MCU pin, or remove the component.',
      });
    }
  }

  // Dedup by (code, message) — guards against pathological cases where the same finding
  // gets emitted twice from different paths.
  const seen = new Set<string>();
  return out.filter((d) => {
    const k = `${d.code}|${d.message}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}
