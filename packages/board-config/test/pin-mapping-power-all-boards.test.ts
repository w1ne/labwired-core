/**
 * Cross-board power pin etype tests (Fix 1).
 *
 * Every board in PIN_MAPS must resolve its canonical power/ground rail pins
 * to power_out — regardless of whether an explicit PIN_ELECTRICAL_OVERRIDES
 * entry exists for that board.  Prior to Fix 1, STM32/nRF/RP2040 boards lacked
 * per-board overrides and defaulted to bidirectional, causing false
 * PWR_RAIL_UNDRIVEN errors on bundled diagrams.
 */

import { describe, expect, it } from 'vitest';
import { getPinEtype, PIN_MAPS } from '../src/pin-mapping';

/**
 * For each board, the "primary power_out" pin that MUST resolve to power_out.
 * Boards not listed here are not expected to have a power rail in their map —
 * they will be verified via the name-based rule only (i.e., they would resolve
 * IF the pin existed).
 */
const BOARD_POWER_PINS: Record<string, { positive: string; ground: string }> = {
  // STM32 family — VCC is 3V3 supply, GND is ground.
  stm32f103:    { positive: 'VCC', ground: 'GND' },
  stm32f401:    { positive: 'VCC', ground: 'GND' },
  stm32f401cdu6: { positive: 'VCC', ground: 'GND' },
  stm32l476:    { positive: 'VCC', ground: 'GND' },
  stm32h563:    { positive: 'VCC', ground: 'GND' },
  // RP2040 (Pico) — 3V3 output pin + GND.
  rp2040:       { positive: '3V3', ground: 'GND' },
  // nRF52840 DK — VDD (3V3) + GND.
  nrf52840:             { positive: 'VDD', ground: 'GND' },
  'nrf52840-onboarding': { positive: 'VDD', ground: 'GND' },
  // ESP32 family — 3V3 + GND (already had overrides, regression guard).
  esp32:        { positive: '3V3', ground: 'GND' },
  esp32c3:      { positive: '3V3', ground: 'GND' },
  esp32s3:      { positive: '3V3', ground: 'GND' },
  'esp32-s3-zero': { positive: '3V3', ground: 'GND' },
};

describe('cross-board power pin etypes', () => {
  it('every board that declares a positive rail pin resolves it to power_out', () => {
    for (const [board, pins] of Object.entries(BOARD_POWER_PINS)) {
      const result = getPinEtype(board, pins.positive);
      expect(result, `${board}:${pins.positive} should not be null`).not.toBeNull();
      expect(result?.etype, `${board}:${pins.positive} should be power_out`).toBe('power_out');
      expect(result?.internalPullup, `${board}:${pins.positive} should have internalPullup:false`).toBe(false);
    }
  });

  it('every board that declares a GND pin resolves it to power_out', () => {
    for (const [board, pins] of Object.entries(BOARD_POWER_PINS)) {
      const result = getPinEtype(board, pins.ground);
      expect(result, `${board}:${pins.ground} should not be null`).not.toBeNull();
      expect(result?.etype, `${board}:${pins.ground} should be power_out`).toBe('power_out');
      expect(result?.internalPullup, `${board}:${pins.ground} should have internalPullup:false`).toBe(false);
    }
  });

  it('power pin etype only applies when the pin exists in the board map', () => {
    // A power-looking pin name that is NOT in a board's map should return null,
    // not silently succeed.
    // RP2040 does not have a GND.99 pin.
    expect(getPinEtype('rp2040', 'GND.99')).toBeNull();
    // esp32c3 does not have AGND.
    expect(getPinEtype('esp32c3', 'AGND')).toBeNull();
  });

  it('GPIO pins are NOT affected — they stay bidirectional with pullup', () => {
    expect(getPinEtype('stm32f103', 'PA5')?.etype).toBe('bidirectional');
    expect(getPinEtype('nrf52840', 'P0.00')?.etype).toBe('bidirectional');
    expect(getPinEtype('rp2040', 'GP0')?.etype).toBe('bidirectional');
    expect(getPinEtype('esp32c3', 'GPIO8')?.etype).toBe('bidirectional');
  });

  it('every board in PIN_MAPS resolves every mapped pin to an etype (comprehensive sweep)', () => {
    for (const board of Object.keys(PIN_MAPS)) {
      for (const pin of Object.keys(PIN_MAPS[board])) {
        expect(getPinEtype(board, pin), `${board}:${pin}`).not.toBeNull();
      }
    }
  });
});
