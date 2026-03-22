import { CSSProperties } from 'react';
export interface SimControlsProps {
    running: boolean;
    onPlay: () => void;
    onPause: () => void;
    onStep: () => void;
    onReset: () => void;
    /** Current program counter value. */
    pc?: number;
    /** Total cycles executed. */
    cycles?: number;
    style?: CSSProperties;
}
export declare function SimControls({ running, onPlay, onPause, onStep, onReset, pc, cycles, style, }: SimControlsProps): import("react/jsx-runtime").JSX.Element;
