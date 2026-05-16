import { describe, expect, it } from 'vitest';
import { BOARD_CONFIGS } from './bundled-configs';

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
});
