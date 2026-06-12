/**
 * composeDiagnostics() — unified diagram validation.
 * Merges legacy-14 diagnostic codes + kernel ERC codes.
 * Both MCP surfaces call this; parity by construction.
 */
import type { ValidateDiagram } from './legacy-diagnostics';
import { diagnoseDiagram as legacyDiagnose } from './legacy-diagnostics';
import { erc } from './erc';
import type { DiagramV2 } from './schema';

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
 * Shape-guard for composeDiagnostics: returns a SCHEMA_MALFORMED error result
 * for null/non-object/missing-parts input, protecting downstream code from throws.
 */
function malformedResult(detail: string): ComposeDiagnosticsResult {
  return {
    ok: false,
    error_count: 1,
    warning_count: 0,
    diagnostics: [{
      severity: 'error',
      code: 'SCHEMA_MALFORMED',
      message: `Diagram input is malformed: ${detail}`,
      hint: 'Provide an object with at least { board: string, parts: [], wires: [] }',
    }],
  };
}

/**
 * Run legacy-14 diagnostics + kernel ERC, merge results, deduplicate.
 * Legacy codes are preserved as-is. Kernel codes are appended.
 * When the same logical condition is reported by both, keep BOTH codes
 * (their semantics differ: legacy checks structural wiring, kernel checks electrical rules).
 *
 * Input shape-guard: null/non-object/missing-board input returns SCHEMA_MALFORMED
 * instead of throwing, so callers never need try/catch for structural inputs.
 * Missing wires/nets/connections default to [].
 */
export function composeDiagnostics(diagram: ValidateDiagram): ComposeDiagnosticsResult {
  // Shape guard — protect against null, non-object, or missing required fields.
  if (diagram === null || diagram === undefined || typeof diagram !== 'object') {
    return malformedResult('input is null or not an object');
  }
  if (!Array.isArray((diagram as { parts?: unknown }).parts)) {
    return malformedResult('parts must be an array');
  }
  // Tolerate missing wires/nets/connections — default to empty arrays.
  const safeDiagram: ValidateDiagram = {
    ...(diagram as object),
    wires: Array.isArray((diagram as { wires?: unknown }).wires)
      ? (diagram as ValidateDiagram).wires
      : [],
  } as ValidateDiagram;

  const legacyDiags = legacyDiagnose(safeDiagram);

  // erc() accepts Diagram | DiagramV2; ValidateDiagram is structurally compatible
  const ercDiags = erc(safeDiagram as Parameters<typeof erc>[0]);

  // Build the set of part IDs referenced in connections[], if any.
  // Used below to suppress wire-heuristic false warnings on nets-canonical diagrams.
  const v2connections = ((diagram as unknown as DiagramV2).connections) ?? [];
  const connectedPartIds = new Set<string>();
  for (const [ref] of v2connections) {
    const colonIdx = typeof ref === 'string' ? ref.indexOf(':') : -1;
    if (colonIdx > 0) connectedPartIds.add(ref.slice(0, colonIdx));
  }

  // Filter legacy diagnostics that are wire-heuristic false warnings:
  //   - COMPONENT_DANGLING fires when a board_io part has zero MCU wire connections.
  //     In a nets-canonical v2 diagram the part IS connected (via connections[]),
  //     so suppress COMPONENT_DANGLING for any part that appears in connections[].
  //     A genuinely dangling part (no wires AND no connections entries) is still flagged.
  //
  //   - BOARDIO_MULTIPLE_WIRES: counts wires it actually found, still valid on
  //     wire-based diagrams even when connections[] also exist. Kept as-is.
  const filteredLegacyDiags = legacyDiags.filter((d) => {
    if (d.code === 'COMPONENT_DANGLING' && d.location?.part_id) {
      // Suppress when the dangling part IS referenced in connections[]
      if (connectedPartIds.has(d.location.part_id)) return false;
    }
    return true;
  });

  const composed: ComposedDiagnostic[] = [
    ...filteredLegacyDiags.map((d) => ({
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
