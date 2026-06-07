import { describe, it, expect } from 'vitest';
import { findPinFunction } from '../src/pin-mapping';
describe('findPinFunction', () => {
  it('resolves an ADC function for an stm32l476 analog pin', () => { expect(findPinFunction('stm32l476', 'PA0', 'adc')).toBeTruthy(); });
  it('returns null for a pin with no such function', () => { expect(findPinFunction('stm32l476', 'PA0', 'i2c')).toBeNull(); });
});
