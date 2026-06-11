// Diagram schema v2: first-class named nets with point-to-point wires kept
// as accepted legacy sugar. v1 diagrams (and versionless input) migrate
// losslessly; validation and compilation operate on v2 only.

import type { Diagram, Part, Wire } from './types';

/** Kind of a declared net. */
export type NetKind = 'signal' | 'power';

/** Protocol meaning attached to a signal net. */
export type NetProtocol =
  | 'i2c_sda' | 'i2c_scl'
  | 'spi_mosi' | 'spi_miso' | 'spi_sck' | 'spi_cs'
  | 'uart_tx' | 'uart_rx'
  | 'pwm' | 'adc' | 'gpio' | 'irq';

/** A first-class named net. */
export interface NetDecl {
  name: string;
  kind: NetKind;
  /** Rail voltage in volts; meaningful when kind === 'power'. */
  voltage?: number;
  /** Protocol role of the net; meaningful when kind === 'signal'. */
  protocol?: NetProtocol;
}

/** A connection binds "partId:pinName" to a declared net name. */
export type Connection = [pinRef: string, netName: string];

/** Diagram schema v2. `wires` carries accepted v1 point-to-point sugar. */
export interface DiagramV2 {
  version: 2;
  board: string;
  parts: Part[];
  nets: NetDecl[];
  connections: Connection[];
  wires: Wire[];
}

/** A parsed "part:pin" reference. The pin segment may carry a ".N" suffix. */
export interface PinRef {
  part: string;
  pin: string;
}

/** Parse "partId:pinName" (pin may include a ".N" suffix). Null if malformed. */
export function parsePinRef(ref: string): PinRef | null {
  const idx = ref.indexOf(':');
  if (idx <= 0 || idx === ref.length - 1) return null;
  return { part: ref.slice(0, idx), pin: ref.slice(idx + 1) };
}

/**
 * Migrate any accepted diagram input to v2. v1 (or versionless) diagrams
 * wrap losslessly: parts/board/wires preserved, empty nets/connections.
 * v2 input passes through. Never mutates its input.
 */
export function migrateToV2(input: Diagram | DiagramV2): DiagramV2 {
  if ('version' in input && (input as DiagramV2).version === 2) {
    const v2 = input as DiagramV2;
    return {
      version: 2,
      board: v2.board,
      parts: [...v2.parts],
      nets: [...(v2.nets ?? [])],
      connections: [...(v2.connections ?? [])],
      wires: [...(v2.wires ?? [])],
    };
  }
  const v1 = input as Diagram;
  return {
    version: 2,
    board: v1.board,
    parts: [...v1.parts],
    nets: [],
    connections: [],
    wires: [...(v1.wires ?? [])],
  };
}
