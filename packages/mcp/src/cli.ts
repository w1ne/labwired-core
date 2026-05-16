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
