// Touch-friendly bottom sheet for the mobile run view. Surfaces the two things
// you can't do by tapping the canvas directly: drag continuous inputs
// (ultrasonic hand-distance, thermistor temperature, ADC potentiometer/LDR)
// and read the serial monitor. Buttons are pressed on the canvas itself.
//
// The controls drive the SAME bridge handlers as the desktop inspector
// (handleDistanceChange / setNtcTemperature / handleAnalogChange) — the Rust
// device stays the single source of truth; this is just a faithful control.

import { useLayoutEffect, useRef, useState } from 'react';
import {
  COMPONENT_REGISTRY,
  type ComponentState,
  type Diagram,
  type SimulatorBridge,
} from '@labwired/ui';
import { BleAnalyzer } from '../instruments/BleAnalyzer';
import { IoLinkAnalyzer } from '../instruments/IoLinkAnalyzer';
import { LogicAnalyzerPanel } from '../instruments/LogicAnalyzerPanel';

export interface MobileInputsSheetProps {
  diagram: Diagram;
  /** Live board-IO state keyed by part id (for ADC current values). */
  boardIoStates: Record<string, ComponentState>;
  uartOutput: string;
  /** Update a part attribute (e.g. ultrasonic `distance`); synced to the bridge
   *  by App's attribute effect. */
  onUpdateAttr: (partId: string, attrs: Record<string, string>) => void;
  /** NTC thermistor temperatures keyed by part id + setter. */
  ntcTemperatures: Record<string, number>;
  onNtcChange: (partId: string, tempC: number) => void;
  /** ADC value setter (0–4095), keyed by part id (matches the board_io binding). */
  onAnalogChange: (partId: string, value: number) => void;
  /** Live bridge for the instrument tabs (BLE / logic / IO-Link). */
  bridge: SimulatorBridge | null;
  /** Whether the sim is running — drives instrument poll cadence. */
  running: boolean;
  /** Update a part attribute used by the logic-analyzer decoder selector. */
  onPartAttrChange: (partId: string, attrs: Record<string, string>) => void;
}

type Tab = 'inputs' | 'serial' | 'ble' | 'logic' | 'iolink';

function partLabel(attrs: Record<string, unknown> | undefined, fallback: string): string {
  const label = attrs?.label;
  return typeof label === 'string' && label.length > 0 ? label : fallback;
}

export function MobileInputsSheet({
  diagram,
  boardIoStates,
  uartOutput,
  onUpdateAttr,
  ntcTemperatures,
  onNtcChange,
  onAnalogChange,
  bridge,
  running,
  onPartAttrChange,
}: MobileInputsSheetProps) {
  const ultrasonicParts = diagram.parts.filter((p) => p.type === 'ultrasonic');
  const thermistorParts = diagram.parts.filter((p) => p.type === 'ntc-thermistor');
  const adcParts = diagram.parts.filter(
    (p) => COMPONENT_REGISTRY.get(p.type)?.boardIoKind === 'adc_input',
  );
  const hasInputs = ultrasonicParts.length > 0 || thermistorParts.length > 0 || adcParts.length > 0;

  // Instrument tabs. BLE + Serial are always available (parity with the desktop
  // always-on Air Tracer); Logic and IO-Link appear only when the diagram
  // actually contains the matching part, so we never show an empty analyzer.
  const logicAnalyzerPart = diagram.parts.find((p) => p.type === 'logic-analyzer');
  const hasIoLink = diagram.parts.some((p) => p.type === 'iolink-master');

  const tabs: { id: Tab; label: string }[] = [
    ...(hasInputs ? [{ id: 'inputs' as Tab, label: 'Inputs' }] : []),
    { id: 'serial', label: 'Serial' },
    { id: 'ble', label: 'BLE' },
    ...(logicAnalyzerPart ? [{ id: 'logic' as Tab, label: 'Logic' }] : []),
    ...(hasIoLink ? [{ id: 'iolink' as Tab, label: 'IO-Link' }] : []),
  ];

  // Default to the tab that actually has content.
  const [tab, setTab] = useState<Tab>(hasInputs ? 'inputs' : 'serial');
  // Drawer: collapsed by default so the canvas owns the screen. Only the slim
  // tab bar shows until the user taps a tab (or the expand chevron). Tapping the
  // already-open tab collapses it again — a one-tap peek/dismiss.
  const [open, setOpen] = useState(false);
  const selectTab = (id: Tab) => {
    if (tab === id && open) setOpen(false);
    else { setTab(id); setOpen(true); }
  };
  // Instrument tabs need real vertical space (their panels are h-full); the
  // inputs/serial tabs stay compact. A taller body kicks in for tool tabs.
  const isTool = tab === 'ble' || tab === 'logic' || tab === 'iolink';

  const serialRef = useRef<HTMLPreElement | null>(null);
  useLayoutEffect(() => {
    if (tab !== 'serial' || !open || !serialRef.current) return;
    serialRef.current.scrollTop = serialRef.current.scrollHeight;
  }, [uartOutput, tab, open]);

  const Slider = ({
    label,
    value,
    display,
    min,
    max,
    step,
    onChange,
  }: {
    label: string;
    value: number;
    display: string;
    min: number;
    max: number;
    step: number;
    onChange: (v: number) => void;
  }) => (
    <label className="block">
      <div className="flex items-center justify-between text-fg-tertiary text-[11px] font-mono mb-1">
        <span className="truncate">{label}</span>
        <span className="text-fg-primary shrink-0 ml-2">{display}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        // Tall track = easy thumb grab on touch.
        className="w-full h-8 accent-accent"
        style={{ touchAction: 'none' }}
      />
    </label>
  );

  return (
    <div
      style={{ paddingBottom: 'env(safe-area-inset-bottom)' }}
      className="shrink-0 bg-[rgba(13,14,18,0.96)] backdrop-blur border-t border-white/[0.08] rounded-t-2xl shadow-[0_-8px_24px_-12px_rgba(0,0,0,0.6)]"
    >
      {/* Grab handle — the canonical bottom-sheet affordance; tap to toggle. */}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label={open ? 'Collapse panel' : 'Expand panel'}
        className="w-full flex justify-center pt-2 pb-1"
      >
        <span className="h-1 w-9 rounded-full bg-white/20" aria-hidden />
      </button>
      {/* Tab bar. Horizontally scrollable so extra tool tabs (BLE / Logic /
          IO-Link) never overflow on a narrow phone. */}
      <div className="flex items-center gap-1.5 px-2.5 pb-1.5 overflow-x-auto">
        {tabs.map((t) => {
          const active = tab === t.id && open;
          return (
            <button
              key={t.id}
              type="button"
              onClick={() => selectTab(t.id)}
              className={`h-8 px-3.5 rounded-full text-[13px] font-medium shrink-0 transition-colors ${
                active
                  ? 'bg-accent/15 text-accent'
                  : 'text-fg-tertiary active:bg-white/[0.06]'
              }`}
            >
              {t.label}
            </button>
          );
        })}
      </div>

      {open && isTool && (
        // Instrument panels are `h-full` flex columns — give them a fixed,
        // generous height so their internal scroll/waveform areas size right.
        <div className="border-t border-white/[0.06]" style={{ height: '58vh' }}>
          {tab === 'ble' && <BleAnalyzer bridge={bridge} running={running} />}
          {tab === 'logic' && logicAnalyzerPart && (
            // Waveforms can be wider than the phone — let them scroll sideways.
            <div className="h-full overflow-x-auto">
              <LogicAnalyzerPanel
                diagram={diagram}
                analyzerId={logicAnalyzerPart.id}
                bridge={bridge}
                running={running}
                decoder={logicAnalyzerPart.attrs.decoder ?? 'raw'}
                onDecoderChange={(decoder) => onPartAttrChange(logicAnalyzerPart.id, { decoder })}
              />
            </div>
          )}
          {tab === 'iolink' && <IoLinkAnalyzer bridge={bridge} running={running} />}
        </div>
      )}

      {open && !isTool && (
        <div className="px-4 pb-4" style={{ maxHeight: '40vh', overflowY: 'auto' }}>
          {tab === 'inputs' && (
            <div className="flex flex-col gap-4 pt-1">
              {!hasInputs && (
                <p className="text-fg-tertiary text-[12.5px] leading-snug py-3">
                  This lab has no adjustable inputs. Tap buttons directly on the canvas;
                  outputs (LEDs, displays) react live as the firmware runs.
                </p>
              )}

              {ultrasonicParts.map((p) => {
                const cm = Number.parseFloat(p.attrs.distance ?? '100');
                const v = Number.isFinite(cm) ? cm : 100;
                return (
                  <Slider
                    key={p.id}
                    label={partLabel(p.attrs, 'HC-SR04 hand distance')}
                    value={v}
                    display={`${v.toFixed(0)} cm`}
                    min={1}
                    max={200}
                    step={1}
                    onChange={(nv) => onUpdateAttr(p.id, { distance: String(nv) })}
                  />
                );
              })}

              {thermistorParts.map((p) => {
                const t = ntcTemperatures[p.id] ?? 25.0;
                return (
                  <Slider
                    key={p.id}
                    label={partLabel(p.attrs, 'NTC thermistor')}
                    value={t}
                    display={`${t.toFixed(1)} °C`}
                    min={-20}
                    max={120}
                    step={0.5}
                    onChange={(v) => onNtcChange(p.id, v)}
                  />
                );
              })}

              {adcParts.map((p) => {
                const v = boardIoStates[p.id]?.analogValue ?? 0;
                return (
                  <Slider
                    key={p.id}
                    label={partLabel(p.attrs, p.id)}
                    value={v}
                    display={`${v} / 4095`}
                    min={0}
                    max={4095}
                    step={1}
                    onChange={(nv) => onAnalogChange(p.id, nv)}
                  />
                );
              })}
            </div>
          )}

          {tab === 'serial' && (
            <pre
              ref={serialRef}
              className="m-0 mt-1 px-3 py-2 text-[11.5px] font-mono text-fg-primary bg-[#0a0b10] rounded-lg overflow-auto whitespace-pre-wrap break-all"
              style={{ minHeight: '8rem', maxHeight: '34vh' }}
            >
              {uartOutput && uartOutput.length > 0
                ? uartOutput
                : 'Run the simulation — serial output streams here.'}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
