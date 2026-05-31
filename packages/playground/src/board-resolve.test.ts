import { describe, it, expect } from 'vitest';
import { resolveBoardForPart } from './board-resolve';
import type { BoardConfig } from './bundled-configs';

const dk = { boardId: 'nrf52840-dk', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const sensor = { boardId: 'nrf52840-ble-sensor', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const collector = { boardId: 'nrf52840-ble-collector', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const boards = [dk, sensor, collector];

const part = (id: string, type: string, attrs: Record<string, unknown> = {}) =>
  ({ id, type, attrs } as { id: string; type: string; attrs: Record<string, unknown> });

describe('resolveBoardForPart', () => {
  it('prefers attrs.boardId over the mcuComponentType first-match (the nRF collision)', () => {
    expect(resolveBoardForPart(part('mcu', 'nrf52840-dk', { boardId: 'nrf52840-ble-sensor' }), dk, boards)).toBe(sensor);
    expect(resolveBoardForPart(part('mcu-collector', 'nrf52840-dk', { boardId: 'nrf52840-ble-collector' }), dk, boards)).toBe(collector);
  });
  it('falls back to primaryBoard for the legacy id==="mcu" part with no boardId', () => {
    expect(resolveBoardForPart(part('mcu', 'nrf52840-dk'), dk, boards)).toBe(dk);
  });
  it('falls back to mcuComponentType match for a non-mcu part with no boardId', () => {
    expect(resolveBoardForPart(part('mcu-collector', 'nrf52840-dk'), dk, boards)).toBe(dk);
  });
  it('returns null when nothing matches', () => {
    expect(resolveBoardForPart(part('mcu-collector', 'no-such-type'), dk, boards)).toBeNull();
  });
  it('ignores a non-existent attrs.boardId, falling through', () => {
    expect(resolveBoardForPart(part('mcu-collector', 'nrf52840-dk', { boardId: 'ghost' }), dk, boards)).toBe(dk);
  });
});
