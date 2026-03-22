import { CSSProperties } from 'react';
export interface MemoryInspectorProps {
    /** Memory data bytes. */
    data: Uint8Array;
    /** Base address of the first byte. */
    baseAddress: number;
    /** Bytes per row. Default: 16. */
    bytesPerRow?: number;
    style?: CSSProperties;
}
export declare function MemoryInspector({ data, baseAddress, bytesPerRow, style, }: MemoryInspectorProps): import("react/jsx-runtime").JSX.Element;
