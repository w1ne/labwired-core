import { EditorState, Diagram, Part, WireEndpoint } from './types';
export declare function useEditorState(initialDiagram?: Diagram): {
    state: EditorState;
    addPart: (part: Part) => void;
    movePart: (id: string, x: number, y: number) => void;
    rotatePart: (id: string) => void;
    deleteSelected: () => void;
    updateAttrs: (id: string, attrs: Record<string, string>) => void;
    select: (id: string | null, add?: boolean) => void;
    selectRect: (ids: string[]) => void;
    startWire: (endpoint: WireEndpoint) => void;
    completeWire: (endpoint: WireEndpoint) => void;
    cancelWire: () => void;
    deleteWire: (index: number) => void;
    loadDiagram: (diagram: Diagram) => void;
    undo: () => void;
    redo: () => void;
};
