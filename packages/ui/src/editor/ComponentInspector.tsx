// Properties inspector for a non-MCU part — editable attrs + live state. Moved
// from the playground into @labwired/ui, inline-styled (self-contained).
import type { CSSProperties, ReactNode } from 'react';

export interface AttrField {
  key: string;
  label: string;
  type: 'text' | 'select' | 'color' | 'range';
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
  runtimeControl?: ReactNode;
  actions?: ReactNode;
}

const C = { elevated: 'var(--lw-bg-elevated, #1A1D26)', border: 'var(--lw-border, #262A33)', accent: 'var(--lw-accent, #5B9DFF)', fgPrimary: 'var(--lw-fg-primary, #F2F4F9)', fgSecondary: 'var(--lw-fg-secondary, #9098A8)', fgTertiary: 'var(--lw-fg-tertiary, #5A6178)' };
const labelCls: CSSProperties = { fontSize: 11, textTransform: 'uppercase', letterSpacing: '0.04em', color: C.fgTertiary };
const inputCls: CSSProperties = { borderRadius: 6, border: `1px solid ${C.border}`, background: C.elevated, padding: '4px 8px', fontSize: 13, color: C.fgPrimary, outline: 'none' };

export function ComponentInspector({ partType, partId, attrs, fields, live, onChange, runtimeControl, actions }: ComponentInspectorProps) {
  const hasLive = live && (live.active !== undefined || live.analogValue !== undefined);

  return (
    <div style={{ display: 'flex', height: '100%', flexDirection: 'column' }}>
      <div style={{ display: 'flex', flex: 1, flexDirection: 'column', gap: 12, overflow: 'auto', padding: 12 }}>
        {hasLive && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={labelCls}>State</span>
            {live!.active !== undefined && (
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, borderRadius: 999, padding: '2px 8px', fontSize: 11, fontWeight: 500, background: live!.active ? 'rgba(74,222,128,0.15)' : C.elevated, color: live!.active ? '#86efac' : C.fgTertiary }}>
                <span style={{ width: 6, height: 6, borderRadius: 999, background: live!.active ? '#4ade80' : C.fgTertiary }} />
                {live!.active ? 'ON' : 'OFF'}
              </span>
            )}
            {live!.analogValue !== undefined && <span style={{ fontFamily: 'ui-monospace, monospace', fontSize: 12, color: C.fgSecondary }}>{live!.analogValue}</span>}
          </div>
        )}

        {fields.length === 0 ? (
          <div style={{ fontSize: 12, color: C.fgTertiary }}>No editable properties.</div>
        ) : (
          fields.map((f) => {
            const value = attrs[f.key] ?? f.defaultValue ?? '';
            return (
              <div key={f.key} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <label style={labelCls}>{f.label}</label>
                {f.type === 'select' ? (
                  <select style={inputCls} value={attrs[f.key] ?? ''} onChange={(e) => onChange(f.key, e.target.value)}>
                    {(f.options ?? []).map((o) => <option key={o} value={o}>{o}</option>)}
                  </select>
                ) : f.type === 'color' ? (
                  <input type="color" style={{ height: 32, width: 64, borderRadius: 6, border: `1px solid ${C.border}`, background: C.elevated }} value={attrs[f.key] ?? '#000000'} onChange={(e) => onChange(f.key, e.target.value)} />
                ) : f.type === 'range' ? (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <input aria-label={f.label} type="range" min={f.min} max={f.max} step={f.step} style={{ minWidth: 0, flex: 1 }} value={value} onChange={(e) => onChange(f.key, e.target.value)} />
                    <input type="text" inputMode="decimal" style={{ ...inputCls, width: 72, textAlign: 'right', fontFamily: 'ui-monospace, monospace' }} value={value} onChange={(e) => onChange(f.key, e.target.value)} />
                  </div>
                ) : (
                  <input type="text" style={inputCls} value={attrs[f.key] ?? ''} onChange={(e) => onChange(f.key, e.target.value)} />
                )}
              </div>
            );
          })
        )}
        {runtimeControl}
        <div style={{ marginTop: 'auto', fontFamily: 'ui-monospace, monospace', fontSize: 10, color: C.fgTertiary }}>{partType} · {partId}</div>
      </div>
      {actions}
    </div>
  );
}
