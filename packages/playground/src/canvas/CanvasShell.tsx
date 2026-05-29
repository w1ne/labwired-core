// Phase 1 + 2a of the canvas refactor:
//   Phase 1 mounted Tldraw with one read-only ChipShape that wrapped the
//   existing StudioShell.
//   Phase 2a turns on canvas interactivity — pan/zoom enabled, the chip
//   is draggable, layout (position + zoom) is persisted across reloads
//   via tldraw's built-in `persistenceKey` (IndexedDB).
//
// The chip's body still renders today's StudioShell via React context.
// Phase 2b lifts WasmSimulator into a per-chip session so a second
// ChipShape can run independent firmware.
import { useEffect, useState, type ReactNode } from 'react';
import { Tldraw, type Editor, createShapeId } from 'tldraw';
import 'tldraw/tldraw.css';
import { ChipShapeUtil, ChipChildrenProvider } from './ChipShape';
import './canvas.css';

const CHIP_ID = createShapeId('chip-default');
const CHIP_WIDTH = 1280;
const CHIP_HEIGHT = 800;
/// Bump when the canvas layout schema changes incompatibly — clears
/// stale IndexedDB state instead of restoring broken chip positions.
const PERSISTENCE_KEY = 'lw-canvas-v1';

export function CanvasShell({ children }: { children: ReactNode }) {
  const [editor, setEditor] = useState<Editor | null>(null);

  useEffect(() => {
    if (!editor) return;
    const shape = editor.getShape(CHIP_ID);
    if (!shape) {
      // tldraw's createShape type is closed over its built-in shape
      // union; our custom 'chip' shape is registered at runtime via
      // `shapeUtils` on <Tldraw>, so the cast is safe.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (editor.createShape as any)({
        id: CHIP_ID,
        type: 'chip',
        x: -CHIP_WIDTH / 2,
        y: -CHIP_HEIGHT / 2,
        props: { w: CHIP_WIDTH, h: CHIP_HEIGHT, chipId: 'chip-default' },
      });
      editor.zoomToFit({ animation: { duration: 0 } });
    }
  }, [editor]);

  return (
    <ChipChildrenProvider content={children}>
      <div className="lw-canvas-root">
        <Tldraw
          hideUi
          shapeUtils={[ChipShapeUtil]}
          persistenceKey={PERSISTENCE_KEY}
          onMount={(ed) => {
            setEditor(ed);
            // Phase 2a: canvas is interactive but stays in "select" mode
            // so pinch/wheel = zoom and middle-drag = pan. The chip is
            // draggable; click-and-hold inside the shape lets the
            // embedded StudioShell take pointer events (HTMLContainer
            // with pointerEvents: 'all').
            ed.setCurrentTool('select');
          }}
        />
      </div>
    </ChipChildrenProvider>
  );
}
