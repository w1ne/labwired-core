import { CompileError } from './CodeEditor';
export interface CompileResult {
    success: boolean;
    elf?: Uint8Array;
    errors: CompileError[];
    output: string;
}
export interface CompileOptions {
    source: string;
    language: 'c' | 'cpp' | 'arduino';
    target: string;
}
/**
 * Compile source code to an ELF binary.
 *
 * This currently uses a mock implementation that returns pre-built firmware.
 * When a compile server is available, it will POST to /api/compile.
 */
export declare function compileCode(options: CompileOptions): Promise<CompileResult>;
/** Example sketches shipped with the editor (Arduino API). */
export declare const EXAMPLE_SKETCHES: {
    name: string;
    source: string;
    language?: 'c' | 'cpp' | 'arduino';
}[];
