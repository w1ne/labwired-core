// A draggable, resizable, closable floating window — one per selected component,
// so several inspectors can be arranged freely. Moved from the playground into
// @labwired/ui (inline-styled, self-contained — no Tailwind/theme-token coupling)
// so any consumer (proto.cat) reuses the SAME window chrome.
import { useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';

export interface ChipWindowProps {
  title: ReactNode;
  initial: { x: number; y: number };
  width?: number;
  height?: number;
  zIndex?: number;
  onClose: () => void;
  onFocus?: () => void;
  children: ReactNode;
}

// Themeable via CSS vars — a consumer (proto.cat) overrides --lw-* to its scheme.
const C = {
  surface: 'var(--lw-bg-surface, #13151B)',
  elevated: 'var(--lw-bg-elevated, #1A1D26)',
  border: 'var(--lw-border, #262A33)',
  fgTertiary: 'var(--lw-fg-tertiary, #5A6178)',
  fgPrimary: 'var(--lw-fg-primary, #F2F4F9)',
};

export function ChipWindow({
  title,
  initial,
  width = 440,
  height = 280,
  zIndex = 60,
  onClose,
  onFocus,
  children,
}: ChipWindowProps) {
  const [pos, setPos] = useState(initial);
  const [size, setSize] = useState({ w: width, h: height });
  const dragRef = useRef<{ dx: number; dy: number } | null>(null);

  const startDrag = (e: React.MouseEvent) => {
    onFocus?.();
    dragRef.current = { dx: e.clientX - pos.x, dy: e.clientY - pos.y };
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      const x = Math.max(8 - size.w + 80, Math.min(window.innerWidth - 80, ev.clientX - dragRef.current.dx));
      const y = Math.max(8, Math.min(window.innerHeight - 40, ev.clientY - dragRef.current.dy));
      setPos({ x, y });
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  };

  const startResize = (e: React.MouseEvent) => {
    e.stopPropagation();
    onFocus?.();
    const start = { mx: e.clientX, my: e.clientY, w: size.w, h: size.h };
    const onMove = (ev: MouseEvent) =>
      setSize({ w: Math.max(240, start.w + (ev.clientX - start.mx)), h: Math.max(160, start.h + (ev.clientY - start.my)) });
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  };

  const shell: CSSProperties = {
    position: 'fixed', left: pos.x, top: pos.y, width: size.w, height: size.h, zIndex,
    display: 'flex', flexDirection: 'column', overflow: 'hidden', borderRadius: 8,
    border: `1px solid ${C.border}`, background: C.surface, boxShadow: '0 20px 50px rgba(0,0,0,0.5)',
  };
  const bar: CSSProperties = {
    display: 'flex', flexShrink: 0, alignItems: 'center', gap: 8, cursor: 'move', userSelect: 'none',
    borderBottom: `1px solid ${C.border}`, background: C.elevated, padding: '6px 10px',
  };

  return (
    <div style={shell} onMouseDown={onFocus} role="dialog" aria-label="Inspector window">
      <div style={bar} onMouseDown={startDrag}>
        <div style={{ display: 'flex', minWidth: 0, flex: 1, alignItems: 'center', gap: 8, fontSize: 12, color: C.fgPrimary }}>
          {title}
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close window"
          title="Close"
          style={{ width: 20, height: 20, borderRadius: 4, border: 'none', background: 'transparent', color: C.fgTertiary, cursor: 'pointer', fontSize: 15, lineHeight: 1 }}
        >
          ×
        </button>
      </div>
      <div style={{ minHeight: 0, flex: 1 }}>{children}</div>
      <div
        onMouseDown={startResize}
        aria-label="Resize window"
        style={{
          position: 'absolute', bottom: 0, right: 0, width: 14, height: 14, cursor: 'nwse-resize',
          background:
            'linear-gradient(135deg, transparent 0 50%, rgba(255,255,255,0.35) 50% 60%, transparent 60% 70%, rgba(255,255,255,0.35) 70% 80%, transparent 80%)',
        }}
      />
    </div>
  );
}
