// Floating per-chip properties window. Opens when the user clicks a
// chip on the canvas; shows ONLY the properties that chip exposes
// (Serial / Registers / Trace / Memory / Source / YAML — i.e. the
// dev drawer + inspector panel from the legacy StudioShell). The
// chip itself stays on the canvas as a visual node.
//
// State: ChipsProvider owns `inspectorOpen` so a chip-click
// elsewhere can reopen the window after the user dismissed it with
// the X. Position + size persist in localStorage.
import { useCallback, useEffect, useRef, useState } from 'react';
import { useChips } from './ChipSession';
import { useChipInspectorContent } from './ChipContent';

const STORAGE_KEY = 'lw-inspector-window-v2';

interface PersistedLayout {
  x: number;
  y: number;
  w: number;
  h: number;
}

function load(): PersistedLayout {
  if (typeof window === 'undefined') return defaultLayout();
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return defaultLayout();
    const parsed = JSON.parse(raw) as PersistedLayout;
    return {
      x: parsed.x ?? defaultLayout().x,
      y: parsed.y ?? defaultLayout().y,
      w: parsed.w ?? defaultLayout().w,
      h: parsed.h ?? defaultLayout().h,
    };
  } catch {
    return defaultLayout();
  }
}

function defaultLayout(): PersistedLayout {
  // Default: right-docked panel that doesn't cover the left half of
  // the canvas (where chips live).
  if (typeof window === 'undefined') return { x: 600, y: 70, w: 560, h: 600 };
  const w = Math.min(560, Math.max(420, Math.round(window.innerWidth * 0.36)));
  const h = Math.min(window.innerHeight - 100, 640);
  return {
    x: Math.max(24, window.innerWidth - w - 24),
    y: 60,
    w,
    h,
  };
}

function save(p: PersistedLayout) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(p));
  } catch {
    /* quota — skip */
  }
}

export function ChipInspectorWindow() {
  const { activeChipId, sessions, inspectorOpen, setInspectorOpen } = useChips();
  const inspectorContent = useChipInspectorContent();
  const [layout, setLayout] = useState<PersistedLayout>(() => load());
  const dragRef = useRef<{
    startX: number;
    startY: number;
    origX: number;
    origY: number;
  } | null>(null);
  const resizeRef = useRef<{
    startX: number;
    startY: number;
    origW: number;
    origH: number;
  } | null>(null);

  useEffect(() => {
    save(layout);
  }, [layout]);

  const onHeaderMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if ((e.target as HTMLElement).closest('button')) return;
      dragRef.current = {
        startX: e.clientX,
        startY: e.clientY,
        origX: layout.x,
        origY: layout.y,
      };
      e.preventDefault();
    },
    [layout.x, layout.y],
  );

  const onResizeMouseDown = useCallback(
    (e: React.MouseEvent) => {
      resizeRef.current = {
        startX: e.clientX,
        startY: e.clientY,
        origW: layout.w,
        origH: layout.h,
      };
      e.preventDefault();
      e.stopPropagation();
    },
    [layout.w, layout.h],
  );

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (dragRef.current) {
        const dx = e.clientX - dragRef.current.startX;
        const dy = e.clientY - dragRef.current.startY;
        setLayout((p) => ({
          ...p,
          x: Math.max(0, Math.min(window.innerWidth - 80, dragRef.current!.origX + dx)),
          y: Math.max(44, Math.min(window.innerHeight - 80, dragRef.current!.origY + dy)),
        }));
      }
      if (resizeRef.current) {
        const dw = e.clientX - resizeRef.current.startX;
        const dh = e.clientY - resizeRef.current.startY;
        setLayout((p) => ({
          ...p,
          w: Math.max(360, resizeRef.current!.origW + dw),
          h: Math.max(240, resizeRef.current!.origH + dh),
        }));
      }
    };
    const onUp = () => {
      dragRef.current = null;
      resizeRef.current = null;
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, []);

  const session = sessions[activeChipId];
  if (!inspectorOpen || !session) return null;

  return (
    <div
      className="lw-inspector-window"
      style={{
        position: 'fixed',
        left: layout.x,
        top: layout.y,
        width: layout.w,
        height: layout.h,
        zIndex: 9999,
        background: '#0a0a0f',
        border: '1px solid rgba(232, 62, 140, 0.35)',
        borderRadius: 10,
        boxShadow: '0 24px 64px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(232, 62, 140, 0.05)',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      <div
        className="lw-inspector-window-header"
        onMouseDown={onHeaderMouseDown}
        style={{
          height: 32,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          background: 'rgba(255, 255, 255, 0.04)',
          borderBottom: '1px solid rgba(255, 255, 255, 0.06)',
          color: 'rgba(255, 255, 255, 0.85)',
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          fontSize: 11,
          letterSpacing: 0.3,
          cursor: 'move',
          userSelect: 'none',
        }}
      >
        <span style={{ color: '#e83e8c' }}>●</span>
        <span style={{ marginLeft: 8, fontWeight: 600 }}>{session.chipId}</span>
        <span style={{ marginLeft: 10, opacity: 0.5 }}>·</span>
        <span style={{ marginLeft: 10, opacity: 0.6 }}>properties</span>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={() => setInspectorOpen(false)}
          aria-label="Close inspector"
          style={{
            width: 22,
            height: 22,
            borderRadius: '50%',
            background: 'rgba(255, 255, 255, 0.05)',
            border: 'none',
            color: 'rgba(255, 255, 255, 0.7)',
            fontSize: 14,
            lineHeight: 1,
            cursor: 'pointer',
          }}
        >
          ×
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: 'auto', position: 'relative' }}>
        {inspectorContent}
      </div>
      <div
        onMouseDown={onResizeMouseDown}
        style={{
          position: 'absolute',
          right: 0,
          bottom: 0,
          width: 16,
          height: 16,
          cursor: 'nwse-resize',
          background:
            'linear-gradient(135deg, transparent 50%, rgba(255,255,255,0.18) 50%)',
        }}
        aria-label="Resize"
      />
    </div>
  );
}
