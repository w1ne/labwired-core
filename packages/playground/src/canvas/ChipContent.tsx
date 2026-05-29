// Two-stream content bridge for chip-on-canvas + per-chip properties:
//   - <ChipBoardContext>: streams the active chip's graphical
//     board (EditorCanvas + LED/run controls + empty-state picker)
//     into the active ChipNode body on the React Flow canvas.
//   - <ChipInspectorContext>: streams the active chip's properties
//     (Serial/Registers/Trace/Memory/Source/YAML tabs + inspector
//     panel) into the floating ChipInspectorWindow.
//
// App.tsx provides both as ReactNodes through this pair so the
// canvas and the inspector each render the correct slice of the
// existing StudioShell composition.
import { createContext, useContext, type ReactNode } from 'react';

const BoardCtx = createContext<ReactNode>(null);
const InspectorCtx = createContext<ReactNode>(null);

export function ChipContentProvider({
  board,
  inspector,
  children,
}: {
  board: ReactNode;
  inspector: ReactNode;
  children: ReactNode;
}) {
  return (
    <BoardCtx.Provider value={board}>
      <InspectorCtx.Provider value={inspector}>{children}</InspectorCtx.Provider>
    </BoardCtx.Provider>
  );
}

export const useChipBoardContent = () => useContext(BoardCtx);
export const useChipInspectorContent = () => useContext(InspectorCtx);
