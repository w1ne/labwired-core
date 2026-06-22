// The full per-chip inspector — control surface + tabs (Serial / Registers /
// Trace / Memory / Source / YAML) for one MCU, rendered inside a ChipWindow.
// Moved from the playground into @labwired/ui, inline-styled, and parameterized:
// instead of a playground BoardConfig it takes optional sourceCode / sourceFilename
// / systemYaml strings, so any consumer can reuse it.
import { useState } from 'react';
import type { ComponentProps, CSSProperties, ReactNode } from 'react';
import { SerialMonitor } from '../components/SerialMonitor/SerialMonitor';
import { RegisterGrid } from '../components/RegisterGrid/RegisterGrid';
import { InstructionTrace } from '../components/InstructionTrace/InstructionTrace';
import { MemoryInspector } from '../components/MemoryInspector/MemoryInspector';

type DevTab = 'serial' | 'registers' | 'trace' | 'memory' | 'source' | 'yaml';
const TAB_LABEL: Record<DevTab, string> = { serial: 'Serial', registers: 'Registers', trace: 'Trace', memory: 'Memory', source: 'Source', yaml: 'YAML' };

const C = { elevated: 'var(--lw-bg-elevated, #1A1D26)', border: 'var(--lw-border, #262A33)', accent: 'var(--lw-accent, #5B9DFF)', fgPrimary: 'var(--lw-fg-primary, #F2F4F9)', fgSecondary: 'var(--lw-fg-secondary, #9098A8)', fgTertiary: 'var(--lw-fg-tertiary, #5A6178)' };

export interface ChipInspectorProps {
  /** Whether this chip is the focused (foreground) one — gates live data. */
  isForeground: boolean;
  controls?: ReactNode;
  actions?: ReactNode;
  serialOutput: string;
  onClearSerial: () => void;
  registers?: ComponentProps<typeof RegisterGrid>['registers'];
  traceEntries?: ComponentProps<typeof InstructionTrace>['entries'];
  stackMemory?: ComponentProps<typeof MemoryInspector>['data'];
  stackBase?: number;
  hasLiveSim?: boolean;
  /** Static board identity (optional). */
  sourceCode?: string;
  sourceFilename?: string;
  systemYaml?: string;
}

function Hint({ label }: { label: string }) {
  return <div style={{ display: 'flex', height: '100%', alignItems: 'center', justifyContent: 'center', padding: 16, textAlign: 'center', fontSize: 12, color: C.fgTertiary }}>{label}</div>;
}

export function ChipInspector({
  isForeground, controls, actions, serialOutput, onClearSerial,
  registers, traceEntries, stackMemory, stackBase, hasLiveSim,
  sourceCode, sourceFilename, systemYaml,
}: ChipInspectorProps) {
  const tabs: DevTab[] = ['serial', 'registers', 'trace', 'memory', ...(sourceCode ? ['source' as const] : []), ...(systemYaml ? ['yaml' as const] : [])];
  const [active, setActive] = useState<DevTab>('serial');
  const focusHint = 'Click this window to focus the chip, then inspect.';
  const pre: CSSProperties = { height: '100%', overflow: 'auto', whiteSpace: 'pre-wrap', padding: 12, fontFamily: 'ui-monospace, monospace', fontSize: 12, color: C.fgSecondary, margin: 0 };

  let body: ReactNode = null;
  if (active === 'serial') body = <SerialMonitor output={serialOutput} onClear={onClearSerial} style={{ height: '100%' }} />;
  else if (active === 'registers') body = isForeground && hasLiveSim && registers ? <RegisterGrid registers={registers} style={{ maxHeight: '100%', overflow: 'auto' }} /> : <Hint label={isForeground ? 'Run the chip to inspect CPU registers.' : focusHint} />;
  else if (active === 'trace') body = isForeground && hasLiveSim ? <InstructionTrace entries={traceEntries ?? []} style={{ maxHeight: '100%', overflow: 'auto' }} /> : <Hint label={isForeground ? 'Run the chip to see the instruction trace.' : focusHint} />;
  else if (active === 'memory') body = isForeground && hasLiveSim ? <MemoryInspector data={stackMemory ?? new Uint8Array()} baseAddress={stackBase ?? 0} style={{ maxHeight: '100%', overflow: 'auto' }} /> : <Hint label={isForeground ? 'Run the chip to inspect memory.' : focusHint} />;
  else if (active === 'source') body = sourceCode ? (
    <div style={{ display: 'flex', height: '100%', flexDirection: 'column' }}>
      {sourceFilename && <div style={{ flexShrink: 0, borderBottom: `1px solid ${C.border}`, background: C.elevated, padding: '6px 12px', fontFamily: 'ui-monospace, monospace', fontSize: 11, color: C.fgTertiary }}>{sourceFilename}</div>}
      <pre style={{ ...pre, whiteSpace: 'pre', flex: 1 }}>{sourceCode}</pre>
    </div>
  ) : <Hint label="Source not bundled for this chip." />;
  else if (active === 'yaml') body = <pre style={pre}>{systemYaml}</pre>;

  const tab = (t: DevTab): CSSProperties => ({ padding: '4px 8px', fontSize: 11, cursor: 'pointer', background: 'transparent', border: 'none', borderBottom: `2px solid ${active === t ? C.accent : 'transparent'}`, color: active === t ? C.fgPrimary : C.fgTertiary });

  return (
    <div style={{ display: 'flex', height: '100%', flexDirection: 'column' }}>
      {controls && <div style={{ display: 'flex', flexShrink: 0, alignItems: 'center', gap: 8, borderBottom: `1px solid ${C.border}`, background: C.elevated, padding: '4px 8px' }}>{controls}</div>}
      <div role="tablist" style={{ display: 'flex', flexShrink: 0, alignItems: 'center', gap: 2, borderBottom: `1px solid ${C.border}`, padding: '0 6px' }}>
        {tabs.map((t) => <button key={t} role="tab" aria-selected={active === t} onClick={() => setActive(t)} style={tab(t)}>{TAB_LABEL[t]}</button>)}
      </div>
      <div style={{ minHeight: 0, flex: 1, overflow: 'hidden' }}>{body}</div>
      {actions}
    </div>
  );
}
