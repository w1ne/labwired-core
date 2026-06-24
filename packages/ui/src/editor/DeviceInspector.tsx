import { useMemo, type CSSProperties } from 'react';
import { PropertyPanel } from './PropertyPanel';
import { RegisterGrid } from '../components/RegisterGrid/RegisterGrid';
import { SerialMonitor } from '../components/SerialMonitor/SerialMonitor';
import type { Part } from './types';
import type { SimulatorBridge } from '../wasm/simulator-bridge';
import type { SimulationState } from '../hooks/useSimulationLoop';

const noop = () => {};

// Inline styles keep this framework-agnostic (no Tailwind dependency on the
// consumer). The `.lw-inspector` / `.panel-*` class styling is shipped separately
// as `@labwired/ui/inspector.css`, which consumers import.
const S: Record<string, CSSProperties> = {
  root: { display: 'flex', flexDirection: 'column', height: '100%', overflowY: 'auto', color: '#e4e4e7' },
  live: { borderTop: '1px solid #18181b', marginTop: 4 },
  liveHead: {
    padding: '12px 12px 4px', fontSize: 9, fontWeight: 900, textTransform: 'uppercase',
    letterSpacing: '0.1em', color: '#34d399',
  },
  disasm: {
    padding: '0 12px 8px', fontFamily: 'ui-monospace, monospace', fontSize: 10, color: '#71717a',
    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
  },
  section: { padding: '0 12px 12px' },
  sectionTitle: {
    marginBottom: 4, fontSize: 9, fontWeight: 900, textTransform: 'uppercase',
    letterSpacing: '0.1em', color: '#71717a',
  },
};

/**
 * The LabWired inspector — the reused panels, read-only. Top: PropertyPanel for
 * the selected part(s) (the literal "properties"). Bottom, once a sim bridge is
 * supplied and running: live registers + serial polled straight off the
 * SimulatorBridge. Mutators are no-ops — this views a design, it doesn't edit it.
 *
 * Requires `@labwired/ui/inspector.css` to be imported by the consumer for the
 * PropertyPanel styling.
 */
export function DeviceInspector({
  parts,
  selectedIds,
  bridge,
  state,
}: {
  parts: Part[];
  selectedIds: Set<string>;
  bridge: SimulatorBridge | null;
  state: SimulationState;
}) {
  const selectedParts = useMemo(
    () => parts.filter((p) => selectedIds.has(p.id)),
    [parts, selectedIds],
  );

  // Registers come straight off the bridge; recompute each frame (cycles tick).
  const registers = useMemo(() => {
    const m = new Map<string, number>();
    if (!bridge) return m;
    try {
      bridge.getRegisterNames().forEach((n, i) => m.set(n, bridge.getRegister(i)));
    } catch {
      /* bridge torn down mid-poll */
    }
    return m;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bridge, state.cycles]);

  return (
    <div className="lw-inspector" style={S.root}>
      {/* Properties of the selected part — the reused PropertyPanel. */}
      <PropertyPanel parts={selectedParts} onUpdateAttrs={noop} onDelete={noop} onRotate={noop} />

      {/* Live instrumentation, only once a running bridge is supplied. */}
      {bridge && (
        <div style={S.live}>
          <div style={S.liveHead}>
            Live · pc 0x{state.pc.toString(16)} · {state.cycles.toLocaleString()} cyc
          </div>
          {state.disassembly && <div style={S.disasm}>{state.disassembly}</div>}
          <div style={S.section}>
            <div style={S.sectionTitle}>Registers</div>
            <RegisterGrid registers={registers} pc={state.pc} />
          </div>
          <div style={S.section}>
            <div style={S.sectionTitle}>Serial</div>
            <SerialMonitor output={state.uartOutput} />
          </div>
        </div>
      )}
    </div>
  );
}
