/**
 * Regression: a wire to a pin that doesn't exist on the part must be a hard
 * validation ERROR (PIN_NOT_ON_COMPONENT), not a silent pass.
 *
 * Repro is the real diagram an agent shipped via labwired_open_hardware_lab
 * (share HdH8Qv7EdcYR): a "virtual pet" where every functional peripheral was
 * wired to a hallucinated pin — rgb-led.DIN (it's R/G/B/GND), buzzer.PWM (+/-),
 * potentiometer.OUT (1/W/2), button.OUT (1/2). The MCU pins were all real, so
 * the old validator reported the board clean while the renderer drew nothing —
 * "not connected but passed validation".
 */
import { describe, expect, it } from 'vitest';
import { composeDiagnostics } from '../src';
import type { ValidateDiagram } from '../src';

const PET_DIAGRAM: ValidateDiagram = {
  board: 'stm32l476',
  parts: [
    { id: 'mcu', type: 'mcu', attrs: {} },
    { id: 'oled', type: 'oled-ssd1306', attrs: {} },
    { id: 'feed_btn', type: 'button', attrs: {} },
    { id: 'mood_led', type: 'rgb-led', attrs: {} },
    { id: 'buzzer', type: 'buzzer', attrs: {} },
    { id: 'hunger_knob', type: 'potentiometer', attrs: {} },
  ],
  wires: [
    { from: { part: 'mcu', pin: 'PB8' }, to: { part: 'oled', pin: 'SCL' } },
    { from: { part: 'mcu', pin: 'PB9' }, to: { part: 'oled', pin: 'SDA' } },
    { from: { part: 'mcu', pin: 'PC13' }, to: { part: 'feed_btn', pin: 'OUT' } },
    { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'mood_led', pin: 'DIN' } },
    { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'buzzer', pin: 'PWM' } },
    { from: { part: 'mcu', pin: 'PA0' }, to: { part: 'hunger_knob', pin: 'OUT' } },
  ],
} as unknown as ValidateDiagram;

describe('PIN_NOT_ON_COMPONENT', () => {
  it('rejects the agent-shipped virtual-pet diagram (hallucinated peripheral pins)', () => {
    const result = composeDiagnostics(PET_DIAGRAM);
    expect(result.ok).toBe(false);

    const offenders = result.diagnostics
      .filter((d) => d.code === 'PIN_NOT_ON_COMPONENT')
      .map((d) => d.location?.part_id)
      .sort();
    // button.OUT, rgb-led.DIN, buzzer.PWM, potentiometer.OUT all caught.
    expect(offenders).toEqual(['buzzer', 'feed_btn', 'hunger_knob', 'mood_led']);
  });

  it('the message lists the real pins so the agent can self-correct', () => {
    const d = composeDiagnostics(PET_DIAGRAM).diagnostics
      .find((x) => x.code === 'PIN_NOT_ON_COMPONENT' && x.location?.part_id === 'mood_led');
    expect(d?.message).toContain('R, G, B, GND');
  });

  it('passes once the same lab is wired to real pins', () => {
    const fixed: ValidateDiagram = {
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'mcu', attrs: {} },
        { id: 'feed_btn', type: 'button', attrs: {} },
        { id: 'mood_led', type: 'rgb-led', attrs: {} },
        { id: 'buzzer', type: 'buzzer', attrs: {} },
        { id: 'hunger_knob', type: 'potentiometer', attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PC13' }, to: { part: 'feed_btn', pin: '1' } },
        { from: { part: 'mcu', pin: 'PA6' }, to: { part: 'mood_led', pin: 'R' } },
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'buzzer', pin: '+' } },
        { from: { part: 'mcu', pin: 'PA0' }, to: { part: 'hunger_knob', pin: 'W' } },
      ],
    } as unknown as ValidateDiagram;
    expect(composeDiagnostics(fixed).diagnostics.filter((d) => d.code === 'PIN_NOT_ON_COMPONENT')).toEqual([]);
  });
});
