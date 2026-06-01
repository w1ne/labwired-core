import { describe, it, expect } from 'vitest';
import {
  toHex,
  kindLabel,
  linkPhaseIndex,
  PHASES,
  errorCount,
  filterErrorsOnly,
  toCsv,
} from './iolinkDecode';
import type { IolinkXfer } from '@labwired/ui';

function xfer(over: Partial<IolinkXfer>): IolinkXfer {
  return {
    seq: 0,
    kind: 'cyclic',
    com: 'com2',
    pd_out: [],
    pd_in: [0xa5],
    od: 0,
    ck_ok: true,
    pd_valid: true,
    link_state: 'operate',
    raw_master: [0x00, 0x00, 0x00, 0x1b],
    raw_device: [0x20, 0xa5, 0x00, 0x0d],
    ...over,
  };
}

describe('iolinkDecode', () => {
  it('formats bytes as spaced uppercase hex, with a dash for empty', () => {
    expect(toHex([0x0a, 0xff, 0x00])).toBe('0A FF 00');
    expect(toHex([])).toBe('—');
  });

  it('labels frame kinds', () => {
    expect(kindLabel('wake_up')).toBe('WAKE-UP');
    expect(kindLabel('idle')).toBe('IDLE');
    expect(kindLabel('operate_req')).toBe('OPERATE');
    expect(kindLabel('cyclic')).toBe('CYCLIC');
  });

  it('maps link state to a phase-strip index', () => {
    expect(PHASES).toEqual(['WAKE-UP', 'STARTUP', 'PREOPERATE', 'OPERATE']);
    expect(linkPhaseIndex('startup')).toBe(1);
    expect(linkPhaseIndex('operate')).toBe(3);
  });

  it('counts only frames with a false CRC (null verdicts are not errors)', () => {
    const rows = [
      xfer({ ck_ok: true }),
      xfer({ ck_ok: false }),
      xfer({ ck_ok: null, kind: 'wake_up' }),
    ];
    expect(errorCount(rows)).toBe(1);
    expect(filterErrorsOnly(rows)).toHaveLength(1);
    expect(filterErrorsOnly(rows)[0].ck_ok).toBe(false);
  });

  it('exports CSV with a header and one row per xfer', () => {
    const csv = toCsv([xfer({ seq: 3 })]);
    const lines = csv.trim().split('\n');
    expect(lines[0]).toBe('seq,kind,link_state,pd_out,pd_in,ck_ok,raw_master,raw_device');
    expect(lines[1]).toContain('3,cyclic,operate');
    expect(lines[1]).toContain('A5');
  });
});
