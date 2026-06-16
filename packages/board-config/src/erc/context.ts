import type { DiagramV2, PinRef } from '../schema';
import type { ResolvedNet } from '../normalize';
import { resolveNets } from '../normalize';
import { getCatalogPart, type CatalogPart, type PinDecl, type PinEtype } from '../catalog';
import type { NetProtocol } from '../schema';
import { getPinEtype, getPinMapping, PIN_MAPS } from '../pin-mapping';
import type { Part } from '../types';
// getCatalogPart is imported above; used in isMcuPart for catalog-declared MCU types.

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

/**
 * True when the part is the MCU.
 *
 * A part counts as the MCU when:
 *   (a) its type is the literal string 'mcu', OR
 *   (b) its type directly keys a PIN_MAPS entry (e.g. 'esp32-s3-zero'), OR
 *   (c) its catalog entry has deviceClass === 'mcu' (e.g. 'stm32-dev', 'nucleo-f401re',
 *       'nrf52840-dk', 'rpi-pico', etc.) — in which case pin lookups fall back to
 *       ctx.diagram.board via mcuBoardKey().
 *
 * Case (c) was previously missed: catalog-typed MCU boards whose type string is
 * NOT itself a PIN_MAPS key (e.g. 'stm32-dev' vs 'stm32f103') were treated as
 * peripheral parts, causing power-rail pins to resolve via the catalog (no catalog
 * pins → null) rather than via the board's pin map.
 */
export function isMcuPart(_ctx: ErcContext, part: Part): boolean {
  if (part.type === 'mcu') return true;
  if (PIN_MAPS[part.type] !== undefined) return true;
  return getCatalogPart(part.type)?.deviceClass === 'mcu';
}

/** Board key used for pin lookups of an MCU part. */
export function mcuBoardKey(ctx: ErcContext, part: Part): string {
  return PIN_MAPS[part.type] ? part.type : ctx.diagram.board;
}

/** Strip a trailing `.N` multi-instance suffix from a pin name (e.g. `GND.2` → `GND`).
 *  The raw name is preserved in diagnostics/subjects; only the lookup key is normalised. */
function stripPinSuffix(pin: unknown): string {
  if (typeof pin !== 'string') return '';
  return pin.replace(/\.\d+$/, '');
}

/** Effective electrical pin info for a member; null = legacy/unknown (skip pin rules). */
export function effectivePin(ctx: ErcContext, member: PinRef): EffectivePin | null {
  const part = ctx.partsById.get(member.part);
  if (!part) return null;
  const rawPin = member.pin;
  if (typeof rawPin !== 'string') return null;
  const strippedPin = stripPinSuffix(rawPin);
  if (isMcuPart(ctx, part)) {
    const boardKey = mcuBoardKey(ctx, part);
    // Try the raw pin name first so that real dotted names (e.g. nRF52840's
    // P0.00..P1.15) are found before the suffix-strip is applied.
    // Fall back to the stripped name so `GND.2` → `GND` disambiguation works.
    const el = getPinEtype(boardKey, rawPin) ?? getPinEtype(boardKey, strippedPin);
    if (!el) return null;
    const fn = getPinMapping(boardKey, rawPin) ?? getPinMapping(boardKey, strippedPin);
    // Role from the pin map's declared functions, when unambiguous.
    const role = roleFromFunctions(fn);
    return { etype: el.etype, internalPullup: el.internalPullup, ...(role ? { role } : {}) };
  }
  const cat: CatalogPart | undefined = getCatalogPart(part.type);
  // Try raw pin name first, then stripped, for the same reason as the MCU path.
  const decl: PinDecl | undefined =
    cat?.pins?.find((p) => p.name === rawPin) ?? cat?.pins?.find((p) => p.name === strippedPin);
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
