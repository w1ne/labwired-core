import { useRef, useState, useCallback, useEffect } from 'react';
import type { Part, PinDef, ComponentState, EditorState, WireEndpoint } from './types';
import { COMPONENT_REGISTRY } from './components/index';
import { WireLayer } from './WireLayer';

const GRID = 10;

function snap(v: number): number {
  return Math.round(v / GRID) * GRID;
}

interface EditorCanvasProps {
  state: EditorState;
  boardIoStates?: Record<string, ComponentState>;
  onMovePart: (id: string, x: number, y: number) => void;
  onResizePart?: (id: string, scale: number) => void;
  onSelect: (id: string | null, add?: boolean) => void;
  onSelectRect?: (ids: string[]) => void;
  onStartWire: (endpoint: WireEndpoint) => void;
  onCompleteWire: (endpoint: WireEndpoint) => void;
  onCancelWire: () => void;
  onDeleteWire: (index: number) => void;
  onDropPart?: (type: string, x: number, y: number) => void;
  /** Toggle a button/switch component on/off (double-click). */
  onButtonToggle?: (partId: string, active: boolean) => void;
  /** Set analog value for adc_input components (e.g. potentiometer). Value 0-4095. */
  onAnalogChange?: (partId: string, value: number) => void;
}

export function EditorCanvas({
  state,
  boardIoStates,
  onMovePart,
  onResizePart,
  onSelect,
  onSelectRect,
  onStartWire,
  onCompleteWire,
  onCancelWire,
  onDeleteWire,
  onDropPart,
  onButtonToggle,
  onAnalogChange,
}: EditorCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [viewBox, setViewBox] = useState({ x: -100, y: -50, w: 1200, h: 800 });
  const [dragging, setDragging] = useState<{
    partId: string;
    offsetX: number;
    offsetY: number;
    startX: number;
    startY: number;
    moved: boolean;
  } | null>(null);
  const [panning, setPanning] = useState<{ startClientX: number; startClientY: number; startVB: typeof viewBox } | null>(null);
  const [cursorSvg, setCursorSvg] = useState<{ x: number; y: number } | null>(null);
  const [hoveredPin, setHoveredPin] = useState<{ partId: string; pinId: string } | null>(null);
  // Resize handle dragging
  const [resizing, setResizing] = useState<{
    partId: string;
    startDist: number;
    startScale: number;
    cx: number;
    cy: number;
  } | null>(null);
  // Rubber-band selection
  const [selectBox, setSelectBox] = useState<{ x1: number; y1: number; x2: number; y2: number } | null>(null);

  const clientToSvg = useCallback(
    (clientX: number, clientY: number) => {
      const svg = svgRef.current;
      if (!svg) return { x: 0, y: 0 };
      const rect = svg.getBoundingClientRect();
      const scaleX = viewBox.w / rect.width;
      const scaleY = viewBox.h / rect.height;
      return {
        x: viewBox.x + (clientX - rect.left) * scaleX,
        y: viewBox.y + (clientY - rect.top) * scaleY,
      };
    },
    [viewBox],
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      if ((e.target as Element).tagName === 'svg' || (e.target as Element).classList.contains('editor-grid')) {
        if (state.wireInProgress) {
          onCancelWire();
          return;
        }
        // Start rubber-band selection (or pan if not shift)
        const pos = clientToSvg(e.clientX, e.clientY);
        if (e.shiftKey) {
          setSelectBox({ x1: pos.x, y1: pos.y, x2: pos.x, y2: pos.y });
        } else {
          setPanning({ startClientX: e.clientX, startClientY: e.clientY, startVB: { ...viewBox } });
          onSelect(null);
        }
      }
    },
    [viewBox, clientToSvg, onSelect, onCancelWire, state.wireInProgress],
  );

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      const pos = clientToSvg(e.clientX, e.clientY);
      setCursorSvg(pos);

      if (selectBox) {
        setSelectBox({ ...selectBox, x2: pos.x, y2: pos.y });
        return;
      }

      if (panning) {
        const svg = svgRef.current;
        if (!svg) return;
        const rect = svg.getBoundingClientRect();
        const scaleX = panning.startVB.w / rect.width;
        const scaleY = panning.startVB.h / rect.height;
        setViewBox({
          ...panning.startVB,
          x: panning.startVB.x - (e.clientX - panning.startClientX) * scaleX,
          y: panning.startVB.y - (e.clientY - panning.startClientY) * scaleY,
        });
        return;
      }

      if (resizing && onResizePart) {
        const dx = pos.x - resizing.cx;
        const dy = pos.y - resizing.cy;
        const dist = Math.sqrt(dx * dx + dy * dy);
        const newScale = Math.max(0.3, Math.min(4, resizing.startScale * (dist / resizing.startDist)));
        onResizePart(resizing.partId, Math.round(newScale * 10) / 10);
        return;
      }

      if (dragging) {
        const snappedX = snap(pos.x - dragging.offsetX);
        const snappedY = snap(pos.y - dragging.offsetY);
        if (!dragging.moved && (Math.abs(pos.x - dragging.startX) > 3 || Math.abs(pos.y - dragging.startY) > 3)) {
          setDragging({ ...dragging, moved: true });
        }
        onMovePart(dragging.partId, snappedX, snappedY);
      }
    },
    [clientToSvg, panning, dragging, resizing, selectBox, onMovePart, onResizePart],
  );

  const handleMouseUp = useCallback(
    (e: React.MouseEvent) => {
      // Finish rubber-band selection
      if (selectBox) {
        const minX = Math.min(selectBox.x1, selectBox.x2);
        const maxX = Math.max(selectBox.x1, selectBox.x2);
        const minY = Math.min(selectBox.y1, selectBox.y2);
        const maxY = Math.max(selectBox.y1, selectBox.y2);
        const ids = state.diagram.parts
          .filter((p) => {
            const def = COMPONENT_REGISTRY.get(p.type);
            if (!def) return false;
            const s = p.scale ?? 1;
            const px = p.x + (def.width * s) / 2;
            const py = p.y + (def.height * s) / 2;
            return px >= minX && px <= maxX && py >= minY && py <= maxY;
          })
          .map((p) => p.id);
        onSelectRect?.(ids);
        setSelectBox(null);
        return;
      }

      if (resizing) {
        setResizing(null);
        return;
      }
      if (dragging && !dragging.moved) {
        onSelect(dragging.partId, e.shiftKey);
      }
      setDragging(null);
      setPanning(null);
    },
    [dragging, resizing, selectBox, onSelect, onSelectRect, state.diagram.parts],
  );

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      const factor = e.deltaY > 0 ? 1.1 : 0.9;
      const pos = clientToSvg(e.clientX, e.clientY);
      const newW = Math.min(Math.max(viewBox.w * factor, 200), 6000);
      const newH = Math.min(Math.max(viewBox.h * factor, 150), 4500);
      const scale = newW / viewBox.w;
      setViewBox({
        x: pos.x - (pos.x - viewBox.x) * scale,
        y: pos.y - (pos.y - viewBox.y) * scale,
        w: newW,
        h: newH,
      });
    },
    [clientToSvg, viewBox],
  );

  const handlePartMouseDown = useCallback(
    (e: React.MouseEvent, part: Part) => {
      e.stopPropagation();
      if (state.wireInProgress) return;
      const pos = clientToSvg(e.clientX, e.clientY);
      setDragging({
        partId: part.id,
        offsetX: pos.x - part.x,
        offsetY: pos.y - part.y,
        startX: pos.x,
        startY: pos.y,
        moved: false,
      });
    },
    [clientToSvg, state.wireInProgress],
  );

  const handlePinClick = useCallback(
    (e: React.MouseEvent, partId: string, pinId: string) => {
      e.stopPropagation();
      const endpoint: WireEndpoint = { part: partId, pin: pinId };
      if (state.wireInProgress) {
        onCompleteWire(endpoint);
      } else {
        onStartWire(endpoint);
      }
    },
    [state.wireInProgress, onStartWire, onCompleteWire],
  );

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const type = e.dataTransfer.getData('application/x-component-type');
      if (!type || !onDropPart) return;
      const pos = clientToSvg(e.clientX, e.clientY);
      const def = COMPONENT_REGISTRY.get(type);
      onDropPart(type, snap(pos.x - (def?.width ?? 40) / 2), snap(pos.y - (def?.height ?? 40) / 2));
    },
    [clientToSvg, onDropPart],
  );

  // Double-click handler for interactive components (buttons, potentiometers)
  const handlePartDoubleClick = useCallback(
    (e: React.MouseEvent, part: Part) => {
      e.stopPropagation();
      const def = COMPONENT_REGISTRY.get(part.type);
      if (!def) return;

      if (def.boardIoKind === 'button' && onButtonToggle) {
        const currentActive = boardIoStates?.[part.id]?.active ?? false;
        onButtonToggle(part.id, !currentActive);
      } else if (def.boardIoKind === 'adc_input' && onAnalogChange) {
        // Cycle through preset values: 0 → 1024 → 2048 → 3072 → 4095 → 0
        const current = boardIoStates?.[part.id]?.analogValue ?? 0;
        const presets = [0, 1024, 2048, 3072, 4095];
        const nextIdx = (presets.findIndex((v) => v >= current) + 1) % presets.length;
        onAnalogChange(part.id, presets[nextIdx]);
      }
    },
    [boardIoStates, onButtonToggle, onAnalogChange],
  );

  // Scroll-wheel on potentiometer/analog parts to fine-tune value
  const handlePartWheel = useCallback(
    (e: React.WheelEvent, part: Part) => {
      const def = COMPONENT_REGISTRY.get(part.type);
      if (!def || def.boardIoKind !== 'adc_input' || !onAnalogChange) return;
      e.stopPropagation();
      e.preventDefault();
      const current = boardIoStates?.[part.id]?.analogValue ?? 2048;
      const step = e.shiftKey ? 256 : 64;
      const delta = e.deltaY > 0 ? -step : step;
      onAnalogChange(part.id, Math.max(0, Math.min(4095, current + delta)));
    },
    [boardIoStates, onAnalogChange],
  );

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === 'INPUT') return;
      if (e.key === 'Escape') onCancelWire();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onCancelWire]);

  // Compute select box rect for rendering
  const selRect = selectBox ? {
    x: Math.min(selectBox.x1, selectBox.x2),
    y: Math.min(selectBox.y1, selectBox.y2),
    w: Math.abs(selectBox.x2 - selectBox.x1),
    h: Math.abs(selectBox.y2 - selectBox.y1),
  } : null;

  return (
    <svg
      ref={svgRef}
      className="editor-canvas"
      viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.w} ${viewBox.h}`}
      style={{ width: '100%', height: '100%', background: '#1a1a2e', cursor: panning ? 'grabbing' : 'default' }}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={() => { setDragging(null); setPanning(null); setSelectBox(null); setResizing(null); }}
      onWheel={handleWheel}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      <defs>
        <pattern id="editor-grid-sm" width={GRID} height={GRID} patternUnits="userSpaceOnUse">
          <circle cx={GRID / 2} cy={GRID / 2} r={0.5} fill="rgba(255,255,255,0.06)" />
        </pattern>
        <pattern id="editor-grid-lg" width={GRID * 10} height={GRID * 10} patternUnits="userSpaceOnUse">
          <rect width={GRID * 10} height={GRID * 10} fill="url(#editor-grid-sm)" />
          <circle cx={GRID * 5} cy={GRID * 5} r={1} fill="rgba(255,255,255,0.12)" />
        </pattern>
      </defs>
      <rect
        className="editor-grid"
        x={viewBox.x - 1000} y={viewBox.y - 1000}
        width={viewBox.w + 2000} height={viewBox.h + 2000}
        fill="url(#editor-grid-lg)"
      />

      <WireLayer
        wires={state.diagram.wires}
        parts={state.diagram.parts}
        wireFrom={state.wireInProgress}
        cursorPos={cursorSvg}
        onDeleteWire={onDeleteWire}
      />

      {state.diagram.parts.map((part) => {
        const def = COMPONENT_REGISTRY.get(part.type);
        if (!def) return null;

        const isSelected = state.selectedIds.has(part.id);
        const ioState = boardIoStates?.[part.id];
        const compState: ComponentState = {
          selected: isSelected,
          active: ioState?.active ?? false,
          ...ioState,
        };
        const sc = part.scale ?? 1;
        const sw = def.width * sc;
        const sh = def.height * sc;

        return (
          <g
            key={part.id}
            transform={`translate(${part.x}, ${part.y})`}
            style={{ cursor: dragging?.partId === part.id ? 'grabbing' : 'grab' }}
            onMouseDown={(e) => handlePartMouseDown(e, part)}
            onDoubleClick={(e) => handlePartDoubleClick(e, part)}
            onWheel={(e) => handlePartWheel(e, part)}
          >
            <g transform={`scale(${sc}) rotate(${part.rotate}, ${def.width / 2}, ${def.height / 2})`}>
              {def.render(part.attrs, compState)}
              {def.pins.map((pin: PinDef) => {
                const isHovered = hoveredPin?.partId === part.id && hoveredPin?.pinId === pin.id;
                const isWiring = state.wireInProgress !== null;
                return (
                  <circle
                    key={pin.id}
                    cx={pin.x}
                    cy={pin.y}
                    r={(isHovered ? 6 : 4) / sc}
                    fill={isWiring ? '#27c93f' : '#e83e8c'}
                    stroke="#fff"
                    strokeWidth={1 / sc}
                    opacity={isHovered || isWiring ? 0.9 : 0.5}
                    style={{ cursor: 'crosshair' }}
                    onMouseDown={(e) => handlePinClick(e, part.id, pin.id)}
                    onMouseEnter={() => setHoveredPin({ partId: part.id, pinId: pin.id })}
                    onMouseLeave={() => setHoveredPin(null)}
                  />
                );
              })}
            </g>
            {/* Resize handles on selected components */}
            {isSelected && onResizePart && (
              <>
                {/* Selection outline */}
                <rect
                  x={-2} y={-2} width={sw + 4} height={sh + 4}
                  fill="none" stroke="#569cd6" strokeWidth={1} strokeDasharray="4,2"
                  pointerEvents="none"
                />
                {/* Corner resize handles */}
                {[[0, 0], [sw, 0], [0, sh], [sw, sh]].map(([hx, hy], i) => (
                  <rect
                    key={i}
                    x={hx - 4} y={hy - 4} width={8} height={8}
                    fill="#569cd6" stroke="#fff" strokeWidth={1}
                    style={{ cursor: 'nwse-resize' }}
                    onMouseDown={(e) => {
                      e.stopPropagation();
                      const pos = clientToSvg(e.clientX, e.clientY);
                      const cx = part.x + sw / 2;
                      const cy = part.y + sh / 2;
                      const dx = pos.x - cx;
                      const dy = pos.y - cy;
                      setResizing({
                        partId: part.id,
                        startDist: Math.sqrt(dx * dx + dy * dy),
                        startScale: sc,
                        cx,
                        cy,
                      });
                    }}
                  />
                ))}
              </>
            )}
          </g>
        );
      })}

      {/* Rubber-band selection rectangle */}
      {selRect && (
        <rect
          x={selRect.x} y={selRect.y} width={selRect.w} height={selRect.h}
          fill="rgba(86,156,214,0.15)" stroke="#569cd6" strokeWidth={1} strokeDasharray="4,4"
          pointerEvents="none"
        />
      )}
    </svg>
  );
}
