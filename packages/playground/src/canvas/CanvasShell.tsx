// Phase 1+2a+2b+4 canvas shell.
//   Phase 1   - Tldraw shell with one read-only chip-shape.
//   Phase 2a  - Pan/zoom + drag.
//   Phase 2b  - Multi-chip: one ChipShape per session (active = full
//               size, inactive = compact ChipCard).
//   Phase 4   - BLE air auto-edge between any two nRF52840 chips.
//   Polish    - Active chip sizes to viewport (minus a margin so
//               compact chips fit on the right), auto-zoomToFit after
//               every shape reconciliation so newly added chips don't
//               land off-screen.
import { useEffect, useState, type ReactNode } from 'react';
import { Tldraw, type Editor, createShapeId, type TLShapeId } from 'tldraw';
import 'tldraw/tldraw.css';
import { ChipShapeUtil, ChipChildrenProvider } from './ChipShape';
import { BleAirEdgeShapeUtil, useBleAirEdgeSync } from './BleAirEdge';
import { BleTracePanelProvider } from './BleTracePanel';
import { useChips } from './ChipSession';
import { useBackgroundChips } from './useBackgroundChips';
import './canvas.css';

const COMPACT_W = 260;
const COMPACT_H = 180;
const GAP = 40;
const MIN_ACTIVE_W = 720;
const MIN_ACTIVE_H = 480;
const PERSISTENCE_KEY = 'lw-canvas-v3';

const shapeIdFor = (chipId: string): TLShapeId => createShapeId(`chip-${chipId}`);

function useViewportSize() {
  const [size, setSize] = useState(() => ({
    w: typeof window === 'undefined' ? 1280 : window.innerWidth,
    h: typeof window === 'undefined' ? 800 : window.innerHeight,
  }));
  useEffect(() => {
    const onResize = () => setSize({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return size;
}

export function CanvasShell({ children }: { children: ReactNode }) {
  return (
    <BleTracePanelProvider>
      <ChipChildrenProvider content={children}>
        <CanvasInner />
      </ChipChildrenProvider>
    </BleTracePanelProvider>
  );
}

function CanvasInner() {
  const [editor, setEditor] = useState<Editor | null>(null);
  const { order, activeChipId, addChip } = useChips();
  const viewport = useViewportSize();
  useBackgroundChips(true);
  useBleAirEdgeSync(editor);

  useEffect(() => {
    if (!editor) return;
    const inactiveCount = Math.max(0, order.length - 1);
    // Active chip fills the viewport minus a reserve on the right for
    // the inactive ChipCards (column of compact tiles) and a small
    // outer margin so the chip doesn't bleed past the screen edges.
    const reserveForCompact = inactiveCount > 0 ? COMPACT_W + GAP * 2 : 0;
    const activeW = Math.max(MIN_ACTIVE_W, viewport.w - reserveForCompact - 32);
    const activeH = Math.max(MIN_ACTIVE_H, viewport.h - 32);
    syncShapes(editor, order, activeChipId, activeW, activeH);
  }, [editor, order, activeChipId, viewport.w, viewport.h]);

  return (
    <div className="lw-canvas-root">
      <Tldraw
        hideUi
        shapeUtils={[ChipShapeUtil, BleAirEdgeShapeUtil]}
        persistenceKey={PERSISTENCE_KEY}
        onMount={(ed) => {
          setEditor(ed);
          ed.setCurrentTool('select');
        }}
      />
      <button
        type="button"
        onClick={() => addChip()}
        className="lw-canvas-add-chip"
        aria-label="Add chip"
        title="Add chip"
      >
        + add chip
      </button>
    </div>
  );
}

function syncShapes(
  editor: Editor,
  order: string[],
  activeChipId: string,
  activeW: number,
  activeH: number,
) {
  let inactiveIndex = 0;
  for (const chipId of order) {
    const isActive = chipId === activeChipId;
    const w = isActive ? activeW : COMPACT_W;
    const h = isActive ? activeH : COMPACT_H;
    const x = isActive ? -activeW / 2 : activeW / 2 + GAP;
    const y = isActive
      ? -activeH / 2
      : -activeH / 2 + inactiveIndex * (COMPACT_H + GAP);
    if (!isActive) inactiveIndex += 1;

    const id = shapeIdFor(chipId);
    const existing = editor.getShape(id);
    if (!existing) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (editor.createShape as any)({
        id,
        type: 'chip',
        x,
        y,
        props: { w, h, chipId },
      });
    } else {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (editor.updateShape as any)({
        id,
        type: 'chip',
        x,
        y,
        props: { w, h, chipId },
      });
    }
  }

  // Drop shapes that no longer correspond to a session (active or
  // inactive). Edge shapes are managed by useBleAirEdgeSync.
  const liveIds = new Set(order.map(shapeIdFor));
  const orphanChipShapes = Array.from(editor.getCurrentPageShapeIds())
    .map((id) => ({ id, shape: editor.getShape(id) }))
    .filter(({ shape }) => shape && shape.type === 'chip')
    .filter(({ id }) => !liveIds.has(id as TLShapeId))
    .map(({ id }) => id);
  if (orphanChipShapes.length > 0) editor.deleteShapes(orphanChipShapes);

  // After reconciling shapes, fit everything in view so newly added
  // chips don't land off-screen (tldraw culls shapes outside camera
  // bounds — they exist in the store but render `display: none`).
  editor.zoomToFit({ animation: { duration: 200 } });
}
