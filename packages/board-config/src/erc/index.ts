import type { Diagram } from '../types';
import type { DiagramV2 } from '../schema';
import { migrateToV2 } from '../schema';
import { buildContext } from './context';
import { schemaRules } from './schema-rules';
import { matrixRules } from './matrix-rules';
import { powerRules } from './power-rules';
import { busRules } from './bus-rules';
import type { Diagnostic } from './diagnostic';

export type { Diagnostic, Severity } from './diagnostic';
export { uartCrossover } from './bus-rules';

/** Run all ERC rule families. Accepts v1 or v2; migrates internally. */
export function erc(input: Diagram | DiagramV2): Diagnostic[] {
  const d = migrateToV2(input);
  const out: Diagnostic[] = [...schemaRules(d)];
  const ctx = buildContext(d);
  out.push(...matrixRules(ctx));
  out.push(...powerRules(ctx));
  out.push(...busRules(ctx));
  return out;
}
