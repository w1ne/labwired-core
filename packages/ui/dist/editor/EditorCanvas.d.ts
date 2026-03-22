import { ComponentState, EditorState, WireEndpoint } from './types';
interface EditorCanvasProps {
    state: EditorState;
    boardIoStates?: Record<string, ComponentState>;
    onMovePart: (id: string, x: number, y: number) => void;
    onSelect: (id: string | null, add?: boolean) => void;
    onSelectRect?: (ids: string[]) => void;
    onStartWire: (endpoint: WireEndpoint) => void;
    onCompleteWire: (endpoint: WireEndpoint) => void;
    onCancelWire: () => void;
    onDeleteWire: (index: number) => void;
    onDropPart?: (type: string, x: number, y: number) => void;
}
export declare function EditorCanvas({ state, boardIoStates, onMovePart, onSelect, onSelectRect, onStartWire, onCompleteWire, onCancelWire, onDeleteWire, onDropPart, }: EditorCanvasProps): import("react/jsx-runtime").JSX.Element;
export {};
