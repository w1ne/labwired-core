import { CSSProperties, useMemo } from 'react';
import { BoardIoBinding, BoardIoState } from '../../wasm/simulator-bridge';
import { Led } from '../Led/Led';

export interface BoardCanvasProps {
  /** Board name displayed on the MCU block. */
  boardName: string;
  /** Chip identifier (e.g. "STM32F107"). */
  chipId: string;
  /** Board IO bindings from the system manifest. */
  boardIo: BoardIoBinding[];
  /** Current board IO states. */
  boardIoStates: BoardIoState[];
  /** Called when user presses/releases a button binding. */
  onButtonToggle?: (id: string, pressed: boolean) => void;
  /** Canvas width. Default: 600. */
  width?: number;
  /** Canvas height. Default: 400. */
  height?: number;
  style?: CSSProperties;
}

interface NodeLayout {
  id: string;
  kind: string;
  label: string;
  sublabel: string;
  x: number;
  y: number;
  active: boolean | null;
}

/**
 * SVG board visualization showing the MCU block and connected board IO nodes.
 * Adapted from the VS Code topology panel (vscode/media/topology.js).
 */
export function BoardCanvas({
  boardName,
  chipId,
  boardIo,
  boardIoStates,
  onButtonToggle,
  width = 600,
  height = 400,
  style,
}: BoardCanvasProps) {
  const stateMap = useMemo(() => {
    const map = new Map<string, boolean>();
    for (const s of boardIoStates) {
      map.set(s.id, s.active);
    }
    return map;
  }, [boardIoStates]);

  const nodes = useMemo((): NodeLayout[] => {
    if (boardIo.length === 0) return [];

    const cx = width / 2;
    const cy = height / 2;
    const radius = Math.min(width, height) * 0.32;

    return boardIo.map((io, index) => {
      const angle = (index / boardIo.length) * 2 * Math.PI - Math.PI / 2;
      return {
        id: io.id,
        kind: io.kind,
        label: io.id,
        sublabel: `${io.peripheral.toUpperCase()}[${io.pin}]`,
        x: cx + radius * Math.cos(angle),
        y: cy + radius * Math.sin(angle),
        active: stateMap.get(io.id) ?? null,
      };
    });
  }, [boardIo, boardIoStates, width, height, stateMap]);

  const cx = width / 2;
  const cy = height / 2;
  const mcuW = 180;
  const mcuH = 100;

  return (
    <div style={{
      background: 'var(--lw-bg, #fff)',
      border: 'var(--lw-border, 2px solid #000)',
      borderRadius: 'var(--lw-radius, 12px)',
      boxShadow: 'var(--lw-shadow, 4px 4px 0px #000)',
      overflow: 'hidden',
      ...style,
    }}>
      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        style={{ display: 'block' }}
      >
        <defs>
          <style>{`
            .mcu-block { fill: #1e1e28; stroke: #000; stroke-width: 2; rx: 10; }
            .mcu-label { fill: #fff; font-family: 'Outfit', sans-serif; font-size: 16px; font-weight: 700; }
            .mcu-sub { fill: #888; font-family: 'JetBrains Mono', monospace; font-size: 11px; }
            .wire { stroke: #ccc; stroke-width: 2; stroke-dasharray: 6 3; }
            .node-card { fill: #fff; stroke: #000; stroke-width: 2; rx: 8; }
            .node-label { fill: #000; font-family: 'Outfit', sans-serif; font-size: 12px; font-weight: 700; }
            .node-sub { fill: #888; font-family: 'JetBrains Mono', monospace; font-size: 10px; }
            .pill-on { fill: #27c93f; }
            .pill-off { fill: #888; }
            .pill-unknown { fill: #444; }
            .pill-text { fill: #fff; font-family: 'Outfit', sans-serif; font-size: 9px; font-weight: 700; }
          `}</style>
        </defs>

        {/* Wires from MCU to each node */}
        {nodes.map((node) => (
          <line
            key={`wire-${node.id}`}
            x1={cx}
            y1={cy}
            x2={node.x}
            y2={node.y}
            className="wire"
          />
        ))}

        {/* MCU block */}
        <g transform={`translate(${cx - mcuW / 2}, ${cy - mcuH / 2})`}>
          <rect width={mcuW} height={mcuH} className="mcu-block" />
          <text x={mcuW / 2} y={40} textAnchor="middle" className="mcu-label">
            {chipId}
          </text>
          <text x={mcuW / 2} y={62} textAnchor="middle" className="mcu-sub">
            {boardName}
          </text>
        </g>

        {/* Node cards */}
        {nodes.map((node) => {
          const cardW = 140;
          const cardH = 72;
          const pillState = getStateView(node.kind, node.active);

          return (
            <g
              key={node.id}
              transform={`translate(${node.x - cardW / 2}, ${node.y - cardH / 2})`}
              style={{ cursor: node.kind === 'button' ? 'pointer' : 'default' }}
              onMouseDown={node.kind === 'button' ? () => onButtonToggle?.(node.id, true) : undefined}
              onMouseUp={node.kind === 'button' ? () => onButtonToggle?.(node.id, false) : undefined}
              onMouseLeave={node.kind === 'button' && node.active ? () => onButtonToggle?.(node.id, false) : undefined}
            >
              <rect width={cardW} height={cardH} className="node-card" />
              <text x={10} y={20} className="node-label">{node.label}</text>
              <text x={10} y={36} className="node-sub">{node.sublabel}</text>

              {/* State pill */}
              <rect
                x={cardW - 70}
                y={cardH - 24}
                width={60}
                height={16}
                rx={8}
                className={`pill-${pillState.css}`}
              />
              <text
                x={cardW - 40}
                y={cardH - 13}
                textAnchor="middle"
                className="pill-text"
              >
                {pillState.label}
              </text>

              {/* LED glow indicator for LED nodes */}
              {node.kind === 'led' && (
                <foreignObject x={10} y={cardH - 28} width={24} height={24}>
                  <Led active={node.active === true} size={18} />
                </foreignObject>
              )}
            </g>
          );
        })}

        {/* Empty state */}
        {nodes.length === 0 && (
          <text
            x={cx}
            y={cy + 80}
            textAnchor="middle"
            className="mcu-sub"
          >
            No board IO configured
          </text>
        )}
      </svg>
    </div>
  );
}

function getStateView(kind: string, active: boolean | null): { label: string; css: string } {
  if (active === null) return { label: 'N/A', css: 'unknown' };
  if (kind === 'button') return active ? { label: 'PRESSED', css: 'on' } : { label: 'RELEASED', css: 'off' };
  return active ? { label: 'ON', css: 'on' } : { label: 'OFF', css: 'off' };
}
