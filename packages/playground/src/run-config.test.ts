import { describe, expect, it } from 'vitest';
import type { Diagram } from '@labwired/ui';
import { resolveRunSystemConfig } from './run-config';

describe('resolveRunSystemConfig', () => {
  it('uses the current diagram system YAML for bundled demo firmware runs', () => {
    const diagram: Diagram = {
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'dist', type: 'ultrasonic', x: 100, y: 100, rotate: 0, attrs: { distance: '80' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'dist', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'dist', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'dist', pin: 'TRIG' } },
        { from: { part: 'mcu', pin: 'PB10' }, to: { part: 'dist', pin: 'ECHO' } },
      ],
    };

    const config = resolveRunSystemConfig({
      diagram,
      chipYaml: 'chip: stm32l476',
      bundledSystemYaml: 'external_devices:\n  - id: "dist"\n    distance_cm: 30',
    preferDiagram: true,
    onFallback: () => {
      throw new Error('diagram config should be valid');
    },
  });

    expect(config.systemYaml).toContain('distance_cm: 80');
    expect(config.systemYaml).not.toContain('distance_cm: 30');
  });
});
