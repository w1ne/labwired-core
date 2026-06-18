// canvasPreview — render the editor board <svg> to a PNG data URL at share time.
//
// Social link cards want a 1200x630 image. We serialize the live board SVG,
// rasterize it through an <Image>, and letterbox it (fit-contain) onto a
// brand-coloured canvas. Everything here is best-effort: a failure (no DOM,
// canvas tainted, image decode error) returns `null` and NEVER throws, so the
// caller can fall back to the static og:image without breaking the share.
//
// Reuses the `canvas.toDataURL('image/png')` pattern from
// components/epd-ssd1680-tricolor.tsx.

const DEFAULT_WIDTH = 1200;
const DEFAULT_HEIGHT = 630;
const BRAND_BG = '#12121a';

export interface RenderCanvasPngOptions {
  width?: number;
  height?: number;
  bg?: string;
}

/**
 * Render an SVG element to a letterboxed PNG data URL (default 1200x630).
 *
 * Returns `null` (never throws) when:
 *  - there is no `document` (SSR / non-browser),
 *  - the SVG can't be serialized or decoded,
 *  - the canvas 2D context or `toDataURL` is unavailable / throws.
 */
export async function renderCanvasPng(
  svg: SVGSVGElement,
  opts: RenderCanvasPngOptions = {},
): Promise<string | null> {
  if (typeof document === 'undefined') return null;

  const width = opts.width ?? DEFAULT_WIDTH;
  const height = opts.height ?? DEFAULT_HEIGHT;
  const bg = opts.bg ?? BRAND_BG;

  try {
    // 1. Serialize the live SVG to a standalone document string.
    const serialized = new XMLSerializer().serializeToString(svg);

    // 2. Load it into an Image via a base64 data URL. base64 (not utf8) keeps
    //    the URL robust against `#`, quotes, and other chars in the markup.
    const svgUrl = `data:image/svg+xml;base64,${svgToBase64(serialized)}`;
    const img = await loadImage(svgUrl);
    if (!img) return null;

    // 3. Draw onto an offscreen canvas, letterboxed/centered (fit-contain).
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d');
    if (!ctx) return null;

    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, width, height);

    // Intrinsic size of the rasterized SVG; guard against 0 to avoid NaN scale.
    const srcW = img.width || width;
    const srcH = img.height || height;
    const scale = Math.min(width / srcW, height / srcH);
    const drawW = srcW * scale;
    const drawH = srcH * scale;
    const dx = (width - drawW) / 2;
    const dy = (height - drawH) / 2;
    ctx.drawImage(img, dx, dy, drawW, drawH);

    // 4. Export. toDataURL throws on a tainted canvas — caught below.
    return canvas.toDataURL('image/png');
  } catch {
    return null;
  }
}

/** UTF-8-safe base64 of a string (btoa alone mishandles non-Latin1 bytes). */
function svgToBase64(svg: string): string {
  const bytes = new TextEncoder().encode(svg);
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

/** Resolve to a loaded Image, or `null` if it errors. Never rejects. */
function loadImage(url: string): Promise<HTMLImageElement | null> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = url;
  });
}
