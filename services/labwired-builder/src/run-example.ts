import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, mkdir, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { safeEnv } from './safe-env.js';

const execFileAsync = promisify(execFile);

const RUN_TIMEOUT_MS = 300_000; // the C3 thermal example runs 50M steps per scenario

/**
 * Examples whose firmware ELF + manifests are BAKED INTO the builder image.
 * Only these ids may be run via /run-example — the id is never interpolated
 * into a shell, but we still allowlist it (path-safety + a clear contract).
 *
 * Each example declares the IO-Link test scripts to run (relative to the
 * example dir under <repoRoot>/core/examples/<id>/). Scenarios run in order;
 * all must pass for the example to pass.
 */
interface ExampleSpec {
  /** Test scripts to run, relative to the example directory. */
  scripts: string[];
}

const EXAMPLE_ALLOWLIST: Record<string, ExampleSpec> = {
  'esp32c3-mlx90640-thermal': {
    scripts: ['test-iolink.yaml', 'test-iolink-fault.yaml'],
  },
};

export interface RunExampleRequest {
  example_id: string;
}

export interface RunExampleScenarioResult {
  script: string;
  passed: boolean;
  status: string;
  stop_reason: string;
  /** Per-assertion pass/fail for transparency. */
  assertions: { assertion: unknown; passed: boolean }[];
  /** "MASTER ..." lines the IO-Link master emitted into the UART log. */
  master_verdict_lines: string[];
  /** Tail of the UART log for context. */
  uart_excerpt: string;
}

export interface RunExampleResult {
  ok: boolean;
  example_id: string;
  /** True iff every scenario passed. */
  passed: boolean;
  exit_code: number;
  scenarios: RunExampleScenarioResult[];
  /** Flattened "MASTER ..." lines across all scenarios (the honest proof). */
  master_verdict_lines: string[];
  /** Combined UART tail across scenarios. */
  uart_excerpt: string;
  error?: string;
}

function repoRoot(): string {
  // The service runs with the repo root as LABWIRED_REPO_ROOT (matches the MCP
  // resolution). In the builder image this is /app/repo (set in the Dockerfile).
  const fromEnv = process.env.LABWIRED_REPO_ROOT;
  if (fromEnv) return resolve(fromEnv);
  return process.cwd();
}

function masterLines(uart: string): string[] {
  return uart
    .split('\n')
    .map((l) => l.replace(/\r$/, ''))
    .filter((l) => l.startsWith('MASTER '));
}

function fail(example_id: string, error: string): RunExampleResult {
  return {
    ok: false,
    example_id,
    passed: false,
    exit_code: 1,
    scenarios: [],
    master_verdict_lines: [],
    uart_excerpt: '',
    error,
  };
}

/**
 * Run a BAKED-IN example end-to-end inside the builder container: invoke the
 * `labwired` CLI against the example's IO-Link test scripts (which reference the
 * pre-built firmware ELF + the chip/system YAMLs already present in the image)
 * and report the real verdict the IO-Link master observed. No agent-supplied
 * firmware, no source: the example is the unit of verification.
 */
export async function runExample(req: RunExampleRequest): Promise<RunExampleResult> {
  const id = req.example_id;
  // Path-safety + allowlist: only known examples, and the id must be a plain
  // slug (defence-in-depth against traversal even though it is allowlisted).
  if (typeof id !== 'string' || !/^[a-z0-9][a-z0-9-]*$/.test(id)) {
    return fail(String(id), `invalid example_id "${id}" (expected a lowercase slug)`);
  }
  const spec = EXAMPLE_ALLOWLIST[id];
  if (!spec) {
    const known = Object.keys(EXAMPLE_ALLOWLIST).join(', ');
    return fail(id, `unknown example_id "${id}". Known: ${known || '(none)'}.`);
  }

  const exampleDir = join(repoRoot(), 'core', 'examples', id);
  if (!existsSync(exampleDir)) {
    return fail(id, `example dir not present in image: ${exampleDir}`);
  }

  const bin = process.env.LABWIRED_BIN ?? 'labwired';
  const scenarios: RunExampleScenarioResult[] = [];
  let worstExit = 0;

  for (const script of spec.scripts) {
    const scriptPath = join(exampleDir, script);
    if (!existsSync(scriptPath)) {
      return fail(id, `test script missing in image: ${scriptPath}`);
    }
    const outputDir = join('/tmp', `lwb-example-${randomUUID()}`);
    await mkdir(outputDir, { recursive: true });
    try {
      // Run from the example dir so the script's relative inputs
      // (./firmware/...elf, ./system-iolink.yaml → ../../configs/...) resolve.
      let exitCode = 0;
      try {
        await execFileAsync(
          bin,
          ['test', '--script', script, '--output-dir', outputDir, '--no-uart-stdout'],
          { cwd: exampleDir, timeout: RUN_TIMEOUT_MS, env: safeEnv(), maxBuffer: 16 * 1024 * 1024 },
        );
      } catch (err) {
        const e = err as { code?: number | string };
        exitCode = typeof e.code === 'number' ? e.code : 1;
      }
      if (exitCode > worstExit) worstExit = exitCode;

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
        // result.json missing → treat as failure below.
      }

      let uart = '';
      try {
        uart = await readFile(join(outputDir, 'uart.log'), 'utf8');
      } catch {
        // uart.log may be absent on early failure.
      }

      const allAsserted = assertions.length > 0 && assertions.every((a) => a.passed);
      const passed = status === 'pass' && allAsserted;

      scenarios.push({
        script,
        passed,
        status,
        stop_reason: stopReason,
        assertions,
        master_verdict_lines: masterLines(uart),
        uart_excerpt: uart.slice(-4000),
      });
    } finally {
      await rm(outputDir, { recursive: true, force: true }).catch(() => {});
    }
  }

  const passed = scenarios.length === spec.scripts.length && scenarios.every((s) => s.passed);
  return {
    ok: true,
    example_id: id,
    passed,
    exit_code: worstExit,
    scenarios,
    master_verdict_lines: scenarios.flatMap((s) => s.master_verdict_lines),
    uart_excerpt: scenarios.map((s) => `# ${s.script}\n${s.uart_excerpt}`).join('\n\n').slice(-8000),
  };
}

/** Exposed for tests + the publish gate. */
export function allowlistedExamples(): string[] {
  return Object.keys(EXAMPLE_ALLOWLIST);
}
