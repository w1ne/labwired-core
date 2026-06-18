// Unit test for the edge function's SECURITY-CRITICAL id-validation + URL
// building (the pure part of the per-share og:image rewrite). The HTMLRewriter
// wiring itself needs the Workers runtime (`wrangler pages dev`) and is covered
// end-to-end there, not here.
import { describe, it, expect } from 'vitest';
import { isValidShareId, shareImageUrlFor } from '../functions/shareMeta';

describe('isValidShareId', () => {
  it('accepts well-formed base64url share ids', () => {
    expect(isValidShareId('aB3_-xyz0123')).toBe(true);
    expect(isValidShareId('a')).toBe(true);
    expect(isValidShareId('A'.repeat(24))).toBe(true);
  });

  it('rejects injection / malformed / empty ids', () => {
    expect(isValidShareId('"><script>')).toBe(false);
    expect(isValidShareId('a/b')).toBe(false);
    expect(isValidShareId('a b')).toBe(false);
    expect(isValidShareId('a.b')).toBe(false);
    expect(isValidShareId('a%2F')).toBe(false);
    expect(isValidShareId('')).toBe(false);
    expect(isValidShareId('A'.repeat(25))).toBe(false); // overlong
    expect(isValidShareId(null)).toBe(false);
    expect(isValidShareId(undefined)).toBe(false);
  });
});

describe('shareImageUrlFor', () => {
  it('builds the image endpoint URL for a valid id', () => {
    expect(shareImageUrlFor('aB3_-xyz0123')).toBe(
      'https://api.labwired.com/v1/shares/aB3_-xyz0123/image',
    );
  });

  it('returns null (no rewrite) for malformed ids', () => {
    expect(shareImageUrlFor('"><script>alert(1)</script>')).toBeNull();
    expect(shareImageUrlFor('A'.repeat(25))).toBeNull();
    expect(shareImageUrlFor('')).toBeNull();
    expect(shareImageUrlFor(null)).toBeNull();
  });

  it('never emits an unescaped reflected value (no <, >, ", space)', () => {
    // Any id that could carry markup is rejected outright (null), so the URL
    // can never contain attacker markup.
    for (const bad of ['"', '<', '>', ' ', '/', '\\', '#', '%']) {
      expect(shareImageUrlFor(`a${bad}b`)).toBeNull();
    }
  });
});
