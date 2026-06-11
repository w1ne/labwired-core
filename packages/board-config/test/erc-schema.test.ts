import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const base = (over: Partial<DiagramV2>): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [{ id: 'mcu', type: 'esp32-s3-zero' }],
  nets: [], connections: [], wires: [], ...over,
});

const codes = (d: DiagramV2) => erc(d).map((x) => x.code);

describe('schema-integrity rules', () => {
  it('clean minimal diagram has no errors', () => {
    expect(erc(base({})).filter((d) => d.severity === 'error')).toEqual([]);
  });
  it('SCHEMA_PINREF_MALFORMED for unparseable connection refs', () => {
    expect(codes(base({ nets: [{ name: 'N', kind: 'signal' }], connections: [['nocolon', 'N']] })))
      .toContain('SCHEMA_PINREF_MALFORMED');
  });
  it('SCHEMA_NET_UNDECLARED when a connection names a net not in nets[]', () => {
    expect(codes(base({ connections: [['mcu:GPIO8', 'GHOST']] })))
      .toContain('SCHEMA_NET_UNDECLARED');
  });
  it('SCHEMA_NET_DUPLICATE for duplicate net names', () => {
    expect(codes(base({ nets: [{ name: 'A', kind: 'signal' }, { name: 'A', kind: 'power' }] })))
      .toContain('SCHEMA_NET_DUPLICATE');
  });
  it('SCHEMA_PART_UNKNOWN for a part type missing from the catalog, with closest-match hint', () => {
    const out = erc(base({ parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'x', type: 'bme28' }] }));
    const d = out.find((x) => x.code === 'SCHEMA_PART_UNKNOWN')!;
    expect(d).toBeDefined();
    expect(d.hint).toContain('bme280');
  });
  it('SCHEMA_CONN_UNKNOWN_PART when a connection references a part id not in parts[]', () => {
    expect(codes(base({ nets: [{ name: 'N', kind: 'signal' }], connections: [['ghost:1', 'N']] })))
      .toContain('SCHEMA_CONN_UNKNOWN_PART');
  });
  it('SCHEMA_BOARD_UNKNOWN when diagram.board has no pin map', () => {
    expect(codes(base({ board: 'imaginary-board-9000' }))).toContain('SCHEMA_BOARD_UNKNOWN');
  });
  it('accepts v1 input (wires-only) via migration', () => {
    const v1 = { board: 'esp32-s3-zero', parts: [{ id: 'mcu', type: 'esp32-s3-zero' }], wires: [] };
    expect(() => erc(v1 as never)).not.toThrow();
  });
});
