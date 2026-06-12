import { diag, type Diagnostic } from './diagnostic';
import { effectivePin, type ErcContext } from './context';
import type { PinEtype } from '../catalog';

type Cell = { code: string; severity: 'error' | 'warning' } | null;

/** The acted-on cells of the (symmetric) pin-pair matrix; everything else OK. */
export function pairFinding(a: PinEtype, b: PinEtype): Cell {
  const pair = (x: PinEtype, y: PinEtype) => (a === x && b === y) || (a === y && b === x);
  if (a === 'nc' || b === 'nc') return { code: 'NET_NC_CONNECTED', severity: 'error' };
  if (pair('output', 'output')) return { code: 'NET_DRIVER_CONFLICT', severity: 'error' };
  if (pair('output', 'power_out')) return { code: 'NET_DRIVER_CONFLICT', severity: 'error' };
  if (pair('power_out', 'power_out')) return { code: 'NET_RAIL_SHORT', severity: 'error' };
  if (pair('open_drain', 'output')) return { code: 'NET_DRIVER_CONFLICT', severity: 'warning' };
  if ((a === 'unspecified') !== (b === 'unspecified'))
    return { code: 'NET_UNSPECIFIED_PIN', severity: 'warning' };
  return null;
}

export function matrixRules(ctx: ErcContext): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seen = new Set<string>();

  for (const net of ctx.nets) {
    const typed = net.members
      .map((m) => ({ m, pin: effectivePin(ctx, m) }))
      .filter((x): x is { m: (typeof x)['m']; pin: NonNullable<(typeof x)['pin']> } => x.pin !== null);

    for (let i = 0; i < typed.length; i++) {
      for (let j = i + 1; j < typed.length; j++) {
        const f = pairFinding(typed[i].pin.etype, typed[j].pin.etype);
        if (!f) continue;
        const subj = [`${typed[i].m.part}:${typed[i].m.pin}`, `${typed[j].m.part}:${typed[j].m.pin}`];
        const key = `${f.code}|${net.name}|${subj.join('|')}`;
        if (seen.has(key)) continue;
        seen.add(key);
        const label =
          f.code === 'NET_RAIL_SHORT' ? 'rail short' :
          f.code === 'NET_NC_CONNECTED' ? 'NC pin connected' :
          'driver conflict';
        out.push(diag(f.code, f.severity,
          `${label} on net '${net.name}': ${subj[0]} (${typed[i].pin.etype}) with ${subj[1]} (${typed[j].pin.etype})`,
          'Rewire so the net has at most one push-pull driver and no shorted rails',
          [...subj, net.name]));
      }
    }
  }

  // Bridge clause: one pin member of two declared power nets at different voltages.
  for (const [pinKey, nets] of ctx.netsByPin) {
    const powers = nets.filter((n) => n.declared && n.kind === 'power' && n.voltage !== undefined);
    for (let i = 0; i < powers.length; i++) {
      for (let j = i + 1; j < powers.length; j++) {
        if (powers[i].voltage !== powers[j].voltage) {
          out.push(diag('NET_RAIL_SHORT', 'error',
            `pin ${pinKey} bridges power nets '${powers[i].name}' (${powers[i].voltage}V) and '${powers[j].name}' (${powers[j].voltage}V)`,
            'A pin cannot sit on two rails of different voltage',
            [pinKey, powers[i].name, powers[j].name]));
        }
      }
    }
  }

  return out;
}
