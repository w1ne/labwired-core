/**
 * Static board catalog for the high-level `labwired_list_boards` /
 * `labwired_run_lab` tools.
 *
 * Boards are "chip + opinionated wiring + demo firmware path". They mirror the
 * playground's bundled-configs.ts. When that file evolves we sync this one.
 *
 * Paths are resolved relative to a repo-root anchor discovered at runtime by
 * `resolveRepoRoot()` — walks up from the MCP server's __dirname looking for
 * `core/configs/chips/`. Falls back to env LABWIRED_REPO_ROOT if walking fails.
 */
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { existsSync } from 'node:fs';

export interface BoardCatalogEntry {
  id: string;
  name: string;
  chip_family: string;
  arch: string;
  description: string;
  /** Path relative to repo root for the chip descriptor YAML. */
  chip_yaml: string;
  /** Path relative to repo root for the bundled system manifest YAML. */
  system_yaml: string;
  /** Optional pre-built ELF for "demo run" with no firmware upload. */
  demo_firmware?: string;
}

/**
 * Hand-maintained until we pull a JSON catalog out of the playground. Keep in
 * sync with packages/playground/src/bundled-configs.ts.
 */
export const BOARDS: BoardCatalogEntry[] = [
  {
    id: 'stm32f103-blinky',
    name: 'STM32F103 Blinky',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: 'Classic LED blink on Cortex-M3. Toggles PA5 via GPIO.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/demo-blinky/system.yaml',
    demo_firmware: 'core/target/thumbv7m-none-eabi/release/demo-blinky',
  },
  {
    id: 'ntc-thermistor-lab',
    name: 'NTC Thermistor',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: 'Analog temperature sensor via Steinhart-Hart math.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/ntc-thermistor-lab/system.yaml',
  },
  {
    id: 'ssd1306-hello-lab',
    name: 'SSD1306 OLED',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: '128×64 monochrome OLED — full SSD1306 mode machine and framebuffer.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/ssd1306-hello-lab/system.yaml',
  },
  {
    id: 'bme280-weather-lab',
    name: 'BME280 Weather',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: 'Temperature, humidity, pressure over I²C.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/bme280-weather-lab/system.yaml',
  },
  {
    id: 'ili9341-tft-lab',
    name: 'ILI9341 TFT Color',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: '320×240 SPI TFT — model state machine, 16-bit framebuffer.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/ili9341-tft-lab/system.yaml',
  },
  {
    id: 'epaper-tricolor-lab',
    name: 'E-Paper 2.9" Tri-color',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: 'SSD1680 tri-color e-paper — two-plane composition, byte-identical to silicon.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/epaper-tricolor-lab/system.yaml',
  },
  {
    id: 'mpu6050-sensor-lab',
    name: 'MPU6050 IMU',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: '6-DoF accelerometer + gyroscope over I²C.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/mpu6050-sensor-lab/system.yaml',
  },
  {
    id: 'adxl345-sensor-lab',
    name: 'ADXL345 Sensor Lab',
    chip_family: 'stm32f103',
    arch: 'ARM Cortex-M3',
    description: '3-axis accelerometer over I²C.',
    chip_yaml: 'core/configs/chips/stm32f103.yaml',
    system_yaml: 'core/examples/adxl345-sensor-lab/system.yaml',
  },
];

/** Try to find the repo root by walking up from this file until core/configs is found. */
function findRepoRoot(): string | null {
  const fromEnv = process.env.LABWIRED_REPO_ROOT;
  if (fromEnv && existsSync(join(fromEnv, 'core/configs/chips'))) return resolve(fromEnv);

  let cursor = dirname(fileURLToPath(import.meta.url));
  for (let i = 0; i < 8; i++) {
    if (existsSync(join(cursor, 'core/configs/chips'))) return cursor;
    const parent = dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }
  return null;
}

const REPO_ROOT = findRepoRoot();

export function getBoard(id: string): BoardCatalogEntry | undefined {
  return BOARDS.find((b) => b.id === id);
}

export async function readBoardYamls(
  board: BoardCatalogEntry,
): Promise<{ chipYaml: string; systemYaml: string }> {
  if (!REPO_ROOT) {
    throw new Error(
      'Could not locate LabWired repo root. Set LABWIRED_REPO_ROOT env var to the absolute path of your labwired checkout (e.g. /home/you/Projects/labwired).',
    );
  }
  const chipYaml = await readFile(join(REPO_ROOT, board.chip_yaml), 'utf-8');
  const systemYaml = await readFile(join(REPO_ROOT, board.system_yaml), 'utf-8');
  return { chipYaml, systemYaml };
}

/** Absolute on-disk path to the board's system YAML in the labwired repo. */
export function boardSystemYamlPath(board: BoardCatalogEntry): string {
  if (!REPO_ROOT) {
    throw new Error(
      'Could not locate LabWired repo root. Set LABWIRED_REPO_ROOT env var.',
    );
  }
  return join(REPO_ROOT, board.system_yaml);
}

/** Absolute on-disk path to the board's chip descriptor YAML. */
export function boardChipYamlPath(board: BoardCatalogEntry): string {
  if (!REPO_ROOT) {
    throw new Error(
      'Could not locate LabWired repo root. Set LABWIRED_REPO_ROOT env var.',
    );
  }
  return join(REPO_ROOT, board.chip_yaml);
}

export async function readDemoFirmware(board: BoardCatalogEntry): Promise<Buffer | null> {
  if (!board.demo_firmware || !REPO_ROOT) return null;
  try {
    return await readFile(join(REPO_ROOT, board.demo_firmware));
  } catch {
    return null;
  }
}

export function listBoards(filter?: string): BoardCatalogEntry[] {
  if (!filter) return BOARDS;
  const q = filter.toLowerCase();
  return BOARDS.filter(
    (b) =>
      b.id.toLowerCase().includes(q) ||
      b.name.toLowerCase().includes(q) ||
      b.chip_family.toLowerCase().includes(q),
  );
}
