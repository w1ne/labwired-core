import { CSSProperties } from 'react';
export interface SerialMonitorProps {
    /** UART output text. */
    output: string;
    /** Called when the user wants to clear the output. */
    onClear?: () => void;
    /** Called when user sends data (TX). If provided, shows input field. */
    onSend?: (data: string) => void;
    style?: CSSProperties;
}
export declare function SerialMonitor({ output, onClear, onSend, style }: SerialMonitorProps): import("react/jsx-runtime").JSX.Element;
