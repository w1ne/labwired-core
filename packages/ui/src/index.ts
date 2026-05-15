// Components
export { BoardCanvas } from './components/BoardCanvas/BoardCanvas';
export { Led } from './components/Led/Led';
export { PushButton } from './components/PushButton/PushButton';
export { SimControls } from './components/SimControls/SimControls';
export { RegisterGrid } from './components/RegisterGrid/RegisterGrid';
export { MemoryInspector } from './components/MemoryInspector/MemoryInspector';
export { InstructionTrace } from './components/InstructionTrace/InstructionTrace';
export { SerialMonitor } from './components/SerialMonitor/SerialMonitor';
export { Adxl345Visualizer } from './components/Adxl345Visualizer/Adxl345Visualizer';
export { Ssd1306Display } from './components/Ssd1306Display/Ssd1306Display';
export type { Ssd1306DisplayProps } from './components/Ssd1306Display/Ssd1306Display';

// Hooks
export { useSimulator } from './hooks/useSimulator';
export { useSimulationLoop } from './hooks/useSimulationLoop';

// WASM bridge
export { SimulatorBridge } from './wasm/simulator-bridge';

// Types
export type { BoardCanvasProps } from './components/BoardCanvas/BoardCanvas';
export type { LedProps } from './components/Led/Led';
export type { PushButtonProps } from './components/PushButton/PushButton';
export type { SimControlsProps } from './components/SimControls/SimControls';
export type { RegisterGridProps } from './components/RegisterGrid/RegisterGrid';
export type { MemoryInspectorProps } from './components/MemoryInspector/MemoryInspector';
export type { InstructionTraceProps, TraceEntry } from './components/InstructionTrace/InstructionTrace';
export type { SerialMonitorProps } from './components/SerialMonitor/SerialMonitor';
export type { Adxl345Sample, Adxl345VisualizerProps } from './components/Adxl345Visualizer/Adxl345Visualizer';
export type { GuidedLabProps, GuidedLabStep } from './components/GuidedLab/GuidedLab';
export type { UseSimulatorOptions, UseSimulatorResult } from './hooks/useSimulator';
export type { UseSimulationLoopOptions, UseSimulationLoopResult, SimulationState } from './hooks/useSimulationLoop';
export type {
  AnalogState,
  BoardIoBinding,
  BoardIoState,
  I2cSensorState,
  PeripheralInfo,
  SimulatorConfig,
  WasmModule,
} from './wasm/simulator-bridge';

// Editor
export { EditorCanvas } from './editor/EditorCanvas';
export { WireLayer } from './editor/WireLayer';
export { ComponentPalette } from './editor/ComponentPalette';
export { PropertyPanel } from './editor/PropertyPanel';
export { useEditorState } from './editor/useEditorState';
export { diagramToConfig } from './editor/diagramToConfig';
export { validateDiagram, validateWireConnection } from './editor/circuitValidation';
export { routeWire } from './editor/wire-router';
export { getPinMapping, findPinFunction } from './editor/pin-mapping';
export type { PinFunction, PinMapping } from './editor/pin-mapping';
export { COMPONENT_REGISTRY, getComponentsByCategory } from './editor/components/index';
export type {
  Diagram, Part, Wire, WireEndpoint, PinDef, PinSide,
  ComponentDef, ComponentState, AttrFieldDef,
  EditorState, EditorAction,
} from './editor/types';
export { createEmptyDiagram, nextWireColor } from './editor/types';
export { CodeEditor } from './editor/CodeEditor';
export type { CompileError } from './editor/CodeEditor';
export { compileCode, EXAMPLE_SKETCHES } from './editor/compile-service';
export type { CompileResult, CompileOptions } from './editor/compile-service';
export { startTone, stopTone, resumeAudio } from './editor/audio-engine';
export { encodeProject, decodeProject, isEmbedMode, generateShareUrl, generateEmbedUrl } from './editor/sharing';

/**
 * @deprecated Replaced by `InspectorCard` in the Playground Studio shell. Will be removed in a future release.
 */
export { GuidedLab } from './components/GuidedLab/GuidedLab';
