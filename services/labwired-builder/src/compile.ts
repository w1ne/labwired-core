import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { writeFile, readFile, mkdir, rm, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { safeEnv } from './safe-env.js';
import { PIO_BOARDS, CHIP_FAMILIES, resolveBoard, type PioBoard } from './boards.js';

const execFileAsync = promisify(execFile);

const MAX_SOURCE_BYTES = 256 * 1024;
const COMPILE_TIMEOUT_MS = 240_000; // pio can fetch toolchains on a cold cache
const MAX_LOG = 16 * 1024;
const PIO_ENV = 'sim'; // fixed [env:sim] name in the generated platformio.ini

export type CompileLanguage = 'c' | 'cpp';

export interface CompileRequest {
  /** Agent-authored firmware source. Dropped into src/ of a project whose
   *  platformio.ini WE generate — the caller supplies no build scripts, so the
   *  surface is "pio building our project", not arbitrary build-time code. */
  source: string;
  /** LabWired board id (see PIO_BOARDS). */
  board?: string;
  /** Alias for `board` — proto.cat names this field `labwired_board_id`. */
  labwired_board_id?: string;
  /** Fallback when no board id matches: resolve via CHIP_FAMILIES. */
  chip_family?: string;
  language?: CompileLanguage;
  /** Extra PlatformIO library dependencies (registry ids, git urls). Merged on
   *  top of the board's default libDeps. Accepts a list or a comma/newline
   *  string. This is the ONE agent-influenced build field, and it only adds
   *  libraries — no build scripts/flags. */
  lib_deps?: string[] | string;
}

export interface CompileDiagnostic {
  severity: 'error' | 'warning';
  file?: string;
  line?: number;
  column?: number;
  message: string;
}

/** A flashable binary + its flash offset, for in-browser Web Serial flashing
 *  (esptool-js). Only emitted for ESP32 (Arduino) targets. */
export interface FlashImage {
  offset: number;
  dataBase64: string;
}

export interface CompileResult {
  ok: boolean;
  /** Base64 ELF, present only when ok. Feed directly to /run. */
  elfBase64?: string;
  diagnostics: CompileDiagnostic[];
  /** Truncated build log, present on failure. */
  log?: string;
  /** Whether the LabWired sim can currently RUN this target (compile != run). */
  runnable?: boolean;
  /** Resolved PlatformIO board id (e.g. 'esp32dev'). */
  platformioBoard?: string;
  /** Resolved framework (e.g. 'arduino'). */
  framework?: string;
  /** How the board was resolved, e.g. 'board[esp32]' or 'chip_family[esp32s3]'. */
  mappingSource?: string;
  /** ESP32 flashable images (bootloader/partitions/boot_app0/firmware). */
  flashImages?: FlashImage[];
  /** Flash chip the images target (esp32 | esp32s3 | …). */
  flashChip?: string;
}

/** Boards the hosted compiler can build. Single source of truth for the MCP tool. */
export function supportedCompileBoards(): { board: string; runnable: boolean }[] {
  return Object.entries(PIO_BOARDS).map(([board, b]) => ({ board, runnable: b.runnable }));
}

/** Chip families the compiler can resolve as a fallback (proto.cat parity). */
export function supportedChipFamilies(): { chipFamily: string; runnable: boolean }[] {
  return Object.entries(CHIP_FAMILIES).map(([chipFamily, b]) => ({ chipFamily, runnable: b.runnable }));
}

/** Every distinct platformio.ini the runtime can ask for — the union of
 *  PIO_BOARDS and CHIP_FAMILIES, deduped by ini content. The build-time baker
 *  (warm-cache.ts) compiles each one so the egress-denied runtime can build any
 *  resolvable target offline. Derived from the catalog — no second list. */
export function bakeTargets(): { label: string; ini: string; isArduino: boolean }[] {
  const seen = new Set<string>();
  const out: { label: string; ini: string; isArduino: boolean }[] = [];
  const consider = (label: string, b: PioBoard) => {
    if (b.builder === 'zephyr') return; // Zephyr is baked via west in the image, not pio
    const ini = iniFor(b);
    if (seen.has(ini)) return;
    seen.add(ini);
    out.push({ label, ini, isArduino: b.framework === 'arduino' });
  };
  for (const [id, b] of Object.entries(PIO_BOARDS)) consider(id, b);
  for (const [fam, b] of Object.entries(CHIP_FAMILIES)) consider(`chip:${fam}`, b);
  return out;
}

function normalizeLibDeps(input?: string[] | string): string[] {
  if (!input) return [];
  const list = Array.isArray(input) ? input : input.replace(/,/g, '\n').split('\n');
  return list.map((d) => d.trim()).filter(Boolean);
}

/** Render a platformio.ini for a resolved board. Pure. The only caller-influenced
 *  field is lib_deps (additive library list); platform/board/framework/extra are
 *  ours. */
function iniFor(b: PioBoard, libDeps: string[] = []): string {
  const merged = [...(b.libDeps ?? []), ...libDeps];
  const lines = [
    `[env:${PIO_ENV}]`,
    `platform = ${b.platform}`,
    `board = ${b.board}`,
  ];
  if (b.framework) lines.push(`framework = ${b.framework}`);
  if (b.extra) lines.push(...b.extra);
  if (merged.length) lines.push('lib_deps =', ...merged.map((d) => `    ${d}`));
  return lines.join('\n') + '\n';
}

/** Generate a platformio.ini for a board id. Pure; tested. No agent-controlled
 *  fields except an optional additive lib_deps list. Returns null for an unknown
 *  board id (chip-family resolution lives in compile()). */
export function generatePlatformioIni(board: string, libDeps?: string[] | string): string | null {
  const b = PIO_BOARDS[board];
  if (!b) return null;
  if (b.builder === 'zephyr') return null; // Zephyr targets have no platformio.ini
  return iniFor(b, normalizeLibDeps(libDeps));
}

// --- Zephyr (west) build path -------------------------------------------------
// A Zephyr target is built with `west build` against the image's pre-baked
// workspace (ZEPHYR_BASE), not PlatformIO. We scaffold a minimal Zephyr
// application around the agent's single source file: a CMakeLists.txt that
// pulls in the source, and a prj.conf that turns on the console (printk) and
// GPIO. The agent supplies ONLY src/main.{c,cpp}; the build files are ours.

/** The scaffold files for a Zephyr application wrapping the agent's source.
 *  Pure; tested. Keys are project-relative paths. */
export function generateZephyrFiles(source: string, lang: CompileLanguage): Record<string, string> {
  const srcName = lang === 'cpp' ? 'main.cpp' : 'main.c';
  const cmake = [
    'cmake_minimum_required(VERSION 3.20.0)',
    'find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})',
    'project(labwired_app)',
    `target_sources(app PRIVATE src/${srcName})`,
    '',
  ].join('\n');
  const prj = ['CONFIG_PRINTK=y', 'CONFIG_GPIO=y', ''].join('\n');
  return {
    'CMakeLists.txt': cmake,
    'prj.conf': prj,
    [`src/${srcName}`]: source,
  };
}

const GCC_LINE = /^(.+?):(\d+):(?:(\d+):)?\s*(fatal error|error|warning):\s*(.*)$/;

/** Parse pio/gcc build output into structured diagnostics. Pure; tested. */
export function parseGccDiagnostics(log: string): CompileDiagnostic[] {
  const out: CompileDiagnostic[] = [];
  const seen = new Set<string>();
  for (const raw of log.split('\n')) {
    const m = GCC_LINE.exec(raw.trim());
    if (!m) continue;
    const [, file, line, col, sev, message] = m;
    const key = `${file}:${line}:${col}:${message}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      severity: sev === 'warning' ? 'warning' : 'error',
      file,
      line: Number(line),
      ...(col ? { column: Number(col) } : {}),
      message,
    });
  }
  return out;
}

// Chips whose 2nd-stage bootloader sits at flash offset 0x0 (newer parts);
// classic ESP32/ESP32-S2 keep it at 0x1000.
const BOOTLOADER_AT_ZERO = new Set(['esp32s3', 'esp32c3', 'esp32s2', 'esp32c6', 'esp32h2']);

function pioCoreDir(): string {
  return process.env.PLATFORMIO_CORE_DIR ?? join(process.env.HOME ?? '/root', '.platformio');
}

/** Locate boot_app0.bin shipped with the Arduino-ESP32 framework package. */
async function findBootApp0(): Promise<string | null> {
  const pkgs = join(pioCoreDir(), 'packages');
  let entries: string[];
  try {
    entries = await readdir(pkgs);
  } catch {
    return null;
  }
  const fw = entries.find((e) => e.startsWith('framework-arduinoespressif32'));
  if (!fw) return null;
  return join(pkgs, fw, 'tools', 'partitions', 'boot_app0.bin');
}

/** Assemble ESP32 flashable images from a finished build dir. Mirrors the
 *  esptool-js layout proto.cat's flasher expects. Best-effort: any missing bin is
 *  skipped rather than failing the compile. */
async function collectFlashImages(buildDir: string, chip: string): Promise<FlashImage[]> {
  const images: FlashImage[] = [];
  const bootOffset = BOOTLOADER_AT_ZERO.has(chip) ? 0x0 : 0x1000;
  const add = async (offset: number, path: string | null) => {
    if (!path) return;
    try {
      images.push({ offset, dataBase64: (await readFile(path)).toString('base64') });
    } catch {
      /* binary not produced for this target — skip */
    }
  };
  await add(bootOffset, join(buildDir, 'bootloader.bin'));
  await add(0x8000, join(buildDir, 'partitions.bin'));
  await add(0xe000, await findBootApp0());
  await add(0x10000, join(buildDir, 'firmware.bin'));
  return images;
}

function fail(message: string): CompileResult {
  return { ok: false, diagnostics: [{ severity: 'error', message }] };
}

/** Build a Zephyr target with `west build`. The workspace (ZEPHYR_BASE) and the
 *  gnuarmemb toolchain are pre-baked into the image, so this runs offline like
 *  the PlatformIO path. Reads build/zephyr/zephyr.elf on success. */
async function compileZephyr(
  target: PioBoard,
  source: string,
  lang: CompileLanguage,
  mappingSource: string,
): Promise<CompileResult> {
  const meta = {
    platformioBoard: target.zephyrBoard,
    framework: 'zephyr',
    mappingSource,
    runnable: target.runnable,
  };
  const proj = join('/tmp', `lwz-${randomUUID()}`);
  const build = join(proj, 'build');
  try {
    for (const [rel, content] of Object.entries(generateZephyrFiles(source, lang))) {
      const abs = join(proj, rel);
      await mkdir(join(abs, '..'), { recursive: true });
      await writeFile(abs, content);
    }
    const west = process.env.WEST_BIN ?? 'west';
    // `west build` locates the Zephyr workspace from its cwd, so it must run
    // inside the pre-baked workspace topdir (WEST_WORKSPACE); the app + build
    // dirs are passed as absolute paths and live outside it.
    const cwd = process.env.WEST_WORKSPACE;
    try {
      const { stdout, stderr } = await execFileAsync(
        west,
        ['build', '-p', 'always', '-b', target.zephyrBoard!, '-d', build, proj],
        { timeout: COMPILE_TIMEOUT_MS, env: safeEnv(), maxBuffer: 16 * 1024 * 1024, ...(cwd ? { cwd } : {}) },
      );
      const elf = await readFile(join(build, 'zephyr', 'zephyr.elf'));
      return {
        ok: true,
        elfBase64: elf.toString('base64'),
        diagnostics: parseGccDiagnostics(`${stdout ?? ''}\n${stderr ?? ''}`),
        ...meta,
      };
    } catch (err) {
      const e = err as { stdout?: string; stderr?: string; message?: string };
      const log = `${e.stdout ?? ''}\n${e.stderr ?? ''}\n${e.message ?? ''}`.trim().slice(-MAX_LOG);
      const diagnostics = parseGccDiagnostics(log);
      return {
        ok: false,
        diagnostics: diagnostics.length ? diagnostics : [{ severity: 'error', message: log.slice(-2000) || 'zephyr build failed' }],
        log,
        ...meta,
      };
    }
  } finally {
    await rm(proj, { recursive: true, force: true }).catch(() => {});
  }
}

export async function compile(req: CompileRequest): Promise<CompileResult> {
  const boardId = req.board ?? req.labwired_board_id;
  const resolved = resolveBoard(boardId, req.chip_family);
  if (!resolved) {
    const boards = Object.keys(PIO_BOARDS).join(', ');
    const families = Object.keys(CHIP_FAMILIES).join(', ');
    return fail(
      `No compile target for board=${boardId ?? 'none'} chip_family=${req.chip_family ?? 'none'}. ` +
        `Supported boards: ${boards}. Supported chip families: ${families}.`,
    );
  }
  const { board: target, source: mappingSource } = resolved;

  if (typeof req.source !== 'string' || !req.source.trim()) {
    return fail('source is required and must be a non-empty string.');
  }
  if (Buffer.byteLength(req.source, 'utf8') > MAX_SOURCE_BYTES) {
    return fail(`source exceeds the ${MAX_SOURCE_BYTES}-byte limit.`);
  }

  const lang: CompileLanguage = req.language === 'cpp' ? 'cpp' : 'c';

  if (target.builder === 'zephyr') {
    return compileZephyr(target, req.source, lang, mappingSource);
  }

  const proj = join('/tmp', `lwc-${randomUUID()}`);
  try {
    await mkdir(join(proj, 'src'), { recursive: true });
    const ini = iniFor(target, normalizeLibDeps(req.lib_deps));
    await writeFile(join(proj, 'platformio.ini'), ini);
    await writeFile(join(proj, 'src', lang === 'cpp' ? 'main.cpp' : 'main.c'), req.source);

    const pio = process.env.PIO_BIN ?? 'pio';
    const meta = {
      platformioBoard: target.board,
      framework: target.framework,
      mappingSource,
      runnable: target.runnable,
    };
    try {
      const { stdout, stderr } = await execFileAsync(
        pio,
        ['run', '-d', proj, '-e', PIO_ENV],
        { timeout: COMPILE_TIMEOUT_MS, env: safeEnv(), maxBuffer: 16 * 1024 * 1024 },
      );
      const buildDir = join(proj, '.pio', 'build', PIO_ENV);
      const elf = await readFile(join(buildDir, 'firmware.elf'));

      const isEsp = target.framework === 'arduino' && !!target.platform?.startsWith('espressif');
      const flashChip = isEsp ? (target.espChip ?? req.chip_family ?? 'esp32') : undefined;
      const flashImages = isEsp ? await collectFlashImages(buildDir, flashChip!) : undefined;

      return {
        ok: true,
        elfBase64: elf.toString('base64'),
        diagnostics: parseGccDiagnostics(`${stdout ?? ''}\n${stderr ?? ''}`),
        ...meta,
        ...(flashImages && flashImages.length ? { flashImages, flashChip } : {}),
      };
    } catch (err) {
      const e = err as { stdout?: string; stderr?: string; message?: string };
      const log = `${e.stdout ?? ''}\n${e.stderr ?? ''}\n${e.message ?? ''}`.trim().slice(-MAX_LOG);
      const diagnostics = parseGccDiagnostics(log);
      return {
        ok: false,
        diagnostics: diagnostics.length ? diagnostics : [{ severity: 'error', message: log.slice(-2000) || 'compilation failed' }],
        log,
        ...meta,
      };
    }
  } finally {
    await rm(proj, { recursive: true, force: true }).catch(() => {});
  }
}
