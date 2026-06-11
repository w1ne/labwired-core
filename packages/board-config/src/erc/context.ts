import type { DiagramV2, PinRef } from '../schema';
import type { ResolvedNet } from '../normalize';
import { resolveNets } from '../normalize';
import { getCatalogPart, type CatalogPart, type PinDecl, type PinEtype } from '../catalog';
import type { NetProtocol } from '../schema';
import { getPinEtype, getPinMapping, PIN_MAPS } from '../pin-mapping';
import type { Part } from '../types';

/** Everything a rule needs, resolved once. */
export interface ErcContext {
  diagram: DiagramV2;
  nets: ResolvedNet[];
  partsById: Map<string, Part>;
  /** Resolved net(s) per "part:pin" member key. */
  netsByPin: Map<string, ResolvedNet[]>;
}

/** Effective pin info: catalog decl, or MCU map lookup, or null (legacy/unknown). */
export interface EffectivePin {
  etype: PinEtype;
  role?: NetProtocol;
  required?: boolean;
  internalPullup?: boolean;
}

export function buildContext(diagram: DiagramV2): ErcContext {
  const nets = resolveNets(diagram);
  const partsById = new Map(diagram.parts.map((p) => [p.id, p]));
  const netsByPin = new Map<string, ResolvedNet[]>();
  for (const net of nets) {
    for (const m of net.members) {
      const k = `${m.part}:${m.pin}`;
      const arr = netsByPin.get(k) ?? [];
      arr.push(net);
      netsByPin.set(k, arr);
    }
  }
  return { diagram, nets, partsById, netsByPin };
}

/** True when the part is the MCU (its type has a pin map, or it is 'mcu'). */
export function isMcuPart(ctx: ErcContext, part: Part): boolean {
  return part.type === 'mcu' || PIN_MAPS[part.type] !== undefined;
}

/** Board key used for pin lookups of an MCU part. */
export function mcuBoardKey(ctx: ErcContext, part: Part): string {
  return PIN_MAPS[part.type] ? part.type : ctx.diagram.board;
}

/** Effective electrical pin info for a member; null = legacy/unknown (skip pin rules). */
export function effectivePin(ctx: ErcContext, member: PinRef): EffectivePin | null {
  const part = ctx.partsById.get(member.part);
  if (!part) return null;
  if (isMcuPart(ctx, part)) {
    const el = getPinEtype(mcuBoardKey(ctx, part), member.pin);
    if (!el) return null;
    const fn = getPinMapping(mcuBoardKey(ctx, part), member.pin);
    // Role from the pin map's declared functions, when unambiguous.
    const role = roleFromFunctions(fn);
    return { etype: el.etype, internalPullup: el.internalPullup, ...(role ? { role } : {}) };
  }
  const cat: CatalogPart | undefined = getCatalogPart(part.type);
  const decl: PinDecl | undefined = cat?.pins?.find((p) => p.name === member.pin);
  if (!decl) return null;
  return { etype: decl.etype, ...(decl.role ? { role: decl.role } : {}), ...(decl.required ? { required: true } : {}) };
}

function roleFromFunctions(fn: ReturnType<typeof getPinMapping>): NetProtocol | undefined {
  // Real PinFunction shape: { type: 'gpio'|'adc'|'i2c'|'spi'|'timer'|'uart', peripheral, channel?, role? }
  if (!fn) return undefined;
  for (const f of fn.functions) {
    if (f.type === 'i2c' && f.role === 'sda') return 'i2c_sda';
    if (f.type === 'i2c' && f.role === 'scl') return 'i2c_scl';
    if (f.type === 'spi' && f.role === 'mosi') return 'spi_mosi';
    if (f.type === 'spi' && f.role === 'miso') return 'spi_miso';
    if (f.type === 'spi' && f.role === 'sck') return 'spi_sck';
    if (f.type === 'spi' && f.role === 'nss') return 'spi_cs';
    if (f.type === 'uart' && f.role === 'tx') return 'uart_tx';
    if (f.type === 'uart' && f.role === 'rx') return 'uart_rx';
  }
  return undefined;
}

