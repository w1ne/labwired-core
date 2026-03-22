import { CSSProperties } from 'react';
export interface LedProps {
    /** Whether the LED is active (lit). */
    active: boolean;
    /** LED color when active. Default: '#ff3333' (red). */
    color?: string;
    /** Size in pixels. Default: 20. */
    size?: number;
    /** Label shown below the LED. */
    label?: string;
    style?: CSSProperties;
}
export declare function Led({ active, color, size, label, style }: LedProps): import("react/jsx-runtime").JSX.Element;
