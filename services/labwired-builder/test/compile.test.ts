import { describe, expect, it } from 'vitest';
import {
  compile,
  generatePlatformioIni,
  parseGccDiagnostics,
  supportedCompileBoards,
  bakeTargets,
} from '../src/compile.js';
import { resolveBoard } from '../src/boards.js';

describe('generatePlatformioIni', () => {
  it('generates a controlled ini for a known board (no agent-supplied fields)', () => {
    const ini = generatePlatformioIni('stm32l476')!;
    expect(ini).toContain('[env:sim]');
    expect(ini).toContain('platform = ststm32');
    expect(ini).toContain('board = nucleo_l476rg');
    // No extra_scripts / arbitrary build hooks.
    expect(ini).not.toContain('extra_scripts');
  });

  it('includes our parity overrides for esp32-s3 but never agent input', () => {
    const ini = generatePlatformioIni('esp32-s3-zero')!;
    expect(ini).toContain('board_build.flash_size = 4MB');
  });

  it('returns null for an unknown board', () => {
    expect(generatePlatformioIni('not-a-board')).toBeNull();
  });

  it('injects an additive lib_deps block (the one caller-influenced field)', () => {
    const ini = generatePlatformioIni('esp32', ['zinggjm/GxEPD2', 'adafruit/Adafruit GFX Library'])!;
    expect(ini).toContain('lib_deps =');
    expect(ini).toContain('    zinggjm/GxEPD2');
    expect(ini).toContain('    adafruit/Adafruit GFX Library');
  });

  it('accepts a comma/newline lib_deps string', () => {
    const ini = generatePlatformioIni('esp32', 'a/one, b/two')!;
    expect(ini).toContain('    a/one');
    expect(ini).toContain('    b/two');
  });
});

describe('resolveBoard (board id → chip family precedence)', () => {
  it('prefers an exact board id', () => {
    const r = resolveBoard('esp32', 'esp32s3')!;
    expect(r.board.board).toBe('esp32dev');
    expect(r.source).toBe('board[esp32]');
  });

  it('falls back to chip family when the board id is unknown', () => {
    const r = resolveBoard('proto-cat-only-id', 'esp32s3')!;
    expect(r.board.board).toBe('lolin_s3_mini');
    expect(r.source).toBe('chip_family[esp32s3]');
  });

  it('returns null when neither matches', () => {
    expect(resolveBoard('nope', 'also-nope')).toBeNull();
  });
});

describe('bakeTargets', () => {
  it('covers every distinct ini (boards ∪ chip families), deduped', () => {
    const targets = bakeTargets();
    const inis = targets.map((t) => t.ini);
    // No duplicate platformio.ini files baked.
    expect(new Set(inis).size).toBe(inis.length);
    // A stm32cube and an arduino l476 are DIFFERENT inis and both present.
    expect(inis.some((i) => i.includes('board = nucleo_l476rg') && i.includes('framework = stm32cube'))).toBe(true);
    expect(inis.some((i) => i.includes('board = nucleo_l476rg') && i.includes('framework = arduino'))).toBe(true);
  });
});

describe('supportedCompileBoards', () => {
  it('marks Cortex-M and ESP32 boards runnable', () => {
    const map = Object.fromEntries(supportedCompileBoards().map((b) => [b.board, b.runnable]));
    expect(map['stm32l476']).toBe(true);
    expect(map['esp32-s3-zero']).toBe(true);
    expect(map['esp32']).toBe(true);
    expect(map['esp32-c3-supermini']).toBe(true);
  });
});

describe('parseGccDiagnostics', () => {
  it('parses errors and warnings with file/line/col', () => {
    const log = [
      'src/main.c:7:5: error: \'foo\' undeclared (first use in this function)',
      'src/main.c:12:1: warning: unused variable \'x\'',
      'Compiling .pio/build/sim/src/main.o',
    ].join('\n');
    const diags = parseGccDiagnostics(log);
    expect(diags).toEqual([
      { severity: 'error', file: 'src/main.c', line: 7, column: 5, message: "'foo' undeclared (first use in this function)" },
      { severity: 'warning', file: 'src/main.c', line: 12, column: 1, message: "unused variable 'x'" },
    ]);
  });

  it('maps fatal error to error and dedupes repeats', () => {
    const log = [
      'src/main.c:1:10: fatal error: nope.h: No such file or directory',
      'src/main.c:1:10: fatal error: nope.h: No such file or directory',
    ].join('\n');
    const diags = parseGccDiagnostics(log);
    expect(diags).toHaveLength(1);
    expect(diags[0].severity).toBe('error');
  });
});

describe('compile (validation, no toolchain needed)', () => {
  it('rejects an unsupported board before invoking pio', async () => {
    const r = await compile({ source: 'int main(){}', board: 'commodore64' });
    expect(r.ok).toBe(false);
    expect(r.diagnostics[0].message).toMatch(/no compile target/i);
  });

  it('rejects empty source', async () => {
    const r = await compile({ source: '   ', board: 'stm32l476' });
    expect(r.ok).toBe(false);
    expect(r.diagnostics[0].message).toMatch(/source is required/i);
  });
});
