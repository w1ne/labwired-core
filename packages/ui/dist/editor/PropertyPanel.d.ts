import { Part } from './types';
interface PropertyPanelProps {
    parts: Part[];
    onUpdateAttrs: (id: string, attrs: Record<string, string>) => void;
    onDelete: () => void;
    onRotate: (id: string) => void;
}
export declare function PropertyPanel({ parts, onUpdateAttrs, onDelete, onRotate }: PropertyPanelProps): import("react/jsx-runtime").JSX.Element | null;
export {};
