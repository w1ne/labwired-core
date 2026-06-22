import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { writeFile, readFile, mkdir, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { safeEnv } from './safe-env.js';
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';

const execFileAsync = promisify(execFile);

export interface RunRequest {
  elfBase64: string;
  systemYaml: string;
  /** Optional chip descriptor YAML.  When provided it is written alongside
   *  system.yaml and the `chip: "inline"` placeholder in systemYaml is
   *  rewritten to `chip: "chip.yaml"` so the CLI can resolve it. */
  chipYaml?: string;
  maxSteps: number;
}

export interface PeripheralState {
  id: string;
  type: string;
  state: unknown;
}

export interface RunDiagnosis {
  summary: string;
  faulting_pc?: string;
  symbol?: string;
  last_instructions?: string[];
  hint?: string;
}

export interface RunResult {
  status: string;
  stopReason: string;
  stepsExecuted: number;
  cycles: number;
  instructions: number;
  serial: string;
  peripherals: PeripheralState[];
  timedOut: boolean;
  diagnosis?: RunDiagnosis;
}

/** Attempt to resolve a decimal PC address to a symbol via arm-none-eabi-addr2line.
 *  Returns null if the tool is absent or fails — never throws. */
async function resolveSymbol(elfPath: string, pcDecimal: number): Promise<string | null> {
  try {
    const pcHex = '0x' + pcDecimal.toString(16);
    const { stdout } = await execFileAsync(
      'arm-none-eabi-addr2line',
      ['-f', '-e', elfPath, pcHex],
      { timeout: 5000, env: safeEnv() },
    );
    const lines = stdout.trim().split('\n');
    // Output is: function name on line 0, file:line on line 1
    const fn = lines[0]?.trim();
    if (fn && fn !== '??' && fn !== '') return fn;
    return null;
  } catch {
    return null;
  }
}

/** Build last_instructions from the trace: take up to last 5 entries.
 *  Each entry is a human-readable string like "0x080001a8 (cycle 207)". */
function buildLastInstructions(trace: unknown[]): string[] {
  const tail = trace.slice(-5);
  return tail.map((entry) => {
    const e = entry as Record<string, unknown>;
    const pcDec = typeof e.pc === 'number' ? e.pc : 0;
    const pcHex = '0x' + pcDec.toString(16).padStart(8, '0');
    const cycle = typeof e.cycle === 'number' ? ` (cycle ${e.cycle})` : '';
    const mnemonic = typeof e.mnemonic === 'string' && e.mnemonic ? ` ${e.mnemonic}` : '';
    return `${pcHex}${mnemonic}${cycle}`;
  });
}

/** Build a diagnosis from the stop_reason, cpu_state, and optional trace. */
async function buildDiagnosis(
  stopReason: string,
  maxSteps: number,
  cpuState: Record<string, unknown> | null,
  trace: unknown[] | null,
  elfPath: string,
): Promise<RunDiagnosis | undefined> {
  // No diagnosis needed for clean completion
  if (!stopReason || stopReason === 'finished' || stopReason === 'pass') {
    return { summary: 'Ran to completion.' };
  }

  const pcDecimal = (cpuState?.pc as number | undefined) ?? null;
  const pcHex = pcDecimal !== null ? '0x' + pcDecimal.toString(16).padStart(8, '0') : undefined;
  const lastInstructions = trace && trace.length > 0 ? buildLastInstructions(trace) : undefined;

  // Resolve symbol from ELF if PC is available
  const symbol = pcDecimal !== null ? (await resolveSymbol(elfPath, pcDecimal)) ?? undefined : undefined;

  if (stopReason === 'max_steps' || stopReason === 'StepLimit' || stopReason === 'step_limit') {
    return {
      summary:
        `Ran the full ${maxSteps}-step budget without halting — likely an infinite loop or a ` +
        `busy-wait on a status bit that never changes (a common cause: polling a peripheral ` +
        `the twin does not model, e.g. an RCC ready flag or an unmodeled timer).`,
      faulting_pc: pcHex,
      symbol,
      last_instructions: lastInstructions,
      hint:
        'Check for spin-loops that wait on a hardware status bit. ' +
        'Prefer software delay loops (count-down) over peripheral-flag polls when targeting the sim. ' +
        'Consult docs/firmware-scaffolds/README.md for the list of modeled peripherals.',
    };
  }

  if (stopReason === 'config_error') {
    return {
      summary: 'Simulation failed at configuration time — the system YAML or chip descriptor could not be loaded. Check the diagram and target configuration.',
      hint: 'Ensure the diagram board matches the target and all required peripherals are present.',
    };
  }

  // Generic fault / memory error
  if (
    stopReason.includes('fault') ||
    stopReason.includes('error') ||
    stopReason.includes('invalid') ||
    stopReason.includes('bus')
  ) {
    const addrNote = pcHex ? ` at PC ${pcHex}` : '';
    return {
      summary:
        `Simulation stopped due to a hardware fault or invalid memory access${addrNote}. ` +
        `This is typically caused by accessing an address outside modeled flash/RAM/peripherals ` +
        `(an unmodeled peripheral register or a bad pointer).`,
      faulting_pc: pcHex,
      symbol,
      last_instructions: lastInstructions,
      hint:
        'Verify that all peripheral base addresses used in firmware are in the modeled set. ' +
        'See docs/firmware-scaffolds/README.md for the modeled peripheral list.',
    };
  }

  // Unknown stop reason — still give something useful
  return {
    summary: `Simulation stopped with reason: "${stopReason}".`,
    faulting_pc: pcHex,
    symbol,
    last_instructions: lastInstructions,
  };
}

/** Resolve the manifest's `chip:` field so the CLI can load it.
 *  - explicit chipYamlOverride  → write it as chip.yaml, rewrite `chip: "inline"`.
 *  - bare id (e.g. "esp32c3")   → resolve from CHIP_YAMLS, rewrite to chip.yaml.
 *  - path / ".yaml" / "inline"  → leave untouched (CLI resolves on disk).
 *  Throws a listing error on an unknown bare id. */
export function resolveChipInManifest(
  systemYaml: string,
  chipYamlOverride?: string,
): { systemYaml: string; chipYaml?: string } {
  const rewriteToFile = (s: string) =>
    s.replace(/^chip:\s*["']?[A-Za-z0-9_.\-/]+["']?\s*$/m, 'chip: "chip.yaml"');

  if (chipYamlOverride) {
    return { systemYaml: rewriteToFile(systemYaml), chipYaml: chipYamlOverride };
  }
  const m = systemYaml.match(/^chip:\s*["']?([A-Za-z0-9_.\-/]+)["']?\s*$/m);
  if (!m) return { systemYaml };
  const val = m[1];
  if (val === 'inline' || val.includes('/') || val.endsWith('.yaml')) {
    return { systemYaml };
  }
  const yaml = CHIP_YAMLS[val];
  if (!yaml) {
    const known = Object.keys(CHIP_YAMLS).sort().join(', ');
    throw new Error(
      `unknown chip id "${val}". Known chip ids: ${known}. ` +
        'Call labwired_lookup with of:"chips" for ids and their peripheral names.',
    );
  }
  return { systemYaml: rewriteToFile(systemYaml), chipYaml: yaml };
}

export async function run(req: RunRequest): Promise<RunResult> {
  const tmp = join('/tmp', `lwb-run-${randomUUID()}`);
  await mkdir(tmp, { recursive: true });
  try {
    const elfPath = join(tmp, 'firmware.elf');
    const systemPath = join(tmp, 'system.yaml');
    const scriptPath = join(tmp, 'script.yaml');
    const outputDir = join(tmp, 'output');
    await mkdir(outputDir, { recursive: true });

    // Write ELF bytes decoded from base64
    await writeFile(elfPath, Buffer.from(req.elfBase64, 'base64'));

    const resolved = resolveChipInManifest(req.systemYaml, req.chipYaml);
    const systemYaml = resolved.systemYaml;
    if (resolved.chipYaml) {
      await writeFile(join(tmp, 'chip.yaml'), resolved.chipYaml);
    }

    // Write system manifest
    await writeFile(systemPath, systemYaml);
    // Write minimal script.yaml — inputs are overridden by -f/-s CLI flags
    const scriptYaml = [
      'schema_version: "1.0"',
      'inputs:',
      `  firmware: "${elfPath}"`,
      `  system: "${systemPath}"`,
      'limits:',
      `  max_steps: ${req.maxSteps}`,
      'assertions: []',
    ].join('\n') + '\n';
    await writeFile(scriptPath, scriptYaml);

    const bin = process.env.LABWIRED_BIN ?? 'labwired';
    const args = [
      'test',
      '-f', elfPath,
      '-s', systemPath,
      '-c', scriptPath,
      '--output-dir', outputDir,
      '--max-steps', String(req.maxSteps),
      '--trace',
      '--trace-max', '200',
      '--no-key',
      '--no-uart-stdout',
    ];

    // Suppress secrets from the subprocess env via safeEnv().
    // The CLI exits with code 3 on a simulation runtime error but still writes
    // result.json — swallow the non-zero exit so we can read that structured
    // result below.
    await execFileAsync(bin, args, { timeout: 60000, env: safeEnv() }).catch(() => {});

    const resultJson = await readFile(join(outputDir, 'result.json'), 'utf8');
    const result = JSON.parse(resultJson);

    let serial = '';
    try {
      serial = await readFile(join(outputDir, 'uart.log'), 'utf8');
    } catch {
      // uart.log may not exist if nothing was emitted
    }

    // Read trace (bounded to 200 entries; may not exist on config error)
    let trace: unknown[] | null = null;
    try {
      const traceJson = await readFile(join(outputDir, 'trace.json'), 'utf8');
      trace = JSON.parse(traceJson) as unknown[];
    } catch {
      // trace.json not present (e.g. config_error before sim starts)
    }

    const stopReason: string = result.stop_reason ?? '';
    // Real values observed: "max_steps" when the step limit is hit.
    const timedOut = stopReason === 'max_steps' || stopReason === 'StepLimit';

    const cpuState = result.cpu_state && typeof result.cpu_state === 'object'
      ? result.cpu_state as Record<string, unknown>
      : null;

    const diagnosis = await buildDiagnosis(
      stopReason,
      req.maxSteps,
      cpuState,
      trace,
      elfPath,
    );

    return {
      status: result.status ?? '',
      stopReason,
      stepsExecuted: result.steps_executed ?? 0,
      cycles: result.cycles ?? 0,
      instructions: result.instructions ?? 0,
      serial,
      peripherals: [],  // No top-level peripherals in result.json (v1: status + serial + cycles only)
      timedOut,
      diagnosis,
    };
  } finally {
    await rm(tmp, { recursive: true, force: true }).catch(() => {});
  }
}
