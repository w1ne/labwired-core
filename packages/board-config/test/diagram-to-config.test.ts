import { describe, it, expect } from 'vitest';
import { diagramToConfig } from '../src/diagram-to-config';
const diagram = { board: 'stm32l476', parts: [{ id: 'led1', type: 'led' }], wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } }] };
describe('diagramToConfig', () => {
  it('emits a system.yaml with the LED as a board_io led on gpioa pin 5', () => {
    const { systemYaml, chipYaml } = diagramToConfig(diagram);
    expect(systemYaml).toContain('id: "led1"'); expect(systemYaml).toContain('gpioa'); expect(systemYaml).toContain('pin: 5'); expect(chipYaml).toContain('0x08000000');
  });
  it('throws on an unknown board', () => { expect(() => diagramToConfig({ board: 'nope', parts: [], wires: [] })).toThrow(/Unknown board/); });

  it('maps wired H563 CAN blocks into a reusable diagnostic tester external device', () => {
    const { systemYaml } = diagramToConfig({
      board: 'stm32h563',
      parts: [
        { id: 'mcu', type: 'nucleo-h563zi', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'can_xcvr', type: 'can-transceiver', x: 300, y: 100, rotate: 0, attrs: {} },
        {
          id: 'uds_tester',
          type: 'can-diagnostic-tool',
          x: 500,
          y: 100,
          rotate: 0,
          attrs: { request_id: '0x7E0', request_data: '03 22 F1 90' },
        },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PD1' }, to: { part: 'can_xcvr', pin: 'TXD' } },
        { from: { part: 'mcu', pin: 'PD0' }, to: { part: 'can_xcvr', pin: 'RXD' } },
        { from: { part: 'can_xcvr', pin: 'CAN_H' }, to: { part: 'uds_tester', pin: 'CAN_H' } },
        { from: { part: 'can_xcvr', pin: 'CAN_L' }, to: { part: 'uds_tester', pin: 'CAN_L' } },
      ],
    }, 'name: "stm32h563-test"\n');

    expect(systemYaml).toContain('id: "uds_tester"');
    expect(systemYaml).toContain('type: "can-diagnostic-tester"');
    expect(systemYaml).toContain('connection: "fdcan1"');
    expect(systemYaml).toContain('request_id: "0x7E0"');
    expect(systemYaml).toContain('request_data: "03 22 F1 90"');
  });

  it('does not emit a CAN diagnostic tester when the transceiver is not fully bound to FDCAN', () => {
    const { systemYaml } = diagramToConfig({
      board: 'stm32h563',
      parts: [
        { id: 'mcu', type: 'nucleo-h563zi', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'can_xcvr', type: 'can-transceiver', x: 300, y: 100, rotate: 0, attrs: {} },
        { id: 'uds_tester', type: 'can-diagnostic-tool', x: 500, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PD1' }, to: { part: 'can_xcvr', pin: 'TXD' } },
        { from: { part: 'can_xcvr', pin: 'CAN_H' }, to: { part: 'uds_tester', pin: 'CAN_H' } },
        { from: { part: 'can_xcvr', pin: 'CAN_L' }, to: { part: 'uds_tester', pin: 'CAN_L' } },
      ],
    }, 'name: "stm32h563-test"\n');

    expect(systemYaml).not.toContain('type: "can-diagnostic-tester"');
  });
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
