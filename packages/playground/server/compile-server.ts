/**
 * Lightweight Arduino/C compile server for LabWired playground.
 * Compiles source code to ELF using arm-none-eabi-gcc.
 *
 * Usage: npx tsx server/compile-server.ts
 */
import { createServer, IncomingMessage, ServerResponse } from 'node:http';
import { writeFile, readFile, mkdir, rm } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { join, dirname } from 'node:path';
import { randomUUID } from 'node:crypto';

const execFileAsync = promisify(execFile);

const PORT = 3001;
const GCC = 'arm-none-eabi-gcc';
const OBJCOPY = 'arm-none-eabi-objcopy';
const ARDUINO_CORE = join(dirname(new URL(import.meta.url).pathname), 'arduino-core');

interface CompileRequest {
  source: string;
  language: 'c' | 'cpp' | 'arduino';
  target: string;
}

interface CompileResponse {
  success: boolean;
  elf?: string; // base64-encoded ELF
  errors: string[];
  output: string;
}

const CFLAGS = [
  '-mcpu=cortex-m3',
  '-mthumb',
  '-g',
  '-O1',
  '-ffreestanding',
  '-nostdlib',
  '-ffunction-sections',
  '-fdata-sections',
  '-Wall',
  `-I${ARDUINO_CORE}`,
];

const LDFLAGS = [
  '-mcpu=cortex-m3',
  '-mthumb',
  '-nostdlib',
  '-Wl,--gc-sections',
  `-T${join(ARDUINO_CORE, 'stm32f103.ld')}`,
];

async function compile(req: CompileRequest): Promise<CompileResponse> {
  const tmpDir = join('/tmp', `labwired-compile-${randomUUID()}`);
  await mkdir(tmpDir, { recursive: true });

  try {
    const isArduino = req.language === 'arduino';
    const ext = req.language === 'cpp' || isArduino ? '.cpp' : '.c';
    const sketchFile = join(tmpDir, `sketch${ext}`);
    const mainObj = join(ARDUINO_CORE, 'main.c');
    const elfFile = join(tmpDir, 'firmware.elf');

    // For Arduino: prepend include
    let source = req.source;
    if (isArduino) {
      source = `#include "Arduino.h"\n${source}`;
    }

    await writeFile(sketchFile, source);

    const compiler = req.language === 'cpp' || isArduino ? 'arm-none-eabi-g++' : GCC;
    const args: string[] = [];

    if (isArduino) {
      // Compile sketch + main wrapper together
      args.push(
        ...CFLAGS,
        '-std=c++17',
        '-fno-exceptions',
        '-fno-rtti',
        sketchFile,
        mainObj,
        ...LDFLAGS,
        '-o', elfFile,
      );
    } else {
      // Plain C/C++ — user provides their own main()
      const stdFlag = req.language === 'cpp' ? '-std=c++17' : '-std=c11';
      args.push(
        ...CFLAGS,
        stdFlag,
        ...(req.language === 'cpp' ? ['-fno-exceptions', '-fno-rtti'] : []),
        sketchFile,
        ...LDFLAGS,
        '-o', elfFile,
      );
    }

    let output = '';
    try {
      const result = await execFileAsync(compiler, args, { timeout: 10000 });
      output = result.stdout + result.stderr;
    } catch (e: unknown) {
      const err = e as { stdout?: string; stderr?: string; message?: string };
      const errOutput = (err.stdout || '') + (err.stderr || '');
      // Parse errors
      const errors = errOutput
        .split('\n')
        .filter((l: string) => l.includes('error:') || l.includes('warning:'))
        .map((l: string) => l.replace(sketchFile, 'sketch' + ext));
      return {
        success: false,
        errors,
        output: errOutput.replace(new RegExp(tmpDir, 'g'), '.') || err.message || 'Compilation failed',
      };
    }

    // Read ELF
    const elfData = await readFile(elfFile);
    const elfBase64 = elfData.toString('base64');

    return {
      success: true,
      elf: elfBase64,
      errors: [],
      output: output || 'Compilation successful.',
    };
  } finally {
    await rm(tmpDir, { recursive: true, force: true }).catch(() => {});
  }
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on('data', (chunk) => chunks.push(chunk));
    req.on('end', () => resolve(Buffer.concat(chunks).toString()));
    req.on('error', reject);
  });
}

const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
  // CORS
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  if (req.method === 'POST' && req.url === '/api/compile') {
    try {
      const body = await readBody(req);
      const request: CompileRequest = JSON.parse(body);
      const result = await compile(request);
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(result));
    } catch (e) {
      res.writeHead(400, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ success: false, errors: [String(e)], output: '' }));
    }
    return;
  }

  res.writeHead(404);
  res.end('Not found');
});

server.listen(PORT, () => {
  console.log(`LabWired compile server running on http://localhost:${PORT}`);
  console.log(`Arduino core: ${ARDUINO_CORE}`);
});
