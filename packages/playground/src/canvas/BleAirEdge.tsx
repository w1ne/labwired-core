// Phase 4: an auto-spawned tldraw shape that visualises the BLE
// virtual air between any two nRF52840 chips on the canvas. Rendered
// as an animated dashed line connecting the chips' centres; clicking
// the edge opens the BLE packet trace panel.
//
// Identity: BleAirEdge isn't a tldraw arrow — it's a self-positioning
// custom shape with two endpoints (chipIdA, chipIdB). Its geometry
// follows whatever positions the two ChipShapes currently have so it
// stays connected when the user drags either chip.
import { useEffect, useRef, useState } from 'react';
import {
  HTMLContainer,
  Rectangle2d,
  ShapeUtil,
  T,
  type TLBaseShape,
  type Editor,
} from 'tldraw';
import { useChips } from './ChipSession';
import { useBleTracePanel } from './BleTracePanel';

export interface BleAirEdgeProps {
  fromChipId: string;
  toChipId: string;
  w: number;
  h: number;
}

export type BleAirEdgeShape = TLBaseShape<'ble-air-edge', BleAirEdgeProps>;

declare module '@tldraw/tlschema' {
  interface TLGlobalShapePropsMap {
    'ble-air-edge': BleAirEdgeProps;
  }
}

export class BleAirEdgeShapeUtil extends ShapeUtil<BleAirEdgeShape> {
  static override type = 'ble-air-edge' as const;
  static override props = {
    fromChipId: T.string,
    toChipId: T.string,
    w: T.number,
    h: T.number,
  };

  override getDefaultProps(): BleAirEdgeShape['props'] {
    return { fromChipId: '', toChipId: '', w: 100, h: 100 };
  }
  override getGeometry(shape: BleAirEdgeShape) {
    return new Rectangle2d({ width: shape.props.w, height: shape.props.h, isFilled: false });
  }
  override canResize() {
    return false;
  }
  override canEdit() {
    return false;
  }
  override hideRotateHandle() {
    return true;
  }
  override hideResizeHandles() {
    return true;
  }
  override hideSelectionBoundsFg() {
    return true;
  }
  override component(shape: BleAirEdgeShape) {
    return <BleAirEdgeBody shape={shape} />;
  }
  override getIndicatorPath(shape: BleAirEdgeShape) {
    const path = new Path2D();
    path.rect(0, 0, shape.props.w, shape.props.h);
    return path;
  }
}

function BleAirEdgeBody({ shape }: { shape: BleAirEdgeShape }) {
  const { open } = useBleTracePanel();
  const active = useTrafficPulse();
  return (
    <HTMLContainer
      id={shape.id}
      style={{
        width: shape.props.w,
        height: shape.props.h,
        pointerEvents: 'all',
        cursor: 'pointer',
      }}
    >
      <svg
        width={shape.props.w}
        height={shape.props.h}
        viewBox={`0 0 ${shape.props.w} ${shape.props.h}`}
        onClick={open}
        style={{ display: 'block' }}
      >
        <line
          x1={0}
          y1={shape.props.h / 2}
          x2={shape.props.w}
          y2={shape.props.h / 2}
          stroke="#33dd66"
          strokeWidth={2}
          strokeDasharray="8 6"
          strokeLinecap="round"
          opacity={active ? 0.85 : 0.3}
        >
          {active && (
            <animate
              attributeName="stroke-dashoffset"
              from="0"
              to="-28"
              dur="0.8s"
              repeatCount="indefinite"
            />
          )}
        </line>
        <text
          x={shape.props.w / 2}
          y={shape.props.h / 2 - 10}
          textAnchor="middle"
          fontSize={11}
          fontFamily="ui-monospace, SFMono-Regular, Menlo, monospace"
          fill="#33dd66"
          opacity={active ? 0.9 : 0.4}
        >
          {active ? 'BLE air · live' : 'BLE air · idle'}
        </text>
      </svg>
    </HTMLContainer>
  );
}

/// Returns true when the shared virtual-air ring buffer has grown in
/// the last ~1.5s. Polls a single bridge — they all expose the same
/// process-static trace. Cheap (4Hz) and decouples the animation
/// from "any RADIO peripheral exists".
function useTrafficPulse(): boolean {
  const { sessions, order } = useChips();
  const [active, setActive] = useState(false);
  const lastSeenLen = useRef(0);
  const lastGrowthAt = useRef(0);

  useEffect(() => {
    let alive = true;
    const tick = () => {
      if (!alive) return;
      // Find any live bridge; if none, mark idle.
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

/// Reconcile a BLE air edge between every pair of nRF52840 chips on
/// the canvas. Called every time the chip layout changes; idempotent.
export function useBleAirEdgeSync(editor: Editor | null) {
  const { sessions, order } = useChips();
  useEffect(() => {
    if (!editor) return;
    const nrfChips = order.filter((id) => {
      const s = sessions[id];
      return s && s.board && (s.board.boardId ?? '').includes('nrf52840');
    });
    // Existing edge shapes on the page
    const existing = Array.from(editor.getCurrentPageShapeIds())
      .map((id) => editor.getShape(id))
      .filter((s): s is BleAirEdgeShape => !!s && s.type === 'ble-air-edge');

    const desired = new Set<string>();
    for (let i = 0; i < nrfChips.length; i++) {
      for (let j = i + 1; j < nrfChips.length; j++) {
        const a = nrfChips[i]!;
        const b = nrfChips[j]!;
        desired.add(`${a}::${b}`);
      }
    }

    // Drop edges whose endpoints no longer exist
    const stale = existing.filter(
      (s) => !desired.has(`${s.props.fromChipId}::${s.props.toChipId}`),
    );
    if (stale.length > 0) editor.deleteShapes(stale.map((s) => s.id));

    // Add missing edges. Position is recomputed each render from
    // current chip-shape geometry.
    for (const pair of desired) {
      const [a, b] = pair.split('::');
      if (!a || !b) continue;
      const have = existing.find(
        (s) => s.props.fromChipId === a && s.props.toChipId === b,
      );
      const geom = computeEdgeGeometry(editor, a, b);
      if (!geom) continue;
      if (!have) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (editor.createShape as any)({
          type: 'ble-air-edge',
          x: geom.x,
          y: geom.y,
          props: {
            fromChipId: a,
            toChipId: b,
            w: geom.w,
            h: geom.h,
          },
        });
      } else {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (editor.updateShape as any)({
          id: have.id,
          type: 'ble-air-edge',
          x: geom.x,
          y: geom.y,
          props: { fromChipId: a, toChipId: b, w: geom.w, h: geom.h },
        });
      }
    }
  }, [editor, sessions, order]);
}

function computeEdgeGeometry(editor: Editor, fromChipId: string, toChipId: string) {
  // Find the chip-shapes by their chipId prop (shapeId convention
  // matches createShapeId(`chip-${chipId}`) used by CanvasShell).
  const all = Array.from(editor.getCurrentPageShapeIds())
    .map((id) => editor.getShape(id))
    .filter((s) => s && s.type === 'chip');
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const from = all.find((s: any) => s?.props.chipId === fromChipId) as any;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const to = all.find((s: any) => s?.props.chipId === toChipId) as any;
  if (!from || !to) return null;
  // Endpoints: centre of each chip-shape (in page coords).
  const fx = from.x + from.props.w / 2;
  const fy = from.y + from.props.h / 2;
  const tx = to.x + to.props.w / 2;
  const ty = to.y + to.props.h / 2;
  const x = Math.min(fx, tx);
  const y = Math.min(fy, ty) - 20;
  const w = Math.max(40, Math.abs(tx - fx));
  const h = Math.max(40, Math.abs(ty - fy)) + 40;
  return { x, y, w, h };
}
