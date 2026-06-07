import { describe, it, expect } from 'vitest';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { run } from '../src/run';
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';

const __dirname = dirname(fileURLToPath(import.meta.url));

describe('run', () => {
  it('runs a known stm32l476 ELF and returns status + serial', async () => {
    const elfBase64 = (await readFile(join(__dirname, 'fixtures/blink-l476.elf'))).toString('base64');
    const systemYaml = await readFile(join(__dirname, 'fixtures/blink-l476.system.yaml'), 'utf8');
    const r = await run({ elfBase64, systemYaml, chipYaml: CHIP_YAMLS.stm32l476, maxSteps: 200000 });
    expect(typeof r.status).toBe('string');
    expect(typeof r.serial).toBe('string');
    expect(r.cycles).toBeGreaterThanOrEqual(0);
    // Real stop_reason from CLI is "max_steps" (lowercase) when step limit hit
    expect(r.timedOut).toBe(r.stopReason === 'max_steps' || r.stopReason === 'StepLimit');
  }, 60000);

  it('returns a non-empty diagnosis.summary on a step-limit run', async () => {
    const elfBase64 = (await readFile(join(__dirname, 'fixtures/blink-l476.elf'))).toString('base64');
    const systemYaml = await readFile(join(__dirname, 'fixtures/blink-l476.system.yaml'), 'utf8');
    // Use a tiny step budget to guarantee a step-limit stop
    const r = await run({ elfBase64, systemYaml, chipYaml: CHIP_YAMLS.stm32l476, maxSteps: 500 });
    expect(r.stopReason).toBe('max_steps');
    expect(r.diagnosis).toBeDefined();
    expect(typeof r.diagnosis!.summary).toBe('string');
    expect(r.diagnosis!.summary.length).toBeGreaterThan(0);
    // faulting_pc should be a hex address string
    expect(r.diagnosis!.faulting_pc).toMatch(/^0x[0-9a-f]+$/i);
    // hint should be present for step-limit
    expect(typeof r.diagnosis!.hint).toBe('string');
  }, 60000);
});
