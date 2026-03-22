export interface CompileError {
    line: number;
    column: number;
    message: string;
    severity: 'error' | 'warning';
}
interface CodeEditorProps {
    source: string;
    language?: string;
    onChange: (source: string) => void;
    errors?: CompileError[];
    readOnly?: boolean;
}
export declare function CodeEditor({ source, language, onChange, errors, readOnly, }: CodeEditorProps): import("react/jsx-runtime").JSX.Element;
export {};
