import { useEffect, useRef } from 'react';

export interface Ili9341DisplayProps {
  framebuffer: Uint8Array | null;
  /**
   * CSS display width in pixels.
   * Native panel resolution is 240 wide × 320 tall.
   * Height is computed automatically to preserve the 3:4 aspect ratio.
   */
  width?: number;
}

/**
 * Renders a 240×320 ILI9341 RGB565 TFT framebuffer onto a canvas element.
 *
 * The canvas is rendered at native 240×320 pixels and CSS-scaled to `width`
 * with `image-rendering: pixelated` so individual pixels stay sharp.
 *
 * Framebuffer layout: pixel (col, row) occupies bytes at index
 * `(row * 240 + col) * 2` (high byte) and `(row * 240 + col) * 2 + 1` (low byte).
 * Each pixel is RGB565 big-endian:
 *   bits [15:11] = R (5-bit), [10:5] = G (6-bit), [4:0] = B (5-bit)
 */
export function Ili9341Display({ framebuffer, width = 240 }: Ili9341DisplayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const w = 240;
    const h = 320;
    const img = ctx.createImageData(w, h);

    if (framebuffer && framebuffer.length >= w * h * 2) {
      for (let i = 0; i < w * h; i++) {
        const hi = framebuffer[i * 2];
        const lo = framebuffer[i * 2 + 1];
        const px = (hi << 8) | lo;
        // RGB565 → RGB888
        const r = Math.round(((px >> 11) & 0x1f) * 255 / 31);
        const g = Math.round(((px >> 5) & 0x3f) * 255 / 63);
        const b = Math.round((px & 0x1f) * 255 / 31);
        const o = i * 4;
        img.data[o]     = r;
        img.data[o + 1] = g;
        img.data[o + 2] = b;
        img.data[o + 3] = 255;
      }
    }
    ctx.putImageData(img, 0, 0);
  }, [framebuffer]);

  const aspectHeight = Math.round((width / 240) * 320);

  return (
    <canvas
      ref={canvasRef}
      width={240}
      height={320}
      style={{
        width: `${width}px`,
        height: `${aspectHeight}px`,
        imageRendering: 'pixelated',
        background: '#000',
        borderRadius: 6,
        border: '1px solid #1a1a1a',
        display: 'block',
      }}
    />
  );
}
