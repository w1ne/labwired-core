import type { ReactNode } from 'react';

export interface AttrField {
  key: string;
  label: string;
  type: 'text' | 'select' | 'color';
  options?: string[];
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
  actions,
}: ComponentInspectorProps) {
  const hasLive = live && (live.active !== undefined || live.analogValue !== undefined);

  let fieldNodes: ReactNode;
  if (fields.length === 0) {
    fieldNodes = <div className="text-xs text-fg-tertiary">No editable properties.</div>;
  } else {
    fieldNodes = fields.map((f) => (
      <label key={f.key} className="flex flex-col gap-1">
        <span className={labelCls}>{f.label}</span>
        {f.type === 'select' ? (
          <select className={inputCls} value={attrs[f.key] ?? ''} onChange={(e) => onChange(f.key, e.target.value)}>
            {(f.options ?? []).map((o) => (
              <option key={o} value={o}>{o}</option>
            ))}
          </select>
        ) : f.type === 'color' ? (
          <input
            type="color"
            className="h-8 w-16 rounded-md border border-border bg-bg-elevated"
            value={attrs[f.key] ?? '#000000'}
            onChange={(e) => onChange(f.key, e.target.value)}
          />
        ) : (
          <input
            type="text"
            className={inputCls}
            value={attrs[f.key] ?? ''}
            onChange={(e) => onChange(f.key, e.target.value)}
          />
        )}
      </label>
    ));
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

      <div className="mt-auto font-mono text-[10px] text-fg-tertiary">{partType} · {partId}</div>
    </div>
    {actions}
    </div>
  );
}
