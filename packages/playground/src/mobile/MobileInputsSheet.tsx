// Touch-friendly bottom sheet for the mobile run view. Surfaces the two things
// you can't do by tapping the canvas directly: drag continuous inputs
// (ultrasonic hand-distance, thermistor temperature, ADC potentiometer/LDR)
// and read the serial monitor. Buttons are pressed on the canvas itself.
//
// The controls drive the SAME bridge handlers as the desktop inspector
// (handleDistanceChange / setNtcTemperature / handleAnalogChange) — the Rust
// device stays the single source of truth; this is just a faithful control.

import { useLayoutEffect, useRef, useState } from 'react';
import { COMPONENT_REGISTRY, type ComponentState, type Diagram } from '@labwired/ui';

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
}

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
}: MobileInputsSheetProps) {
  const ultrasonicParts = diagram.parts.filter((p) => p.type === 'ultrasonic');
  const thermistorParts = diagram.parts.filter((p) => p.type === 'ntc-thermistor');
  const adcParts = diagram.parts.filter(
    (p) => COMPONENT_REGISTRY.get(p.type)?.boardIoKind === 'adc_input',
  );
  const hasInputs = ultrasonicParts.length > 0 || thermistorParts.length > 0 || adcParts.length > 0;

  // Default to the tab that actually has content.
  const [tab, setTab] = useState<'inputs' | 'serial'>(hasInputs ? 'inputs' : 'serial');
  const [open, setOpen] = useState(true);

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
    <div className="shrink-0 bg-[rgba(13,14,18,0.96)] backdrop-blur border-t border-white/[0.08]">
      {/* Tab bar / collapse handle */}
      <div className="flex items-center gap-1 px-2 h-11">
        <button
          type="button"
          onClick={() => { setTab('inputs'); setOpen(true); }}
          className={`h-9 px-3 rounded-lg text-[13px] font-semibold ${
            tab === 'inputs' && open ? 'bg-white/[0.1] text-fg-primary' : 'text-fg-tertiary'
          }`}
        >
          Inputs
        </button>
        <button
          type="button"
          onClick={() => { setTab('serial'); setOpen(true); }}
          className={`h-9 px-3 rounded-lg text-[13px] font-semibold ${
            tab === 'serial' && open ? 'bg-white/[0.1] text-fg-primary' : 'text-fg-tertiary'
          }`}
        >
          Serial
        </button>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          aria-label={open ? 'Collapse panel' : 'Expand panel'}
          className="ml-auto h-9 w-9 flex items-center justify-center rounded-lg text-fg-tertiary"
        >
          {open ? '▾' : '▴'}
        </button>
      </div>

      {open && (
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
