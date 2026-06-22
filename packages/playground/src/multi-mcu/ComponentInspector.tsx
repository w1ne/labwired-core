import type { ReactNode } from 'react';

export interface AttrField {
  key: string;
  label: string;
  type: 'text' | 'select' | 'color' | 'range' | 'textarea';
  options?: string[];
  min?: number;
  max?: number;
  step?: number;
  defaultValue?: string;
}

export interface ComponentLiveState {
  active?: boolean;
  analogValue?: number;
}

export interface ComponentInspectorProps {
  partType: string;
  partId: string;
  attrs: Record<string, string>;
  fields: AttrField[];
  live?: ComponentLiveState;
  onChange: (key: string, value: string) => void;
  /** Live simulator-backed controls that belong in the component properties body. */
  runtimeControl?: ReactNode;
  /** Standard part actions (Rotate/Size/Delete) — rendered at the bottom. */
  actions?: ReactNode;
}

const inputCls =
  'rounded-md border border-border bg-bg-elevated px-2 py-1 text-sm text-fg-primary outline-none focus:border-accent';
const labelCls = 'text-[11px] uppercase tracking-wide text-fg-tertiary';

export function ComponentInspector({
  partType,
  partId,
  attrs,
  fields,
  live,
  onChange,
  runtimeControl,
  actions,
}: ComponentInspectorProps) {
  const hasLive = live && (live.active !== undefined || live.analogValue !== undefined);

  let fieldNodes: ReactNode;
  if (fields.length === 0) {
    fieldNodes = <div className="text-xs text-fg-tertiary">No editable properties.</div>;
  } else {
    fieldNodes = fields.map((f) => {
      const value = attrs[f.key] ?? f.defaultValue ?? '';
      const inputId = `${partId}-${f.key}-input`;
      const rangeId = `${partId}-${f.key}-range`;
      return (
      <div key={f.key} className="flex flex-col gap-1">
        <label className={labelCls} htmlFor={inputId}>{f.label}</label>
        {f.type === 'select' ? (
          <select id={inputId} className={inputCls} value={attrs[f.key] ?? ''} onChange={(e) => onChange(f.key, e.target.value)}>
            {(f.options ?? []).map((o) => (
              <option key={o} value={o}>{o}</option>
            ))}
          </select>
        ) : f.type === 'color' ? (
          <input
            id={inputId}
            type="color"
            className="h-8 w-16 rounded-md border border-border bg-bg-elevated"
            value={attrs[f.key] ?? '#000000'}
            onChange={(e) => onChange(f.key, e.target.value)}
          />
        ) : f.type === 'range' ? (
          <div className="flex items-center gap-2">
            <input
              id={rangeId}
              aria-label={f.label}
              type="range"
              min={f.min}
              max={f.max}
              step={f.step}
              className="min-w-0 flex-1"
              value={value}
              onChange={(e) => onChange(f.key, e.target.value)}
            />
            <input
              id={inputId}
              type="text"
              inputMode="decimal"
              className={`${inputCls} w-20 text-right font-mono`}
              value={value}
              onChange={(e) => onChange(f.key, e.target.value)}
            />
          </div>
        ) : f.type === 'textarea' ? (
          <textarea
            id={inputId}
            rows={4}
            className={inputCls}
            value={attrs[f.key] ?? ''}
            onChange={(e) => onChange(f.key, e.target.value)}
          />
        ) : (
          <input
            id={inputId}
            type="text"
            className={inputCls}
            value={attrs[f.key] ?? ''}
            onChange={(e) => onChange(f.key, e.target.value)}
          />
        )}
      </div>
      );
    });
  }

  return (
    <div className="flex h-full flex-col">
    <div className="flex flex-1 flex-col gap-3 overflow-auto p-3">
      {hasLive && (
        <div className="flex items-center gap-2">
          <span className={labelCls}>State</span>
          {live!.active !== undefined && (
            <span
              className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium ${
                live!.active ? 'bg-green-400/15 text-green-300' : 'bg-bg-elevated text-fg-tertiary'
              }`}
            >
              <span className={`h-1.5 w-1.5 rounded-full ${live!.active ? 'bg-green-400' : 'bg-fg-tertiary'}`} />
              {live!.active ? 'ON' : 'OFF'}
            </span>
          )}
          {live!.analogValue !== undefined && (
            <span className="font-mono text-xs text-fg-secondary">{live!.analogValue}</span>
          )}
        </div>
      )}

      {fieldNodes}
      {runtimeControl}

      <div className="mt-auto font-mono text-[10px] text-fg-tertiary">{partType} · {partId}</div>
    </div>
    {actions}
    </div>
  );
}
