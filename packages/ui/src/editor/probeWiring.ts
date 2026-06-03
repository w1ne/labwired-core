import { COMPONENT_REGISTRY } from './components/index';
import type { Diagram, WireEndpoint } from './types';

function getPinDef(diagram: Diagram, endpoint: WireEndpoint) {
  const part = diagram.parts.find((candidate) => candidate.id === endpoint.part);
  const def = part ? COMPONENT_REGISTRY.get(part.type) : null;
  return def?.pins.find((pin) => pin.id === endpoint.pin) ?? null;
}

export function isProbeEndpoint(diagram: Diagram, endpoint: WireEndpoint): boolean {
  return getPinDef(diagram, endpoint)?.probe === true;
}

export function hasProbeEndpoint(diagram: Diagram, from: WireEndpoint, to: WireEndpoint): boolean {
  return isProbeEndpoint(diagram, from) || isProbeEndpoint(diagram, to);
}
