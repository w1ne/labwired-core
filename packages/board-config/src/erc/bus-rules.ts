import { diag, type Diagnostic } from './diagnostic';
import { effectivePin, type ErcContext } from './context';
import { getCatalogPart } from '../catalog';
import type { PinRef } from '../schema';
import { parseAddr } from '../attrs';

export function busRules(ctx: ErcContext): Diagnostic[] {
  const out: Diagnostic[] = [];

  // --- I2C: group device SDA pins by their resolved net (= the bus) ---
  const busDevices = new Map<string, { id: string; addr: number | undefined }[]>();
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    const sda = cat?.pins?.find((p) => p.role === 'i2c_sda');
    if (!sda) continue;
    const nets = ctx.netsByPin.get(`${part.id}:${sda.name}`) ?? [];
    for (const net of nets) {
      const arr = busDevices.get(net.name) ?? [];
      arr.push({ id: part.id, addr: parseAddr(part.attrs?.i2c_address) });
      busDevices.set(net.name, arr);
    }
  }
  for (const [netName, devs] of busDevices) {
    const byAddr = new Map<number, string[]>();
    for (const d of devs) {
      if (d.addr === undefined) continue; // unknown address: cannot judge
      byAddr.set(d.addr, [...(byAddr.get(d.addr) ?? []), d.id]);
    }
    for (const [addr, ids] of byAddr) {
      if (ids.length > 1) {
        out.push(diag('I2C_ADDR_CONFLICT', 'error',
          `devices ${ids.join(', ')} share I2C address 0x${addr.toString(16)} on net '${netName}'`,
          'Change one device address (attrs.i2c_address / address-select pins)',
          [...ids, netName]));
      }
    }
  }

  // --- I2C pull-ups: every open-drain i2c net needs a pull path ---
  const powerNets = new Set(
    ctx.nets.filter((n) => n.kind === 'power' && (n.voltage ?? 0) > 0).map((n) => n.name),
  );
  for (const net of ctx.nets) {
    const isI2c =
      net.protocol === 'i2c_sda' || net.protocol === 'i2c_scl' ||
      net.members.some((m) => {
        const p = effectivePin(ctx, m);
        return p?.role === 'i2c_sda' || p?.role === 'i2c_scl';
      });
    if (!isI2c) continue;
    const pulled = hasPullPath(ctx, net.members, powerNets);
    if (!pulled) {
      out.push(diag('I2C_NO_PULLUP', 'warning',
        `open-drain net '${net.name}' has no pull-up (no resistor to a rail, no MCU internal pullup enabled)`,
        "Add a pull-up resistor to a power net, or set the MCU part's attrs.internal_pullups to include the pin",
        [net.name]));
    }
  }

  // --- SPI CS coverage ---
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    const cs = cat?.pins?.find((p) => p.role === 'spi_cs');
    if (!cs) continue;
    const nets = ctx.netsByPin.get(`${part.id}:${cs.name}`) ?? [];
    const driven = nets.some((n) =>
      n.members.some((m) => {
        if (m.part === part.id) return false;
        const p = effectivePin(ctx, m);
        return p?.etype === 'output' || p?.etype === 'bidirectional';
      }),
    );
    if (!driven) {
      out.push(diag('SPI_NO_CS', 'warning',
        `SPI device '${part.id}' chip-select '${cs.name}' is not driven by any MCU/output pin`,
        'Wire the CS pin to a free MCU GPIO',
        [`${part.id}:${cs.name}`]));
    }
  }

  // --- UART crossover ---
  for (const net of ctx.nets) {
    const roles = net.members
      .map((m) => ({ m, p: effectivePin(ctx, m) }))
      .filter((x) => x.p?.role === 'uart_tx' || x.p?.role === 'uart_rx');
    out.push(...uartCrossover(net.name, roles.map((x) => ({ key: `${x.m.part}:${x.m.pin}`, role: x.p!.role! as string }))));
  }

  // --- Floating required inputs ---
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    for (const pin of cat?.pins ?? []) {
      if (!pin.required) continue;
      if (!(ctx.netsByPin.get(`${part.id}:${pin.name}`)?.length)) {
        out.push(diag('PIN_INPUT_FLOATING', 'warning',
          `required input ${part.id}:${pin.name} is not connected to any net`,
          'Wire the pin (it must be driven for the part to function)',
          [`${part.id}:${pin.name}`]));
      }
    }
  }

  return out;
}

/** Exported for unit tests: two same-direction UART pins on one net = error. */
export function uartCrossover(
  netName: string,
  pins: { key: string; role: string }[],
): Diagnostic[] {
  const out: Diagnostic[] = [];
  const tx = pins.filter((p) => p.role === 'uart_tx');
  const rx = pins.filter((p) => p.role === 'uart_rx');
  for (const group of [tx, rx]) {
    if (group.length > 1) {
      out.push(diag('UART_CROSSOVER', 'error',
        `net '${netName}' connects ${group.length} UART ${group === tx ? 'TX' : 'RX'} pins together (${group.map((g) => g.key).join(', ')})`,
        'UART wiring crosses over: TX connects to RX',
        [...group.map((g) => g.key), netName]));
    }
  }
  return out;
}

/** A pull path exists when a passive part bridges this net to a positive rail,
 * or an MCU member pin has internal pullups enabled via attrs. */
function hasPullPath(
  ctx: ErcContext,
  members: PinRef[],
  powerNets: Set<string>,
): boolean {
  for (const m of members) {
    const part = ctx.partsById.get(m.part);
    if (!part) continue;
    const cat = getCatalogPart(part.type);
    // passive bridge: the part's OTHER pins sit on a positive power net
    if (cat?.deviceClass === 'passive' && cat.pins) {
      for (const other of cat.pins) {
        if (other.name === m.pin) continue;
        const otherNets = ctx.netsByPin.get(`${part.id}:${other.name}`) ?? [];
        if (otherNets.some((n) => powerNets.has(n.name))) return true;
      }
    }
    // MCU internal pullup, opted in via attrs
    const p = effectivePin(ctx, m);
    if (p?.internalPullup) {
      const list = (part.attrs?.internal_pullups ?? '').split(',').map((s) => s.trim());
      if (list.includes(m.pin)) return true;
    }
  }
  return false;
}
