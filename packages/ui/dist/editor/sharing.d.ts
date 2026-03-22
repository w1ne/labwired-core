import { Diagram } from './types';
/**
 * Encode a diagram + source to a URL-safe base64 string.
 * Uses built-in compression via CompressionStream when available.
 */
export declare function encodeProject(diagram: Diagram, source: string): Promise<string>;
/**
 * Decode a project from a URL hash string.
 */
export declare function decodeProject(hash: string): Promise<{
    diagram: Diagram;
    source: string;
} | null>;
/**
 * Check if the page is in embed mode (?embed=true).
 */
export declare function isEmbedMode(): boolean;
/**
 * Generate a shareable URL with the project encoded in the hash.
 */
export declare function generateShareUrl(diagram: Diagram, source: string): Promise<string>;
/**
 * Generate an embed URL.
 */
export declare function generateEmbedUrl(diagram: Diagram, source: string): Promise<string>;
