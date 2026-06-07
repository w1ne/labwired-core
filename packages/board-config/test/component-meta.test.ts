import { describe, it, expect } from 'vitest';
import { COMPONENT_META } from '../src/component-meta';
describe('COMPONENT_META', () => {
  it('marks an LED as a board_io led', () => { expect(COMPONENT_META['led']?.boardIoKind).toBe('led'); });
  it('marks the PCD8544 as an spi_device', () => { expect(COMPONENT_META['pcd8544']?.boardIoKind).toBe('spi_device'); });
  it('has no React/render fields', () => { expect((COMPONENT_META['led'] as Record<string, unknown>).render).toBeUndefined(); });
});
