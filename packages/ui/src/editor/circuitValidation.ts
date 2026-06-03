import { COMPONENT_REGISTRY } from './components/index';
import { getPinMapping } from './pin-mapping';
import type { Diagram, WireEndpoint } from './types';
import { diagnoseDiagram } from './circuitDiagnostics';
import { hasProbeEndpoint, isProbeEndpoint } from './probeWiring';

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

/** Power-rail pin names every board has — bypass alt-function checks. */
const POWER_PINS = new Set(['VCC', 'GND', '3V3', '5V', 'VIN', 'VBUS', 'VDD', 'VSS']);

function validateBoardIoPinCompatibility(board: string, mcuPin: string, kind: string): string | null {
  if (POWER_PINS.has(mcuPin.toUpperCase())) return null;
  const pin = getPinMapping(board, mcuPin);
  if (!pin) {
    return `Pin ${mcuPin} is not available on this board model.`;
  }

  // Alt-function "lacks" checks intentionally removed — see circuitDiagnostics
  // for rationale. A wire from MCU.PB0 → e-paper.DC is valid even though PB0
  // isn't an SPI alt-function pin, because DC is a plain GPIO control line.
  // The boardIoKind is component-aggregate; we can't tell from one wire which
  // signal it carries.
  void kind;
  return null;
}

function validateWireEndpoints(diagram: Diagram, from: WireEndpoint, to: WireEndpoint): string | null {
  const a = getRole(diagram, from);
  const b = getRole(diagram, to);

  if (!a.part || !b.part || !a.def || !b.def) {
    return 'Both ends of a wire must connect to known components.';
  }

  if (a.part.id === b.part.id) {
    return 'A component cannot be wired to itself.';
  }

  const boardIoEnd = a.boardIoKind ? a : b.boardIoKind ? b : null;
  const otherEnd = boardIoEnd === a ? b : a;

  if (boardIoEnd) {
    if (hasProbeEndpoint(diagram, from, to)) {
      return null;
    }
    if (!otherEnd.isMcu) {
      return `${boardIoEnd.def?.label ?? 'This component'} must connect directly to the MCU.`;
    }

    const compatibilityError = validateBoardIoPinCompatibility(
      diagram.board,
      otherEnd === a ? from.pin : to.pin,
      boardIoEnd.boardIoKind!,
    );
    if (compatibilityError) return compatibilityError;
  }

  return null;
}

export function validateWireConnection(diagram: Diagram, from: WireEndpoint, to: WireEndpoint): string | null {
  const basicError = validateWireEndpoints(diagram, from, to);
  if (basicError) return basicError;

  for (const wire of diagram.wires) {
    const sameDirection = wire.from.part === from.part && wire.from.pin === from.pin
      && wire.to.part === to.part && wire.to.pin === to.pin;
    const reverseDirection = wire.from.part === to.part && wire.from.pin === to.pin
      && wire.to.part === from.part && wire.to.pin === from.pin;
    if (sameDirection || reverseDirection) {
      return 'Those two pins are already connected.';
    }
  }

  const newEndpoints = [from, to];
  for (const endpoint of newEndpoints) {
    const role = getRole(diagram, endpoint);
    if (!role.boardIoKind) continue;
    if (newEndpoints.some((candidate) => isProbeEndpoint(diagram, candidate))) continue;

    const existing = diagram.wires.find((wire) =>
      (wire.from.part === endpoint.part && wire.to.part === 'mcu')
      || (wire.to.part === endpoint.part && wire.from.part === 'mcu'),
    );
    if (existing) {
      return `${role.def?.label ?? 'This component'} already has an MCU connection.`;
    }
  }

  const mcuEndpoint = getRole(diagram, from).isMcu ? from : getRole(diagram, to).isMcu ? to : null;
  const boardIoRole = getRole(diagram, from).boardIoKind ? getRole(diagram, from) : getRole(diagram, to);
  // Power rails fan out to every peripheral — don't block a second VCC/GND/3V3
  // wire the way we block a second signal-pin assignment.
  if (mcuEndpoint && boardIoRole.boardIoKind && !POWER_PINS.has(mcuEndpoint.pin.toUpperCase())) {
    const collision = diagram.wires.find((wire) => {
      const endpoint = wire.from.part === 'mcu' ? wire.from : wire.to.part === 'mcu' ? wire.to : null;
      if (!endpoint) return false;
      if (endpoint.pin !== mcuEndpoint.pin) return false;
      const otherPartId = wire.from.part === 'mcu' ? wire.to.part : wire.from.part;
      if (otherPartId === from.part || otherPartId === to.part) return false;
      const otherPart = diagram.parts.find((part) => part.id === otherPartId);
      const otherDef = otherPart ? COMPONENT_REGISTRY.get(otherPart.type) : null;
      return !!otherDef?.boardIoKind;
    });
    if (collision) {
      return `MCU pin ${mcuEndpoint.pin} is already assigned to another functional component.`;
    }
  }

  return null;
}

/**
 * Legacy string[] API for callers that don't need structured diagnostics
 * (current playground toast surface). New code should use diagnoseDiagram()
 * from ./circuitDiagnostics — that's the single source of truth.
 */
export function validateDiagram(diagram: Diagram): string[] {
  return diagnoseDiagram(diagram)
    .filter((d) => d.severity === 'error')
    .map((d) => d.message);
}
