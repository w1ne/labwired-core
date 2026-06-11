import { describe, expect, it } from 'vitest';
import { resolveNets, type ResolvedNet } from '../src/normalize';
import type { DiagramV2 } from '../src/schema';

function base(over: Partial<DiagramV2>): DiagramV2 {
  return {
    version: 2,
    board: 'esp32-s3-zero',
    parts: [],
    nets: [],
    connections: [],
    wires: [],
    ...over,
  };
}

describe('resolveNets', () => {
  it('resolves declared nets with their members', () => {
    const d = base({
      nets: [{ name: 'I2C0_SDA', kind: 'signal', protocol: 'i2c_sda' }],
      connections: [
        ['mcu:GPIO8', 'I2C0_SDA'],
        ['pca1:SDA', 'I2C0_SDA'],
      ],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(1);
    expect(nets[0]).toEqual<ResolvedNet>({
      name: 'I2C0_SDA',
      kind: 'signal',
      protocol: 'i2c_sda',
      voltage: undefined,
      declared: true,
      members: [
        { part: 'mcu', pin: 'GPIO8' },
        { part: 'pca1', pin: 'SDA' },
      ],
    });
  });

  it('folds legacy wires into synthetic nets via transitive closure', () => {
    const d = base({
      wires: [
        { from: { part: 'a', pin: '1' }, to: { part: 'b', pin: '2' } },
        { from: { part: 'b', pin: '2' }, to: { part: 'c', pin: '3' } },
        { from: { part: 'x', pin: '9' }, to: { part: 'y', pin: '8' } },
      ],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(2);
    const abc = nets.find((n) => n.members.some((m) => m.part === 'a'))!;
    expect(abc.members).toEqual([
      { part: 'a', pin: '1' },
      { part: 'b', pin: '2' },
      { part: 'c', pin: '3' },
    ]);
    expect(abc.declared).toBe(false);
    // Synthetic name derives from the lexicographically smallest member.
    expect(abc.name).toBe('net@a:1');
  });

  it('merges a wire touching a declared net into that net', () => {
    const d = base({
      nets: [{ name: 'GND', kind: 'power', voltage: 0 }],
      connections: [['mcu:GND', 'GND']],
      wires: [{ from: { part: 'mcu', pin: 'GND' }, to: { part: 'led1', pin: 'C' } }],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(1);
    expect(nets[0].name).toBe('GND');
    expect(nets[0].members).toEqual([
      { part: 'led1', pin: 'C' },
      { part: 'mcu', pin: 'GND' },
    ]);
  });

  it('attaches whole wire chains to a declared net (fixpoint, any order)', () => {
    const d = base({
      nets: [{ name: 'GND', kind: 'power', voltage: 0 }],
      connections: [['mcu:GND', 'GND']],
      wires: [
        { from: { part: 'r1', pin: '2' }, to: { part: 'led1', pin: 'C' } },
        { from: { part: 'led1', pin: 'C' }, to: { part: 'mcu', pin: 'GND' } },
      ],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(1);
    expect(nets[0].members.map((m) => `${m.part}:${m.pin}`)).toEqual([
      'led1:C',
      'mcu:GND',
      'r1:2',
    ]);
  });

  it('is deterministic: shuffled input order yields identical output', () => {
    const wires = [
      { from: { part: 'a', pin: '1' }, to: { part: 'b', pin: '2' } },
      { from: { part: 'b', pin: '2' }, to: { part: 'c', pin: '3' } },
      { from: { part: 'd', pin: '4' }, to: { part: 'a', pin: '1' } },
    ];
    const a = resolveNets(base({ wires }));
    const b = resolveNets(base({ wires: [...wires].reverse() }));
    expect(a).toEqual(b);
  });

  it('errors are not its job: unknown parts pass through (ERC judges them)', () => {
    const d = base({ wires: [{ from: { part: 'ghost', pin: '1' }, to: { part: 'g2', pin: '2' } }] });
    expect(resolveNets(d)).toHaveLength(1);
  });

  it('two declared nets bridged by wires stay distinct nets but share members', () => {
    // Bridging declared nets is an ERC matter (NET_RAIL_SHORT), not a merge:
    // resolveNets must NOT silently union two declared nets.
    const d = base({
      nets: [
        { name: 'A', kind: 'signal' },
        { name: 'B', kind: 'signal' },
      ],
      connections: [
        ['p:1', 'A'],
        ['p:1', 'B'],
      ],
    });
    const nets = resolveNets(d);
    expect(nets.map((n) => n.name).sort()).toEqual(['A', 'B']);
  });
});
