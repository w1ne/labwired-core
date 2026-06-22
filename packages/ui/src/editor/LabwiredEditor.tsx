// LabwiredEditor — the reusable, MODULAR editor surface. A controlled shell that
// composes the board (EditorCanvas) + LabWired's floating-window inspectors (core,
// always) with OPTIONAL regions — palette · code pane · property panel · sim dock —
// each omitted unless its prop is supplied. The host owns diagram + sim/compile/
// bridge lifecycle and injects them. One shared editor; each host enables only the
// regions it needs (proto.cat: canvas + inspectors; playground: + palette/code/dock).
import { useMemo, useState, type CSSProperties, type ReactNode } from 'react';
import { EditorCanvas } from './EditorCanvas';
import { ChipWindow } from './ChipWindow';
import { ChipInspector } from './ChipInspector';
import { ComponentInspector, type AttrField } from './ComponentInspector';
import { ComponentPalette } from './ComponentPalette';
import { CodeEditor } from './CodeEditor';
import { PropertyPanel } from './PropertyPanel';
import { SimDock, type SimDockProps } from './SimDock';
import { resolveComponentDef } from './components/index';
import type { EditorState, WireEndpoint, DisplayBuffer, Part, ComponentState } from './types';
import type { CompileError } from './CodeEditor';
import type { ComponentProps } from 'react';

const noop = () => {};

export interface EditorSimData {
  hasLiveSim?: boolean;
  serialOutput?: string;
  onClearSerial?: () => void;
  registers?: ComponentProps<typeof ChipInspector>['registers'];
  traceEntries?: ComponentProps<typeof ChipInspector>['traceEntries'];
  stackMemory?: ComponentProps<typeof ChipInspector>['stackMemory'];
  stackBase?: number;
  sourceCode?: string;
  sourceFilename?: string;
  systemYaml?: string;
}

export interface LabwiredEditorProps {
  // ── Core (controlled) ──
  state: EditorState;
  interactionMode?: 'edit' | 'run';
  displayBuffers?: Record<string, DisplayBuffer>;
  /** Live per-part simulation state (button/analog) painted on the canvas. */
  boardIoStates?: Record<string, ComponentState>;
  /** Canvas-level validation banner (e.g. an illegal wire in progress). */
  validationMessage?: string | null;
  /** Pins to flag as invalid while wiring. */
  invalidPins?: WireEndpoint[];
  onMovePart?: (id: string, x: number, y: number) => void;
  onResizePart?: (id: string, scale: number) => void;
  onSelect?: (id: string | null, add?: boolean) => void;
  onSelectRect?: (ids: string[]) => void;
  onStartWire?: (ep: WireEndpoint) => void;
  onCompleteWire?: (ep: WireEndpoint) => void;
  onCancelWire?: () => void;
  onDeleteWire?: (index: number) => void;
  onButtonToggle?: (id: string, active: boolean) => void;
  onAnalogChange?: (partId: string, value: number) => void;
  onAttrChange?: (partId: string, key: string, value: string) => void;
  attrOverrides?: Record<string, Record<string, string>>;
  /** Overlay anchored to the selected part on the canvas (e.g. a chip toolbar). */
  selectedPartOverlay?: (part: Part, box: { x: number; y: number; width: number; height: number }) => ReactNode;
  /** Sim data for the focused MCU's ChipInspector tabs. */
  sim?: EditorSimData;
  /**
   * Built-in floating inspector windows (one per selected part). Default true.
   * Hosts with their own richer windows pass false and render them via `overlays`
   * (selection still flows through onSelect). proto.cat uses the built-ins;
   * the playground owns its windows.
   */
  renderWindows?: boolean;
  /** Free-form overlay layer rendered at the editor root (host-owned windows, toasts). */
  overlays?: ReactNode;

  // ── Optional regions ──
  /** Left component palette (drag/click to add). Supply onAddPart + onDropPart. */
  palette?: boolean;
  onAddPart?: (type: string) => void;
  onDropPart?: (type: string, x: number, y: number) => void;
  /** Top code pane. */
  codePane?: { source: string; onChange: (s: string) => void; language?: string; errors?: CompileError[]; readOnly?: boolean } | false;
  /** Right property panel for selected parts. */
  propertyPanel?: boolean;
  propertyLabWidget?: ReactNode;
  /** When provided, the panel rail gains a collapse/expand toggle driven by `propertyPanel`. */
  onSetPropertyPanel?: (open: boolean) => void;
  onDeleteSelected?: () => void;
  onRotatePart?: (id: string) => void;
  /** Bottom simulation dock. */
  simDock?: SimDockProps | false;
  /** Host chrome slots. */
  headerSlot?: ReactNode;
  footerSlot?: ReactNode;
}

interface WinState { id: string; x: number; y: number; z: number }

export function LabwiredEditor(props: LabwiredEditorProps) {
  const {
    state, interactionMode = 'edit', displayBuffers,
    boardIoStates, validationMessage, invalidPins,
    onMovePart = noop, onResizePart, onSelect = noop, onSelectRect,
    onStartWire = noop, onCompleteWire = noop, onCancelWire = noop, onDeleteWire = noop,
    onButtonToggle, onAnalogChange, onAttrChange = noop, attrOverrides = {}, selectedPartOverlay, sim,
    renderWindows = true, overlays,
    palette, onAddPart = noop, onDropPart,
    codePane, propertyPanel, propertyLabWidget, onSetPropertyPanel, onDeleteSelected = noop, onRotatePart = noop,
    simDock, headerSlot, footerSlot,
  } = props;

  const [windows, setWindows] = useState<WinState[]>([]);
  const [zTop, setZTop] = useState(60);

  const focusWin = (id: string) => { setZTop((z) => z + 1); setWindows((ws) => ws.map((w) => (w.id === id ? { ...w, z: zTop + 1 } : w))); };
  const openWin = (id: string) =>
    setWindows((ws) => {
      if (ws.some((w) => w.id === id)) { focusWin(id); return ws; }
      const n = ws.length;
      setZTop((z) => z + 1);
      return [...ws, { id, x: 160 + n * 28, y: 110 + n * 28, z: zTop + 1 }];
    });
  const closeWin = (id: string) => setWindows((ws) => ws.filter((w) => w.id !== id));

  const partById = useMemo(() => new Map(state.diagram.parts.map((p) => [p.id, p])), [state.diagram.parts]);
  const selectedParts = useMemo(
    () => state.diagram.parts.filter((p) => state.selectedIds.has(p.id)),
    [state.diagram.parts, state.selectedIds],
  );

  const sidebar: CSSProperties = { width: 280, flexShrink: 0, borderColor: 'var(--lw-border, #262A33)', overflow: 'hidden', background: 'var(--lw-bg-surface, #13151B)' };
  const collapseBtn = (_side: 'left' | 'right'): CSSProperties => ({
    position: 'absolute', top: 8, left: -1,
    zIndex: 5, width: 18, height: 36, display: 'grid', placeItems: 'center', cursor: 'pointer', fontSize: 12,
    border: '1px solid var(--lw-border, #262A33)', borderRadius: 6,
    background: 'var(--lw-bg-elevated, #1A1D26)', color: 'var(--lw-fg-secondary, #9098A8)',
  });

  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      {headerSlot}
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {palette && (
          <div style={{ ...sidebar, borderRight: '1px solid var(--lw-border, #262A33)' }}>
            <ComponentPalette onAddPart={onAddPart} />
          </div>
        )}

        {/* Center: optional code pane (top) + canvas */}
        <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minWidth: 0 }}>
          {codePane && (
            <div style={{ height: '40%', minHeight: 0, borderBottom: '1px solid var(--lw-border, #262A33)' }}>
              <CodeEditor source={codePane.source} onChange={codePane.onChange} language={codePane.language} errors={codePane.errors} readOnly={codePane.readOnly} />
            </div>
          )}
          <div style={{ position: 'relative', flex: 1, minHeight: 0 }}>
            <EditorCanvas
              state={state}
              interactionMode={interactionMode}
              displayBuffers={displayBuffers}
              boardIoStates={boardIoStates}
              validationMessage={validationMessage}
              invalidPins={invalidPins}
              onMovePart={onMovePart}
              onResizePart={onResizePart}
              onSelect={(id, add) => { onSelect(id, add); if (id && renderWindows) openWin(id); }}
              onSelectRect={onSelectRect}
              onStartWire={onStartWire}
              onCompleteWire={onCompleteWire}
              onCancelWire={onCancelWire}
              onDeleteWire={onDeleteWire}
              onButtonToggle={onButtonToggle}
              onAnalogChange={onAnalogChange}
              onDropPart={onDropPart}
              selectedPartOverlay={selectedPartOverlay}
            />
            {simDock && (
              <div style={{ position: 'absolute', bottom: 16, left: '50%', transform: 'translateX(-50%)', zIndex: 40 }}>
                <SimDock {...simDock} />
              </div>
            )}
          </div>
        </div>

        {propertyPanel && (
          <div style={{ ...sidebar, borderLeft: '1px solid var(--lw-border, #262A33)', position: 'relative' }}>
            {onSetPropertyPanel && (
              <button
                type="button"
                onClick={() => onSetPropertyPanel(false)}
                title="Hide properties"
                aria-label="Hide properties"
                style={collapseBtn('right')}
              >›</button>
            )}
            <PropertyPanel
              parts={selectedParts}
              onUpdateAttrs={(id, attrs) => Object.entries(attrs).forEach(([k, v]) => onAttrChange(id, k, v))}
              onDelete={onDeleteSelected}
              onRotate={onRotatePart}
              onResize={onResizePart}
              labWidget={propertyLabWidget}
            />
          </div>
        )}
        {!propertyPanel && onSetPropertyPanel && (
          <div style={{ width: 20, flexShrink: 0, borderLeft: '1px solid var(--lw-border, #262A33)', background: 'var(--lw-bg-surface, #13151B)', display: 'flex', justifyContent: 'center', paddingTop: 8 }}>
            <button
              type="button"
              onClick={() => onSetPropertyPanel(true)}
              title="Show properties"
              aria-label="Show properties"
              style={{ width: 18, height: 36, display: 'grid', placeItems: 'center', cursor: 'pointer', fontSize: 12, border: '1px solid var(--lw-border, #262A33)', borderRadius: 6, background: 'var(--lw-bg-elevated, #1A1D26)', color: 'var(--lw-fg-secondary, #9098A8)' }}
            >‹</button>
          </div>
        )}
      </div>
      {footerSlot}

      {/* Host-owned overlay layer (custom windows, toasts). */}
      {overlays}

      {/* Core: floating inspector windows (one per opened part). Skipped when the
          host renders its own via `overlays`. */}
      {renderWindows && windows.map((w) => {
        const part = partById.get(w.id);
        if (!part) return null;
        const def = resolveComponentDef(part.type);
        const isMcu = def.category === 'mcu';
        const attrs = { ...def.defaultAttrs, ...(attrOverrides[w.id] ?? {}) };
        return (
          <ChipWindow
            key={w.id}
            title={<span style={{ fontFamily: 'ui-monospace, monospace', fontSize: 12 }}>{def.label}</span>}
            initial={{ x: w.x, y: w.y }}
            zIndex={w.z}
            width={isMcu ? 460 : 300}
            height={isMcu ? 300 : 240}
            onClose={() => closeWin(w.id)}
            onFocus={() => focusWin(w.id)}
          >
            {isMcu ? (
              <ChipInspector
                isForeground
                hasLiveSim={sim?.hasLiveSim ?? false}
                serialOutput={sim?.serialOutput ?? ''}
                onClearSerial={sim?.onClearSerial ?? noop}
                registers={sim?.registers}
                traceEntries={sim?.traceEntries}
                stackMemory={sim?.stackMemory}
                stackBase={sim?.stackBase}
                sourceCode={sim?.sourceCode}
                sourceFilename={sim?.sourceFilename}
                systemYaml={sim?.systemYaml}
              />
            ) : (
              <ComponentInspector
                partType={def.label}
                partId={part.id}
                attrs={attrs}
                fields={(def.attrFields ?? []) as AttrField[]}
                onChange={(key, value) => onAttrChange(part.id, key, value)}
              />
            )}
          </ChipWindow>
        );
      })}
    </div>
  );
}
