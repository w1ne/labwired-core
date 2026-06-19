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
import type { TraceEntry } from '@labwired/ui';
import type { BoardConfig } from '../bundled-configs';
import { MobileInputsSheet } from './MobileInputsSheet';
import { Toast } from '../studio/Toast';
import { resolveUiFeatures } from '../uiFeatures';

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
  /** Transient status/error message (e.g. "Cannot run: …"). Auto-dismisses. */
  toast?: string | null;
  onDismissToast?: () => void;
  /** MCU parts in the diagram, for the multi-chip switcher (shown only when 2+). */
  chips?: { id: string; name: string }[];
  /** The part id of the foreground chip (the one whose sim/serial is shown). */
  foregroundChipId?: string;
  /** Bring a chip to the foreground (selects it on the canvas). */
  onSelectChip?: (id: string) => void;
  /** Live CPU state for the foreground chip (the drawer's "CPU" tab). */
  registers?: Map<string, number>;
  traceEntries?: TraceEntry[];
  stackMemory?: Uint8Array;
  stackBase?: number;
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
  onDismissToast,
  chips,
  foregroundChipId,
  onSelectChip,
  registers,
  traceEntries,
  stackMemory,
  stackBase,
}: MobileRunViewProps) {
  const features = resolveUiFeatures();
  const [showNav, setShowNav] = useState(false);
  // One-time gesture coach mark: show briefly on entry, then fade out so it
  // doesn't sit on the canvas forever as permanent chrome.
  const [showGestureHint, setShowGestureHint] = useState(true);
  useEffect(() => {
    const t = window.setTimeout(() => setShowGestureHint(false), 4500);
    return () => window.clearTimeout(t);
  }, []);
  useEffect(() => {
    if (!showNav) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setShowNav(false); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [showNav]);

  return (
    <div className="fixed inset-0 flex flex-col bg-bg-base text-fg-primary overflow-hidden">
      <header
        style={{ paddingTop: 'env(safe-area-inset-top)' }}
        className="shrink-0 flex items-center justify-between gap-3 min-h-[52px] px-4 bg-[rgba(13,14,18,0.92)] backdrop-blur border-b border-white/[0.06]"
      >
        <div className="flex items-center gap-2.5 min-w-0">
          <GlobalLogo variant="dark" />
          <span className="text-fg-tertiary/60 shrink-0 text-[13px]" aria-hidden>/</span>
          <span className="text-fg-primary truncate text-[14px] font-medium">{selectedBoard.name}</span>
        </div>
        {features.menu && (
          <button
            type="button"
            onClick={() => setShowNav(true)}
            aria-label="Open menu"
            className="h-9 w-9 flex items-center justify-center rounded-full bg-white/[0.06] text-fg-secondary active:bg-white/[0.12] transition-colors shrink-0"
          >
            <svg viewBox="0 0 16 16" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
              <path d="M2 4h12M2 8h12M2 12h12" />
            </svg>
          </button>
        )}
      </header>

      {/* Multi-MCU switcher: pick which chip is in the foreground (its sim,
          serial, BLE and inputs are what the rest of the view shows). Only
          rendered for multi-chip labs so single-MCU views stay clean. */}
      {chips && chips.length > 1 && (
        <div className="shrink-0 flex items-center gap-1.5 px-3 py-1.5 overflow-x-auto bg-[rgba(13,14,18,0.92)] border-b border-white/[0.06]">
          <span className="text-fg-tertiary text-[11px] shrink-0 mr-0.5">Chip</span>
          {chips.map((c) => {
            const active = c.id === foregroundChipId;
            return (
              <button
                key={c.id}
                type="button"
                onClick={() => onSelectChip?.(c.id)}
                aria-pressed={active}
                className={`h-7 px-3 rounded-full text-[12px] font-medium shrink-0 transition-colors ${
                  active ? 'bg-accent/15 text-accent' : 'text-fg-tertiary bg-white/[0.04] active:bg-white/[0.08]'
                }`}
              >
                {c.name}
              </button>
            );
          })}
        </div>
      )}

      {/* The real canvas, in touch run mode. flex-1 + min-h-0 so it fills the
          space between header and the controls without pushing them off-screen. */}
      <div className="flex-1 min-h-0 relative">
        <EditorCanvas
          state={editorState}
          interactionMode="run"
          fitToContent
          showZoomControls
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
        <div
          className={`pointer-events-none absolute top-3 inset-x-0 flex justify-center transition-opacity duration-700 ${
            showGestureHint ? 'opacity-100' : 'opacity-0'
          }`}
        >
          <span className="px-3 py-1 rounded-full bg-black/35 backdrop-blur-sm text-fg-tertiary text-[11px] tracking-tight">
            Pinch · drag · tap
          </span>
        </div>
      </div>

      {/* Transport controls (Run / Pause / Reset). Borderless so it reads as one
          module with the drawer below it rather than a stack of bordered bands. */}
      <div className="shrink-0 flex justify-center px-3 pt-2 pb-1 bg-[rgba(13,14,18,0.92)]">
        {simControls}
      </div>

      <MobileInputsSheet
        labName={selectedBoard.name}
        labDescription={selectedBoard.description}
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
        registers={registers}
        traceEntries={traceEntries}
        stackMemory={stackMemory}
        stackBase={stackBase}
      />

      {features.menu && showNav && (
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
            {/* Hide "Tools" here — its instruments (BLE / Logic / IO-Link) are
                already surfaced as drawer tabs on mobile, so the link would be a
                redundant no-op (it toggles the desktop-only tools panel). */}
            <GlobalNav variant="dark" orientation="vertical" exclude={['tools']} className="mt-1" />
            <div className="mt-auto text-[11px] text-fg-tertiary leading-snug">
              Need the full editor?<br />
              Open this page on a laptop for wiring, the code editor, and the CPU inspector.
            </div>
          </div>
        </div>
      )}

      <Toast message={toast ?? null} onDismiss={() => onDismissToast?.()} />
    </div>
  );
}
