// Chip-on-canvas node. Two variants:
//   - active (focused chip): larger card, embeds the live board
//     canvas (EditorCanvas) so the chip is the visible thing on
//     the canvas — its pins, LEDs, peripherals.
//   - inactive: compact card with chipId / board name / status;
//     click to focus.
//
// Properties (Serial/Registers/Trace/Memory/Source/YAML) live in
// the floating <ChipInspectorWindow>, not inside the chip-shape.
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { useChips, useChipSession } from './ChipSession';
import { ChipCard } from './ChipCard';
import { useChipBoardContent } from './ChipContent';

export interface ChipNodeData {
  chipId: string;
  isActive: boolean;
  [key: string]: unknown;
}

export function ChipNode({ data }: NodeProps) {
  const chips = useChips();
  const chipId = (data as ChipNodeData).chipId;
  const session = useChipSession(chipId);
  const isActive = chips.activeChipId === chipId;
  const board = useChipBoardContent();

  return (
    <div
      className="lw-chip-node"
      data-active={isActive ? 'true' : 'false'}
      onMouseDown={() => {
        // Clicking ANY chip focuses it + reopens the inspector if
        // it was dismissed. Mouse-down (not click) so a drag that
        // never fires click still triggers focus on press.
        if (chipId !== chips.activeChipId) chips.setActiveChipId(chipId);
        if (!chips.inspectorOpen) chips.setInspectorOpen(true);
      }}
      style={{
        width: '100%',
        height: '100%',
        background: '#0a0a0f',
        borderRadius: 12,
        border: isActive
          ? '1px solid rgba(232, 62, 140, 0.5)'
          : '1px solid rgba(255, 255, 255, 0.08)',
        boxShadow: isActive
          ? '0 16px 48px rgba(232, 62, 140, 0.22)'
          : '0 8px 24px rgba(0, 0, 0, 0.4)',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      <div
        className="lw-chip-node-header"
        style={{
          height: 28,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          padding: '0 10px',
          background: 'rgba(255, 255, 255, 0.04)',
          borderBottom: '1px solid rgba(255, 255, 255, 0.06)',
          color: 'rgba(255, 255, 255, 0.7)',
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          fontSize: 11,
          letterSpacing: 0.2,
          userSelect: 'none',
          cursor: 'move',
        }}
      >
        <span style={{ opacity: 0.5 }}>●●●</span>
        <span style={{ marginLeft: 10 }}>{chipId}</span>
        {session && (
          <span style={{ marginLeft: 'auto', opacity: 0.55 }}>{session.board.name}</span>
        )}
      </div>
      <div
        className="nopan nodrag nowheel"
        style={{ flex: 1, minHeight: 0, overflow: 'hidden', position: 'relative' }}
      >
        {isActive ? board : session ? <ChipCard session={session} /> : null}
      </div>
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
