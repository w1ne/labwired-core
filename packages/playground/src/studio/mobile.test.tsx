import { describe, expect, it } from 'vitest';
import { isMobileViewport, MOBILE_BREAKPOINT } from './useStudioLayout';

describe('mobile viewport detection', () => {
  it('returns true under the breakpoint', () => {
    expect(isMobileViewport(MOBILE_BREAKPOINT - 1)).toBe(true);
  });
  it('returns false at the breakpoint', () => {
    expect(isMobileViewport(MOBILE_BREAKPOINT)).toBe(false);
  });
  it('returns false above the breakpoint', () => {
    expect(isMobileViewport(MOBILE_BREAKPOINT + 200)).toBe(false);
  });
});
