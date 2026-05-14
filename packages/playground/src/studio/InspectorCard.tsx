import { useState, type ReactNode } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

export interface InspectorPin {
  id: string;
  label: string;
}

export interface PartSelection {
  kind: 'part';
  partId: string;
  partType: string;
  label: string;
  pins: InspectorPin[];
  attrs: Record<string, unknown>;
}

export interface WireSelection {
  kind: 'wire';
  wireId: string;
  from: string;
  to: string;
  color: string;
}

export type InspectorSelection = PartSelection | WireSelection;

export interface InspectorCardProps {
  selection: InspectorSelection | null;
  devMode: boolean;
  labWidget?: ReactNode;
  advancedView?: ReactNode;
  onDelete: (id: string) => void;
  onDuplicate: (id: string) => void;
}

export function InspectorCard({
  selection,
  devMode,
  labWidget,
  advancedView,
  onDelete,
  onDuplicate,
}: InspectorCardProps) {
  const [advancedOpen, setAdvancedOpen] = useState(false);

  return (
    <AnimatePresence>
      {selection && (
        <motion.aside
          role="complementary"
          aria-label="Inspector"
          initial={{ opacity: 0, x: 16 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: 16 }}
          transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
          className="lw-glass absolute top-[60px] right-4 bottom-[80px] w-[320px] flex flex-col overflow-hidden z-20"
        >
          {selection.kind === 'part' ? (
            <PartInspector
              selection={selection}
              devMode={devMode}
              labWidget={labWidget}
              advancedView={advancedView}
              advancedOpen={advancedOpen}
              onToggleAdvanced={() => setAdvancedOpen((open) => !open)}
              onDelete={onDelete}
              onDuplicate={onDuplicate}
            />
          ) : (
            <WireInspector selection={selection} onDelete={onDelete} />
          )}
        </motion.aside>
      )}
    </AnimatePresence>
  );
}

interface PartInspectorProps {
  selection: PartSelection;
  devMode: boolean;
  labWidget?: ReactNode;
  advancedView?: ReactNode;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
  onDelete: (id: string) => void;
  onDuplicate: (id: string) => void;
}

function PartInspector({
  selection,
  devMode,
  labWidget,
  advancedView,
  advancedOpen,
  onToggleAdvanced,
  onDelete,
  onDuplicate,
}: PartInspectorProps) {
  return (
    <>
      <header className="px-4 py-3 border-b border-border flex items-center gap-2">
        <div className="w-8 h-8 rounded bg-bg-canvas border border-border flex items-center justify-center text-fg-secondary text-xs font-mono">
          {selection.partType[0]?.toUpperCase()}
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-fg-primary font-semibold truncate">{selection.label}</div>
          <div className="text-fg-tertiary text-[11px] font-mono truncate">{selection.partId}</div>
        </div>
      </header>
      <div className="flex-1 overflow-y-auto">
        <section className="px-4 py-3 border-b border-border">
          <h3 className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-2">Pins</h3>
          <table className="w-full text-[12px] font-mono">
            <tbody>
              {selection.pins.map((pin) => (
                <tr key={pin.id} className="hover:bg-bg-elevated">
                  <td className="py-1 pr-2 text-fg-secondary">{pin.id}</td>
                  <td className="py-1 text-fg-primary">{pin.label}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
        {labWidget && (
          <section className="px-4 py-3 border-b border-border">
            <h3 className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-2">Live</h3>
            {labWidget}
          </section>
        )}
        {devMode && advancedView && (
          <section className="px-4 py-3 border-b border-border">
            <button
              type="button"
              onClick={onToggleAdvanced}
              className="text-fg-secondary text-[11px] uppercase tracking-wider hover:text-fg-primary"
            >
              {advancedOpen ? '▾ Advanced' : '▸ Advanced'}
            </button>
            {advancedOpen && <div className="mt-2">{advancedView}</div>}
          </section>
        )}
      </div>
      <footer className="border-t border-border px-4 py-3 flex gap-2">
        <button
          type="button"
          onClick={() => onDuplicate(selection.partId)}
          className="flex-1 h-8 rounded-button bg-bg-elevated border border-border text-fg-primary hover:border-accent"
        >
          Duplicate
        </button>
        <button
          type="button"
          onClick={() => onDelete(selection.partId)}
          className="flex-1 h-8 rounded-button bg-danger/10 border border-danger/30 text-danger hover:bg-danger/20"
        >
          Delete
        </button>
      </footer>
    </>
  );
}

function WireInspector({ selection, onDelete }: { selection: WireSelection; onDelete: (id: string) => void }) {
  return (
    <>
      <header className="px-4 py-3 border-b border-border">
        <div className="text-fg-primary font-semibold">Wire</div>
        <div className="text-fg-tertiary text-[11px] font-mono">
          {selection.from} → {selection.to}
        </div>
      </header>
      <div className="flex-1 px-4 py-3">
        <div className="flex items-center gap-2 text-[12px] text-fg-secondary">
          <span className="w-3 h-3 rounded-full" style={{ background: selection.color }} />
          {selection.color}
        </div>
      </div>
      <footer className="border-t border-border px-4 py-3">
        <button
          type="button"
          onClick={() => onDelete(selection.wireId)}
          className="w-full h-8 rounded-button bg-danger/10 border border-danger/30 text-danger hover:bg-danger/20"
        >
          Delete wire
        </button>
      </footer>
    </>
  );
}
