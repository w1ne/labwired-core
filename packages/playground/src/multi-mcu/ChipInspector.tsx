// The full per-chip inspector — the WHOLE drawer (control surface + all tabs:
// Serial / Registers / Trace / Memory / Source / YAML) for one chip, rendered
// inside a floating ChipWindow so each chip's drawer can be arranged freely.
//
// Serial is live for every chip (per-chip buffers). Registers / Trace / Memory
// are only polled for the focused (foreground) chip, so non-focused windows
// invite a click to focus. Source / YAML are static board identity.
import { useState } from 'react';
import type { ReactNode } from 'react';
import { SerialMonitor, RegisterGrid, InstructionTrace, MemoryInspector } from '@labwired/ui';
import type { ComponentProps } from 'react';
import type { BoardConfig } from '../bundled-configs';

type DevTab = 'serial' | 'registers' | 'trace' | 'memory' | 'source' | 'yaml';
const TABS: DevTab[] = ['serial', 'registers', 'trace', 'memory', 'source', 'yaml'];
const TAB_LABEL: Record<DevTab, string> = {
  serial: 'Serial',
  registers: 'Registers',
  trace: 'Trace',
  memory: 'Memory',
  source: 'Source',
  yaml: 'YAML',
};

export interface ChipInspectorProps {
  board: BoardConfig;
  /** Whether this chip is the focused (foreground) one — gates live data. */
  isForeground: boolean;
  /** Control surface (Run/Pause/Upload/Restart) — rendered in the title bar. */
  controls?: ReactNode;
  /** Standard part actions (Rotate/Size/Delete) — rendered at the bottom. */
  actions?: ReactNode;
  serialOutput: string;
  onClearSerial: () => void;
  /** Live data — only meaningful when isForeground. */
  registers?: ComponentProps<typeof RegisterGrid>['registers'];
  traceEntries?: ComponentProps<typeof InstructionTrace>['entries'];
  stackMemory?: ComponentProps<typeof MemoryInspector>['data'];
  stackBase?: number;
  hasLiveSim?: boolean;
}

function Hint({ label }: { label: string }) {
  return (
    <div className="flex h-full items-center justify-center p-4 text-center text-xs text-fg-tertiary">
      {label}
    </div>
  );
}

export function ChipInspector({
  board,
  isForeground,
  controls,
  actions,
  serialOutput,
  onClearSerial,
  registers,
  traceEntries,
  stackMemory,
  stackBase,
  hasLiveSim,
}: ChipInspectorProps) {
  const [active, setActive] = useState<DevTab>('serial');
  const focusHint = 'Click this window to focus the chip, then inspect.';

  let body: ReactNode = null;
  if (active === 'serial') {
    body = <SerialMonitor output={serialOutput} onClear={onClearSerial} style={{ height: '100%' }} />;
  } else if (active === 'registers') {
    body = isForeground && hasLiveSim && registers
      ? <RegisterGrid registers={registers} style={{ maxHeight: '100%', overflow: 'auto' }} />
      : <Hint label={isForeground ? 'Run the chip to inspect CPU registers.' : focusHint} />;
  } else if (active === 'trace') {
    body = isForeground && hasLiveSim
      ? <InstructionTrace entries={traceEntries ?? []} style={{ maxHeight: '100%', overflow: 'auto' }} />
      : <Hint label={isForeground ? 'Run the chip to see the instruction trace.' : focusHint} />;
  } else if (active === 'memory') {
    body = isForeground && hasLiveSim
      ? <MemoryInspector data={stackMemory ?? new Uint8Array()} baseAddress={stackBase ?? 0} style={{ maxHeight: '100%', overflow: 'auto' }} />
      : <Hint label={isForeground ? 'Run the chip to inspect memory.' : focusHint} />;
  } else if (active === 'source') {
    body = board.sourceCode ? (
      <div className="flex h-full flex-col">
        {board.sourceFilename && (
          <div className="shrink-0 border-b border-border bg-bg-elevated/40 px-3 py-1.5 font-mono text-[11px] text-fg-tertiary">
            {board.sourceFilename}
          </div>
        )}
        <pre className="flex-1 overflow-auto whitespace-pre p-3 font-mono text-[12px] leading-[1.5] text-fg-secondary">
          {board.sourceCode}
        </pre>
      </div>
    ) : (
      <Hint label="Source not bundled for this chip." />
    );
  } else if (active === 'yaml') {
    body = (
      <pre className="h-full overflow-auto whitespace-pre-wrap p-3 font-mono text-[12px] text-fg-secondary">
        {board.systemYaml}
      </pre>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {controls && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border bg-bg-elevated/30 px-2 py-1">
          {controls}
        </div>
      )}
      <div role="tablist" className="flex shrink-0 items-center gap-0.5 border-b border-border px-1.5">
        {TABS.map((tab) => (
          <button
            key={tab}
            role="tab"
            aria-selected={active === tab}
            onClick={() => setActive(tab)}
            className={`px-2 py-1 text-[11px] transition-colors ${
              active === tab
                ? 'border-b-2 border-accent text-fg-primary'
                : 'border-b-2 border-transparent text-fg-tertiary hover:text-fg-secondary'
            }`}
          >
            {TAB_LABEL[tab]}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">{body}</div>
      {actions}
    </div>
  );
}
