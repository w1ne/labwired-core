// Canvas substrate built on React Flow. Now hosts compact chip
// cards only — the active chip's full per-chip view lives in a
// separate floating <ChipInspectorWindow> rendered above the canvas.
//
// Each ChipCard:
//   - shows chipId / board / status (running / source-ready / empty)
//   - is fully draggable (no active-vs-inactive size distinction)
//   - clicking it makes that chip the active one + reopens the
//     inspector window for it
//
// Auto-edges between nRF52840 chips (BleAirEdge) are spawned by
// useBleAirEdgesFor; clicking an edge opens the BleTracePanel.
import { useCallback, useEffect, useMemo, useState } from 'react';
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
import { ChipNode, type ChipNodeData } from './ChipNode';
import { BleAirEdge, useBleAirEdgesFor } from './BleAirEdge';
import { BleTracePanelProvider } from './BleTracePanel';
import { useChips } from './ChipSession';
import { useBackgroundChips } from './useBackgroundChips';
import { GLOBAL_CHROME_HEIGHT } from '../studio/GlobalChrome';
import './canvas.css';

const CARD_W = 260;
const CARD_H = 200;
const ACTIVE_W = 640;
const ACTIVE_H = 520;
const GAP = 48;
const POSITIONS_KEY = 'lw-canvas-positions-v2';

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
    /* quota — skip */
  }
}

export function CanvasShell() {
  return (
    <BleTracePanelProvider>
      <ReactFlowProvider>
        <CanvasInner />
      </ReactFlowProvider>
    </BleTracePanelProvider>
  );
}

function CanvasInner() {
  const { order, activeChipId, addChip } = useChips();
  const { fitView } = useReactFlow();
  useBackgroundChips(true);

  const [savedPositions, setSavedPositions] = useState<SavedPositions>(() =>
    loadSavedPositions(),
  );

  // Layout: chip cards laid out in a horizontal row across the
  // canvas, equally spaced. User can drag to reposition;
  // savedPositions overrides the default.
  const nodes = useMemo<Node<ChipNodeData>[]>(() => {
    return order.map((chipId, idx) => {
      const id = nodeIdFor(chipId);
      const isActive = chipId === activeChipId;
      const w = isActive ? ACTIVE_W : CARD_W;
      const h = isActive ? ACTIVE_H : CARD_H;
      // Simple row layout: active centred, inactive cards stack to
      // the right. User can drag any chip to reposition;
      // savedPositions overrides default.
      const defaultX = isActive ? -ACTIVE_W / 2 : ACTIVE_W / 2 + GAP + (idx - 1) * (CARD_W + GAP);
      const defaultY = isActive ? -ACTIVE_H / 2 : -CARD_H / 2;
      const saved = savedPositions.byNodeId[id];
      return {
        id,
        type: 'chip',
        position: saved ?? { x: defaultX, y: defaultY },
        data: { chipId, isActive },
        dragHandle: '.lw-chip-node-header',
        width: w,
        height: h,
        style: { width: w, height: h },
        selected: isActive,
      };
    });
  }, [order, activeChipId, savedPositions]);

  const edges = useBleAirEdgesFor(nodes);

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
    applyNodeChanges(changes, nodes);
  }, [nodes]);

  // Auto fit-view when the chip count changes so a freshly added
  // chip doesn't land off-screen.
  useEffect(() => {
    const id = window.setTimeout(() => {
      fitView({ padding: 0.15, duration: 200 });
    }, 50);
    return () => window.clearTimeout(id);
  }, [order.length, fitView]);

  return (
    <div className="lw-canvas-root" style={{ top: GLOBAL_CHROME_HEIGHT }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        proOptions={{ hideAttribution: true }}
        fitView
        minZoom={0.4}
        maxZoom={1.5}
        panOnDrag
        panOnScroll={false}
        zoomOnScroll
        zoomOnPinch
        nodesConnectable={false}
        nodesFocusable={false}
        edgesFocusable={false}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={20}
          size={1}
          color="rgba(255,255,255,0.06)"
        />
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
