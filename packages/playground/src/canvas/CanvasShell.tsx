// Canvas substrate built on React Flow (@xyflow/react). Replaces
// tldraw to avoid the commercial-license watermark.
//
// Structure:
//   - One ChipNode per session in ChipsProvider; active chip is
//     viewport-sized, inactive chips are compact 260x180 tiles
//     laid out in a column to the right of the active chip.
//   - One BleAirEdge per pair of nRF52840 chips (auto-spawned).
//   - "+ add chip" floating action button at top-left.
//   - Pan/zoom + node drag enabled; persistence in localStorage
//     (positions + chip-id mapping).
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  BackgroundVariant,
  MiniMap,
  applyNodeChanges,
  useReactFlow,
  type Node,
  type NodeChange,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { ChipNode, ChipChildrenProvider, type ChipNodeData } from './ChipNode';
import { BleAirEdge, useBleAirEdgesFor } from './BleAirEdge';
import { BleTracePanelProvider } from './BleTracePanel';
import { useChips } from './ChipSession';
import { useBackgroundChips } from './useBackgroundChips';
import './canvas.css';

const COMPACT_W = 260;
const COMPACT_H = 180;
const GAP = 40;
const MIN_ACTIVE_W = 720;
const MIN_ACTIVE_H = 480;
const POSITIONS_KEY = 'lw-canvas-positions-v1';

const nodeIdFor = (chipId: string) => `chip-${chipId}`;

const nodeTypes = { chip: ChipNode };
const edgeTypes = { 'ble-air': BleAirEdge };

interface SavedPositions {
  byNodeId: Record<string, { x: number; y: number }>;
}

function loadSavedPositions(): SavedPositions {
  if (typeof window === 'undefined') return { byNodeId: {} };
  try {
    const raw = window.localStorage.getItem(POSITIONS_KEY);
    if (!raw) return { byNodeId: {} };
    return JSON.parse(raw) as SavedPositions;
  } catch {
    return { byNodeId: {} };
  }
}

function saveSavedPositions(pos: SavedPositions) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(POSITIONS_KEY, JSON.stringify(pos));
  } catch {
    /* quota / private — skip */
  }
}

export function CanvasShell({ children }: { children: ReactNode }) {
  return (
    <BleTracePanelProvider>
      <ChipChildrenProvider content={children}>
        <ReactFlowProvider>
          <CanvasInner />
        </ReactFlowProvider>
      </ChipChildrenProvider>
    </BleTracePanelProvider>
  );
}

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

function CanvasInner() {
  const { order, activeChipId, addChip } = useChips();
  const viewport = useViewportSize();
  const { fitView } = useReactFlow();
  useBackgroundChips(true);

  const [savedPositions, setSavedPositions] = useState<SavedPositions>(() =>
    loadSavedPositions(),
  );

  // Compute the canonical layout (size + default position for each
  // chip). Inactive chips inherit a slot in the right-column unless
  // the user has dragged them somewhere — savedPositions takes
  // precedence.
  const inactiveCount = Math.max(0, order.length - 1);
  const reserveForCompact = inactiveCount > 0 ? COMPACT_W + GAP * 2 : 0;
  const activeW = Math.max(MIN_ACTIVE_W, viewport.w - reserveForCompact - 32);
  const activeH = Math.max(MIN_ACTIVE_H, viewport.h - 32);

  const nodes = useMemo<Node<ChipNodeData>[]>(() => {
    let inactiveIdx = 0;
    return order.map((chipId) => {
      const isActive = chipId === activeChipId;
      const id = nodeIdFor(chipId);
      const w = isActive ? activeW : COMPACT_W;
      const h = isActive ? activeH : COMPACT_H;
      const defaultX = isActive
        ? -activeW / 2
        : activeW / 2 + GAP;
      const defaultY = isActive
        ? -activeH / 2
        : -activeH / 2 + inactiveIdx * (COMPACT_H + GAP);
      if (!isActive) inactiveIdx += 1;
      const saved = savedPositions.byNodeId[id];
      return {
        id,
        type: 'chip',
        position: saved ?? { x: defaultX, y: defaultY },
        data: { chipId },
        // Active chip is positioned by layout, not user-draggable
        // (its size is viewport-fit so dragging it offscreen would
        // be confusing). Compact chips are draggable.
        draggable: !isActive,
        selectable: !isActive,
        // Drag handle is the header strip — see ChipNode.
        dragHandle: '.lw-chip-node-header',
        width: w,
        height: h,
        style: { width: w, height: h },
      };
    });
  }, [order, activeChipId, activeW, activeH, savedPositions]);

  const edges = useBleAirEdgesFor(nodes);

  // Capture drags into savedPositions so the layout persists across
  // chip-list mutations (add/remove/focus) and page reloads.
  const onNodesChange = useCallback((changes: NodeChange[]) => {
    let mutated = false;
    setSavedPositions((prev) => {
      const next = { ...prev.byNodeId };
      for (const c of changes) {
        if (c.type === 'position' && c.position && c.dragging === false) {
          next[c.id] = { x: c.position.x, y: c.position.y };
          mutated = true;
        }
      }
      if (!mutated) return prev;
      const result = { byNodeId: next };
      saveSavedPositions(result);
      return result;
    });
    // We don't store the node list ourselves — `nodes` is recomputed
    // from order + savedPositions every render. But we still need to
    // run applyNodeChanges on a local copy for React Flow's
    // intermediate drag rendering. The functional setSavedPositions
    // above commits the final position.
    applyNodeChanges(changes, nodes); // returned array unused; here only for type completeness.
  }, [nodes]);

  // Auto fit-view on chip count change so newly added chips don't
  // land off-screen the way they did in the tldraw build.
  useEffect(() => {
    const id = window.setTimeout(() => {
      fitView({ padding: 0.08, duration: 200 });
    }, 50);
    return () => window.clearTimeout(id);
  }, [order.length, activeChipId, fitView]);

  return (
    <div className="lw-canvas-root">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        proOptions={{ hideAttribution: true }}
        fitView
        minZoom={0.2}
        maxZoom={1.5}
        panOnDrag
        panOnScroll={false}
        zoomOnScroll
        zoomOnPinch
        nodesConnectable={false}
        nodesFocusable={false}
        edgesFocusable={false}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="rgba(255,255,255,0.06)" />
        <MiniMap
          nodeColor={(n) => (n.id === nodeIdFor(activeChipId) ? '#e83e8c' : 'rgba(255,255,255,0.35)')}
          nodeStrokeWidth={3}
          maskColor="rgba(0,0,0,0.6)"
          style={{ background: '#0a0a0f', border: '1px solid rgba(255,255,255,0.08)' }}
          pannable
          zoomable
        />
      </ReactFlow>
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
