import { useEffect, useRef } from 'react';

export interface Ssd1306DisplayProps {
  framebuffer: Uint8Array | null;
  /** CSS display width in px (native resolution is 128px; will be scaled up). */
  width?: number;
  pixelOnColor?: string;
  pixelOffColor?: string;
}

/**
 * Renders a 128×64 SSD1306 OLED framebuffer onto a canvas element.
 *
 * The canvas is rendered at 128×64 native pixels and CSS-scaled to `width`
 * with `image-rendering: pixelated` so individual pixels stay sharp.
 *
 * GDDRAM layout: byte at index `page * 128 + col` represents 8 vertical pixels
 * in column `col`, page `page`.  Bit 0 = top row of the page (row `page*8`).
 */
export function Ssd1306Display({
  framebuffer,
  width = 256,
  pixelOnColor = '#5BD8FF',
  pixelOffColor = '#0a0a0a',
}: Ssd1306DisplayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Clear to off-pixel colour
    ctx.fillStyle = pixelOffColor;
    ctx.fillRect(0, 0, 128, 64);

    if (!framebuffer || framebuffer.length < 1024) return;

    ctx.fillStyle = pixelOnColor;
    for (let page = 0; page < 8; page++) {
      for (let col = 0; col < 128; col++) {
        const byte = framebuffer[page * 128 + col];
        for (let bit = 0; bit < 8; bit++) {
          if (byte & (1 << bit)) {
            ctx.fillRect(col, page * 8 + bit, 1, 1);
          }
        }
      }
    }
  }, [framebuffer, pixelOnColor, pixelOffColor]);

  const aspectHeight = (width / 128) * 64;

  return (
    <canvas
      ref={canvasRef}
      width={128}
      height={64}
      style={{
        width: `${width}px`,
        height: `${aspectHeight}px`,
        imageRendering: 'pixelated',
        background: pixelOffColor,
        borderRadius: 6,
        border: '1px solid #1a1a1a',
        display: 'block',
      }}
    />
  );
}
