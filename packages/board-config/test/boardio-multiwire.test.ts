/**
 * Regression: BOARDIO_MULTIPLE_WIRES must only fire for genuinely single-signal
 * board_io parts (LED, button). A multi-signal sensor like the HC-SR04
 * ultrasonic (TRIG + ECHO) legitimately needs several MCU connections — it must
 * NOT be flagged, otherwise the hard validation gate (composeDiagnostics on
 * compile/run/share) would reject every valid ultrasonic board.
 *
 * Repro is the real share kKGj-PMrrjT4 (nrf52840 + HC-SR04 + alarm LED) which
 * was correctly wired yet reported invalid by the over-broad rule.
 */
import { describe, expect, it } from 'vitest';
import { composeDiagnostics } from '../src';
import type { ValidateDiagram } from '../src';

describe('BOARDIO_MULTIPLE_WIRES', () => {
  it('does not flag a correctly-wired ultrasonic (TRIG + ECHO + power)', () => {
    const diagram: ValidateDiagram = {
      board: 'nrf52840',
      parts: [
        { id: 'mcu', type: 'nrf52840-dk', attrs: {} },
        { id: 'ultrasonic', type: 'ultrasonic', attrs: {} },
        { id: 'alarm_led', type: 'led', attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VDD' }, to: { part: 'ultrasonic', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'ultrasonic', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'P0.04' }, to: { part: 'ultrasonic', pin: 'TRIG' } },
        { from: { part: 'mcu', pin: 'P0.05' }, to: { part: 'ultrasonic', pin: 'ECHO' } },
        { from: { part: 'mcu', pin: 'P0.06' }, to: { part: 'alarm_led', pin: 'A' } },
      ],
    } as unknown as ValidateDiagram;
    expect(composeDiagnostics(diagram).diagnostics.filter((d) => d.code === 'BOARDIO_MULTIPLE_WIRES')).toEqual([]);
  });

  it('still flags a single-signal LED wired to two MCU GPIOs', () => {
    const diagram: ValidateDiagram = {
      board: 'nrf52840',
      parts: [
        { id: 'mcu', type: 'nrf52840-dk', attrs: {} },
        { id: 'led1', type: 'led', attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'P0.04' }, to: { part: 'led1', pin: 'A' } },
        { from: { part: 'mcu', pin: 'P0.05' }, to: { part: 'led1', pin: 'C' } },
      ],
    } as unknown as ValidateDiagram;
    const codes = composeDiagnostics(diagram).diagnostics.map((d) => d.code);
    expect(codes).toContain('BOARDIO_MULTIPLE_WIRES');
  });
});
