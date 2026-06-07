import { describe, it, expect } from 'vitest';
import { diagramToConfig } from '../src/diagram-to-config';
const diagram = { board: 'stm32l476', parts: [{ id: 'led1', type: 'led' }], wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } }] };
describe('diagramToConfig', () => {
  it('emits a system.yaml with the LED as a board_io led on gpioa pin 5', () => {
    const { systemYaml, chipYaml } = diagramToConfig(diagram);
    expect(systemYaml).toContain('id: "led1"'); expect(systemYaml).toContain('gpioa'); expect(systemYaml).toContain('pin: 5'); expect(chipYaml).toContain('0x08000000');
  });
  it('throws on an unknown board', () => { expect(() => diagramToConfig({ board: 'nope', parts: [], wires: [] })).toThrow(/Unknown board/); });
  it('routes adc_input (potentiometer on PA0) to adc1, not gpioa', () => {
    const adcDiagram = {
      board: 'stm32l476',
      parts: [{ id: 'pot1', type: 'potentiometer' }],
      wires: [{ from: { part: 'mcu', pin: 'PA0' }, to: { part: 'pot1', pin: 'out' } }],
    };
    const { systemYaml } = diagramToConfig(adcDiagram);
    expect(systemYaml).toContain('id: "pot1"');
    expect(systemYaml).toContain('kind: "adc_input"');
    expect(systemYaml).toContain('peripheral: "adc1"');
    expect(systemYaml).not.toContain('peripheral: "gpioa"');
  });
});
