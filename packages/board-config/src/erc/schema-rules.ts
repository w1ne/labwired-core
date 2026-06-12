import type { DiagramV2 } from '../schema';
import { parsePinRef } from '../schema';
import { getCatalogPart, CATALOG } from '../catalog';
import { PIN_MAPS } from '../pin-mapping';
import { diag, type Diagnostic } from './diagnostic';

/** Cheap edit-distance for closest-match hints (insert/delete/replace = 1). */
function editDistance(a: string, b: string): number {
  const dp = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) dp[0][j] = j;
  for (let i = 1; i <= a.length; i++)
    for (let j = 1; j <= b.length; j++)
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
  return dp[a.length][b.length];
}

function closest(input: string, candidates: string[]): string | undefined {
  let best: string | undefined;
  let bestD = 3; // suggest only near-misses
  for (const c of candidates) {
    const d = editDistance(input, c);
    if (d < bestD) { bestD = d; best = c; }
  }
  return best;
}

export function schemaRules(d: DiagramV2): Diagnostic[] {
  const out: Diagnostic[] = [];
  if (!PIN_MAPS[d.board]) {
    out.push(diag('SCHEMA_BOARD_UNKNOWN', 'error',
      `board '${d.board}' has no pin map`,
      `Known boards: ${Object.keys(PIN_MAPS).sort().join(', ')}`));
  }
  const netNames = new Set<string>();
  for (const n of d.nets) {
    if (netNames.has(n.name)) {
      out.push(diag('SCHEMA_NET_DUPLICATE', 'error',
        `net '${n.name}' declared more than once`, 'Give each net a unique name', [n.name]));
    }
    netNames.add(n.name);
  }
  const partIds = new Set(d.parts.map((p) => p.id));
  for (const p of d.parts) {
    if (!getCatalogPart(p.type) && !PIN_MAPS[p.type] && p.type !== 'mcu') {
      const suggestion = closest(p.type, [...Object.keys(CATALOG), ...Object.keys(PIN_MAPS)]);
      out.push(diag('SCHEMA_PART_UNKNOWN', 'error',
        `part '${p.id}' has unknown type '${p.type}'`,
        suggestion ? `Did you mean '${suggestion}'?` : 'See the part catalog for valid types', [p.id]));
    }
  }
  for (const [ref, netName] of d.connections) {
    const pin = parsePinRef(ref);
    if (!pin) {
      out.push(diag('SCHEMA_PINREF_MALFORMED', 'error',
        `connection ref '${ref}' is not 'partId:pinName'`, "Use the form 'partId:pinName'", [ref]));
      continue;
    }
    if (!partIds.has(pin.part)) {
      out.push(diag('SCHEMA_CONN_UNKNOWN_PART', 'error',
        `connection '${ref}' references missing part '${pin.part}'`, 'Add the part or fix the id', [ref]));
    }
    if (!netNames.has(netName)) {
      out.push(diag('SCHEMA_NET_UNDECLARED', 'error',
        `connection '${ref}' references undeclared net '${netName}'`,
        `Declare the net in nets[] (known: ${[...netNames].join(', ') || 'none'})`, [ref, netName]));
    }
  }
  return out;
}
