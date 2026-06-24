import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { safeEnv } from './safe-env.js';

const execFileAsync = promisify(execFile);

// A supplied build runs a single test script that may itself drive a long
// IO-Link scenario (the C3 thermal scenarios run ~50M steps). Match the
// per-scenario budget /run-example uses.
const RUN_TIMEOUT_MS = 300_000;

/**
 * Generic build run: the agent SUPPLIES the firmware ELF (base64), the system
 * manifest (YAML), and the test script (YAML). Nothing is baked in — this is the
 * oracle-run for ANY build, the counterpart to /run-example (which runs a
 * curated in-image example by id). The honest verdict (assertions + the lines
 * the device/master emitted) is reported back so a publisher can gate on a REAL
 * server-side run, never a self-reported claim.
 */
export interface RunBuildRequest {
  /** Base64-encoded firmware ELF. */
  firmware_base64: string;
  /** LabWired system manifest (YAML) — its `chip:` path may use the standard
   *  `../../configs/chips/<chip>.yaml` convention and will resolve against the
   *  in-image config tree. */
  system_yaml: string;
  /** LabWired test script (YAML, schema_version "1.0") with assertions. The
   *  script's own inputs.firmware / inputs.system are overridden by the supplied
   *  firmware + manifest, so the script need not reference any particular file
   *  names — only its limits + assertions are used. */
  test_yaml: string;
}

export interface RunBuildResult {
  /** True iff the build was accepted and a run was attempted (transport-level OK).
   *  Test outcome is `passed`. */
  ok: boolean;
  /** True iff status === "pass" AND every assertion passed. */
  passed: boolean;
  /** Worst process exit code from the CLI. */
  exit_code: number;
  /** Test run status from result.json (e.g. "pass" / "fail"). */
  status: string;
  /** Why the run stopped (e.g. "finished", "max_steps"). */
  stop_reason: string;
  /** Per-assertion pass/fail for transparency. */
  assertions: { assertion: unknown; passed: boolean }[];
  /** Lines the device/master emitted into UART (the honest proof) — every line
   *  prefixed "MASTER " or "TFS " kept, capped. */
  verdict_lines: string[];
  /** Tail of the UART log for context. */
  uart_excerpt: string;
  error?: string;
}

function repoRoot(): string {
  const fromEnv = process.env.LABWIRED_REPO_ROOT;
  if (fromEnv) return fromEnv;
  return process.cwd();
}

/** Lines worth surfacing as the verdict proof: the device's human-readable
 *  verdicts ("TFS ...") and the IO-Link master's observations ("MASTER ..."). */
function verdictLines(uart: string): string[] {
  return uart
    .split('\n')
    .map((l) => l.replace(/\r$/, ''))
    .filter((l) => l.startsWith('MASTER ') || l.startsWith('TFS '))
    .slice(-200);
}

function fail(error: string, exit_code = 1): RunBuildResult {
  return {
    ok: false,
    passed: false,
    exit_code,
    status: '',
    stop_reason: '',
    assertions: [],
    verdict_lines: [],
    uart_excerpt: '',
    error,
  };
}

export async function runBuild(req: RunBuildRequest): Promise<RunBuildResult> {
  if (typeof req?.firmware_base64 !== 'string' || req.firmware_base64.trim() === '') {
    return fail('missing firmware_base64');
  }
  if (typeof req?.system_yaml !== 'string' || req.system_yaml.trim() === '') {
    return fail('missing system_yaml');
  }
  if (typeof req?.test_yaml !== 'string' || req.test_yaml.trim() === '') {
    return fail('missing test_yaml');
  }

  let elf: Buffer;
  try {
    elf = Buffer.from(req.firmware_base64, 'base64');
  } catch {
    return fail('firmware_base64 is not valid base64');
  }
  if (elf.length === 0) return fail('firmware_base64 decoded to 0 bytes');

  // Write the supplied build into an ephemeral dir UNDER the in-image example
  // tree (core/examples/<uuid>/). This is the path contract every example uses:
  // a system manifest's `chip: "../../configs/chips/<chip>.yaml"` then resolves
  // to the real, baked-in config tree. The dir name is a fresh UUID slug (no
  // caller input in any path component → no traversal), and is removed after.
  const id = `_build-${randomUUID()}`;
  const buildDir = join(repoRoot(), 'core', 'examples', id);
  const firmwareDir = join(buildDir, 'firmware');
  const outputDir = join(buildDir, '_output');

  try {
    await mkdir(firmwareDir, { recursive: true });
    await mkdir(outputDir, { recursive: true });

    const elfPath = join(firmwareDir, 'firmware.elf');
    const systemPath = join(buildDir, 'system.yaml');
    const testPath = join(buildDir, 'test.yaml');
    await writeFile(elfPath, elf);
    await writeFile(systemPath, req.system_yaml);
    await writeFile(testPath, req.test_yaml);

    const bin = process.env.LABWIRED_BIN ?? 'labwired';
    // Run from the build dir so the manifest's relative `chip:` (and any other
    // relative inputs) resolve exactly as they would for a real example. Pass
    // -f/-s explicitly so the SUPPLIED firmware + manifest are authoritative,
    // regardless of what the test script's own inputs.* happen to name.
    let exitCode = 0;
    try {
      await execFileAsync(
        bin,
        [
          'test',
          '--script', 'test.yaml',
          '-f', 'firmware/firmware.elf',
          '-s', 'system.yaml',
          '--output-dir', '_output',
          '--no-uart-stdout',
        ],
        { cwd: buildDir, timeout: RUN_TIMEOUT_MS, env: safeEnv(), maxBuffer: 16 * 1024 * 1024 },
      );
    } catch (err) {
      const e = err as { code?: number | string };
      exitCode = typeof e.code === 'number' ? e.code : 1;
    }

    let status = '';
    let stopReason = '';
    let assertions: { assertion: unknown; passed: boolean }[] = [];
    try {
      const result = JSON.parse(await readFile(join(outputDir, 'result.json'), 'utf8'));
      status = result.status ?? '';
      stopReason = result.stop_reason ?? '';
      if (Array.isArray(result.assertions)) {
        assertions = result.assertions.map((a: { assertion: unknown; passed: boolean }) => ({
          assertion: a.assertion,
          passed: a.passed === true,
        }));
      }
    } catch {
      // result.json missing → treated as failure below.
    }

    let uart = '';
    try {
      uart = await readFile(join(outputDir, 'uart.log'), 'utf8');
    } catch {
      // uart.log may be absent on early failure.
    }

    const allAsserted = assertions.length > 0 && assertions.every((a) => a.passed);
    const passed = status === 'pass' && allAsserted;

    return {
      ok: true,
      passed,
      exit_code: exitCode,
      status,
      stop_reason: stopReason,
      assertions,
      verdict_lines: verdictLines(uart),
      uart_excerpt: uart.slice(-8000),
    };
  } catch (err) {
    return fail(err instanceof Error ? err.message : String(err));
  } finally {
    await rm(buildDir, { recursive: true, force: true }).catch(() => {});
  }
}
