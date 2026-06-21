import { useRef, useState, useCallback, useEffect } from 'react';
import type { ReactNode } from 'react';
import type { Part, PinDef, ComponentState, DisplayBuffer, EditorState, WireEndpoint } from './types';
import { COMPONENT_REGISTRY } from './components/index';
import { computeDiagramBounds } from './diagramBounds';
import { validateWireConnection } from './circuitValidation';
import { WireLayer } from './WireLayer';

const GRID = 10;

function snap(v: number): number {
  return Math.round(v / GRID) * GRID;
}

type ViewBox = { x: number; y: number; w: number; h: number };

/**
 * Zoom a viewBox by `factor` around the svg-space anchor (anchorX, anchorY),
 * keeping that anchor fixed on screen. Shared by wheel zoom (desktop) and
 * pinch zoom (touch). factor < 1 zooms in, > 1 zooms out.
 */
function zoomedViewBox(vb: ViewBox, anchorX: number, anchorY: number, factor: number): ViewBox {
  const newW = Math.min(Math.max(vb.w * factor, 200), 6000);
  const newH = Math.min(Math.max(vb.h * factor, 150), 4500);
  const scale = newW / vb.w;
  return {
    x: anchorX - (anchorX - vb.x) * scale,
    y: anchorY - (anchorY - vb.y) * scale,
    w: newW,
    h: newH,
  };
}

/** Movement (in client px) below which a pointerdown→up is treated as a tap. */
const TAP_SLOP = 8;

interface EditorCanvasProps {
  state: EditorState;
  /**
   * 'edit' (default) = full desktop authoring: drag parts, wire pins, select,
   * resize. 'run' = touch-friendly read-only interaction: one-finger pan,
   * two-finger pinch zoom, tap a button to press it. No authoring in run mode.
   */
  interactionMode?: 'edit' | 'run';
  boardIoStates?: Record<string, ComponentState>;
  /** Live display framebuffers, keyed by part id. */
  displayBuffers?: Record<string, DisplayBuffer>;
  validationMessage?: string | null;
  invalidPins?: WireEndpoint[];
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
  /** Commit a note's attribute changes (e.g. after inline text editing). */
  onUpdateAttrs?: (id: string, attrs: Record<string, string>) => void;
  /**
   * Optional overlay anchored to the single selected part — e.g. a chip's
   * control toolbar. Rendered in a <foreignObject> just above the part so it
   * tracks pan/zoom automatically. Return null to render nothing for a part
   * (the caller decides which parts get an overlay, e.g. only MCUs).
   */
  selectedPartOverlay?: (
    part: Part,
    box: { x: number; y: number; width: number; height: number },
  ) => ReactNode;
  /**
   * When true, the viewBox is fit to the diagram's content (centred, with
   * padding) on mount and whenever the set of parts changes — instead of the
   * fixed default window. Used by the mobile run view so a shared circuit fills
   * the screen rather than rendering tiny and off-centre. The user can still
   * pan/zoom freely afterwards; the fit only re-runs when the diagram changes.
   */
  fitToContent?: boolean;
  /**
   * When true (run mode in a constrained pane, e.g. the ChatGPT embed), render
   * on-screen zoom-in / zoom-out / fit buttons so navigation does not depend on
   * pinch/drag gestures, and auto-refit when the container resizes (until the
   * user manually pans/zooms). Requires fitToContent for the auto-refit.
   */
  showZoomControls?: boolean;
}

export function EditorCanvas({
  state,
  interactionMode = 'edit',
  boardIoStates,
  displayBuffers,
  validationMessage,
  invalidPins,
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
  onUpdateAttrs,
  selectedPartOverlay,
  fitToContent = false,
  showZoomControls = false,
}: EditorCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [viewBox, setViewBox] = useState({ x: -100, y: -50, w: 1200, h: 800 });
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  // Set once the user manually pans or pinch-zooms, so auto-refit-on-resize
  // stops clobbering their chosen view. Cleared by an explicit fit.
  const userAdjustedRef = useRef(false);

  // Fit-to-content: when enabled, frame the diagram's parts (centred, padded)
  // whenever the set/placement of parts changes. preserveAspectRatio="meet"
  // letterboxes the difference, so the whole circuit stays visible regardless
  // of viewport aspect. Pan/zoom afterwards is preserved until the next change.
  const fitSignature = fitToContent
    ? state.diagram.parts.map((p) => `${p.id}:${p.x}:${p.y}:${p.rotate}:${p.scale ?? 1}`).join('|')
    : '';
  // Frame the diagram's parts (centred, padded) so the whole circuit fills the
  // viewport. Reused by the fit effect, the Fit button, and resize auto-refit.
  const fitNow = useCallback(() => {
    const bounds = computeDiagramBounds(state.diagram);
    if (!bounds || bounds.width <= 0 || bounds.height <= 0) return;
    // Tight margin so the circuit fills the viewport instead of floating small
    // in a sea of grid. preserveAspectRatio still letterboxes a wide circuit in
    // a tall phone, but a small pad keeps it as large as the fit allows.
    const pad = Math.max(bounds.width, bounds.height) * 0.04 + 8;
    setViewBox({
      x: bounds.x - pad,
      y: bounds.y - pad,
      w: bounds.width + pad * 2,
      h: bounds.height + pad * 2,
    });
    userAdjustedRef.current = false;
  }, [state.diagram]);
  // Zoom about the viewport centre. factor < 1 zooms in, > 1 zooms out.
  const zoomBy = useCallback((factor: number) => {
    userAdjustedRef.current = true;
    setViewBox((vb) => zoomedViewBox(vb, vb.x + vb.w / 2, vb.y + vb.h / 2, factor));
  }, []);
  useEffect(() => {
    if (!fitToContent) return;
    fitNow();
    // fitSignature changes exactly when part placement changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fitToContent, fitSignature]);
  // Auto-refit when the container resizes (e.g. the embed expanding to full
  // screen) — but only until the user has manually adjusted the view. Gated on
  // fitToContent so desktop edit mode is never observed.
  useEffect(() => {
    if (!fitToContent) return;
    const el = svgRef.current?.parentElement;
    if (!el || typeof ResizeObserver === 'undefined') return;
    let raf = 0;
    const ro = new ResizeObserver(() => {
      if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        if (!userAdjustedRef.current) fitNow();
      });
    });
    ro.observe(el);
    return () => {
      if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [fitToContent, fitNow]);
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
  // Connection-clarity emphasis state (F2). hover previews, select latches.
  const [hoveredWire, setHoveredWire] = useState<number | null>(null);
  const [selectedWire, setSelectedWire] = useState<number | null>(null);
  const [selectedPin, setSelectedPin] = useState<{ partId: string; pinId: string } | null>(null);
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
  // Active pointers (touch/mouse/pen) by pointerId, for multi-touch gestures.
  const pointersRef = useRef<Map<number, { x: number; y: number }>>(new Map());
  // In-progress two-finger pinch zoom (run mode), anchored in svg space.
  const pinchRef = useRef<{ startDist: number; startVB: ViewBox; anchorX: number; anchorY: number } | null>(null);
  // In-progress run-mode button press (released on pointerup or when it becomes a pan).
  const tapRef = useRef<{ partId: string; startX: number; startY: number } | null>(null);
  const invalidPinSet = new Set((invalidPins ?? []).map((pin) => `${pin.part}:${pin.pin}`));

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

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      pointersRef.current.set(e.pointerId, { x: e.clientX, y: e.clientY });

      // Second finger down → start a pinch zoom and abandon any pan/drag/select.
      if (pointersRef.current.size === 2) {
        const pts = [...pointersRef.current.values()];
        const dist = Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y) || 1;
        const anchor = clientToSvg((pts[0].x + pts[1].x) / 2, (pts[0].y + pts[1].y) / 2);
        pinchRef.current = { startDist: dist, startVB: { ...viewBox }, anchorX: anchor.x, anchorY: anchor.y };
        userAdjustedRef.current = true;
        setPanning(null);
        setDragging(null);
        setSelectBox(null);
        return;
      }
      if (pointersRef.current.size > 2) return;

      if (interactionMode === 'run') {
        // Touch run mode: pan from anywhere. A tap on a button is handled by the
        // part's own handler (which doesn't stop propagation), so panning is
        // armed here too but is a no-op for a stationary tap.
        svgRef.current?.setPointerCapture?.(e.pointerId);
        setPanning({ startClientX: e.clientX, startClientY: e.clientY, startVB: { ...viewBox } });
        return;
      }

      // Edit mode (desktop): rubber-band select / pan only from empty canvas.
      if ((e.target as Element).tagName === 'svg' || (e.target as Element).classList.contains('editor-grid')) {
        // Clicking empty canvas clears any latched connection emphasis (F2).
        setSelectedWire(null);
        setSelectedPin(null);
        setHoveredWire(null);
        if (state.wireInProgress) {
          onCancelWire();
          return;
        }
        const pos = clientToSvg(e.clientX, e.clientY);
        if (e.shiftKey) {
          setSelectBox({ x1: pos.x, y1: pos.y, x2: pos.x, y2: pos.y });
        } else {
          setPanning({ startClientX: e.clientX, startClientY: e.clientY, startVB: { ...viewBox } });
          onSelect(null);
        }
      }
    },
    [viewBox, clientToSvg, onSelect, onCancelWire, state.wireInProgress, interactionMode],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (pointersRef.current.has(e.pointerId)) {
        pointersRef.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
      }

      // Pinch zoom takes priority while two fingers are down.
      if (pinchRef.current && pointersRef.current.size >= 2) {
        const pts = [...pointersRef.current.values()];
        const dist = Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y) || 1;
        const factor = pinchRef.current.startDist / dist;
        setViewBox(zoomedViewBox(pinchRef.current.startVB, pinchRef.current.anchorX, pinchRef.current.anchorY, factor));
        return;
      }

      // Run-mode button press that turns into a drag → release it; pan takes over.
      if (interactionMode === 'run' && tapRef.current) {
        if (Math.abs(e.clientX - tapRef.current.startX) > TAP_SLOP || Math.abs(e.clientY - tapRef.current.startY) > TAP_SLOP) {
          onButtonToggle?.(tapRef.current.partId, false);
          tapRef.current = null;
        }
      }

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
        const dx = e.clientX - panning.startClientX;
        const dy = e.clientY - panning.startClientY;
        // A real drag (not a stationary tap) counts as a manual view adjustment.
        if (Math.abs(dx) > 2 || Math.abs(dy) > 2) userAdjustedRef.current = true;
        setViewBox({
          ...panning.startVB,
          x: panning.startVB.x - dx * scaleX,
          y: panning.startVB.y - dy * scaleY,
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
    [clientToSvg, panning, dragging, resizing, selectBox, onMovePart, onResizePart, interactionMode, onButtonToggle],
  );

  const handlePointerUp = useCallback(
    (e: React.PointerEvent) => {
      pointersRef.current.delete(e.pointerId);
      if (interactionMode === 'run') svgRef.current?.releasePointerCapture?.(e.pointerId);

      // Release an in-progress run-mode button press.
      if (tapRef.current) {
        onButtonToggle?.(tapRef.current.partId, false);
        tapRef.current = null;
      }
      // End pinch once fewer than two fingers remain; require a fresh touch to pan.
      if (pinchRef.current && pointersRef.current.size < 2) {
        pinchRef.current = null;
        setPanning(null);
      }
      if (interactionMode === 'run') {
        setPanning(null);
        return;
      }

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
    [dragging, resizing, selectBox, onSelect, onSelectRect, state.diagram.parts, interactionMode, onButtonToggle],
  );

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      const factor = e.deltaY > 0 ? 1.1 : 0.9;
      const pos = clientToSvg(e.clientX, e.clientY);
      setViewBox(zoomedViewBox(viewBox, pos.x, pos.y, factor));
    },
    [clientToSvg, viewBox],
  );

  const handlePartPointerDown = useCallback(
    (e: React.PointerEvent, part: Part) => {
      // Run mode: tap a button to press it (released on pointerup / when it
      // becomes a pan). Don't stop propagation so the svg pan tracker still arms.
      if (interactionMode === 'run') {
        const def = COMPONENT_REGISTRY.get(part.type);
        if (def?.boardIoKind === 'button' && onButtonToggle) {
          onButtonToggle(part.id, true);
          tapRef.current = { partId: part.id, startX: e.clientX, startY: e.clientY };
        }
        return;
      }
      // Edit mode: begin dragging the part.
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
    [clientToSvg, state.wireInProgress, interactionMode, onButtonToggle],
  );

  // Set on the pointerdown that started/completed a wire, so the trailing click
  // (which still sees the pre-dispatch state) does not also latch a pin select.
  const pinDrawActedRef = useRef(false);

  const handlePinClick = useCallback(
    (e: React.MouseEvent, partId: string, pinId: string) => {
      e.stopPropagation();
      const endpoint: WireEndpoint = { part: partId, pin: pinId };
      pinDrawActedRef.current = true;
      if (state.wireInProgress) {
        onCompleteWire(endpoint);
      } else {
        onStartWire(endpoint);
      }
    },
    [state.wireInProgress, onStartWire, onCompleteWire],
  );

  // Latch a pin selection for connection emphasis (F2). Fires on the click that
  // follows pointerdown; the wire-draw path (handlePinClick) runs on pointerdown
  // and starts/completes a wire — so only latch when NOT drawing and the
  // pointerdown didn't just perform a wire action.
  const handlePinSelect = useCallback(
    (e: React.MouseEvent, partId: string, pinId: string) => {
      e.stopPropagation();
      if (pinDrawActedRef.current) {
        pinDrawActedRef.current = false;
        return;
      }
      if (state.wireInProgress) return;
      setSelectedPin({ partId, pinId });
    },
    [state.wireInProgress],
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

      if (part.type === 'note') {
        setEditingNoteId(part.id);
        return;
      }

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
    [boardIoStates, onButtonToggle, onAnalogChange, setEditingNoteId],
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
      if (e.key === 'Escape') {
        onCancelWire();
        setSelectedWire(null);
        setSelectedPin(null);
        setHoveredWire(null);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onCancelWire]);

  useEffect(() => {
    if (
      editingNoteId &&
      !state.diagram.parts.some((p) => p.id === editingNoteId && p.type === 'note')
    ) {
      setEditingNoteId(null);
    }
  }, [editingNoteId, state.diagram.parts]);

  // Compute select box rect for rendering
  const selRect = selectBox ? {
    x: Math.min(selectBox.x1, selectBox.x2),
    y: Math.min(selectBox.y1, selectBox.y2),
    w: Math.abs(selectBox.x2 - selectBox.x1),
    h: Math.abs(selectBox.y2 - selectBox.y1),
  } : null;

  const canvas = (
    <svg
      ref={svgRef}
      className="editor-canvas"
      viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.w} ${viewBox.h}`}
      style={{
        width: '100%',
        height: '100%',
        background: '#1a1a2e',
        cursor: panning ? 'grabbing' : 'default',
        // Stop the browser from hijacking touch as page scroll / pinch-zoom so
        // our own pan/pinch gestures work. Critical for run mode on phones.
        touchAction: 'none',
        WebkitUserSelect: 'none',
        userSelect: 'none',
      }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerUp}
      onPointerLeave={(e) => {
        if (interactionMode === 'run') return; // captured; ignore spurious leaves
        pointersRef.current.delete(e.pointerId);
        setDragging(null); setPanning(null); setSelectBox(null); setResizing(null);
      }}
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
        activeWire={hoveredWire ?? selectedWire}
        activePinPartId={(hoveredPin ?? selectedPin)?.partId ?? null}
        activePinId={(hoveredPin ?? selectedPin)?.pinId ?? null}
        onHoverWire={setHoveredWire}
        onSelectWire={setSelectedWire}
      />

      {state.diagram.parts.map((part) => {
        const def = COMPONENT_REGISTRY.get(part.type);
        if (!def) return null;

        const isSelected = state.selectedIds.has(part.id);
        const ioState = boardIoStates?.[part.id];
        const displayBuffer = displayBuffers?.[part.id];
        const compState: ComponentState = {
          selected: isSelected,
          active: ioState?.active ?? false,
          ...ioState,
          ...(displayBuffer ? { displayBuffer } : {}),
          id: part.id,
        };
        const sc = part.scale ?? 1;
        const sw = def.width * sc;
        const sh = def.height * sc;

        return (
          <g
            key={part.id}
            data-part-id={part.id}
            transform={`translate(${part.x}, ${part.y})`}
            style={{
              cursor: interactionMode === 'run'
                ? (def.boardIoKind === 'button' ? 'pointer' : 'default')
                : (dragging?.partId === part.id ? 'grabbing' : 'grab'),
            }}
            onPointerDown={(e) => handlePartPointerDown(e, part)}
            onDoubleClick={(e) => handlePartDoubleClick(e, part)}
            onWheel={(e) => handlePartWheel(e, part)}
          >
            <g transform={`scale(${sc}) rotate(${part.rotate}, ${def.width / 2}, ${def.height / 2})`}>
              {part.type === 'note' && editingNoteId === part.id ? (
                <foreignObject x={0} y={0} width={def.width} height={1} overflow="visible">
                  <div
                    {...{ xmlns: 'http://www.w3.org/1999/xhtml' }}
                    data-note-editor={part.id}
                    contentEditable
                    suppressContentEditableWarning
                    ref={(el) => {
                      if (el && el.textContent !== (part.attrs.text ?? '')) el.textContent = part.attrs.text ?? '';
                      if (el && document.activeElement !== el) el.focus();
                    }}
                    onBlur={(e) => {
                      onUpdateAttrs?.(part.id, { text: e.currentTarget.textContent ?? '' });
                      setEditingNoteId(null);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'Escape') { e.preventDefault(); (e.currentTarget as HTMLElement).blur(); }
                    }}
                    style={{
                      width: `${def.width}px`, boxSizing: 'border-box', padding: '12px',
                      background: '#fffdf2', border: '1.5px solid #F5B642', borderRadius: '8px',
                      font: "12px/1.45 -apple-system, 'Segoe UI', sans-serif", color: '#4a3f1e',
                      whiteSpace: 'pre-wrap', wordBreak: 'break-word', outline: 'none',
                    }}
                  />
                </foreignObject>
              ) : (
                def.render(part.attrs, compState)
              )}
              {def.pins.map((pin: PinDef) => {
                const isHovered = hoveredPin?.partId === part.id && hoveredPin?.pinId === pin.id;
                const isWiring = state.wireInProgress !== null;
                const isWireOrigin = state.wireInProgress?.part === part.id && state.wireInProgress?.pin === pin.id;
                const isInvalid = invalidPinSet.has(`${part.id}:${pin.id}`);
                let isSuggested = false;
                let isBlockedTarget = false;

                if (state.wireInProgress && !isWireOrigin) {
                  const error = validateWireConnection(
                    state.diagram,
                    state.wireInProgress,
                    { part: part.id, pin: pin.id },
                  );
                  isSuggested = error === null;
                  isBlockedTarget = error !== null;
                }

                const fill = isInvalid
                  ? '#ff5f56'
                  : isWireOrigin
                    ? '#ffd166'
                    : isSuggested
                      ? '#27c93f'
                      : isWiring && isBlockedTarget
                        ? '#7a2d34'
                        : '#e83e8c';
                const opacity = isInvalid || isWireOrigin || isHovered || isSuggested
                  ? 0.98
                  : isWiring && isBlockedTarget
                    ? 0.42
                    : isWiring
                      ? 0.65
                      : 0.5;
                const stroke = isSuggested ? '#d7ffe0' : '#fff';
                const radius = (isInvalid || isSuggested || isWireOrigin ? 6 : isHovered ? 6 : 4) / sc;
                return (
                  <circle
                    key={pin.id}
                    cx={pin.x}
                    cy={pin.y}
                    r={radius}
                    fill={fill}
                    stroke={stroke}
                    strokeWidth={(isInvalid ? 1.5 : 1) / sc}
                    opacity={opacity}
                    // In run mode pins are decorative: let taps fall through to
                    // the part so buttons can be pressed and gestures still pan.
                    style={{ cursor: 'crosshair', pointerEvents: interactionMode === 'run' ? 'none' : undefined }}
                    onPointerDown={(e) => handlePinClick(e, part.id, pin.id)}
                    onClick={(e) => handlePinSelect(e, part.id, pin.id)}
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
                    onPointerDown={(e) => {
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

      {/* Overlay anchored to the single selected part (e.g. a chip's control
          toolbar). In a <foreignObject> so its HTML buttons stay crisp and it
          tracks pan/zoom with the canvas. */}
      {selectedPartOverlay && state.selectedIds.size === 1 && (() => {
        const id = [...state.selectedIds][0];
        const part = state.diagram.parts.find((p) => p.id === id);
        if (!part) return null;
        const def = COMPONENT_REGISTRY.get(part.type);
        if (!def) return null;
        const sc = part.scale ?? 1;
        const box = { x: part.x, y: part.y, width: def.width * sc, height: def.height * sc };
        const content = selectedPartOverlay(part, box);
        if (!content) return null;
        const OVERLAY_H = 44;
        const margin = 8;
        // Prefer above the part; flip below when it would clip the top of view.
        const aboveY = box.y - OVERLAY_H;
        const overlayY = aboveY < viewBox.y + margin ? box.y + box.height + margin : aboveY;
        return (
          <foreignObject
            x={box.x}
            y={overlayY}
            width={Math.max(box.width, 260)}
            height={OVERLAY_H}
            style={{ overflow: 'visible' }}
          >
            <div style={{ display: 'inline-flex' }}>{content}</div>
          </foreignObject>
        );
      })()}

      {/* Rubber-band selection rectangle */}
      {selRect && (
        <rect
          x={selRect.x} y={selRect.y} width={selRect.w} height={selRect.h}
          fill="rgba(86,156,214,0.15)" stroke="#569cd6" strokeWidth={1} strokeDasharray="4,4"
          pointerEvents="none"
        />
      )}

      {validationMessage && (
        <g transform={`translate(${viewBox.x + 16}, ${viewBox.y + 16})`} pointerEvents="none">
          <rect
            width={Math.min(Math.max(validationMessage.length * 7, 220), 520)}
            height={42}
            rx={8}
            fill="rgba(42, 12, 16, 0.94)"
            stroke="#ff5f56"
            strokeWidth={1.5}
          />
          <text
            x={14}
            y={17}
            fill="#ff8b86"
            fontFamily="'Outfit', sans-serif"
            fontSize={11}
            fontWeight={700}
          >
            Wiring Error
          </text>
          <text
            x={14}
            y={31}
            fill="#ffd7d5"
            fontFamily="'JetBrains Mono', monospace"
            fontSize={10}
          >
            {validationMessage}
          </text>
        </g>
      )}

      {state.wireInProgress && !validationMessage && (
        <g transform={`translate(${viewBox.x + 16}, ${viewBox.y + 16})`} pointerEvents="none">
          <rect
            width={260}
            height={38}
            rx={8}
            fill="rgba(18, 36, 22, 0.92)"
            stroke="#27c93f"
            strokeWidth={1.2}
          />
          <text
            x={14}
            y={16}
            fill="#9ff0af"
            fontFamily="'Outfit', sans-serif"
            fontSize={11}
            fontWeight={700}
          >
            Wiring Guide
          </text>
          <text
            x={14}
            y={29}
            fill="#d7ffe0"
            fontFamily="'JetBrains Mono', monospace"
            fontSize={10}
          >
            Green pins accept this connection. Dark pins do not.
          </text>
        </g>
      )}
    </svg>
  );

  if (!showZoomControls) return canvas;

  // One segmented control grouping the three actions. Solid background (no
  // backdrop blur): a frosted button over the board reads as a smeary haze in a
  // small embedded pane.
  const zoomBtnStyle: React.CSSProperties = {
    width: 38,
    height: 38,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    border: 'none',
    background: 'transparent',
    color: '#cbd1dc',
    cursor: 'pointer',
  };
  const divider = <div style={{ height: 1, background: 'rgba(255,255,255,0.08)' }} aria-hidden />;
  return (
    <div className="editor-canvas-shell" style={{ position: 'relative', width: '100%', height: '100%' }}>
      {canvas}
      <div
        style={{
          position: 'absolute',
          right: 12,
          bottom: 12,
          zIndex: 5,
          display: 'flex',
          flexDirection: 'column',
          borderRadius: 12,
          overflow: 'hidden',
          border: '1px solid rgba(255,255,255,0.1)',
          background: '#10182b',
          boxShadow: '0 6px 20px -8px rgba(0,0,0,0.6)',
        }}
      >
        <button type="button" aria-label="Zoom in" style={zoomBtnStyle} onClick={() => zoomBy(0.8)}>
          <svg width="17" height="17" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
            <path d="M8 3.5v9M3.5 8h9" />
          </svg>
        </button>
        {divider}
        <button type="button" aria-label="Zoom out" style={zoomBtnStyle} onClick={() => zoomBy(1.25)}>
          <svg width="17" height="17" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
            <path d="M3.5 8h9" />
          </svg>
        </button>
        {divider}
        <button type="button" aria-label="Fit to view" style={zoomBtnStyle} onClick={fitNow}>
          {/* Four corner brackets = "frame / fit to view" (not a fullscreen arrow). */}
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
            <path d="M2 5.5v-3h3M14 5.5v-3h-3M2 10.5v3h3M14 10.5v3h-3" />
          </svg>
        </button>
      </div>
    </div>
  );
}
