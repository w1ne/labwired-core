// React Flow custom edge that visualises the BLE virtual air between
// two nRF52840 chips. Replaces the tldraw BleAirEdgeShape with the
// same behaviour:
//   - Animated dashed stroke when the virtual-air ring buffer grew
//     in the last 1.5s; static at lower opacity when idle.
//   - Click → opens the BleTracePanel.
//   - Auto-spawned by useBleAirEdges() for every pair of nRF52840
//     chips on the canvas.
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  BaseEdge,
  EdgeLabelRenderer,
  getStraightPath,
  type EdgeProps,
  type Edge,
  type Node,
} from '@xyflow/react';
import { useChips } from './ChipSession';
import { useBleTracePanel } from './BleTracePanel';
import type { ChipNodeData } from './ChipNode';

export type BleAirEdgeType = Edge<{ fromChipId: string; toChipId: string }, 'ble-air'>;

export function BleAirEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
}: EdgeProps<BleAirEdgeType>) {
  const { open } = useBleTracePanel();
  const active = useTrafficPulse();
  const [edgePath, labelX, labelY] = getStraightPath({ sourceX, sourceY, targetX, targetY });

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        style={{
          stroke: '#33dd66',
          strokeWidth: 2,
          strokeDasharray: '8 6',
          strokeLinecap: 'round',
          opacity: active ? 0.85 : 0.3,
          animation: active ? 'lw-ble-edge-dash 0.8s linear infinite' : undefined,
        }}
      />
      <EdgeLabelRenderer>
        <button
          type="button"
          onClick={open}
          className="nodrag nopan"
          style={{
            position: 'absolute',
            transform: `translate(-50%, calc(-50% - 18px)) translate(${labelX}px, ${labelY}px)`,
            background: 'rgba(10, 10, 15, 0.92)',
            color: active ? '#33dd66' : 'rgba(51, 221, 102, 0.55)',
            border: '1px solid rgba(51, 221, 102, 0.3)',
            borderRadius: 999,
            padding: '4px 10px',
            fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
            fontSize: 11,
            cursor: 'pointer',
            pointerEvents: 'all',
          }}
        >
          BLE air · {active ? 'live' : 'idle'}
        </button>
      </EdgeLabelRenderer>
    </>
  );
}

/// Polls the shared virtual-air trace; returns true when the buffer
/// grew in the last 1.5s. Any live bridge returns the same snapshot
/// since the trace is a process-static ring in Rust.
function useTrafficPulse(): boolean {
  const { sessions, order } = useChips();
  const [active, setActive] = useState(false);
  const lastSeenLen = useRef(0);
  const lastGrowthAt = useRef(0);

  useEffect(() => {
    let alive = true;
    const tick = () => {
      if (!alive) return;
      let bridge = null;
      for (const id of order) {
        const b = sessions[id]?.bridge;
        if (b) {
          bridge = b;
          break;
        }
      }
      if (!bridge) {
        if (active) setActive(false);
        return;
      }
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const trace = (bridge as any).sim?.air_trace_snapshot?.();
        const len = Array.isArray(trace) ? trace.length : 0;
        if (len !== lastSeenLen.current) {
          lastSeenLen.current = len;
          lastGrowthAt.current = Date.now();
        }
        const live = Date.now() - lastGrowthAt.current < 1500;
        if (live !== active) setActive(live);
      } catch {
        /* swallow */
      }
    };
    const id = window.setInterval(tick, 250);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [sessions, order, active]);

  return active;
}

/// Computes the set of BLE-air edges that should exist on the canvas:
/// one edge per pair of nRF52840 chips. Used by CanvasShell to
/// reconcile the edge list against the live chip session list.
export function useBleAirEdgesFor(nodes: Node[]): BleAirEdgeType[] {
  const { sessions, order } = useChips();
  return useMemo(() => {
    const nrfChips = order.filter((id) => {
      const s = sessions[id];
      return s && (s.board.boardId ?? '').includes('nrf52840');
    });
    const edges: BleAirEdgeType[] = [];
    for (let i = 0; i < nrfChips.length; i++) {
      for (let j = i + 1; j < nrfChips.length; j++) {
        const a = nrfChips[i]!;
        const b = nrfChips[j]!;
        // Only emit the edge if both endpoint nodes actually
        // exist on the canvas (defensive — should always be true
        // since CanvasShell syncs nodes against the same session
        // list).
        if (
          nodes.find((n) => (n.data as ChipNodeData).chipId === a) &&
          nodes.find((n) => (n.data as ChipNodeData).chipId === b)
        ) {
          edges.push({
            id: `ble-air-${a}-${b}`,
            source: `chip-${a}`,
            target: `chip-${b}`,
            type: 'ble-air',
            data: { fromChipId: a, toChipId: b },
          });
        }
      }
    }
    return edges;
  }, [sessions, order, nodes]);
}
