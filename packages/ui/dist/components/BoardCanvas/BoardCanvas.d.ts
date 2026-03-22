import { CSSProperties } from 'react';
import { BoardIoBinding, BoardIoState } from '../../wasm/simulator-bridge';
export interface BoardCanvasProps {
    /** Board name displayed on the MCU block. */
    boardName: string;
    /** Chip identifier (e.g. "STM32F107"). */
    chipId: string;
    /** Board IO bindings from the system manifest. */
    boardIo: BoardIoBinding[];
    /** Current board IO states. */
    boardIoStates: BoardIoState[];
    /** Called when user presses/releases a button binding. */
    onButtonToggle?: (id: string, pressed: boolean) => void;
    /** Canvas width. Default: 600. */
    width?: number;
    /** Canvas height. Default: 400. */
    height?: number;
    style?: CSSProperties;
}
/**
 * SVG board visualization showing the MCU block and connected board IO nodes.
 * Adapted from the VS Code topology panel (vscode/media/topology.js).
 */
export declare function BoardCanvas({ boardName, chipId, boardIo, boardIoStates, onButtonToggle, width, height, style, }: BoardCanvasProps): import("react/jsx-runtime").JSX.Element;
