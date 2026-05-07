import { describe, expect, it } from 'vitest';
import { BOARD_CONFIGS } from './bundled-configs';

describe('BOARD_CONFIGS', () => {
  it('loads bundled manifests directly from the engine-owned YAML files', () => {
    const stm32f103 = BOARD_CONFIGS.find((config) => config.boardId === 'stm32f103-blinky');
    const nucleoF401 = BOARD_CONFIGS.find((config) => config.boardId === 'nucleo-f401re');

    expect(stm32f103).toBeDefined();
    expect(stm32f103?.chipYaml).toContain('name: "stm32f103c8"');
    expect(stm32f103?.systemYaml).toContain('peripheral: "gpioa"');
    expect(stm32f103?.systemYaml).toContain('kind: "led"');

    expect(nucleoF401).toBeDefined();
    expect(nucleoF401?.chipYaml).toContain('name: "stm32f401re"');
    expect(nucleoF401?.systemYaml).toContain('button_user_pc13');
  });

  it('bundles the ADXL345 sensor lab manifest and demo firmware path', () => {
    const adxl345 = BOARD_CONFIGS.find((config) => config.boardId === 'adxl345-sensor-lab');

    expect(adxl345).toBeDefined();
    expect(adxl345?.systemYaml).toContain('type: "adxl345"');
    expect(adxl345?.systemYaml).toContain('kind: "i2c_device"');
    expect(adxl345?.demoFirmwarePath).toContain('demo-adxl345-sensor-lab.elf');
  });
});
