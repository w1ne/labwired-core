// Deterministic wires→nets resolution. Declared nets are authoritative and
// are never merged with each other; legacy point-to-point wires union into
// the declared net they touch, or into synthetic nets named after their
// lexicographically smallest member ("net@part:pin"). Same input always
// produces the same output regardless of array order.

import type { DiagramV2, NetKind, NetProtocol, PinRef } from './schema';
import { parsePinRef } from './schema';

/** A net after resolution: declaration metadata plus its member pins. */
export interface ResolvedNet {
  name: string;
  kind: NetKind;
  voltage: number | undefined;
  protocol: NetProtocol | undefined;
  /** True when the net was declared in `nets`; false for wire-synthesized. */
  declared: boolean;
  /** Member pins, sorted by "part:pin" for determinism. */
  members: PinRef[];
}

const key = (m: PinRef) => `${m.part}:${m.pin}`;

/**
 * Resolve declared nets + legacy wires into the canonical net set.
 *
 * Output order: declared nets appear first in the order they appear in
 * `diagram.nets`; synthetic (wire-only) nets follow, sorted by their
 * generated name (i.e. lexicographic order of the smallest member key).
 * Net membership itself is order-independent; callers must not rely on
 * member array order beyond the per-net sort by "part:pin".
 */
export function resolveNets(diagram: DiagramV2): ResolvedNet[] {
  // Union-find over pin keys, seeded so that pins bound to a declared net
  // belong to that net's component and components of two declared nets are
  // never merged (bridges are preserved as shared members instead).
  const parent = new Map<string, string>();
  const find = (k: string): string => {
    let p = parent.get(k) ?? k;
    if (p !== k) {
      p = find(p);
      parent.set(k, p);
    }
    return p;
  };
  const union = (a: string, b: string) => {
    const ra = find(a);
    const rb = find(b);
    if (ra === rb) return;
    // Deterministic root: lexicographically smaller key wins.
    if (ra < rb) parent.set(rb, ra);
    else parent.set(ra, rb);
  };

  // Declared memberships: net name -> sorted unique member set.
  const declaredMembers = new Map<string, Map<string, PinRef>>();
  for (const net of diagram.nets) declaredMembers.set(net.name, new Map());
  for (const [ref, netName] of diagram.connections) {
    const pin = parsePinRef(ref);
    if (!pin) continue; // malformed refs are ERC's to report (Plan B)
    declaredMembers.get(netName)?.set(key(pin), pin);
  }

  // Wire-attachment fixpoint: attach wire endpoints to declared nets they
  // touch, resolving chains incrementally until no more attachments occur.
  // Remaining wires (touching no declared net) fall through to union-find.
  const declaredPinToNet = new Map<string, string>();
  for (const [name, members] of declaredMembers) {
    for (const k of members.keys()) {
      // A pin connected to several declared nets keeps all memberships
      // (bridge case); first net wins for wire-attachment purposes.
      if (!declaredPinToNet.has(k)) declaredPinToNet.set(k, name);
    }
  }

  // Sort wires for determinism before the fixpoint loop.
  const pendingWires = [...diagram.wires].sort((a, b) => {
    const ka = key(a.from) + '|' + key(a.to);
    const kb = key(b.from) + '|' + key(b.to);
    return ka.localeCompare(kb);
  });

  let changed = true;
  while (changed) {
    changed = false;
    for (let i = pendingWires.length - 1; i >= 0; i--) {
      const w = pendingWires[i]!;
      const ka = key(w.from);
      const kb = key(w.to);
      const netA = declaredPinToNet.get(ka);
      const netB = declaredPinToNet.get(kb);

      if (netA && netB) {
        // Both ends are already in declared nets (bridge or same net).
        // Both pins are already present; nothing to add.
        pendingWires.splice(i, 1);
        continue;
      }
      if (netA) {
        declaredMembers.get(netA)!.set(kb, w.to);
        declaredPinToNet.set(kb, netA);
        pendingWires.splice(i, 1);
        changed = true;
        continue;
      }
      if (netB) {
        declaredMembers.get(netB)!.set(ka, w.from);
        declaredPinToNet.set(ka, netB);
        pendingWires.splice(i, 1);
        changed = true;
        continue;
      }
    }
  }

  // Remaining pending wires touch no declared net: union-find synthetic closure.
  const wirePins = new Map<string, PinRef>();
  for (const w of pendingWires) {
    const ka = key(w.from);
    const kb = key(w.to);
    wirePins.set(ka, w.from);
    wirePins.set(kb, w.to);
    union(ka, kb);
  }

  const out: ResolvedNet[] = [];
  for (const net of diagram.nets) {
    const members = [...declaredMembers.get(net.name)!.values()].sort((x, y) =>
      key(x).localeCompare(key(y)),
    );
    out.push({
      name: net.name,
      kind: net.kind,
      voltage: net.voltage,
      protocol: net.protocol,
      declared: true,
      members,
    });
  }

  // Group synthetic components.
  const groups = new Map<string, PinRef[]>();
  for (const [k, pin] of wirePins) {
    const root = find(k);
    const g = groups.get(root) ?? [];
    g.push(pin);
    groups.set(root, g);
  }
  const synthetic = [...groups.values()]
    .map((members) => members.sort((x, y) => key(x).localeCompare(key(y))))
    .sort((a, b) => key(a[0]!).localeCompare(key(b[0]!)))
    .map<ResolvedNet>((members) => ({
      name: `net@${key(members[0]!)}`,
      kind: 'signal',
      voltage: undefined,
      protocol: undefined,
      declared: false,
      members,
    }));

  return [...out, ...synthetic];
}
