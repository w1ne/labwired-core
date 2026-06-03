import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const execFileP = promisify(execFile);

const CLI_BIN = process.env.LABWIRED_CLI ?? 'labwired';
const EXEC_TIMEOUT_MS = 120_000;

const CLI_NOT_INSTALLED_MSG =
  `The 'labwired' CLI was not found on PATH. ` +
  `@labwired/mcp shells out to the LabWired simulator binary; ` +
  `install it with:\n` +
  `  curl -fsSL https://labwired.com/install.sh | sh\n` +
  `Or set the LABWIRED_CLI env var to the absolute path of an existing binary.`;

export interface CliResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export async function runCli(args: string[]): Promise<CliResult> {
  try {
    const { stdout, stderr } = await execFileP(CLI_BIN, args, {
      timeout: EXEC_TIMEOUT_MS,
      maxBuffer: 32 * 1024 * 1024,
    });
    return { stdout, stderr, exitCode: 0 };
  } catch (err: unknown) {
    const e = err as { stdout?: string; stderr?: string; code?: number | string; message?: string };
    if (e.code === 'ENOENT') {
      return { stdout: '', stderr: CLI_NOT_INSTALLED_MSG, exitCode: 127 };
    }
    return {
      stdout: e.stdout ?? '',
      stderr: e.stderr ?? e.message ?? '',
      exitCode: typeof e.code === 'number' ? e.code : 1,
    };
  }
}

export interface SimRun {
  resultJson: unknown;
  uartLog: string;
  stderr: string;
  exitCode: number;
  outputDir: string;
}

export async function runSimulation(opts: {
  firmwareBase64: string;
  systemYaml: string;
  scriptYaml: string;
  maxCycles?: number;
}): Promise<SimRun> {
  const work = await mkdtemp(join(tmpdir(), 'labwired-mcp-'));
  try {
    const firmwarePath = join(work, 'firmware.elf');
    const systemPath = join(work, 'system.yaml');
    const scriptPath = join(work, 'script.yaml');
    const outputDir = join(work, 'out');

    await writeFile(firmwarePath, Buffer.from(opts.firmwareBase64, 'base64'));
    await writeFile(systemPath, opts.systemYaml);
    await writeFile(scriptPath, opts.scriptYaml);

    const args = [
      'test',
      '--firmware',
      firmwarePath,
      '--system',
      systemPath,
      '--script',
      scriptPath,
      '--output-dir',
      outputDir,
      '--no-uart-stdout',
    ];
    if (opts.maxCycles) {
      args.push('--max-cycles', String(opts.maxCycles));
    }

    const { stderr, exitCode } = await runCli(args);

    let resultJson: unknown = null;
    let uartLog = '';
    try {
      resultJson = JSON.parse(await readFile(join(outputDir, 'result.json'), 'utf-8'));
    } catch {
      /* keep null when missing */
    }
    try {
      uartLog = await readFile(join(outputDir, 'uart.log'), 'utf-8');
    } catch {
      /* keep empty when missing */
    }

    return { resultJson, uartLog, stderr, exitCode, outputDir };
  } finally {
    await rm(work, { recursive: true, force: true }).catch(() => {});
  }
}

export async function validateSystem(systemYaml: string): Promise<CliResult> {
  const work = await mkdtemp(join(tmpdir(), 'labwired-mcp-validate-'));
  try {
    const systemPath = join(work, 'system.yaml');
    await writeFile(systemPath, systemYaml);
    return await runCli(['asset', 'validate', '--system', systemPath]);
  } finally {
    await rm(work, { recursive: true, force: true }).catch(() => {});
  }
}

export async function listChips(filter?: string): Promise<unknown> {
  const args = ['asset', 'list-chips', '--format', 'json'];
  if (filter) args.push('--filter', filter);
  const { stdout, stderr, exitCode } = await runCli(args);
  if (exitCode !== 0) {
    if (exitCode === 127) throw new Error(stderr);
    throw new Error(`labwired asset list-chips failed (exit ${exitCode}): ${stderr || '(no output)'}`);
  }
  try {
    return JSON.parse(stdout);
  } catch {
    return { raw: stdout };
  }
}

// ─── High-level lab runner ─────────────────────────────────────────────────
// `runLab` is the agent-friendly tool surface. The agent picks a board, hands
// us an ELF, says "run it for N cycles". We synthesize the test script YAML,
// invoke `labwired test`, read result.json + uart.log + the gpio trace, and
// pack a structured response.

const GPIO_EVENT_CAP = 10_000;
const UART_CAP_BYTES = 256 * 1024;

export interface RunLabOpts {
  /**
   * Path to the system YAML on disk. Preferred — preserves relative refs like
   * `chip: ../../configs/chips/<x>.yaml` and `descriptor: ../../configs/...`
   * which only resolve when the file is read from its real location.
   */
  systemYamlPath?: string;
  /**
   * Inline system YAML content. Written to a tempdir and used as the manifest.
   * BREAKS relative refs in the YAML — only safe for self-contained manifests.
   * If both are provided, `systemYamlPath` wins.
   */
  systemYaml?: string;
  chipYaml?: string; // currently unused — labwired-cli resolves chip from system manifest
  firmware: Buffer;
  maxCycles?: number;
  captureGpio?: boolean;
}

export interface RunLabResult {
  exit_code: number;
  exit_reason?: string;
  final_cycles?: number;
  final_pc_hex?: string;
  serial_output: string;
  serial_truncated: boolean;
  gpio_events?: Array<{ sim_cycle: number; pin: string; from: 0 | 1; to: 0 | 1 }>;
  gpio_truncated?: boolean;
  gpio_total_count?: number;
  raw_result: unknown;
  stderr: string;
}

export async function runLab(opts: RunLabOpts): Promise<RunLabResult> {
  if (!opts.systemYamlPath && !opts.systemYaml) {
    throw new Error('runLab: provide either systemYamlPath or systemYaml');
  }
  const work = await mkdtemp(join(tmpdir(), 'labwired-mcp-lab-'));
  try {
    const firmwarePath = join(work, 'firmware.elf');
    const scriptPath = join(work, 'script.yaml');
    const outputDir = join(work, 'out');

    await writeFile(firmwarePath, opts.firmware);

    // Use the on-disk path when available so relative `chip:` / `descriptor:`
    // refs in the system YAML resolve correctly. Fall back to writing inline
    // content into the tempdir (which breaks relative refs).
    let systemPath: string;
    if (opts.systemYamlPath) {
      systemPath = opts.systemYamlPath;
    } else {
      systemPath = join(work, 'system.yaml');
      await writeFile(systemPath, opts.systemYaml!);
    }

    // Synthesize a minimal "run for N cycles" script. We assert nothing
    // about stop_reason because only one stop reason can be true per run,
    // and multiple expected_stop_reason assertions all AND together (the
    // CLI's "normal stop" rule already treats max_cycles / max_steps / and
    // no_progress as success at the exit-code level when assertions pass).
    // `inputs:` is required by schema 1.0 even though `--firmware` /
    // `--system` flags override it.
    const maxCycles = opts.maxCycles ?? 10_000_000;
    const script = [
      'schema_version: "1.0"',
      'inputs:',
      `  firmware: ${JSON.stringify(firmwarePath)}`,
      `  system: ${JSON.stringify(systemPath)}`,
      'limits:',
      `  max_steps: ${maxCycles}`, // required by schema
      `  max_cycles: ${maxCycles}`,
      `  no_progress_steps: 1000`,
      'assertions: []',
    ].join('\n') + '\n';
    await writeFile(scriptPath, script);

    const args = [
      'test',
      '--firmware', firmwarePath,
      '--system', systemPath,
      '--script', scriptPath,
      '--output-dir', outputDir,
      '--no-uart-stdout',
      '--max-cycles', String(maxCycles),
    ];

    const { stderr, exitCode } = await runCli(args);

    // result.json + uart.log are always written by `test` mode (when the run starts)
    let raw_result: unknown = null;
    let serial_output = '';
    try {
      raw_result = JSON.parse(await readFile(join(outputDir, 'result.json'), 'utf-8'));
    } catch { /* keep null */ }
    try {
      serial_output = await readFile(join(outputDir, 'uart.log'), 'utf-8');
    } catch { /* keep empty */ }

    const serial_truncated = serial_output.length > UART_CAP_BYTES;
    if (serial_truncated) serial_output = serial_output.slice(-UART_CAP_BYTES);

    // Tease out structured fields from raw_result. Actual CLI schema:
    //   { stop_reason: "max_cycles"|"max_steps"|"no_progress"|...,
    //     cycles: number, cpu_state: { pc: number, ... }, ... }
    let exit_reason: string | undefined;
    let final_cycles: number | undefined;
    let final_pc_hex: string | undefined;
    if (raw_result && typeof raw_result === 'object') {
      const r = raw_result as Record<string, unknown>;
      if (typeof r.stop_reason === 'string') exit_reason = r.stop_reason;
      else if (typeof r.exit_reason === 'string') exit_reason = r.exit_reason; // legacy
      if (typeof r.cycles === 'number') final_cycles = r.cycles;
      else if (typeof r.total_cycles === 'number') final_cycles = r.total_cycles; // legacy
      const cpu = r.cpu_state;
      if (cpu && typeof cpu === 'object' && typeof (cpu as { pc?: unknown }).pc === 'number') {
        const pc = (cpu as { pc: number }).pc;
        final_pc_hex = `0x${pc.toString(16).toUpperCase().padStart(8, '0')}`;
      } else if (typeof r.final_pc === 'number') {
        final_pc_hex = `0x${r.final_pc.toString(16).toUpperCase().padStart(8, '0')}`;
      } else if (typeof r.final_pc === 'string') {
        final_pc_hex = r.final_pc;
      }
    }

    return {
      exit_code: exitCode,
      exit_reason,
      final_cycles,
      final_pc_hex,
      serial_output,
      serial_truncated,
      // GPIO capture is best-effort; the current `test` subcommand may not emit
      // a gpio trace. Left as undefined until the CLI grows that flag.
      gpio_events: undefined,
      raw_result,
      stderr,
    };
  } finally {
    await rm(work, { recursive: true, force: true }).catch(() => {});
  }
}

void GPIO_EVENT_CAP; // reserved for when CLI emits gpio traces
