import { COMPONENT_REGISTRY } from './components/index';
import { findPinFunction, getPinMapping } from './pin-mapping';
import type { Diagram, WireEndpoint } from './types';
import { diagnoseDiagram } from './circuitDiagnostics';

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

function validateBoardIoPinCompatibility(board: string, mcuPin: string, kind: string): string | null {
  const pin = getPinMapping(board, mcuPin);
  if (!pin) {
    return `Pin ${mcuPin} is not available on this board model.`;
  }

  if (kind === 'adc_input' && !findPinFunction(board, mcuPin, 'adc')) {
    return `${mcuPin} does not expose ADC input on this board.`;
  }
  if (kind === 'pwm_output' && !findPinFunction(board, mcuPin, 'timer')) {
    return `${mcuPin} does not expose a timer/PWM output on this board.`;
  }
  if (kind === 'i2c_device' && !findPinFunction(board, mcuPin, 'i2c')) {
    return `${mcuPin} is not an I2C-capable pin on this board.`;
  }
  if (kind === 'spi_device' && !findPinFunction(board, mcuPin, 'spi')) {
    return `${mcuPin} is not an SPI-capable pin on this board.`;
  }

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
  if (mcuEndpoint && boardIoRole.boardIoKind) {
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
