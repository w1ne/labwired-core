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

  const startDrag = (e: React.PointerEvent) => {
    onFocus?.();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    dragRef.current = { dx: e.clientX - pos.x, dy: e.clientY - pos.y };
    const onMove = (ev: PointerEvent) => {
      if (!dragRef.current) return;
      // Clamp so the title bar can't be dragged fully off-screen.
      const x = Math.max(8 - size.w + 80, Math.min(window.innerWidth - 80, ev.clientX - dragRef.current.dx));
      const y = Math.max(8, Math.min(window.innerHeight - 40, ev.clientY - dragRef.current.dy));
      setPos({ x, y });
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  };

  const startResize = (e: React.PointerEvent) => {
    e.stopPropagation();
    onFocus?.();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const start = { mx: e.clientX, my: e.clientY, w: size.w, h: size.h };
    const onMove = (ev: PointerEvent) => {
      setSize({
        w: Math.max(240, start.w + (ev.clientX - start.mx)),
        h: Math.max(140, start.h + (ev.clientY - start.my)),
      });
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  };

  return (
    <div
      className="fixed flex flex-col overflow-hidden rounded-lg border border-border bg-bg-surface shadow-2xl"
      style={{ left: pos.x, top: pos.y, width: size.w, height: size.h, zIndex }}
      onPointerDown={onFocus}
      role="dialog"
      aria-label="Chip serial window"
    >
      <div
        className="flex shrink-0 cursor-move select-none items-center gap-2 border-b border-border bg-bg-elevated px-2.5 py-1.5"
        onPointerDown={startDrag}
        style={{ touchAction: 'none' }}
        data-chip-window-drag-handle
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
        onPointerDown={startResize}
        className="absolute bottom-0 right-0 h-3.5 w-3.5 cursor-nwse-resize"
        style={{
          background:
            'linear-gradient(135deg, transparent 0 50%, rgba(255,255,255,0.35) 50% 60%, transparent 60% 70%, rgba(255,255,255,0.35) 70% 80%, transparent 80%)',
          touchAction: 'none',
        }}
        aria-label="Resize window"
      />
    </div>
  );
}
