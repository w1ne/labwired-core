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
  validateDiagram,
  validateWireConnection,
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
import { BOARD_CONFIGS, type BoardConfig } from './bundled-configs';
import { fetchCatalog, type CatalogEntry } from './catalog-client';
import { BoardPicker } from './BoardPicker';
import {
  CheckIcon, UploadIcon, CodeIcon, PanelBottomIcon,
  ShareIcon, ExportIcon, ImportIcon, UndoIcon, RedoIcon,
  StopIcon, SidebarLeftIcon, SidebarRightIcon,
  ChevronLeftIcon, ChevronRightIcon,
} from './Icons';

type BottomTab = 'output' | 'serial' | 'registers' | 'trace' | 'memory';
type WorkspaceKind = 'diagram' | 'source';

let partCounter = 0;
function nextPartId(type: string): string {
  return `${type}${++partCounter}`;
}

function getWorkspaceStorageKey(boardId: string, kind: WorkspaceKind): string {
  return `labwired-${kind}:${boardId}`;
}

function hasSavedWorkspace(boardId: string): boolean {
  return !!(
    localStorage.getItem(getWorkspaceStorageKey(boardId, 'diagram'))
    || localStorage.getItem(getWorkspaceStorageKey(boardId, 'source'))
  );
}

function makeStarterDiagram(config: BoardConfig): Diagram {
  const mcu: Part = {
    id: 'mcu',
    type: config.mcuComponentType,
    x: 100,
    y: 100,
    rotate: 0,
    attrs: {},
  };

  if (config.boardId === 'stm32f103-blinky') {
    return {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'led_pa5', type: 'led', x: 390, y: 90, rotate: 0, scale: 1.5, attrs: { color: 'green' } },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led_pa5', pin: 'A' },
          color: '#27c93f',
        },
      ],
    };
  }

  if (config.boardId === 'nucleo-f401re') {
    return {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'led2_pa5', type: 'led', x: 390, y: 90, rotate: 0, scale: 1.5, attrs: { color: 'green' } },
        { id: 'button_user_pc13', type: 'button', x: 300, y: -20, rotate: 0, scale: 1.35, attrs: {} },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led2_pa5', pin: 'A' },
          color: '#27c93f',
        },
        {
          from: { part: 'mcu', pin: 'PC13' },
          to: { part: 'button_user_pc13', pin: '1' },
          color: '#569cd6',
        },
      ],
    };
  }

  return {
    ...createEmptyDiagram(config.chipId),
    parts: [mcu],
    wires: [],
  };
}

function getDefaultSource(config: BoardConfig): string {
  if (config.boardId === 'nucleo-f401re') {
    return EXAMPLE_SKETCHES.find((sketch) => sketch.name === 'Button + LED')?.source ?? EXAMPLE_SKETCHES[0].source;
  }
  return EXAMPLE_SKETCHES.find((sketch) => sketch.name === 'Blink')?.source ?? EXAMPLE_SKETCHES[0].source;
}

function loadBoardWorkspace(config: BoardConfig): { diagram: Diagram; source: string } {
  const savedDiagram = localStorage.getItem(getWorkspaceStorageKey(config.boardId, 'diagram'));
  const savedSource = localStorage.getItem(getWorkspaceStorageKey(config.boardId, 'source'));

  let diagram = makeStarterDiagram(config);
  if (savedDiagram) {
    try {
      diagram = JSON.parse(savedDiagram) as Diagram;
    } catch {
      diagram = makeStarterDiagram(config);
    }
  }

  return {
    diagram,
    source: savedSource ?? getDefaultSource(config),
  };
}

const DEFAULT_BOARD = BOARD_CONFIGS[0]; // stm32f103-blinky
const DEMO_AUTOSTART_KEY = 'labwired-demo-autostart-v1';

export function App() {
  const [wasmModule, setWasmModule] = useState<WasmModule | null>(null);
  const [bridge, setBridge] = useState<SimulatorBridge | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const traceRef = useRef<TraceEntry[]>([]);
  const [traceEntries, setTraceEntries] = useState<TraceEntry[]>([]);
  const [canvasValidationMessage, setCanvasValidationMessage] = useState<string | null>(null);
  const [invalidPins, setInvalidPins] = useState<Array<{ part: string; pin: string }>>([]);

  // Board selection (from catalog + bundled configs)
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [selectedBoard, setSelectedBoard] = useState<BoardConfig>(() => {
    const savedId = localStorage.getItem('labwired-board');
    if (savedId) {
      const found = BOARD_CONFIGS.find((c) => c.boardId === savedId);
      if (found) return found;
    }
    return DEFAULT_BOARD;
  });

  // Code editor state
  const [source, setSource] = useState(() => loadBoardWorkspace(selectedBoard).source);
  const [compileErrors, setCompileErrors] = useState<CompileError[]>([]);
  const [compileOutput, setCompileOutput] = useState('');
  const [compiling, setCompiling] = useState(false);
  const [bottomTab, setBottomTab] = useState<BottomTab>('output');
  const [showCode, setShowCode] = useState(true);
  const [showBottomPanel, setShowBottomPanel] = useState(true);
  const [showLeftSidebar, setShowLeftSidebar] = useState(true);
  const [showRightSidebar, setShowRightSidebar] = useState(true);
  const embed = isEmbedMode();
  const autostartTriggeredRef = useRef(false);

  // Editor state
  const editor = useEditorState(
    loadBoardWorkspace(selectedBoard).diagram,
  );

  // Whether simulation has been loaded (bridge exists)
  const simActive = !!bridge;

  // Fetch catalog on mount
  useEffect(() => {
    fetchCatalog().then(setCatalog);
  }, []);

  // Persist selected board
  useEffect(() => {
    localStorage.setItem('labwired-board', selectedBoard.boardId);
  }, [selectedBoard]);

  // Handle board selection
  const handleBoardSelect = useCallback(
    (config: BoardConfig) => {
      const workspace = loadBoardWorkspace(config);
      setSelectedBoard(config);
      editor.loadDiagram(workspace.diagram);
      setSource(workspace.source);
      setCanvasValidationMessage(null);
      setInvalidPins([]);
      // Stop any running simulation
      setRunning(false);
      setBridge(null);
    },
    [editor],
  );

  // Load WASM module lazily
  const loadWasm = useCallback(async () => {
    if (wasmModule) return wasmModule;
    const wasmUrl = new URL(`${import.meta.env.BASE_URL}wasm/labwired_wasm.js`, window.location.origin);
    wasmUrl.searchParams.set('v', String(__BUILD_TIME__));
    const mod = await import(/* @vite-ignore */ wasmUrl.href);
    await mod.default();
    setWasmModule(mod as WasmModule);
    return mod as WasmModule;
  }, [wasmModule]);

  // Compile source code
  const handleCompile = useCallback(async () => {
    const diagramErrors = validateDiagram(editor.state.diagram);
    if (diagramErrors.length > 0) {
      setCanvasValidationMessage(diagramErrors[0]);
      setInvalidPins([]);
      setCompileErrors([]);
      setCompileOutput(`Wiring error: ${diagramErrors[0]}`);
      setBottomTab('output');
      setShowBottomPanel(true);
      return null;
    }

    setCanvasValidationMessage(null);
    setInvalidPins([]);
    setCompiling(true);
    setCompileOutput('Compiling...\n');
    setCompileErrors([]);
    setBottomTab('output');
    setShowBottomPanel(true);
    try {
      const result = await compileCode({
        source,
        language: 'arduino',
        target: selectedBoard.chipId,
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
  }, [editor.state.diagram, source, selectedBoard.chipId]);

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
      let systemYaml: string;
      let chipYaml: string;

      if (result?.success && result.elf) {
        firmware = result.elf;
        // Use diagram-derived config with the selected board's chip YAML
        const config = diagramToConfig(editor.state.diagram, selectedBoard.chipYaml);
        systemYaml = config.systemYaml;
        chipYaml = config.chipYaml;
        setCompileOutput((prev) => prev + '\nUpload successful. Starting simulation...');
      } else if (selectedBoard.demoFirmwarePath) {
        // Fall back to pre-built demo firmware with its matching YAML configs
        const resp = await fetch(selectedBoard.demoFirmwarePath);
        if (!resp.ok) throw new Error(`Failed to load firmware: ${selectedBoard.demoFirmwarePath}`);
        firmware = new Uint8Array(await resp.arrayBuffer());
        systemYaml = selectedBoard.systemYaml;
        chipYaml = selectedBoard.chipYaml;
        setCompileOutput((prev) => prev + '\nUsing pre-built demo firmware.');
      } else {
        // No demo firmware and compile failed
        setCompileOutput(
          (prev) => prev + `\nNo pre-built firmware for ${selectedBoard.name}. Write code and compile it first.`,
        );
        setLoading(false);
        return;
      }

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
  }, [handleCompile, loadWasm, selectedBoard, editor.state.diagram]);

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

  const handleCompleteWire = useCallback((endpoint: { part: string; pin: string }) => {
    if (!editor.state.wireInProgress) return;
    const errorMessage = validateWireConnection(editor.state.diagram, editor.state.wireInProgress, endpoint);
    if (errorMessage) {
      editor.cancelWire();
      setCanvasValidationMessage(errorMessage);
      setInvalidPins([editor.state.wireInProgress, endpoint]);
      setCompileOutput(`Wiring error: ${errorMessage}`);
      setBottomTab('output');
      setShowBottomPanel(true);
      return;
    }
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    editor.completeWire(endpoint);
  }, [editor]);

  const handleStartWire = useCallback((endpoint: { part: string; pin: string }) => {
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    editor.startWire(endpoint);
  }, [editor]);

  const handleCancelWire = useCallback(() => {
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    editor.cancelWire();
  }, [editor]);

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
  const currentExample = EXAMPLE_SKETCHES.find((sketch) => sketch.source === source) ?? null;
  const boardSummary = useMemo(() => {
    const componentCount = Math.max(editor.state.diagram.parts.length - 1, 0);
    const wireCount = editor.state.diagram.wires.length;
    if (selectedBoard.boardId === 'stm32f103-blinky') {
      return {
        title: 'STM32 demo circuit',
        description: 'PA5 drives the onboard LED. Upload runs the bundled blinky immediately.',
        nextStep: simActive ? 'Simulation is running. Watch the LED state and serial pane.' : 'Click Run Demo to boot the bundled STM32 blinky.',
      };
    }
    if (selectedBoard.boardId === 'nucleo-f401re') {
      return {
        title: 'LED + button starter',
        description: 'PA5 drives the LED and PC13 is wired to the user button.',
        nextStep: simActive ? 'Simulation is running. Press the button component to interact.' : 'Click Run Demo to boot the starter circuit.',
      };
    }
    return {
      title: `${selectedBoard.name} starter`,
      description: `${componentCount} components and ${wireCount} wires on the canvas.`,
      nextStep: selectedBoard.demoFirmwarePath
        ? 'Click Run Demo to boot the bundled firmware.'
        : 'Wire a circuit, compile the sketch, then run it.',
    };
  }, [editor.state.diagram.parts.length, editor.state.diagram.wires.length, selectedBoard, simActive]);

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

    const workspace = loadBoardWorkspace(selectedBoard);
    editor.loadDiagram(workspace.diagram);
    setSource(workspace.source);
    for (const p of workspace.diagram.parts) {
      const num = parseInt(p.id.replace(/\D/g, ''), 10);
      if (!isNaN(num) && num > partCounter) partCounter = num;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (autostartTriggeredRef.current || embed) return;
    const hash = window.location.hash.slice(1);
    if (hash) return;
    if (selectedBoard.boardId !== DEFAULT_BOARD.boardId) return;
    if (hasSavedWorkspace(selectedBoard.boardId)) return;
    if (localStorage.getItem(DEMO_AUTOSTART_KEY)) return;

    autostartTriggeredRef.current = true;
    localStorage.setItem(DEMO_AUTOSTART_KEY, '1');
    void handleRun();
  }, [embed, handleRun, selectedBoard.boardId]);

  useEffect(() => {
    localStorage.setItem(
      getWorkspaceStorageKey(selectedBoard.boardId, 'diagram'),
      JSON.stringify(editor.state.diagram),
    );
  }, [editor.state.diagram, selectedBoard.boardId]);

  useEffect(() => {
    localStorage.setItem(getWorkspaceStorageKey(selectedBoard.boardId, 'source'), source);
  }, [source, selectedBoard.boardId]);

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

  const handleResetDemo = useCallback(() => {
    const starter = makeStarterDiagram(selectedBoard);
    localStorage.removeItem(getWorkspaceStorageKey(selectedBoard.boardId, 'diagram'));
    localStorage.removeItem(getWorkspaceStorageKey(selectedBoard.boardId, 'source'));
    editor.loadDiagram(starter);
    setSource(getDefaultSource(selectedBoard));
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    setCompileErrors([]);
    setCompileOutput(`Restored ${selectedBoard.name} demo workspace.`);
    setBottomTab('output');
    setShowBottomPanel(true);
    setRunning(false);
    setBridge(null);
  }, [editor, selectedBoard]);

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
          {/* --- Project group --- */}
          <div className="toolbar-group">
            <span className="logo">LabWired</span>
            <BoardPicker
              catalog={catalog}
              selectedBoardId={selectedBoard.boardId}
              onSelect={handleBoardSelect}
            />
            <select
              className="project-selector"
              value={currentExample?.name ?? ''}
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
          </div>

          <div className="header-separator" />

          {/* --- Build group --- */}
          <div className="toolbar-group">
            <button className="toolbar-btn toolbar-btn-primary toolbar-btn-verify" onClick={handleCompile} disabled={compiling}>
              <CheckIcon size={14} /> {compiling ? 'Checking...' : 'Check Wiring'}
            </button>
            <button className="toolbar-btn toolbar-btn-primary" onClick={handleRun} disabled={compiling || loading}>
              <UploadIcon size={14} /> {selectedBoard.demoFirmwarePath ? 'Run Demo' : 'Run Circuit'}
            </button>
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleResetDemo} title="Reset starter workspace">
              Reset Demo
            </button>
          </div>

          {/* --- Sim controls (inline when active) --- */}
          {simActive && (
            <>
              <div className="header-separator" />
              <div className="toolbar-group">
                <SimControls
                  variant="dark"
                  running={running}
                  onPlay={handlePlay}
                  onPause={handlePause}
                  onStep={handleStep}
                  onReset={handleReset}
                  pc={simState.pc}
                  cycles={simState.cycles}
                />
                <button className="toolbar-btn toolbar-btn-ghost toolbar-btn-stop" onClick={handleStop} title="Stop simulation">
                  <StopIcon size={14} />
                </button>
              </div>
            </>
          )}

          <div className="header-spacer" />

          {/* --- View group --- */}
          <div className="toolbar-group">
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showCode ? 'active' : ''}`}
              onClick={() => setShowCode(!showCode)}
              title="Toggle code editor"
            >
              <CodeIcon size={14} />
            </button>
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showBottomPanel ? 'active' : ''}`}
              onClick={() => setShowBottomPanel(!showBottomPanel)}
              title="Toggle bottom panel"
            >
              <PanelBottomIcon size={14} />
            </button>
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showLeftSidebar ? 'active' : ''}`}
              onClick={() => setShowLeftSidebar(!showLeftSidebar)}
              title="Toggle components panel"
            >
              <SidebarLeftIcon size={14} />
            </button>
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showRightSidebar ? 'active' : ''}`}
              onClick={() => setShowRightSidebar(!showRightSidebar)}
              title="Toggle properties panel"
            >
              <SidebarRightIcon size={14} />
            </button>
          </div>

          <div className="header-separator" />

          {/* --- File group --- */}
          <div className="toolbar-group">
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleShare} title="Share project">
              <ShareIcon size={14} />
            </button>
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleExport} title="Export project">
              <ExportIcon size={14} />
            </button>
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleImport} title="Import project">
              <ImportIcon size={14} />
            </button>
          </div>

          <div className="header-separator" />

          {/* --- History group --- */}
          <div className="toolbar-group">
            <button
              className="toolbar-btn toolbar-btn-ghost"
              onClick={editor.undo}
              disabled={editor.state.undoStack.length === 0}
              title="Undo (Ctrl+Z)"
            >
              <UndoIcon size={14} />
            </button>
            <button
              className="toolbar-btn toolbar-btn-ghost"
              onClick={editor.redo}
              disabled={editor.state.redoStack.length === 0}
              title="Redo (Ctrl+Shift+Z)"
            >
              <RedoIcon size={14} />
            </button>
          </div>

          {error && <span className="header-error">{error}</span>}
        </div>
      )}

      {/* ===== Unified Layout ===== */}
      <div className="editor-layout">
        {/* Component palette (left sidebar) */}
        {showLeftSidebar && (
          <div className="editor-sidebar-left">
            <ComponentPalette onAddPart={handleAddPartFromPalette} />
          </div>
        )}

        {/* Collapsed left sidebar tab */}
        {!showLeftSidebar && (
          <button
            className="sidebar-toggle sidebar-toggle-left"
            onClick={() => setShowLeftSidebar(true)}
            title="Show components"
          >
            <ChevronRightIcon size={12} />
          </button>
        )}

        {/* Main content area */}
        <div className="editor-center">
          <div className="demo-banner">
            <div className="demo-banner-copy">
              <span className="demo-banner-kicker">{boardSummary.title}</span>
              <strong>{selectedBoard.name}</strong>
              <span>{boardSummary.description}</span>
              <span className="demo-banner-next">{boardSummary.nextStep}</span>
            </div>
            <div className="demo-banner-stats">
              <span className="demo-stat"><strong>{Math.max(editor.state.diagram.parts.length - 1, 0)}</strong> parts</span>
              <span className="demo-stat"><strong>{editor.state.diagram.wires.length}</strong> wires</span>
              <span className={`demo-stat ${simActive ? 'live' : ''}`}><strong>{simActive ? 'Live' : 'Idle'}</strong> sim</span>
            </div>
          </div>
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
                validationMessage={canvasValidationMessage}
                invalidPins={invalidPins}
                onMovePart={editor.movePart}
                onResizePart={editor.resizePart}
                onSelect={editor.select}
                onSelectRect={editor.selectRect}
                onStartWire={handleStartWire}
                onCompleteWire={handleCompleteWire}
                onCancelWire={handleCancelWire}
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
                  &times;
                </button>
              </div>
              <div className="bottom-content">
                {bottomContent()}
              </div>
            </div>
          )}
        </div>

        {/* Property panel (right sidebar) */}
        {showRightSidebar && (
          <div className="editor-sidebar-right">
            <PropertyPanel
              parts={selectedParts}
              onUpdateAttrs={editor.updateAttrs}
              onDelete={editor.deleteSelected}
              onRotate={editor.rotatePart}
              onResize={editor.resizePart}
            />
          </div>
        )}

        {/* Collapsed right sidebar tab */}
        {!showRightSidebar && (
          <button
            className="sidebar-toggle sidebar-toggle-right"
            onClick={() => setShowRightSidebar(true)}
            title="Show properties"
          >
            <ChevronLeftIcon size={12} />
          </button>
        )}
      </div>

      {/* ===== Loading overlay (on top of UI, not replacing it) ===== */}
      {loading && (
        <div className="loading-overlay">
          <div className="spinner" />
          <span>{compiling ? 'Compiling...' : 'Loading simulator engine...'}</span>
        </div>
      )}
    </div>
  );
}
