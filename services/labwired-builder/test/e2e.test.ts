import { describe, it, expect } from 'vitest';
import { run } from '../src/run';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

const maybe = process.env.RUN_E2E ? describe : describe.skip;

maybe('e2e: run blink ELF on stm32l476 (run-only)', () => {
  it('runs the committed blink-l476.elf fixture to a step-limit stop with diagnosis present', async () => {
    const elfBase64 = (await readFile(join(__dirname, 'fixtures/blink-l476.elf'))).toString('base64');
    const systemYaml = await readFile(join(__dirname, 'fixtures/blink-l476.system.yaml'), 'utf8');
    const { CHIP_YAMLS } = await import('../../../packages/board-config/src/chip-yamls');
    const r = await run({ elfBase64, systemYaml, chipYaml: CHIP_YAMLS.stm32l476, maxSteps: 500000 });
    // The blink ELF loops indefinitely — expect max_steps or a clean pass
    expect(['pass', 'finished', 'max_steps', 'step_limit', 'StepLimit']).toContain(r.status || r.stopReason);
    expect(r.cycles).toBeGreaterThan(0);
    // diagnosis must always be present
    expect(r.diagnosis).toBeDefined();
    expect(typeof r.diagnosis!.summary).toBe('string');
    expect(r.diagnosis!.summary.length).toBeGreaterThan(0);
  }, 120000);
});
