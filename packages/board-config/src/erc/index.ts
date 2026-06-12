import type { Diagram } from '../types';
import type { DiagramV2 } from '../schema';
import { migrateToV2 } from '../schema';
import { buildContext } from './context';
import { schemaRules } from './schema-rules';
import { matrixRules } from './matrix-rules';
import { powerRules } from './power-rules';
import { busRules } from './bus-rules';
import type { Diagnostic } from './diagnostic';
import { diag } from './diagnostic';

export type { Diagnostic, Severity } from './diagnostic';
export { uartCrossover } from './bus-rules';

/**
 * Run all ERC rule families. Accepts v1 or v2; migrates internally.
 *
 * Shape-guard: null/non-object input returns [SCHEMA_MALFORMED] rather than
 * throwing, matching the totality contract of composeDiagnostics().
 * Missing wires/nets/connections default to [] during migration.
 */
export function erc(input: Diagram | DiagramV2): Diagnostic[] {
  if (input === null || input === undefined || typeof input !== 'object') {
    return [diag('SCHEMA_MALFORMED', 'error',
      'Diagram input is null or not an object',
      'Provide an object with at least { board: string, parts: [], wires: [] }')];
  }
  const d = migrateToV2(input);
  const out: Diagnostic[] = [...schemaRules(d)];
  const ctx = buildContext(d);
  out.push(...matrixRules(ctx));
  out.push(...powerRules(ctx));
  out.push(...busRules(ctx));
  return out;
}
