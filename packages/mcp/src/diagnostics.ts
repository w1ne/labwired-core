/**
 * Re-exports of the kernel's legacy-diagnostics module.
 * Behavior is identical; the implementation now lives in @labwired/board-config.
 */
export type { DiagnosticSeverity, DiagnosticCode, ValidateDiagram, DiagramPart, DiagramWire } from '@labwired/board-config';
export type { LegacyDiagnostic as Diagnostic } from '@labwired/board-config';
export type { LegacyWireEndpoint as WireEndpoint } from '@labwired/board-config';
export { diagnoseDiagram } from '@labwired/board-config';
