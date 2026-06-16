import { describe, it, expect } from 'vitest';
import { EXAMPLE_SKETCHES } from '@labwired/ui';
import { sharedCircuitIsRunnable } from './App';
import { BOARD_CONFIGS } from './bundled-configs';

// The untouched default editor source for a bare board is the "Blink" sketch.
const blink = EXAMPLE_SKETCHES.find((s) => s.name === 'Blink')!.source;
// A bare, firmware-less board (the case that produced dead proximity shares).
const bare = BOARD_CONFIGS.find(
  (b) => !b.demoFirmwarePath && b.kind !== 'lab' && b.boardId !== 'nucleo-f401re',
)!;
// A board that ships pre-built demo firmware.
const withFirmware = BOARD_CONFIGS.find((b) => b.demoFirmwarePath)!;

describe('sharedCircuitIsRunnable', () => {
  it('is false for a firmware-less board still on its untouched default source', () => {
    expect(sharedCircuitIsRunnable(bare, blink)).toBe(false);
  });

  it('is true once the user has written code different from the default', () => {
    expect(sharedCircuitIsRunnable(bare, 'void setup(){} void loop(){ /* mine */ }')).toBe(true);
  });

  it('is true for a board with pre-built demo firmware regardless of source', () => {
    expect(sharedCircuitIsRunnable(withFirmware, blink)).toBe(true);
  });
});
