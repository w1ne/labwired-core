// A draggable, closable floating window — one per chip, so several chips'
// serial monitors can be arranged freely instead of stacked in the drawer.
// Drag by the title bar; close with ×. Position is local to the window.
import { useRef, useState } from 'react';
import type { ReactNode } from 'react';

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

export function ChipWindow({
  title,
  initial,
  width = 440,
  height = 260,
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
      // Clamp so the title bar can't be dragged fully off-screen.
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
    const onMove = (ev: MouseEvent) => {
      setSize({
        w: Math.max(240, start.w + (ev.clientX - start.mx)),
        h: Math.max(140, start.h + (ev.clientY - start.my)),
      });
    };
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  };

  return (
    <div
      className="fixed flex flex-col overflow-hidden rounded-lg border border-border bg-bg-surface shadow-2xl"
      style={{ left: pos.x, top: pos.y, width: size.w, height: size.h, zIndex }}
      onMouseDown={onFocus}
      role="dialog"
      aria-label="Chip serial window"
    >
      <div
        className="flex shrink-0 cursor-move select-none items-center gap-2 border-b border-border bg-bg-elevated px-2.5 py-1.5"
        onMouseDown={startDrag}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">{title}</div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close window"
          title="Close"
          className="flex h-5 w-5 items-center justify-center rounded text-fg-tertiary hover:bg-bg-surface hover:text-fg-primary"
        >
          ×
        </button>
      </div>
      <div className="min-h-0 flex-1">{children}</div>
      {/* Resize handle (bottom-right corner). */}
      <div
        onMouseDown={startResize}
        className="absolute bottom-0 right-0 h-3.5 w-3.5 cursor-nwse-resize"
        style={{
          background:
            'linear-gradient(135deg, transparent 0 50%, rgba(255,255,255,0.35) 50% 60%, transparent 60% 70%, rgba(255,255,255,0.35) 70% 80%, transparent 80%)',
        }}
        aria-label="Resize window"
      />
    </div>
  );
}
