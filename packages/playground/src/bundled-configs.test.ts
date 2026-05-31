import { execFileSync } from 'node:child_process';
import { readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { getPinMapping, PIN_MAPS, COMPONENT_REGISTRY } from '@labwired/ui';
import { BOARD_CONFIGS, pickerBoards } from './bundled-configs';
import { STARTER_LABS } from './studio/ChipRow';

// ── Firmware-asset gate helpers ─────────────────────────────────────────────
// Every board's demoFirmwarePath is fetched by the browser at Run time. If the
// file isn't actually present in the build, the dev server / Cloudflare Pages
// serves index.html (HTML, ~2KB) instead and SimulatorBridge.fromConfig chokes
// → "Simulator init failed". These helpers let the gate below assert each ELF
// is real, committed, non-empty firmware — deterministically, in CI, before
// deploy. (public/wasm/.gitignore is '*', so ELFs must be force-added; an
// un-committed ELF exists on the author's disk but is absent in CI/the deploy.)
const PLAYGROUND_ROOT = path.resolve(__dirname, '..');
const PUBLIC_DIR = path.join(PLAYGROUND_ROOT, 'public');
const REPO_ROOT = path.resolve(PLAYGROUND_ROOT, '../..');

function publicPathForUrl(url: string): string {
  // vitest has no import.meta.env.BASE_URL, so demoFirmwarePath is a bare
  // "/…/wasm/foo.elf" or "wasm/foo.elf"; take everything from "wasm/" on.
  const idx = url.indexOf('wasm/');
  const rel = idx >= 0 ? url.slice(idx) : url.replace(/^\/+/, '');
  return path.join(PUBLIC_DIR, rel);
}

function trackedWasmBasenames(): Set<string> {
  const out = execFileSync('git', ['ls-files', 'packages/playground/public/wasm'], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
  });
  return new Set(
    out
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean)
      .map((rel) => path.basename(rel)),
  );
}

describe('BOARD_CONFIGS', () => {
  it('loads bundled manifests directly from the engine-owned YAML files', () => {
    const stm32f103 = BOARD_CONFIGS.find((config) => config.boardId === 'stm32f103-blinky');
    const nucleoF401 = BOARD_CONFIGS.find((config) => config.boardId === 'nucleo-f401re');
    const blackPill = BOARD_CONFIGS.find((config) => config.boardId === 'stm32f401cdu6-blackpill');

    expect(stm32f103).toBeDefined();
    expect(stm32f103?.chipYaml).toContain('name: "stm32f103c8"');
    expect(stm32f103?.systemYaml).toContain('peripheral: "gpioa"');
    expect(stm32f103?.systemYaml).toContain('kind: "led"');

    expect(nucleoF401).toBeDefined();
    expect(nucleoF401?.chipYaml).toContain('name: "stm32f401re"');
    expect(nucleoF401?.systemYaml).toContain('button_user_pc13');

    expect(blackPill).toBeDefined();
    expect(blackPill?.chipYaml).toContain('name: "stm32f401cdu6"');
    expect(blackPill?.chipYaml).toContain('size: "384KB"');
    expect(blackPill?.chipYaml).toContain('id: "i2c1"');
    expect(blackPill?.chipYaml).toContain('id: "i2c2"');
    expect(blackPill?.chipYaml).toContain('id: "i2c3"');
    // After the streams merged, the chip yaml uses the canonical `type: "i2c"`
    // (with the F1 layout picked via profile/default) — same convention as
    // every other STM32 chip yaml in core/configs/chips/.
    expect(blackPill?.chipYaml).toContain('type: "i2c"');
    for (const peripheralId of [
      'tim1',
      'tim2',
      'tim3',
      'tim4',
      'tim5',
      'tim9',
      'tim10',
      'tim11',
      'usart1',
      'usart2',
      'usart6',
      'spi1',
      'spi2',
      'spi3',
      'spi4',
      'dma1',
      'dma2',
      'adc1',
      'exti',
      'syscfg',
      'pwr',
      'flash_ctrl',
      'crc',
      'otg_fs_global',
    ]) {
      expect(blackPill?.chipYaml).toContain(`id: "${peripheralId}"`);
    }
    expect(blackPill?.chipYaml).toContain('id: "dma1"\n    type: "stub"');
    expect(blackPill?.chipYaml).toContain('id: "dma2"\n    type: "stub"');
    expect(blackPill?.chipYaml).not.toContain('id: "tim8"');
    expect(blackPill?.systemYaml).toContain('led_pc13');
    expect(blackPill?.systemYaml).toContain('active_high: false');
  });

  it('bundles the ADXL345 sensor lab manifest and demo firmware path', () => {
    const adxl345 = BOARD_CONFIGS.find((config) => config.boardId === 'adxl345-sensor-lab');

    expect(adxl345).toBeDefined();
    expect(adxl345?.systemYaml).toContain('type: "adxl345"');
    expect(adxl345?.systemYaml).toContain('kind: "i2c_device"');
    expect(adxl345?.demoFirmwarePath).toContain('demo-adxl345-sensor-lab.elf');
  });

  it('pickerBoards() contains no kind:"lab" entries — labs belong in Examples, not Boards', () => {
    const labsInPicker = pickerBoards().filter((b) => b.kind === 'lab');
    expect(
      labsInPicker,
      `Boards picker must never include kind:"lab" entries. Offenders: ${labsInPicker.map((b) => b.boardId).join(', ')}`,
    ).toHaveLength(0);
  });

  it('every non-hidden kind:"lab" board is surfaced in STARTER_LABS as an Example', () => {
    const starterIds = new Set(STARTER_LABS.map((l) => l.id));
    const unsurfaced = BOARD_CONFIGS.filter(
      (b) => b.kind === 'lab' && !b.hidden && !starterIds.has(b.boardId),
    );
    expect(
      unsurfaced,
      `Non-hidden labs missing from STARTER_LABS (must be surfaced as Examples): ${unsurfaced.map((b) => b.boardId).join(', ')}`,
    ).toHaveLength(0);
  });

  it('every STARTER_LABS id resolves to a real BOARD_CONFIGS entry — no dangling examples', () => {
    const boardIds = new Set(BOARD_CONFIGS.map((c) => c.boardId));
    const dangling = STARTER_LABS.filter((l) => !boardIds.has(l.id));
    expect(
      dangling,
      `STARTER_LABS entries with no matching BOARD_CONFIGS boardId: ${dangling.map((l) => l.id).join(', ')}`,
    ).toHaveLength(0);
  });

  it('every STARTER_LABS example chip is present in the pin-mapping (no PIN_NOT_ON_CHIP at startup)', () => {
    // A representative GPIO pin that exists on every supported MCU under its
    // canonical pin label. For STM32 families this is PA5 (the Nucleo user LED);
    // for RP2040 it's GP5; for nRF52840 it's P0.05; for ESP32 it's GPIO5.
    const REPRESENTATIVE_PINS: Record<string, string> = {
      stm32f103: 'PA5',
      stm32f401: 'PA5',
      stm32l476: 'PA5',
      stm32h563: 'PA5',
      rp2040: 'GP5',
      nrf52840: 'P0.05',
      'nrf52840-onboarding': 'P0.05',
      esp32: 'GPIO5',
      esp32c3: 'GPIO5',
      esp32s3: 'GPIO5',
    };

    const missing: string[] = [];
    for (const lab of STARTER_LABS) {
      const config = BOARD_CONFIGS.find((c) => c.boardId === lab.id);
      if (!config) continue; // covered by the "dangling examples" test above
      const chipId = config.chipId;
      const probe = REPRESENTATIVE_PINS[chipId] ?? 'PA5';
      const mapping = getPinMapping(chipId, probe);
      if (!mapping) {
        missing.push(`${lab.id} (chipId="${chipId}", probe="${probe}")`);
      }
    }
    expect(
      missing,
      `STARTER_LABS examples whose chipId is absent from the pin-mapping — add it to packages/ui/src/editor/pin-mapping.ts: ${missing.join(', ')}`,
    ).toHaveLength(0);
  });

  // ── Chip-onboarding invariants ────────────────────────────────────────────
  // These tests codify the "definition of done" for adding a new chip.
  // A half-onboarded chip (e.g. in BOARD_CONFIGS but missing from PIN_MAPS)
  // will break the circuit validator with "Pin X is not available on this board
  // model". See docs/guides/onboarding-a-chip.md for the full procedure.

  it('every BOARD_CONFIGS.chipId is a key in PIN_MAPS (circuit-validator soundness)', () => {
    const offenders = BOARD_CONFIGS.filter((b) => !(b.chipId in PIN_MAPS)).map(
      (b) => `boardId="${b.boardId}" chipId="${b.chipId}"`,
    );
    expect(
      offenders,
      `BOARD_CONFIGS entries whose chipId is missing from PIN_MAPS — add the chipId alias to packages/ui/src/editor/pin-mapping.ts: ${offenders.join(', ')}`,
    ).toHaveLength(0);
  });

  it('every BOARD_CONFIGS.mcuComponentType is registered in COMPONENT_REGISTRY', () => {
    const offenders = BOARD_CONFIGS.filter((b) => !COMPONENT_REGISTRY.has(b.mcuComponentType)).map(
      (b) => `boardId="${b.boardId}" mcuComponentType="${b.mcuComponentType}"`,
    );
    expect(
      offenders,
      `BOARD_CONFIGS entries whose mcuComponentType is missing from COMPONENT_REGISTRY — register it in packages/ui/src/editor/components/index.ts: ${offenders.join(', ')}`,
    ).toHaveLength(0);
  });

  it('every BOARD_CONFIGS entry has a non-empty chipYaml (sim always has a chip model)', () => {
    const offenders = BOARD_CONFIGS.filter((b) => !b.chipYaml || b.chipYaml.trim().length === 0).map(
      (b) => b.boardId,
    );
    expect(
      offenders,
      `BOARD_CONFIGS entries with empty chipYaml — every board must point to a chip YAML: ${offenders.join(', ')}`,
    ).toHaveLength(0);
  });

  // ── Deterministic firmware-asset gate ────────────────────────────────────
  // The class of bug this prevents: a board's demoFirmwarePath points at an ELF
  // that is gitignored / 0-byte / missing, so the live site serves HTML in its
  // place and the user hits "Simulator init failed" on Run. CI fails HERE
  // instead — making every deploy reproducible from git alone.
  describe('firmware-asset gate', () => {
    const firmwareBoards = BOARD_CONFIGS.filter((c) => c.demoFirmwarePath);
    const tracked = trackedWasmBasenames();

    it('there is at least one firmware board to gate (sanity)', () => {
      expect(firmwareBoards.length).toBeGreaterThan(0);
    });

    it.each(firmwareBoards.map((c) => [c.boardId, c.demoFirmwarePath!] as const))(
      "board '%s' ships a real, committed firmware ELF",
      (_boardId, url) => {
        const file = publicPathForUrl(url);
        const name = path.basename(file);

        // 1. exists on disk
        let size = -1;
        try {
          size = statSync(file).size;
        } catch {
          /* falls to the assertion below */
        }
        expect(size, `${name} not found at ${file} — firmware missing from public/wasm`).toBeGreaterThanOrEqual(0);

        // 2. git-tracked (present in CI + the Cloudflare deploy, not just the
        //    author's disk). public/wasm/.gitignore is '*', so it must be force-added.
        expect(
          tracked.has(name),
          `${name} is not git-tracked — it will be ABSENT in CI and the deploy (public/wasm/.gitignore is '*'; force-add it: git add -f packages/playground/public/wasm/${name})`,
        ).toBe(true);

        // 3. non-empty (catches 0-byte / truncated)
        expect(size, `${name} is empty (0 bytes)`).toBeGreaterThan(0);

        // 4. real ELF magic 0x7F 'E' 'L' 'F' (catches an HTML/JSON file served as firmware)
        const head = Array.from(readFileSync(file).subarray(0, 4));
        expect(
          head,
          `${name} does not start with ELF magic 7f 45 4c 46 — not a real firmware ELF (got ${head.map((b) => b.toString(16).padStart(2, '0')).join(' ')})`,
        ).toEqual([0x7f, 0x45, 0x4c, 0x46]);
      },
    );
  });

  it('keeps demo-assets.json aligned with BoardConfig.boardId', async () => {
    // Source of truth for build-time firmware fetches lives in
    // packages/playground/demo-assets.json (consumed by scripts/fetch-demo-firmware.sh).
    // Each manifest entry must reference an existing BoardConfig.boardId,
    // and the matching field on BoardConfig must end with the asset's
    // filename so the fetch mirror lands at the URL the browser requests.
    //   * default (firmware ELF) → demoFirmwarePath
    //   * kind: 'snapshot' (LWRS boot snapshot) → bootSnapshotUrl
    const manifest = (await import('../demo-assets.json')).default;
    const boardIds = new Set(BOARD_CONFIGS.map((c) => c.boardId));
    for (const asset of manifest.assets) {
      expect(boardIds.has(asset.boardId), `demo-assets.json asset '${asset.filename}' references unknown boardId '${asset.boardId}'`).toBe(true);
      const cfg = BOARD_CONFIGS.find((c) => c.boardId === asset.boardId);
      const kind = (asset as { kind?: string }).kind ?? 'firmware';
      if (kind === 'snapshot') {
        expect(cfg?.bootSnapshotUrl?.endsWith(`/${asset.filename}`), `BoardConfig '${asset.boardId}'.bootSnapshotUrl must end with '/${asset.filename}'`).toBe(true);
      } else {
        expect(cfg?.demoFirmwarePath?.endsWith(`/${asset.filename}`), `BoardConfig '${asset.boardId}'.demoFirmwarePath must end with '/${asset.filename}'`).toBe(true);
      }
    }
  });
});
