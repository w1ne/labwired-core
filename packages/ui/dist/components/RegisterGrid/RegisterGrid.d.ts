import { CSSProperties } from 'react';
export interface RegisterGridProps {
    /** Register name->value map. */
    registers: Map<string, number>;
    /** Highlight the PC register. */
    pc?: number;
    style?: CSSProperties;
}
export declare function RegisterGrid({ registers, style }: RegisterGridProps): import("react/jsx-runtime").JSX.Element;
