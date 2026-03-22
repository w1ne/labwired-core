import { CSSProperties } from 'react';
export interface TraceEntry {
    pc: number;
    disassembly: string;
}
export interface InstructionTraceProps {
    /** Trace entries to display (most recent last). */
    entries: TraceEntry[];
    /** Maximum number of entries to keep visible. Default: 50. */
    maxEntries?: number;
    style?: CSSProperties;
}
export declare function InstructionTrace({ entries, maxEntries, style }: InstructionTraceProps): import("react/jsx-runtime").JSX.Element;
