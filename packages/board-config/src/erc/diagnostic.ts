/** Severity of an ERC finding. Errors block compile; warnings do not. */
export type Severity = 'error' | 'warning';

/** A machine-readable ERC finding (same shape philosophy as ICOMP_*). */
export interface Diagnostic {
  code: string;
  severity: Severity;
  message: string;
  hint: string;
  /** "part:pin" or net names this finding is about, when identifiable. */
  subjects?: string[];
}

export const diag = (
  code: string, severity: Severity, message: string, hint: string, subjects?: string[],
): Diagnostic => (subjects ? { code, severity, message, hint, subjects } : { code, severity, message, hint });
