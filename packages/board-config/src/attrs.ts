/**
 * Shared attribute parsing utilities used by both the ERC engine and the
 * compile path.  Keep this file dependency-free (no imports from other
 * local modules) so it can be imported anywhere without creating cycles.
 */

/**
 * Parse an I2C address attribute that may be expressed as a hex literal
 * ("0x40", "0X40") or as a plain decimal integer ("64").
 *
 * Returns `undefined` when the value is absent, empty, or not a finite
 * number.
 */
export function parseAddr(s: string | undefined): number | undefined {
  if (!s) return undefined;
  const trimmed = s.trim();
  const n = trimmed.toLowerCase().startsWith('0x')
    ? parseInt(trimmed, 16)
    : parseInt(trimmed, 10);
  return Number.isFinite(n) ? n : undefined;
}
