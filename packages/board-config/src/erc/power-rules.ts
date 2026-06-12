import { diag, type Diagnostic } from './diagnostic';
import { effectivePin, type ErcContext } from './context';
import { getCatalogPart } from '../catalog';

export function powerRules(ctx: ErcContext): Diagnostic[] {
  const out: Diagnostic[] = [];

  for (const net of ctx.nets) {
    const pins = net.members.map((m) => ({ m, pin: effectivePin(ctx, m) }));
    const hasPowerIn = pins.some((x) => x.pin?.etype === 'power_in');
    const hasPowerOut = pins.some((x) => x.pin?.etype === 'power_out');

    if (hasPowerIn && !hasPowerOut) {
      out.push(diag('PWR_RAIL_UNDRIVEN', 'error',
        `net '${net.name}' powers parts but has no supply (no power_out pin)`,
        'Connect an MCU rail pin (3V3/5V/GND) or a supply part to the net',
        [net.name]));
    }

    // Voltage mismatch: declared power net with voltage > 0 feeding power_in pins
    // of parts whose operatingVoltage excludes it.
    if (net.declared && net.kind === 'power' && net.voltage !== undefined && net.voltage > 0) {
      for (const { m, pin } of pins) {
        if (pin?.etype !== 'power_in') continue;
        const part = ctx.partsById.get(m.part);
        const range = part && getCatalogPart(part.type)?.operatingVoltage;
        if (range && (net.voltage < range.min || net.voltage > range.max)) {
          out.push(diag('PWR_VOLTAGE_MISMATCH', 'error',
            `${m.part}:${m.pin} on ${net.voltage}V net '${net.name}' but '${part!.type}' operates ${range.min}-${range.max}V`,
            'Move the part to a rail inside its operating range',
            [`${m.part}:${m.pin}`, net.name]));
        }
      }
    }
  }

  // PWR_NO_GROUND: a part with declared power_in pins and an operating range
  // must touch a 0V net somewhere.
  //
  // A net counts as ground when either:
  //   (a) it is a declared power net with voltage === 0, OR
  //   (b) any member resolves to etype === 'power_out' AND its stripped pin
  //       name is 'GND' — the name carries the 0V meaning; power_out alone
  //       is not enough because 3V3/5V rails are also power_out.
  //
  // Rule (b) handles v1-wire-migrated diagrams where the ground path is a
  // synthetic net (no declared voltage) formed by wiring b1:GND → mcu:GND.
  const groundNets = new Set(
    ctx.nets.filter((n) => n.kind === 'power' && n.voltage === 0).map((n) => n.name),
  );
  // Augment with synthetic/undeclared nets whose members include a GND power_out pin.
  for (const net of ctx.nets) {
    if (groundNets.has(net.name)) continue;
    const hasGndPowerOut = net.members.some((m) => {
      const stripped = m.pin.replace(/\.\d+$/, '');
      if (stripped.toUpperCase() !== 'GND') return false;
      const ep = effectivePin(ctx, m);
      return ep?.etype === 'power_out';
    });
    if (hasGndPowerOut) groundNets.add(net.name);
  }

  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    if (!cat?.pins?.some((p) => p.etype === 'power_in') || !cat.operatingVoltage) continue;

    const touchesGround = [...ctx.netsByPin.entries()].some(
      ([k, nets]) => k.startsWith(`${part.id}:`) && nets.some((n) => groundNets.has(n.name)),
    );

    if (!touchesGround) {
      out.push(diag('PWR_NO_GROUND', 'warning',
        `powered part '${part.id}' (${part.type}) has no pin on a 0V net`,
        'Wire its GND pin to the ground net',
        [part.id]));
    }
  }

  return out;
}
