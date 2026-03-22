import { CSSProperties } from 'react';
export interface PushButtonProps {
    /** Binding ID from board_io config. */
    id: string;
    /** Whether the button is currently pressed. */
    pressed: boolean;
    /** Called when the user presses/releases the button. */
    onToggle: (id: string, pressed: boolean) => void;
    /** Label shown next to the button. */
    label?: string;
    /** Size in pixels. Default: 28. */
    size?: number;
    style?: CSSProperties;
}
export declare function PushButton({ id, pressed, onToggle, label, size, style }: PushButtonProps): import("react/jsx-runtime").JSX.Element;
