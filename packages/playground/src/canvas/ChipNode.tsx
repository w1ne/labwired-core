// React Flow custom node for a chip on the canvas. Replaces the
// tldraw ChipShape with the same visual + interaction contract:
//   - Active chip renders its body via React children (the
//     StudioShell content streamed through ChipChildrenContext).
//   - Inactive chips render a compact ChipCard.
//   - 28px header strip is the drag handle so the embedded
//     StudioShell keeps full pointer-event ownership.
import { createContext, useContext, type ReactNode } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { useChips, useChipSession } from './ChipSession';
import { ChipCard } from './ChipCard';

export interface ChipNodeData {
  chipId: string;
  [key: string]: unknown;
}

const ChipChildrenContext = createContext<ReactNode>(null);

export function ChipChildrenProvider({
  children,
  content,
}: {
  children: ReactNode;
  content: ReactNode;
}) {
  return <ChipChildrenContext.Provider value={content}>{children}</ChipChildrenContext.Provider>;
}

export function ChipNode({ data }: NodeProps) {
  const children = useContext(ChipChildrenContext);
  const chips = useChips();
  const chipId = (data as ChipNodeData).chipId;
  const session = useChipSession(chipId);
  const isActive = chips.activeChipId === chipId;

  return (
    <div
      className="lw-chip-node"
      data-active={isActive ? 'true' : 'false'}
      style={{
        // React Flow positions the wrapper; we own the visual.
        // Width/height comes from style; tldraw used shape.props.
        width: '100%',
        height: '100%',
        background: '#0a0a0f',
        borderRadius: 12,
        border: isActive
          ? '1px solid rgba(232, 62, 140, 0.4)'
          : '1px solid rgba(255, 255, 255, 0.08)',
        boxShadow: isActive
          ? '0 24px 64px rgba(232, 62, 140, 0.18)'
          : '0 12px 32px rgba(0, 0, 0, 0.4)',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      <div
        // Drag handle — React Flow uses `nodeDragHandle` to scope
        // drag interactions to a child. Embedded StudioShell stays
        // fully interactive because the rest of the body has
        // `nopan nodrag` class.
        className="lw-chip-node-header"
        style={{
          height: 28,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          background: 'rgba(255, 255, 255, 0.04)',
          borderBottom: '1px solid rgba(255, 255, 255, 0.06)',
          color: 'rgba(255, 255, 255, 0.6)',
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          fontSize: 11,
          letterSpacing: 0.2,
          userSelect: 'none',
          cursor: 'move',
        }}
      >
        <span style={{ opacity: 0.5 }}>●●●</span>
        <span style={{ marginLeft: 12 }}>{chipId}</span>
        {!isActive && session?.bridge && (
          <span style={{ marginLeft: 'auto', opacity: 0.5 }}>● running</span>
        )}
      </div>
      <div
        // `nopan nodrag` lets the embedded UI capture wheel + drag
        // events without the canvas hijacking them.
        className="nopan nodrag nowheel"
        style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}
      >
        {isActive ? children : session ? <ChipCard session={session} /> : null}
      </div>
      {/* Invisible handles let custom edges (BleAirEdge) anchor to
          this node — required by React Flow's edge geometry. */}
      <Handle
        type="source"
        position={Position.Right}
        style={{ opacity: 0, pointerEvents: 'none', right: 0, top: '50%' }}
      />
      <Handle
        type="target"
        position={Position.Left}
        style={{ opacity: 0, pointerEvents: 'none', left: 0, top: '50%' }}
      />
    </div>
  );
}
