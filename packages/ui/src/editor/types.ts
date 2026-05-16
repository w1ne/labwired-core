import type { ReactNode } from 'react';

// -- Diagram schema (serializable to JSON) --

export interface Diagram {
  version: 1;
  board: string;
  parts: Part[];
  wires: Wire[];
}

export interface Part {
  id: string;
  type: string;
  x: number;
  y: number;
  rotate: number;
  scale?: number;
  attrs: Record<string, string>;
}

export interface WireEndpoint {
  part: string;
  pin: string;
}

export interface Wire {
  from: WireEndpoint;
  to: WireEndpoint;
  color: string;
  /** Orthogonal bend points between source and target pins. */
  waypoints?: { x: number; y: number }[];
}

// -- Component definitions (not serialized) --

export type PinSide = 'left' | 'right' | 'top' | 'bottom';

export interface PinDef {
  id: string;
  x: number;
  y: number;
  side: PinSide;
  label?: string;
}

export interface ComponentDef {
  type: string;
  label: string;
  category: 'output' | 'input' | 'passive' | 'mcu' | 'sensor' | 'display' | 'ic';
  width: number;
  height: number;
  pins: PinDef[];
  render: (attrs: Record<string, string>, state?: ComponentState) => ReactNode;
  defaultAttrs: Record<string, string>;
  /** If set, this component maps to a board_io binding in the simulator. */
  boardIoKind?: 'led' | 'button' | 'adc_input' | 'pwm_output' | 'i2c_device' | 'spi_device';
  /** Attribute fields shown in PropertyPanel. */
  attrFields?: AttrFieldDef[];
}

export interface ComponentState {
  active?: boolean;
  selected?: boolean;
  analogValue?: number;
  displayText?: string;
  frequency?: number;
  angle?: number;
  /** Live framebuffer from a simulated display peripheral, if present. */
  displayBuffer?: DisplayBuffer;
}

/**
 * Snapshot of a simulated display's framebuffer, poll-fetched from the wasm
 * sim each frame. `kind` selects how `data` is interpreted:
 *   - `ssd1680_tricolor_290`: 9472 bytes = 4736 black plane | 4736 red plane,
 *     128 px wide × 296 px tall native (portrait), MSB-first packing,
 *     wire encoding (1=white/no-ink, 0=ink) — render layer must compose.
 */
export type DisplayBuffer =
  | {
      kind: 'ssd1680_tricolor_290';
      generation: number;
      data: Uint8Array;
    };

export interface AttrFieldDef {
  key: string;
  label: string;
  type: 'text' | 'select' | 'color';
  options?: string[];
}

// -- Editor state --

export interface EditorState {
  diagram: Diagram;
  selectedIds: Set<string>;
  /** Wire currently being drawn (source pin clicked, waiting for target). */
  wireInProgress: WireEndpoint | null;
  /** Undo history (previous diagram snapshots). */
  undoStack: Diagram[];
  /** Redo stack (cleared on any mutation). */
  redoStack: Diagram[];
}

export type EditorAction =
  | { type: 'ADD_PART'; part: Part }
  | { type: 'MOVE_PART'; id: string; x: number; y: number }
  | { type: 'ROTATE_PART'; id: string }
  | { type: 'DELETE_SELECTED' }
  | { type: 'UPDATE_ATTRS'; id: string; attrs: Record<string, string> }
  | { type: 'START_WIRE'; endpoint: WireEndpoint }
  | { type: 'COMPLETE_WIRE'; endpoint: WireEndpoint; color: string }
  | { type: 'CANCEL_WIRE' }
  | { type: 'DELETE_WIRE'; index: number }
  | { type: 'SELECT'; id: string | null; add?: boolean }
  | { type: 'SELECT_RECT'; ids: string[] }
  | { type: 'LOAD_DIAGRAM'; diagram: Diagram }
  | { type: 'RESIZE_PART'; id: string; scale: number }
  | { type: 'UNDO' }
  | { type: 'REDO' };

// -- Helpers --

export function createEmptyDiagram(board = 'stm32f103'): Diagram {
  return { version: 1, board, parts: [], wires: [] };
}

const WIRE_COLORS = ['#e83e8c', '#27c93f', '#569cd6', '#ffcc00', '#ff6633', '#00cccc'];
let wireColorIndex = 0;

export function nextWireColor(): string {
  const color = WIRE_COLORS[wireColorIndex % WIRE_COLORS.length];
  wireColorIndex++;
  return color;
}
