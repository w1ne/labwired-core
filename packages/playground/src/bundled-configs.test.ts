import { describe, expect, it } from 'vitest';
import { BOARD_CONFIGS, pickerBoards } from './bundled-configs';
import { STARTER_LABS } from './studio/ChipRow';

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
