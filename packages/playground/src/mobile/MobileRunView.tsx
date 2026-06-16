// Mobile run/interact shell. Unlike the old card-stack mobile view, this shows
// the REAL editor canvas (same SVG component as desktop) in run mode: one-finger
// pan, two-finger pinch zoom, tap a button to press it. Below it sit the live
// transport controls and an inputs/serial bottom sheet. No authoring on phone —
// the canvas is read-only except for pressing interactive parts.

import { useEffect, useState, type ReactNode } from 'react';
import {
  EditorCanvas,
  type EditorState,
  type ComponentState,
  type SimulationState,
  type SimulatorBridge,
} from '@labwired/ui';
import { GlobalLogo, GlobalNav } from '../components/GlobalNav';
import type { BoardConfig } from '../bundled-configs';
import { MobileInputsSheet } from './MobileInputsSheet';

export interface MobileRunViewProps {
  selectedBoard: BoardConfig;
  editorState: EditorState;
  boardIoStates: Record<string, ComponentState>;
  displayBuffers: SimulationState['displayBuffers'];
  uartOutput: string;
  /** Press/release a board button (true on down, false on up). */
  onButtonToggle: (partId: string, active: boolean) => void;
  /** Set an ADC value (0–4095) keyed by part id. */
  onAnalogChange: (partId: string, value: number) => void;
  /** Update a part attribute (e.g. ultrasonic `distance`). */
  onUpdateAttr: (partId: string, attrs: Record<string, string>) => void;
  /** NTC thermistor controls. */
  ntcTemperatures: Record<string, number>;
  onNtcChange: (partId: string, tempC: number) => void;
  /** Run/Pause/Reset transport node (reused from the desktop SimDock). */
  simControls: ReactNode;
  /** Open the saved-projects modal from the menu drawer. */
  onOpenProjects: () => void;
  /** Live bridge for the instrument tabs (BLE / logic / IO-Link). */
  bridge: SimulatorBridge | null;
  /** Whether the sim is running — drives instrument poll cadence. */
  running: boolean;
  /** Update a part attribute (logic-analyzer decoder selector). */
  onPartAttrChange: (partId: string, attrs: Record<string, string>) => void;
  toast?: ReactNode;
}

const noop = () => {};

export function MobileRunView({
  selectedBoard,
  editorState,
  boardIoStates,
  displayBuffers,
  uartOutput,
  onButtonToggle,
  onAnalogChange,
  onUpdateAttr,
  ntcTemperatures,
  onNtcChange,
  simControls,
  onOpenProjects,
  bridge,
  running,
  onPartAttrChange,
  toast,
}: MobileRunViewProps) {
  const [showNav, setShowNav] = useState(false);
  useEffect(() => {
    if (!showNav) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setShowNav(false); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [showNav]);

  return (
    <div className="fixed inset-0 flex flex-col bg-bg-base text-fg-primary overflow-hidden">
      <header className="shrink-0 flex items-center justify-between gap-3 h-12 px-3 bg-[rgba(13,14,18,0.9)] backdrop-blur border-b border-white/[0.06]">
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

      {/* The real canvas, in touch run mode. flex-1 + min-h-0 so it fills the
          space between header and the controls without pushing them off-screen. */}
      <div className="flex-1 min-h-0 relative">
        <EditorCanvas
          state={editorState}
          interactionMode="run"
          fitToContent
          boardIoStates={boardIoStates}
          displayBuffers={displayBuffers}
          onButtonToggle={onButtonToggle}
          onAnalogChange={onAnalogChange}
          // Authoring callbacks are gated off in run mode; supply no-ops.
          onMovePart={noop}
          onSelect={noop}
          onStartWire={noop}
          onCompleteWire={noop}
          onCancelWire={noop}
          onDeleteWire={noop}
        />
        <div className="pointer-events-none absolute top-2 inset-x-0 flex justify-center">
          <span className="pointer-events-none px-2.5 py-1 rounded-full bg-black/45 text-fg-tertiary text-[10.5px] font-mono">
            Pinch to zoom · drag to pan · tap buttons
          </span>
        </div>
      </div>

      {/* Transport controls (Run / Pause / Reset) — reused desktop SimDock. */}
      <div className="shrink-0 flex justify-center px-3 py-2 bg-[rgba(13,14,18,0.92)] border-t border-white/[0.06]">
        {simControls}
      </div>

      <MobileInputsSheet
        diagram={editorState.diagram}
        boardIoStates={boardIoStates}
        uartOutput={uartOutput}
        onUpdateAttr={onUpdateAttr}
        ntcTemperatures={ntcTemperatures}
        onNtcChange={onNtcChange}
        onAnalogChange={onAnalogChange}
        bridge={bridge}
        running={running}
        onPartAttrChange={onPartAttrChange}
      />

      {showNav && (
        <div
          className="fixed inset-0 z-50 bg-black/60 backdrop-blur"
          onClick={(e) => { if (e.target === e.currentTarget) setShowNav(false); }}
        >
          <div className="absolute right-0 top-0 bottom-0 w-72 max-w-[80vw] bg-bg-base border-l border-white/[0.06] p-5 flex flex-col gap-4">
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
            <button
              type="button"
              onClick={() => { setShowNav(false); onOpenProjects(); }}
              className="h-10 rounded-lg bg-white/[0.06] text-fg-primary text-[14px] font-medium text-left px-3"
            >
              My projects
            </button>
            <GlobalNav variant="dark" orientation="vertical" className="mt-1" />
            <div className="mt-auto text-[11px] text-fg-tertiary leading-snug">
              Need the full editor?<br />
              Open this page on a laptop for wiring, the code editor, and the CPU inspector.
            </div>
          </div>
        </div>
      )}

      {toast && <div className="fixed bottom-4 inset-x-4 z-40">{toast}</div>}
    </div>
  );
}
