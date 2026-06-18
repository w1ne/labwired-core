// shareMeta — pure, runtime-agnostic logic for the per-share og:image rewrite.
//
// Split out from the HTMLRewriter wiring in `_worker.ts` so the
// SECURITY-CRITICAL parts (id validation + URL building) are unit-testable
// without the Cloudflare Workers runtime (HTMLRewriter is unavailable in
// vitest/jsdom). The worker imports `shareImageUrlFor`; the tests import it
// directly.

// The share id is base64url of 9 random bytes (see the API's shares.ts), i.e.
// 12 base64url chars. We bound the charset to base64url and length to a safe
// ceiling. This is the gate that prevents reflected XSS / meta-injection via
// the attacker-controlled `?share=` query param: anything not matching is
// treated as "no per-share image" and the origin HTML passes through unchanged.
const SHARE_ID_RE = /^[A-Za-z0-9_-]{1,24}$/;

/** Origin that serves the stored PNG (or 302→logo on miss). */
export const SHARE_IMAGE_ORIGIN = 'https://api.labwired.com';

/** True iff `id` is a well-formed share id safe to embed in a URL. */
export function isValidShareId(id: string | null | undefined): id is string {
  return typeof id === 'string' && SHARE_ID_RE.test(id);
}

/**
 * Build the per-share image URL for a `?share=<id>` request, or `null` when the
 * id is missing/malformed (→ caller must NOT rewrite; pass the origin through).
 *
 * The id is `encodeURIComponent`-escaped even though it has already passed the
 * strict charset check — defense in depth. NO network/KV/API lookup happens
 * here; the URL is derived purely from the id, and the API endpoint handles the
 * hit/miss (302→logo) fallback.
 */
export function shareImageUrlFor(id: string | null | undefined): string | null {
  if (!isValidShareId(id)) return null;
  return `${SHARE_IMAGE_ORIGIN}/v1/shares/${encodeURIComponent(id)}/image`;
}
