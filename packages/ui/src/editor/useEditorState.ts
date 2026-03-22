import { useReducer, useCallback } from 'react';
import type { EditorState, EditorAction, Diagram, Part, WireEndpoint } from './types';
import { createEmptyDiagram, nextWireColor } from './types';

const MAX_UNDO = 50;
const EMPTY_SET = new Set<string>();

function pushUndo(state: EditorState): EditorState {
  return {
    ...state,
    undoStack: [...state.undoStack.slice(-(MAX_UNDO - 1)), structuredClone(state.diagram)],
    redoStack: [],
  };
}

function editorReducer(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case 'ADD_PART': {
      const s = pushUndo(state);
      return {
        ...s,
        diagram: { ...s.diagram, parts: [...s.diagram.parts, action.part] },
        selectedIds: new Set([action.part.id]),
      };
    }
    case 'MOVE_PART': {
      return {
        ...state,
        diagram: {
          ...state.diagram,
          parts: state.diagram.parts.map((p) =>
            p.id === action.id ? { ...p, x: action.x, y: action.y } : p,
          ),
        },
      };
    }
    case 'ROTATE_PART': {
      const s = pushUndo(state);
      return {
        ...s,
        diagram: {
          ...s.diagram,
          parts: s.diagram.parts.map((p) =>
            p.id === action.id ? { ...p, rotate: (p.rotate + 90) % 360 } : p,
          ),
        },
      };
    }
    case 'RESIZE_PART': {
      const s = pushUndo(state);
      return {
        ...s,
        diagram: {
          ...s.diagram,
          parts: s.diagram.parts.map((p) =>
            p.id === action.id ? { ...p, scale: action.scale } : p,
          ),
        },
      };
    }
    case 'DELETE_SELECTED': {
      if (state.selectedIds.size === 0) return state;
      const s = pushUndo(state);
      const ids = s.selectedIds;
      return {
        ...s,
        diagram: {
          ...s.diagram,
          parts: s.diagram.parts.filter((p) => !ids.has(p.id)),
          wires: s.diagram.wires.filter((w) => !ids.has(w.from.part) && !ids.has(w.to.part)),
        },
        selectedIds: EMPTY_SET,
      };
    }
    case 'UPDATE_ATTRS': {
      const s = pushUndo(state);
      return {
        ...s,
        diagram: {
          ...s.diagram,
          parts: s.diagram.parts.map((p) =>
            p.id === action.id ? { ...p, attrs: { ...p.attrs, ...action.attrs } } : p,
          ),
        },
      };
    }
    case 'START_WIRE': {
      return { ...state, wireInProgress: action.endpoint };
    }
    case 'COMPLETE_WIRE': {
      if (!state.wireInProgress) return state;
      if (
        state.wireInProgress.part === action.endpoint.part &&
        state.wireInProgress.pin === action.endpoint.pin
      ) {
        return { ...state, wireInProgress: null };
      }
      const s = pushUndo(state);
      const wire = {
        from: s.wireInProgress!,
        to: action.endpoint,
        color: action.color,
      };
      return {
        ...s,
        diagram: { ...s.diagram, wires: [...s.diagram.wires, wire] },
        wireInProgress: null,
      };
    }
    case 'CANCEL_WIRE': {
      return { ...state, wireInProgress: null };
    }
    case 'DELETE_WIRE': {
      const s = pushUndo(state);
      return {
        ...s,
        diagram: {
          ...s.diagram,
          wires: s.diagram.wires.filter((_, i) => i !== action.index),
        },
      };
    }
    case 'SELECT': {
      if (action.id === null) {
        return { ...state, selectedIds: EMPTY_SET };
      }
      if (action.add) {
        // Toggle selection (shift+click)
        const next = new Set(state.selectedIds);
        if (next.has(action.id)) {
          next.delete(action.id);
        } else {
          next.add(action.id);
        }
        return { ...state, selectedIds: next };
      }
      return { ...state, selectedIds: new Set([action.id]) };
    }
    case 'SELECT_RECT': {
      return { ...state, selectedIds: new Set(action.ids) };
    }
    case 'LOAD_DIAGRAM': {
      return {
        ...state,
        diagram: action.diagram,
        selectedIds: EMPTY_SET,
        wireInProgress: null,
        undoStack: [],
        redoStack: [],
      };
    }
    case 'UNDO': {
      if (state.undoStack.length === 0) return state;
      const prev = state.undoStack[state.undoStack.length - 1];
      return {
        ...state,
        diagram: prev,
        undoStack: state.undoStack.slice(0, -1),
        redoStack: [...state.redoStack, structuredClone(state.diagram)],
        selectedIds: EMPTY_SET,
        wireInProgress: null,
      };
    }
    case 'REDO': {
      if (state.redoStack.length === 0) return state;
      const next = state.redoStack[state.redoStack.length - 1];
      return {
        ...state,
        diagram: next,
        undoStack: [...state.undoStack, structuredClone(state.diagram)],
        redoStack: state.redoStack.slice(0, -1),
        selectedIds: EMPTY_SET,
        wireInProgress: null,
      };
    }
    default:
      return state;
  }
}

export function useEditorState(initialDiagram?: Diagram) {
  const [state, dispatch] = useReducer(editorReducer, {
    diagram: initialDiagram ?? createEmptyDiagram(),
    selectedIds: EMPTY_SET,
    wireInProgress: null,
    undoStack: [],
    redoStack: [],
  });

  const addPart = useCallback(
    (part: Part) => dispatch({ type: 'ADD_PART', part }),
    [],
  );
  const movePart = useCallback(
    (id: string, x: number, y: number) => dispatch({ type: 'MOVE_PART', id, x, y }),
    [],
  );
  const rotatePart = useCallback(
    (id: string) => dispatch({ type: 'ROTATE_PART', id }),
    [],
  );
  const resizePart = useCallback(
    (id: string, scale: number) => dispatch({ type: 'RESIZE_PART', id, scale }),
    [],
  );
  const deleteSelected = useCallback(() => dispatch({ type: 'DELETE_SELECTED' }), []);
  const updateAttrs = useCallback(
    (id: string, attrs: Record<string, string>) =>
      dispatch({ type: 'UPDATE_ATTRS', id, attrs }),
    [],
  );
  const select = useCallback(
    (id: string | null, add?: boolean) => dispatch({ type: 'SELECT', id, add }),
    [],
  );
  const selectRect = useCallback(
    (ids: string[]) => dispatch({ type: 'SELECT_RECT', ids }),
    [],
  );
  const startWire = useCallback(
    (endpoint: WireEndpoint) => dispatch({ type: 'START_WIRE', endpoint }),
    [],
  );
  const completeWire = useCallback(
    (endpoint: WireEndpoint) =>
      dispatch({ type: 'COMPLETE_WIRE', endpoint, color: nextWireColor() }),
    [],
  );
  const cancelWire = useCallback(() => dispatch({ type: 'CANCEL_WIRE' }), []);
  const deleteWire = useCallback(
    (index: number) => dispatch({ type: 'DELETE_WIRE', index }),
    [],
  );
  const loadDiagram = useCallback(
    (diagram: Diagram) => dispatch({ type: 'LOAD_DIAGRAM', diagram }),
    [],
  );
  const undo = useCallback(() => dispatch({ type: 'UNDO' }), []);
  const redo = useCallback(() => dispatch({ type: 'REDO' }), []);

  return {
    state,
    addPart,
    movePart,
    rotatePart,
    resizePart,
    deleteSelected,
    updateAttrs,
    select,
    selectRect,
    startWire,
    completeWire,
    cancelWire,
    deleteWire,
    loadDiagram,
    undo,
    redo,
  };
}
