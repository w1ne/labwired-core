// Purpose-built mobile demo shell. NOT a responsive squeeze of the desktop
// playground — a different layout that surfaces just the things a phone
// visitor cares about: the device's name, what its panel is currently
// showing, and a big Run button. No wiring canvas, no inspector, no
// bottom tabs, no command palette. The desktop editor isn't usable on a
// phone anyway; pretending it is just makes the page look amateur.

import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { GlobalLogo, GlobalNav } from './components/GlobalNav';
import type { BoardConfig } from './bundled-configs';

const SSD1680_LANDSCAPE_W = 296;
const SSD1680_LANDSCAPE_H = 128;
const SSD1680_PLANE_BYTES = (SSD1680_LANDSCAPE_W * SSD1680_LANDSCAPE_H) / 8;

/** Compose the SSD1680 black + red planes into an RGB888 image (no alpha)
 *  rendered to a data URL. Mirror of @labwired/ui's internal PanelPixels
 *  but inlined here so the mobile shell stays self-contained. */
function composePanelDataUrl(planes: Uint8Array | null | undefined): string | null {
  if (!planes || planes.length < SSD1680_PLANE_BYTES * 2) return null;
  const black = planes.subarray(0, SSD1680_PLANE_BYTES);
  const red = planes.subarray(SSD1680_PLANE_BYTES, SSD1680_PLANE_BYTES * 2);
  const w = SSD1680_LANDSCAPE_W;
  const h = SSD1680_LANDSCAPE_H;
  const stride = w / 8;
  const rgba = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = y * stride + (x >>> 3);
      const bit = 7 - (x & 7);
      const blackBit = (black[i] >>> bit) & 1;
      const redBit = (red[i] >>> bit) & 1;
      const o = (y * w + x) * 4;
      // Match the desktop SSD1680 component: red dominates when its bit
      // is 0 (Waveshare/GxEPD2 convention).
      if (!redBit) {
        rgba[o] = 220; rgba[o + 1] = 30; rgba[o + 2] = 40; rgba[o + 3] = 255;
      } else if (!blackBit) {
        rgba[o] = 0; rgba[o + 1] = 0; rgba[o + 2] = 0; rgba[o + 3] = 255;
      } else {
        rgba[o] = 245; rgba[o + 1] = 245; rgba[o + 2] = 240; rgba[o + 3] = 255;
      }
    }
  }
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const img = ctx.createImageData(w, h);
  img.data.set(rgba);
  ctx.putImageData(img, 0, 0);
  return canvas.toDataURL('image/png');
}

export interface MobileDemoViewProps {
  selectedBoard: BoardConfig;
  /** Optional pre-composed display buffer from the running sim. */
  panelPlanes?: Uint8Array;
  panelGeneration?: number;
  /** Sim state */
  running: boolean;
  cycles: number;
  runtimeMs: number;
  /** Action handlers */
  onRun: () => void;
  onPause: () => void;
  onReset: () => void;
  /** True while we're between Run click and bridge/snapshot ready. */
  loading?: boolean;
  /** Toast slot (passed through from App). */
  toast?: ReactNode;
}

function formatRuntime(ms: number): string {
  if (!ms) return '00:00';
  const s = Math.floor(ms / 1000);
  const mm = Math.floor(s / 60).toString().padStart(2, '0');
  const ss = (s % 60).toString().padStart(2, '0');
  return `${mm}:${ss}`;
}

export function MobileDemoView({
  selectedBoard,
  panelPlanes,
  panelGeneration,
  running,
  cycles,
  runtimeMs,
  onRun,
  onPause,
  onReset,
  loading,
  toast,
}: MobileDemoViewProps) {
  // Re-compose only when the panel actually refreshed.
  const panelDataUrl = useMemo(() => {
    return composePanelDataUrl(panelPlanes);
  }, [panelPlanes, panelGeneration]);

  const [showNav, setShowNav] = useState(false);

  // Close the nav drawer on Escape or backdrop click.
  const navRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!showNav) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setShowNav(false);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [showNav]);

  const hasPaint = !!panelDataUrl;
  const summary = selectedBoard.summary;

  return (
    <div className="min-h-screen bg-bg-base text-fg-primary flex flex-col">
      {/* Top chrome — minimal */}
      <header className="sticky top-0 z-30 flex items-center justify-between gap-3 h-12 px-3 bg-[rgba(13,14,18,0.9)] backdrop-blur border-b border-white/[0.06]">
        <div className="flex items-center gap-2 min-w-0">
          <GlobalLogo variant="dark" />
          <span className="text-fg-tertiary shrink-0" aria-hidden>›</span>
          <span className="text-fg-secondary truncate text-[14px]">{selectedBoard.name}</span>
        </div>
        <button
          type="button"
          onClick={() => setShowNav(true)}
          aria-label="Open menu"
          className="h-9 w-9 flex items-center justify-center rounded-full bg-white/[0.05] text-fg-secondary"
        >
          <svg viewBox="0 0 16 16" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
            <path d="M2 4h12M2 8h12M2 12h12" />
          </svg>
        </button>
      </header>

      {/* Hero / device card */}
      <main className="flex-1 flex flex-col items-center px-4 py-6 gap-6">
        <div className="w-full max-w-md text-center">
          <div className="inline-flex items-center gap-1.5 h-6 px-2.5 rounded-full bg-success/10 border border-success/30 text-success text-[10.5px] font-semibold uppercase tracking-[0.1em]">
            <span aria-hidden className="w-1.5 h-1.5 rounded-full bg-success shadow-[0_0_6px_rgba(61,214,140,0.7)]" />
            Deterministic · Cycle-accurate
          </div>
          <h1 className="text-[26px] font-bold tracking-tight text-fg-primary mt-3">{selectedBoard.name}</h1>
          {summary?.description && (
            <p className="text-fg-secondary text-[13.5px] leading-snug mt-2 max-w-[32ch] mx-auto">
              {summary.description}
            </p>
          )}
        </div>

        {/* Panel preview — the star of the show */}
        <div
          className="w-full max-w-md rounded-2xl border-2 border-white/[0.06] bg-[#0d0e12] shadow-[0_8px_24px_rgba(0,0,0,0.45)] overflow-hidden"
          aria-label="E-paper display preview"
        >
          {/* Aspect-ratio-preserved frame around the SSD1680 296×128 native res */}
          <div className="relative bg-[#16181f] p-3">
            <div
              className="w-full rounded-md overflow-hidden border border-white/[0.08]"
              style={{ aspectRatio: '296 / 128', background: '#e6e1d3' }}
            >
              {hasPaint ? (
                <img
                  src={panelDataUrl}
                  alt="E-paper panel state"
                  className="block w-full h-full"
                  style={{ imageRendering: 'pixelated' }}
                />
              ) : (
                <div className="w-full h-full flex items-center justify-center text-[#999] text-[12px]">
                  {running ? 'Booting firmware…' : 'Tap Run to paint the panel'}
                </div>
              )}
            </div>
          </div>
          <div className="px-4 py-3 border-t border-white/[0.06] flex items-center justify-between gap-3 text-[11.5px] text-fg-tertiary font-mono">
            <span className="flex items-center gap-1.5">
              <span
                aria-hidden
                className={`w-1.5 h-1.5 rounded-full ${
                  running ? 'bg-magenta animate-pulse' : hasPaint ? 'bg-success' : 'bg-fg-tertiary'
                }`}
              />
              <span className="text-fg-secondary">
                {running ? 'Running' : hasPaint ? 'Painted' : 'Idle'}
              </span>
            </span>
            <span>{formatRuntime(runtimeMs)}</span>
            {cycles > 0 && (
              <span title="Cycles executed">{cycles.toLocaleString()} cy</span>
            )}
          </div>
        </div>

        {/* Big sticky run button */}
        <div className="w-full max-w-md flex flex-col gap-3">
          <button
            type="button"
            onClick={running ? onPause : onRun}
            disabled={loading}
            className={`h-14 w-full rounded-full font-bold text-[16px] flex items-center justify-center gap-2 transition-all duration-150 active:scale-[0.98] disabled:opacity-60 ${
              running
                ? 'bg-magenta text-bg-base'
                : 'bg-accent text-bg-base shadow-[0_10px_28px_-10px_rgba(91,157,255,0.6)]'
            }`}
          >
            <span aria-hidden className="text-[18px]">{running ? '⏸' : '▶'}</span>
            <span>{loading ? 'Loading…' : running ? 'Pause' : 'Run'}</span>
          </button>
          {hasPaint && (
            <button
              type="button"
              onClick={onReset}
              className="h-11 w-full rounded-full font-medium text-[14px] text-fg-secondary bg-white/[0.05] border border-white/[0.08]"
            >
              ↻ Reset demo
            </button>
          )}
        </div>

        <p className="text-fg-tertiary text-[12px] text-center max-w-[40ch] leading-snug px-4">
          The same firmware ELF that flashes to your physical board runs here in your browser.
          Cycle-accurate. Deterministic. <a className="text-accent" href="https://labwired.com/ci.html">Use in CI →</a>
        </p>
      </main>

      {/* Slide-in nav drawer */}
      {showNav && (
        <div
          className="fixed inset-0 z-50 bg-black/60 backdrop-blur"
          onClick={(e) => { if (e.target === e.currentTarget) setShowNav(false); }}
        >
          <div
            ref={navRef}
            className="absolute right-0 top-0 bottom-0 w-72 max-w-[80vw] bg-bg-base border-l border-white/[0.06] p-5 flex flex-col gap-4"
          >
            <div className="flex items-center justify-between">
              <GlobalLogo variant="dark" />
              <button
                type="button"
                onClick={() => setShowNav(false)}
                aria-label="Close menu"
                className="h-8 w-8 rounded-full bg-white/[0.05] text-fg-secondary flex items-center justify-center"
              >
                ✕
              </button>
            </div>
            <nav className="flex flex-col gap-1 mt-2">
              <GlobalNav variant="dark" />
            </nav>
            <div className="mt-auto text-[11px] text-fg-tertiary leading-snug">
              Need the full editor?<br />
              Open this page on a laptop for the canvas, wiring, code editor, and CPU inspector.
            </div>
          </div>
        </div>
      )}

      {/* Toast slot */}
      {toast && (
        <div className="fixed bottom-4 inset-x-4 z-40">{toast}</div>
      )}
    </div>
  );
}
