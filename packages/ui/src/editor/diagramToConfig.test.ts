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

  it('maps wired ultrasonic parts into HC-SR04 external devices', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'range1', type: 'ultrasonic', x: 200, y: 100, rotate: 0, attrs: { distance: '42' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'range1', pin: 'TRIG' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PB10' }, to: { part: 'range1', pin: 'ECHO' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'range1', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'range1', pin: 'GND' }, color: '#888888' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('external_devices:');
    expect(systemYaml).toContain('id: "range1"');
    expect(systemYaml).toContain('type: "hc-sr04"');
    expect(systemYaml).toContain('trig_pin: "PA8"');
    expect(systemYaml).toContain('echo_pin: "PB10"');
    expect(systemYaml).toContain('distance_cm: 42');
    expect(systemYaml).not.toContain('kind: "button"');
  });
});
