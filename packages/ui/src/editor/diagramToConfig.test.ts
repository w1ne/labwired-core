import { describe, expect, it } from 'vitest';
import { diagramToConfig } from './diagramToConfig';
import type { Diagram } from './types';

describe('diagramToConfig', () => {
  it('maps wired LED parts into board_io bindings', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32f103',
      parts: [
        { id: 'mcu', type: 'stm32-dev', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'led_custom', type: 'led', x: 200, y: 100, rotate: 0, attrs: { color: 'green' } },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led_custom', pin: 'A' },
          color: '#27c93f',
        },
      ],
    };

    const { systemYaml, chipYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "led_custom"');
    expect(systemYaml).toContain('peripheral: "gpioa"');
    expect(systemYaml).toContain('pin: 5');
    expect(systemYaml).toContain('kind: "led"');
    expect(chipYaml).toContain('name: "stm32f103c8"');
  });
});
