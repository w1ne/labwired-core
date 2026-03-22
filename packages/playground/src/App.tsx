import { useState, useCallback, useRef, useMemo, useEffect } from 'react';
import {
  SimControls,
  RegisterGrid,
  MemoryInspector,
  InstructionTrace,
  SerialMonitor,
  SimulatorBridge,
  useSimulationLoop,
  EditorCanvas,
  ComponentPalette,
  PropertyPanel,
  CodeEditor,
  compileCode,
  EXAMPLE_SKETCHES,
  useEditorState,
  diagramToConfig,
  COMPONENT_REGISTRY,
  createEmptyDiagram,
  decodeProject,
  generateShareUrl,
  isEmbedMode,
  type CompileError,
  type TraceEntry,
  type WasmModule,
  type Part,
  type Diagram,
} from '@labwired/ui';
import { DEMO_PROJECTS, ensureFirmwareLoaded, type DemoProject } from './demos';

type BottomTab = 'output' | 'serial' | 'registers' | 'trace' | 'memory';

let partCounter = 0;
function nextPartId(type: string): string {
  return `${type}${++partCounter}`;
}

function makeInitialDiagram(board: string): Diagram {
  return {
    ...createEmptyDiagram(board),
    parts: [{ id: 'mcu', type: 'mcu', x: 100, y: 100, rotate: 0, attrs: {} }],
  };
}

export function App() {
  const [wasmModule, setWasmModule] = useState<WasmModule | null>(null);
  const [bridge, setBridge] = useState<SimulatorBridge | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<DemoProject>(DEMO_PROJECTS[0]);
  const [running, setRunning] = useState(false);
  const traceRef = useRef<TraceEntry[]>([]);
  const [traceEntries, setTraceEntries] = useState<TraceEntry[]>([]);

  // Code editor state
  const [source, setSource] = useState(EXAMPLE_SKETCHES[0].source);
  const [compileErrors, setCompileErrors] = useState<CompileError[]>([]);
  const [compileOutput, setCompileOutput] = useState('');
  const [compiling, setCompiling] = useState(false);
  const [bottomTab, setBottomTab] = useState<BottomTab>('output');
  const [showCode, setShowCode] = useState(true);
  const [showBottomPanel, setShowBottomPanel] = useState(true);
  const embed = isEmbedMode();

  // Editor state
  const editor = useEditorState(makeInitialDiagram('stm32f103'));

  // Whether simulation has been loaded (bridge exists)
  const simActive = !!bridge;

  // Load WASM module lazily
  const loadWasm = useCallback(async () => {
    if (wasmModule) return wasmModule;
    const wasmUrl = new URL(`${import.meta.env.BASE_URL}wasm/labwired_wasm.js`, window.location.origin);
    const mod = await import(/* @vite-ignore */ wasmUrl.href);
    await mod.default();
    setWasmModule(mod as WasmModule);
    return mod as WasmModule;
  }, [wasmModule]);

  // Compile source code
  const handleCompile = useCallback(async () => {
    setCompiling(true);
    setCompileOutput('Compiling...\n');
    setCompileErrors([]);
    setBottomTab('output');
    setShowBottomPanel(true);
    try {
      const result = await compileCode({
        source,
        language: 'arduino',
        target: editor.state.diagram.board,
      });
      setCompileErrors(result.errors);
      setCompileOutput(result.output);
      return result;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setCompileOutput(`Compile error: ${msg}`);
      return null;
    } finally {
      setCompiling(false);
    }
  }, [source, editor.state.diagram.board]);

  // "Upload" (compile + run): convert diagram → config, init simulator
  const handleRun = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // Try compiling first
      const result = await handleCompile();

      const mod = await loadWasm();

      // Use compiled ELF if available, otherwise fall back to demo firmware
      let firmware: Uint8Array;
      if (result?.success && result.elf) {
        firmware = result.elf;
        setCompileOutput((prev) => prev + '\nUpload successful. Starting simulation...');
      } else {
        // Fall back to pre-built demo firmware
        const project = await ensureFirmwareLoaded(selectedProject);
        setSelectedProject(project);
        firmware = project.firmware;
        setCompileOutput((prev) => prev + '\nUsing pre-built demo firmware.');
      }

      const { systemYaml, chipYaml } = diagramToConfig(editor.state.diagram);

      const b = await SimulatorBridge.fromConfig(mod, {
        systemYaml,
        chipYaml,
        firmware,
      });
      setBridge(b);
      setRunning(true);
      traceRef.current = [];
      setTraceEntries([]);
      setBottomTab('serial');
      setShowBottomPanel(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [handleCompile, loadWasm, selectedProject, editor.state.diagram]);

  // Stop simulation
  const handleStop = useCallback(() => {
    setRunning(false);
    setBridge(null);
  }, []);

  // Drive the simulation loop
  const { state: simState, stepOnce, clearUart } = useSimulationLoop({
    bridge,
    running,
    cyclesPerFrame: 5000,
  });

  // Accumulate trace entries
  const prevPcRef = useRef(0);
  if (simState.pc !== prevPcRef.current && simState.disassembly) {
    prevPcRef.current = simState.pc;
    const entry: TraceEntry = { pc: simState.pc, disassembly: simState.disassembly };
    traceRef.current = [...traceRef.current.slice(-200), entry];
    if (traceRef.current.length !== traceEntries.length) {
      setTraceEntries(traceRef.current);
    }
  }

  // Build register map
  const registers = useMemo(() => {
    if (!bridge) return new Map<string, number>();
    const names = bridge.getRegisterNames();
    const map = new Map<string, number>();
    names.forEach((name: string, i: number) => map.set(name, bridge.getRegister(i)));
    return map;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bridge, simState.pc]);

  const stackBase = registers.get('SP') ?? registers.get('R13') ?? 0x20005000;
  const stackMemory = useMemo(() => {
    if (!bridge) return new Uint8Array(0);
    return bridge.readMemory(stackBase, 64);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bridge, stackBase]);

  const handleButtonToggle = useCallback(
    (id: string, pressed: boolean) => { bridge?.setBoardIoInput(id, pressed); },
    [bridge],
  );

  const handlePlay = useCallback(() => setRunning(true), []);
  const handlePause = useCallback(() => setRunning(false), []);
  const handleStep = useCallback(() => { setRunning(false); stepOnce(); }, [stepOnce]);
  const handleReset = useCallback(() => {
    setRunning(false); clearUart(); traceRef.current = []; setTraceEntries([]);
  }, [clearUart]);

  const analogStates = useMemo(() => bridge?.getAnalogStates() ?? [], [bridge, simState.pc]);

  const boardIoStateMap = useMemo(() => {
    const map: Record<string, { active?: boolean; analogValue?: number; displayText?: string; frequency?: number; angle?: number }> = {};
    for (const s of simState.boardIoStates) map[s.id] = { active: s.active };
    for (const a of analogStates) {
      if (!map[a.id]) map[a.id] = {};
      if (a.kind === 'adc_input' && a.value !== undefined) {
        map[a.id].analogValue = a.value;
      }
      if (a.kind === 'pwm_output') {
        map[a.id].active = a.active;
      }
    }

    // Enrich PWM outputs with frequency/angle from peripheral snapshots
    if (bridge) {
      const ioConfig = bridge.getBoardIoConfig();
      for (const binding of ioConfig) {
        if (binding.kind !== 'pwm_output' || !map[binding.id]) continue;
        try {
          const snap = bridge.getPeripheralSnapshot(binding.peripheral) as
            { psc?: number; arr?: number; cnt?: number } | null;
          if (snap && typeof snap.arr === 'number' && snap.arr > 0) {
            const clockHz = 8_000_000;
            const freq = Math.round(clockHz / ((snap.psc ?? 0) + 1) / (snap.arr + 1));
            map[binding.id].frequency = freq;
            if (freq >= 40 && freq <= 60) {
              map[binding.id].angle = map[binding.id].active ? 90 : 0;
            }
          }
        } catch {
          // Peripheral might not support snapshot
        }
      }
    }

    return map;
  }, [simState.boardIoStates, analogStates, bridge]);

  // Interactive analog component handler
  const handleAnalogChange = useCallback(
    (partId: string, value: number) => {
      if (!bridge) return;
      const config = bridge.getBoardIoConfig();
      const binding = config.find((b) => b.id === partId);
      if (binding) {
        bridge.setAdcValue(binding.peripheral, value);
      }
    },
    [bridge],
  );

  // Editor: add part from palette
  const handleAddPartFromPalette = useCallback(
    (type: string) => {
      const def = COMPONENT_REGISTRY.get(type);
      if (!def) return;
      const part: Part = {
        id: nextPartId(type), type, x: 400, y: 200, rotate: 0,
        attrs: { ...def.defaultAttrs },
      };
      editor.addPart(part);
    },
    [editor],
  );

  const handleDropPart = useCallback(
    (type: string, x: number, y: number) => {
      const def = COMPONENT_REGISTRY.get(type);
      if (!def) return;
      const part: Part = {
        id: nextPartId(type), type, x, y, rotate: 0,
        attrs: { ...def.defaultAttrs },
      };
      editor.addPart(part);
    },
    [editor],
  );

  const selectedParts = editor.state.diagram.parts.filter((p) => editor.state.selectedIds.has(p.id));

  // Load from URL hash (sharing) or localStorage
  useEffect(() => {
    const hash = window.location.hash.slice(1);
    if (hash) {
      decodeProject(hash).then((project) => {
        if (project) {
          editor.loadDiagram(project.diagram);
          setSource(project.source);
          for (const p of project.diagram.parts) {
            const num = parseInt(p.id.replace(/\D/g, ''), 10);
            if (!isNaN(num) && num > partCounter) partCounter = num;
          }
          history.replaceState(null, '', window.location.pathname + window.location.search);
          return;
        }
      });
      return;
    }

    const saved = localStorage.getItem('labwired-diagram');
    if (saved) {
      try {
        const diagram = JSON.parse(saved) as Diagram;
        editor.loadDiagram(diagram);
        for (const p of diagram.parts) {
          const num = parseInt(p.id.replace(/\D/g, ''), 10);
          if (!isNaN(num) && num > partCounter) partCounter = num;
        }
      } catch { /* ignore */ }
    }
    const savedSource = localStorage.getItem('labwired-source');
    if (savedSource) setSource(savedSource);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    localStorage.setItem('labwired-diagram', JSON.stringify(editor.state.diagram));
  }, [editor.state.diagram]);

  useEffect(() => {
    localStorage.setItem('labwired-source', source);
  }, [source]);

  // Export/Import
  const handleExport = useCallback(() => {
    const data = { diagram: editor.state.diagram, source };
    const json = JSON.stringify(data, null, 2);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = 'project.json'; a.click();
    URL.revokeObjectURL(url);
  }, [editor.state.diagram, source]);

  const handleImport = useCallback(() => {
    const input = document.createElement('input');
    input.type = 'file'; input.accept = '.json';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      const text = await file.text();
      try {
        const data = JSON.parse(text);
        if (data.diagram) {
          editor.loadDiagram(data.diagram as Diagram);
          if (data.source) setSource(data.source);
        } else {
          editor.loadDiagram(data as Diagram);
        }
      } catch { alert('Invalid project file'); }
    };
    input.click();
  }, [editor]);

  // Share
  const handleShare = useCallback(async () => {
    const url = await generateShareUrl(editor.state.diagram, source);
    await navigator.clipboard.writeText(url);
    setCompileOutput('Share URL copied to clipboard!');
    setBottomTab('output');
    setShowBottomPanel(true);
  }, [editor.state.diagram, source]);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      if ((e.target as HTMLElement).closest('.monaco-editor')) return;

      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (editor.state.selectedIds.size > 0) {
          editor.deleteSelected();
        }
      }
      if (e.key === 'r' || e.key === 'R') {
        if (editor.state.selectedIds.size === 1) {
          editor.rotatePart([...editor.state.selectedIds][0]);
        }
      }
      if (e.ctrlKey && e.shiftKey && e.key === 'Z') {
        e.preventDefault(); editor.redo();
      } else if (e.ctrlKey && e.key === 'z') {
        e.preventDefault(); editor.undo();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [editor]);

  if (loading) {
    return (
      <div className="loading-overlay">
        <div className="spinner" />
        <span>{compiling ? 'Compiling...' : 'Loading simulator engine...'}</span>
      </div>
    );
  }

  // Bottom tab content
  const bottomContent = () => {
    switch (bottomTab) {
      case 'output':
        return <pre className="compile-output">{compileOutput || 'Ready to compile.'}</pre>;
      case 'serial':
        return <SerialMonitor output={simState.uartOutput} onClear={clearUart} style={{ height: '100%' }} />;
      case 'registers':
        return <RegisterGrid registers={registers} style={{ maxHeight: '100%', overflow: 'auto' }} />;
      case 'trace':
        return <InstructionTrace entries={traceEntries} style={{ maxHeight: '100%', overflow: 'auto' }} />;
      case 'memory':
        return <MemoryInspector data={stackMemory} baseAddress={stackBase} style={{ maxHeight: '100%', overflow: 'auto' }} />;
    }
  };

  return (
    <div className="playground">
      {/* ===== Header ===== */}
      {!embed && (
        <div className="playground-header">
          <span className="logo">LabWired</span>

          {/* Example sketches */}
          <select
            className="project-selector"
            value=""
            onChange={(e) => {
              const sketch = EXAMPLE_SKETCHES.find((s) => s.name === e.target.value);
              if (sketch) setSource(sketch.source);
            }}
          >
            <option value="" disabled>Examples...</option>
            {EXAMPLE_SKETCHES.map((s) => (
              <option key={s.name} value={s.name}>{s.name}</option>
            ))}
          </select>

          {/* Demo project selector */}
          <select
            className="project-selector"
            value={selectedProject.id}
            onChange={(e) => {
              const proj = DEMO_PROJECTS.find((p) => p.id === e.target.value);
              if (proj) setSelectedProject(proj);
            }}
          >
            {DEMO_PROJECTS.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>

          <div className="header-separator" />

          <button className="toolbar-btn toolbar-btn-verify" onClick={handleCompile} disabled={compiling}>
            {compiling ? 'Compiling...' : 'Verify'}
          </button>
          <button className="toolbar-btn" onClick={handleRun} disabled={compiling || loading}>
            Upload
          </button>

          {/* Sim controls inline when simulation is active */}
          {simActive && (
            <>
              <div className="header-separator" />
              <SimControls
                running={running}
                onPlay={handlePlay}
                onPause={handlePause}
                onStep={handleStep}
                onReset={handleReset}
                pc={simState.pc}
                cycles={simState.cycles}
              />
              <button className="toolbar-btn toolbar-btn-ghost toolbar-btn-stop" onClick={handleStop}>
                Stop
              </button>
            </>
          )}

          <div className="header-spacer" />

          <button
            className={`toolbar-btn toolbar-btn-ghost ${showCode ? 'active' : ''}`}
            onClick={() => setShowCode(!showCode)}
            title="Toggle code editor"
          >
            Code
          </button>
          <button className="toolbar-btn toolbar-btn-ghost" onClick={handleShare}>Share</button>
          <button className="toolbar-btn toolbar-btn-ghost" onClick={handleExport}>Export</button>
          <button className="toolbar-btn toolbar-btn-ghost" onClick={handleImport}>Import</button>
          <button
            className="toolbar-btn toolbar-btn-ghost"
            onClick={editor.undo}
            disabled={editor.state.undoStack.length === 0}
          >
            Undo
          </button>
          <button
            className="toolbar-btn toolbar-btn-ghost"
            onClick={editor.redo}
            disabled={editor.state.redoStack.length === 0}
          >
            Redo
          </button>

          {error && <span className="header-error">{error}</span>}
        </div>
      )}

      {/* ===== Unified Layout ===== */}
      <div className="editor-layout">
        {/* Component palette (left sidebar) */}
        <div className="editor-sidebar-left">
          <ComponentPalette onAddPart={handleAddPartFromPalette} />
        </div>

        {/* Main content area */}
        <div className="editor-center">
          <div className="editor-top-split">
            {/* Code editor (left pane) */}
            {showCode && (
              <div className="editor-code-pane">
                <CodeEditor
                  source={source}
                  onChange={setSource}
                  errors={compileErrors}
                />
              </div>
            )}
            {/* Circuit canvas (always visible — shows live state during sim) */}
            <div className="editor-canvas-pane">
              <EditorCanvas
                state={editor.state}
                boardIoStates={boardIoStateMap}
                onMovePart={editor.movePart}
                onResizePart={editor.resizePart}
                onSelect={editor.select}
                onSelectRect={editor.selectRect}
                onStartWire={editor.startWire}
                onCompleteWire={editor.completeWire}
                onCancelWire={editor.cancelWire}
                onDeleteWire={editor.deleteWire}
                onDropPart={handleDropPart}
                onButtonToggle={handleButtonToggle}
                onAnalogChange={handleAnalogChange}
              />
            </div>
          </div>

          {/* Bottom panel — tabbed: output / serial / registers / trace / memory */}
          {showBottomPanel && (
            <div className="editor-bottom-pane">
              <div className="bottom-tabs">
                {(['output', 'serial', 'registers', 'trace', 'memory'] as BottomTab[]).map((tab) => (
                  <button
                    key={tab}
                    className={`bottom-tab ${bottomTab === tab ? 'active' : ''} ${
                      !simActive && tab !== 'output' && tab !== 'serial' ? 'disabled' : ''
                    }`}
                    onClick={() => setBottomTab(tab)}
                    disabled={!simActive && tab !== 'output' && tab !== 'serial'}
                  >
                    {tab === 'output' ? 'Output' :
                     tab === 'serial' ? 'Serial' :
                     tab === 'registers' ? 'Registers' :
                     tab === 'trace' ? 'Trace' : 'Memory'}
                  </button>
                ))}
                <button
                  className="bottom-tab bottom-tab-close"
                  onClick={() => setShowBottomPanel(false)}
                  title="Hide panel"
                >
                  ×
                </button>
              </div>
              <div className="bottom-content">
                {bottomContent()}
              </div>
            </div>
          )}

          {/* Collapsed bottom panel toggle */}
          {!showBottomPanel && (
            <button
              className="bottom-panel-toggle"
              onClick={() => setShowBottomPanel(true)}
            >
              Output / Serial / Debug
            </button>
          )}
        </div>

        {/* Property panel (right sidebar) */}
        <div className="editor-sidebar-right">
          <PropertyPanel
            parts={selectedParts}
            onUpdateAttrs={editor.updateAttrs}
            onDelete={editor.deleteSelected}
            onRotate={editor.rotatePart}
            onResize={editor.resizePart}
          />
        </div>
      </div>
    </div>
  );
}
