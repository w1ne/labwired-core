// Pure presentation helpers for the IO-Link Analyzer. The protocol decode
// (CRC6, M-sequence framing, PD/OD extraction) already happens in the Rust
// master and arrives as IolinkXfer records; these are display formatters only,
// kept pure so they unit-test in isolation and can back a CLI exporter later.
import type { IolinkXfer } from '@labwired/ui';

/** The IO-Link startup→operate phases, in order, for the phase strip. */
export const PHASES = ['WAKE-UP', 'STARTUP', 'PREOPERATE', 'OPERATE'] as const;

/** Format a byte array as spaced uppercase hex; '—' when empty. */
export function toHex(bytes: number[]): string {
  if (!bytes || bytes.length === 0) return '—';
  return bytes.map((b) => (b & 0xff).toString(16).padStart(2, '0').toUpperCase()).join(' ');
}

/** Short label for a frame kind. */
export function kindLabel(kind: IolinkXfer['kind']): string {
  switch (kind) {
    case 'wake_up':
      return 'WAKE-UP';
    case 'idle':
      return 'IDLE';
    case 'operate_req':
      return 'OPERATE';
    case 'cyclic':
      return 'CYCLIC';
    default:
      return kind;
  }
}

/**
 * Map the latest record's link_state to a PHASES index for the phase strip.
 * The core master models a binary startup→operate machine; we present the
 * canonical IO-Link phases, marking everything up to the current one as done.
 */
export function linkPhaseIndex(linkState: IolinkXfer['link_state']): number {
  return linkState === 'operate' ? 3 : 1;
}

/** A frame is an error only if it has an explicit failed CRC (null = no verdict). */
export function isError(x: IolinkXfer): boolean {
  return x.ck_ok === false;
}

/** Count of frames with a failed CRC. */
export function errorCount(rows: IolinkXfer[]): number {
  return rows.reduce((n, x) => (isError(x) ? n + 1 : n), 0);
}

/** Keep only failed-CRC frames. */
export function filterErrorsOnly(rows: IolinkXfer[]): IolinkXfer[] {
  return rows.filter(isError);
}

/** A CK cell value: 'ok' | 'bad' | 'na'. */
export function ckState(x: IolinkXfer): 'ok' | 'bad' | 'na' {
  if (x.ck_ok === null) return 'na';
  return x.ck_ok ? 'ok' : 'bad';
}

export interface AnnotatedIolinkXfer {
  row: IolinkXfer;
  pdInChanged: boolean;
}

function bytesEqual(a: number[], b: number[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((value, index) => (value & 0xff) === (b[index] & 0xff));
}

/** Mark cyclic process-data changes against the previous valid PD input value. */
export function annotatePdChanges(rows: IolinkXfer[]): AnnotatedIolinkXfer[] {
  let previousPdIn: number[] | null = null;

  return rows.map((row) => {
    const hasValidPdIn = row.pd_valid !== false && row.pd_in.length > 0;
    const pdInChanged = hasValidPdIn && previousPdIn !== null && !bytesEqual(row.pd_in, previousPdIn);

    if (hasValidPdIn) previousPdIn = row.pd_in.slice();

    return { row, pdInChanged };
  });
}

/** Serialize a capture to CSV for the Copy button. */
export function toCsv(rows: IolinkXfer[]): string {
  const header = 'seq,kind,link_state,pd_out,pd_in,ck_ok,raw_master,raw_device';
  const body = rows
    .map((x) =>
      [
        x.seq,
        x.kind,
        x.link_state,
        toHex(x.pd_out).replace(/ /g, ''),
        toHex(x.pd_in).replace(/ /g, ''),
        x.ck_ok === null ? '' : x.ck_ok,
        toHex(x.raw_master).replace(/ /g, ''),
        toHex(x.raw_device).replace(/ /g, ''),
      ].join(','),
    )
    .join('\n');
  return `${header}\n${body}\n`;
}
