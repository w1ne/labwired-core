/**
 * composeDiagnostics() — unified diagram validation.
 * Merges legacy-14 diagnostic codes + kernel ERC codes.
 * Both MCP surfaces call this; parity by construction.
 */
import type { ValidateDiagram } from './legacy-diagnostics';
import { diagnoseDiagram as legacyDiagnose } from './legacy-diagnostics';
import { erc } from './erc';

export type { ValidateDiagram } from './legacy-diagnostics';

// Unified diagnostic: union of legacy shape and kernel shape.
export interface ComposedDiagnostic {
  severity: 'error' | 'warning';
  code: string;
  message: string;
  /** Legacy field (fix suggestion) */
  fix?: string;
  /** Kernel field (fix suggestion) */
  hint?: string;
  /** Legacy field */
  location?: { part_id?: string; pin?: string };
  /** Kernel field */
  subjects?: string[];
}

export interface ComposeDiagnosticsResult {
  ok: boolean;
  error_count: number;
  warning_count: number;
  diagnostics: ComposedDiagnostic[];
}

/**
 * Run legacy-14 diagnostics + kernel ERC, merge results, deduplicate.
 * Legacy codes are preserved as-is. Kernel codes are appended.
 * When the same logical condition is reported by both, keep BOTH codes
 * (their semantics differ: legacy checks structural wiring, kernel checks electrical rules).
 */
export function composeDiagnostics(diagram: ValidateDiagram): ComposeDiagnosticsResult {
  const legacyDiags = legacyDiagnose(diagram);

  // erc() accepts Diagram | DiagramV2; ValidateDiagram is structurally compatible
  const ercDiags = erc(diagram as Parameters<typeof erc>[0]);

  const composed: ComposedDiagnostic[] = [
    ...legacyDiags.map((d) => ({
      severity: d.severity,
      code: d.code,
      message: d.message,
      ...(d.fix ? { fix: d.fix } : {}),
      ...(d.location ? { location: d.location } : {}),
    })),
    ...ercDiags.map((d) => ({
      severity: d.severity,
      code: d.code,
      message: d.message,
      ...(d.hint ? { hint: d.hint } : {}),
      ...(d.subjects?.length ? { subjects: d.subjects } : {}),
    })),
  ];

  // Deduplicate: same code + same message = keep first occurrence
  const seen = new Set<string>();
  const deduped = composed.filter((d) => {
    const k = `${d.code}|${d.message}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });

  const errors = deduped.filter((d) => d.severity === 'error').length;
  const warnings = deduped.filter((d) => d.severity === 'warning').length;

  return {
    ok: errors === 0,
    error_count: errors,
    warning_count: warnings,
    diagnostics: deduped,
  };
}
