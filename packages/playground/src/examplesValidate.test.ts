// Parity gate: every bundled example must pass the SAME validation gate that the
// share API (`composeDiagnostics`) runs at the storage boundary. If an example
// fails here, sharing it from the site would 422 and fall back to a giant
// hash-encoded URL. This test enumerates the offenders so the root causes get
// fixed in the catalog / ERC rules rather than bypassed.
import { describe, expect, it } from 'vitest';
import { composeDiagnostics } from '@labwired/board-config';
import { makeStarterDiagram } from './App';
import { BOARD_CONFIGS } from './bundled-configs';

describe('bundled examples pass the share validation gate', () => {
  for (const config of BOARD_CONFIGS) {
    it(`${config.boardId} is shareable (composeDiagnostics ok)`, () => {
      const diagram = makeStarterDiagram(config);
      const result = composeDiagnostics(diagram as never);
      if (!result.ok) {
        const errs = result.diagnostics
          .filter((d) => d.severity === 'error')
          .map((d) => `${d.code}: ${d.message}`)
          .join('\n  ');
        throw new Error(`${config.boardId} has ${result.error_count} error(s):\n  ${errs}`);
      }
      expect(result.ok).toBe(true);
    });
  }
});
