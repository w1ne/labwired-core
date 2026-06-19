// Desktop "About this lab" affordance: a quiet ⓘ button that toggles a small
// popover with the lab's name + description (authored in bundled-configs).
// Deliberately lightweight — anchored top-left under the TopChrome bar, never
// a modal, and dismissible (click the button again, click outside, or Esc) so
// it never permanently covers the canvas.

import { useEffect, useRef, useState } from 'react';
import { InfoIcon } from './Icons';

export interface LabInfoButtonProps {
  name: string;
  description: string;
  runHint?: string;
}

export function LabInfoButton({ name, description, runHint }: LabInfoButtonProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Dismiss on outside click / Esc — same lightweight pattern as other popovers.
  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    window.addEventListener('pointerdown', onPointer);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onPointer);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="absolute top-2 left-3 z-20">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label="About this lab"
        aria-expanded={open}
        title="About this lab"
        className={`flex items-center justify-center h-7 w-7 rounded-pill border border-white/[0.08] backdrop-blur transition-colors duration-micro outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
          open
            ? 'bg-accent/15 text-accent'
            : 'bg-white/[0.04] text-fg-tertiary hover:bg-white/[0.08] hover:text-fg-secondary'
        }`}
      >
        <InfoIcon size={15} />
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="About this lab"
          className="absolute top-9 left-0 w-[300px] max-w-[80vw] p-3 rounded-2xl bg-[rgba(13,14,18,0.96)] backdrop-blur border border-white/[0.08] shadow-[0_8px_24px_-12px_rgba(0,0,0,0.6)]"
        >
          <div className="text-fg-primary text-[13px] font-medium tracking-tight mb-1">{name}</div>
          <p className="text-fg-secondary text-[12px] leading-snug m-0">{description}</p>
          {runHint && (
            <p className="text-fg-tertiary text-[11.5px] leading-snug mt-2 mb-0 flex items-start gap-1.5">
              <span aria-hidden className="mt-px text-accent">▶</span>
              <span className="break-words">{runHint}</span>
            </p>
          )}
        </div>
      )}
    </div>
  );
}
