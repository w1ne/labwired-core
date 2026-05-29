// Phase 1+2a+2b canvas shell.
//   Phase 1 mounted Tldraw with a single read-only chip-shape wrapping
//   the StudioShell.
//   Phase 2a turned the canvas interactive (pan/zoom, drag).
//   Phase 2b makes the canvas multi-chip: it reads `chips.order` from
//   ChipsProvider and renders one ChipShape per session. The active
//   chip-shape is full-sized (carries the StudioShell as its body);
//   inactive chips are compact ChipCard tiles positioned to the right.
import { useEffect, useState, type ReactNode } from 'react';
import { Tldraw, type Editor, createShapeId, type TLShapeId } from 'tldraw';
import 'tldraw/tldraw.css';
import { ChipShapeUtil, ChipChildrenProvider } from './ChipShape';
import { BleAirEdgeShapeUtil, useBleAirEdgeSync } from './BleAirEdge';
import { BleTracePanelProvider } from './BleTracePanel';
import { useChips } from './ChipSession';
import { useBackgroundChips } from './useBackgroundChips';
import './canvas.css';

const ACTIVE_W = 1280;
const ACTIVE_H = 800;
const COMPACT_W = 260;
const COMPACT_H = 180;
const GAP = 40;
const PERSISTENCE_KEY = 'lw-canvas-v2';

const shapeIdFor = (chipId: string): TLShapeId => createShapeId(`chip-${chipId}`);

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
  useBackgroundChips(true);
  useBleAirEdgeSync(editor);

  useEffect(() => {
    if (!editor) return;
    syncShapes(editor, order, activeChipId);
  }, [editor, order, activeChipId]);

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

function syncShapes(editor: Editor, order: string[], activeChipId: string) {
  // Reconcile shapes vs. session list:
  //   - active chip → full-size box (carries StudioShell)
  //   - inactive chips → compact cards laid out to the right of active
  //   - shapes for sessions no longer present → delete
  let inactiveIndex = 0;
  for (const chipId of order) {
    const isActive = chipId === activeChipId;
    const w = isActive ? ACTIVE_W : COMPACT_W;
    const h = isActive ? ACTIVE_H : COMPACT_H;
    const x = isActive
      ? -ACTIVE_W / 2
      : ACTIVE_W / 2 + GAP + inactiveIndex * (COMPACT_W + GAP);
    const y = isActive ? -ACTIVE_H / 2 : -ACTIVE_H / 2;
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

  // Drop shapes that no longer correspond to a session.
  const liveIds = new Set(order.map(shapeIdFor));
  const allChipShapeIds = Array.from(editor.getCurrentPageShapeIds()).filter((id) => {
    const s = editor.getShape(id);
    return s && s.type === 'chip';
  });
  const stale = allChipShapeIds.filter((id) => !liveIds.has(id as TLShapeId));
  if (stale.length > 0) editor.deleteShapes(stale);

  // Centre the camera on the active chip whenever its identity changes.
  const activeShape = editor.getShape(shapeIdFor(activeChipId));
  if (activeShape) {
    editor.centerOnPoint({ x: 0, y: 0 }, { animation: { duration: 200 } });
  }
}
