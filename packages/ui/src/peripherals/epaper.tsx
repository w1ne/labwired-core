import { useEffect, useRef } from 'react';
import type { CSSProperties } from 'react';
import type { DisplayBuffer } from '../editor/types';

// Native panel resolution (portrait, silicon coordinates) for the 2.9" tri-color
// modules (SSD1680 / UC8151D). 128 px wide × 296 px tall, 16 bytes/row, MSB-first.
const PANEL_W = 128;
const PANEL_H = 296;
const PANEL_W_BYTES = PANEL_W / 8;
const PLANE_BYTES = PANEL_W_BYTES * PANEL_H; // 4736 (black) + 4736 (red) = 9472

// User-facing landscape rendering: rotate the native portrait framebuffer 90° CW
// to match how the module is typically mounted (296 wide × 128 tall).
export const EPAPER_LANDSCAPE_W = PANEL_H; // 296
export const EPAPER_LANDSCAPE_H = PANEL_W; // 128

/**
 * Decode the wire-encoded black + red planes of a 2.9" tri-color e-paper into a
 * single RGBA buffer in landscape orientation (296×128), rotated 90° CW.
 *
 * Wire encoding (matches GxEPD2 and the Ssd1680/Uc8151d panel models, AFTER the
 * sim loop's `invertRedPlane` normalization where applicable):
 *   black plane: bit 1 = white (no ink), bit 0 = black
 *   red plane:   bit 1 = no-red,         bit 0 = red
 * Composition: red dominates — red bit 0 → red; else black bit 0 → black; else paper.
 *
 * This is the single source of truth shared by the editor ComponentDefs
 * (epd-ssd1680/uc8151d) and external consumers via `<EpaperPanel>`. Returns null
 * if the plane buffer isn't the expected 9472 bytes.
 */
export function decodeTricolorFramebuffer(planes: Uint8Array): Uint8ClampedArray | null {
  if (planes.length !== PLANE_BYTES * 2) return null;
  const out = new Uint8ClampedArray(EPAPER_LANDSCAPE_W * EPAPER_LANDSCAPE_H * 4);
  for (let nativeY = 0; nativeY < PANEL_H; nativeY++) {
    for (let nativeXByte = 0; nativeXByte < PANEL_W_BYTES; nativeXByte++) {
      const idx = nativeY * PANEL_W_BYTES + nativeXByte;
      const blackByte = planes[idx];
      const redByte = planes[PLANE_BYTES + idx];
      for (let bit = 0; bit < 8; bit++) {
        const nativeX = nativeXByte * 8 + bit;
        const mask = 1 << (7 - bit);
        const blackBit = (blackByte & mask) !== 0;
        const redBit = (redByte & mask) !== 0;
        // native (x, y) → landscape (lx, ly) with a 90° CW rotation.
        const lx = nativeY;
        const ly = PANEL_W - 1 - nativeX;
        const off = (ly * EPAPER_LANDSCAPE_W + lx) * 4;
        if (!redBit) {
          out[off] = 196; out[off + 1] = 30; out[off + 2] = 30; out[off + 3] = 255; // warm red dye
        } else if (!blackBit) {
          out[off] = 30; out[off + 1] = 30; out[off + 2] = 30; out[off + 3] = 255; // black ink
        } else {
          out[off] = 244; out[off + 1] = 241; out[off + 2] = 232; out[off + 3] = 255; // off-white paper
        }
      }
    }
  }
  return out;
}

export interface EpaperPanelProps {
  /** The display buffer from `useSimulationLoop`'s `state.displayBuffers[partId]`. */
  buffer: DisplayBuffer | null | undefined;
  className?: string;
  style?: CSSProperties;
}

/**
 * Standalone live e-paper panel — paints a tri-color framebuffer onto a canvas.
 * No editor/board model required: hand it the `DisplayBuffer` from the sim loop
 * and it renders the 296×128 landscape page. Repaints only when the panel
 * actually refreshes (generation changes). Pixels are nearest-neighbour scaled.
 */
export function EpaperPanel({ buffer, className, style }: EpaperPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const gen = buffer?.generation ?? -1;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !buffer) return;
    const rgba = decodeTricolorFramebuffer(buffer.data);
    if (!rgba) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const img = ctx.createImageData(EPAPER_LANDSCAPE_W, EPAPER_LANDSCAPE_H);
    img.data.set(rgba);
    ctx.putImageData(img, 0, 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gen, buffer?.kind]);

  return (
    <canvas
      ref={canvasRef}
      width={EPAPER_LANDSCAPE_W}
      height={EPAPER_LANDSCAPE_H}
      className={className}
      style={{ imageRendering: 'pixelated', ...style }}
    />
  );
}
