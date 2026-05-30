import { useEffect, useRef } from 'react';

export interface Pcd8544DisplayProps {
  framebuffer: Uint8Array | null;
  /** CSS display width in px (native resolution is 84px; scaled up). */
  width?: number;
  /** Dark pixel colour (classic Nokia 5110 ink). */
  pixelOnColor?: string;
  /** LCD background (greenish-grey). */
  pixelOffColor?: string;
}

const W = 84;
const H = 48;
const BANKS = H / 8; // 6

/**
 * Renders an 84×48 PCD8544 (Nokia 5110) framebuffer onto a canvas.
 *
 * DDRAM layout (same banked scheme as the SSD1306): byte at index
 * `bank * 84 + col` holds 8 vertical pixels of column `col` in `bank`;
 * bit 0 = top row of the bank (row `bank*8`). 504 bytes total.
 *
 * Canvas is native 84×48 and CSS-scaled with `image-rendering: pixelated`.
 */
export function Pcd8544Display({
  framebuffer,
  width = 252,
  pixelOnColor = '#2e3a26',
  pixelOffColor = '#b3c69a',
}: Pcd8544DisplayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.fillStyle = pixelOffColor;
    ctx.fillRect(0, 0, W, H);

    if (!framebuffer || framebuffer.length < W * BANKS) return;

    ctx.fillStyle = pixelOnColor;
    for (let bank = 0; bank < BANKS; bank++) {
      for (let col = 0; col < W; col++) {
        const byte = framebuffer[bank * W + col];
        for (let bit = 0; bit < 8; bit++) {
          if (byte & (1 << bit)) {
            ctx.fillRect(col, bank * 8 + bit, 1, 1);
          }
        }
      }
    }
  }, [framebuffer, pixelOnColor, pixelOffColor]);

  const aspectHeight = (width / W) * H;

  return (
    <canvas
      ref={canvasRef}
      width={W}
      height={H}
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
