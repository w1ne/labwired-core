import { describe, expect, it } from 'vitest';
import { getPinEtype, getPinMapping, PIN_MAPS } from '../src/pin-mapping';

describe('pin map electrical extension', () => {
  it('esp32-s3-zero GPIO pins are bidirectional with internal pullups', () => {
    expect(getPinEtype('esp32-s3-zero', 'GPIO8')).toEqual({
      etype: 'bidirectional',
      internalPullup: true,
    });
  });

  it('power pins carry power_out etype', () => {
    expect(getPinEtype('esp32-s3-zero', '3V3')).toEqual({
      etype: 'power_out',
      internalPullup: false,
    });
    expect(getPinEtype('esp32-s3-zero', 'GND')).toEqual({
      etype: 'power_out',
      internalPullup: false,
    });
  });

  it('5V pin is mapped and resolves to power_out for S3 boards', () => {
    // getPinMapping must return non-null (pin exists in the map)
    expect(getPinMapping('esp32-s3-zero', '5V')).not.toBeNull();
    expect(getPinMapping('esp32s3', '5V')).not.toBeNull();
    // getPinEtype must resolve to power_out (not null, not bidirectional)
    expect(getPinEtype('esp32-s3-zero', '5V')).toEqual({
      etype: 'power_out',
      internalPullup: false,
    });
    expect(getPinEtype('esp32s3', '5V')).toEqual({
      etype: 'power_out',
      internalPullup: false,
    });
  });

  it('unknown pin or board returns null', () => {
    expect(getPinEtype('esp32-s3-zero', 'NOPE')).toBeNull();
    expect(getPinEtype('not-a-board', 'GPIO8')).toBeNull();
  });

  it('every board in PIN_MAPS resolves every mapped pin to an etype (default bidirectional)', () => {
    for (const board of Object.keys(PIN_MAPS)) {
      for (const pin of Object.keys(PIN_MAPS[board])) {
        expect(getPinEtype(board, pin), `${board}:${pin}`).not.toBeNull();
      }
    }
  });

  it('existing lookups unchanged', () => {
    // Regression guard: extension must not break the legacy surface.
    expect(getPinMapping('esp32-s3-zero', 'GPIO8')).not.toBeNull();
  });
});
