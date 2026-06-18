// Pure, deterministic builder for the copy-paste <iframe> embed snippet.
// Kept side-effect-free so it can be unit-tested without a DOM.

/** Height presets offered in the Embed dialog (px). Width is always 100%. */
export const EMBED_HEIGHTS = {
  Compact: 420,
  Tall: 600,
} as const;

export type EmbedHeightPreset = keyof typeof EMBED_HEIGHTS;

/** Escape a string for safe interpolation inside a double-quoted HTML attribute. */
function escapeAttr(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

export interface BuildEmbedSnippetOptions {
  height: number;
}

/**
 * Build the responsive, sandboxed <iframe> markup for embedding a lab.
 * Width is always 100% (responsive); height comes from the chosen preset.
 */
export function buildEmbedSnippet(url: string, opts: BuildEmbedSnippetOptions): string {
  const src = escapeAttr(url);
  return (
    `<iframe src="${src}" title="LabWired lab" width="100%" height="${opts.height}" ` +
    `style="border:0;border-radius:8px" loading="lazy" ` +
    `sandbox="allow-scripts allow-same-origin allow-popups"></iframe>`
  );
}
