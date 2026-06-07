import { describe, it, expect } from 'vitest';
import { CHIP_YAMLS } from '../src/chip-yamls';
describe('CHIP_YAMLS', () => {
  it('has an stm32l476 entry with the correct flash/ram base', () => {
    const y = CHIP_YAMLS['stm32l476'];
    expect(y).toContain('0x08000000'); expect(y).toContain('0x20000000'); expect(y).toContain('arch: "arm"');
  });
});
