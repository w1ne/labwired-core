import { useMemo } from 'react';
import type { ComponentDef, DisplayBuffer } from '../types';

const W = 160;
const H = 78;

// Panel face area inside the PCB silkscreen. Sized to roughly match the
// Waveshare 2.9" module's visible glass when scaled to a 160x78 footprint.
const FACE_X = 8;
const FACE_Y = 6;
const FACE_W = W - 16; // 144
const FACE_H = H - 30; // 48

// Panel native resolution (portrait, silicon coordinates).
const PANEL_W = 128;
const PANEL_H = 296;
const PANEL_W_BYTES = PANEL_W / 8;
const PLANE_BYTES = PANEL_W_BYTES * PANEL_H; // 4736

// User-facing landscape rendering: we rotate the native portrait
// framebuffer 90° clockwise to match how the module is typically mounted
// (296 px wide, 128 px tall — the longer dimension is horizontal).
const LANDSCAPE_W = PANEL_H; // 296
const LANDSCAPE_H = PANEL_W; // 128

/**
 * Compose the wire-encoded black + red planes into a single rendered
 * Uint8ClampedArray (RGBA) in landscape orientation (296×128).
 *
 * Wire encoding (matches what GxEPD2 sends and what Ssd1680Tricolor290 stores):
 *   black plane: bit 1 = white (no ink), bit 0 = black
 *   red plane:   bit 1 = no-red,         bit 0 = red
 * Composition: red dominates — if red bit == 0 the pixel is red regardless
 * of the black bit; else if black bit == 0 the pixel is black; else white.
 */
function composeRgba(planes: Uint8Array): Uint8ClampedArray | null {
  if (planes.length !== PLANE_BYTES * 2) return null;
  const out = new Uint8ClampedArray(LANDSCAPE_W * LANDSCAPE_H * 4);
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
        // Map native (x, y) → landscape (lx, ly) with a 90° CW rotation:
        //   lx = nativeY,           ly = (PANEL_W - 1) - nativeX
        const lx = nativeY;
        const ly = (PANEL_W - 1) - nativeX;
        const off = (ly * LANDSCAPE_W + lx) * 4;
        if (!redBit) {
          // red ink — slightly warm, matches Waveshare's actual red dye
          out[off] = 196; out[off + 1] = 30; out[off + 2] = 30; out[off + 3] = 255;
        } else if (!blackBit) {
          out[off] = 30; out[off + 1] = 30; out[off + 2] = 30; out[off + 3] = 255;
        } else {
          // off-white paper background
          out[off] = 244; out[off + 1] = 241; out[off + 2] = 232; out[off + 3] = 255;
        }
      }
    }
  }
  return out;
}

/** Convert RGBA pixels to a PNG data URL via an off-screen canvas. */
function rgbaToPngDataUrl(rgba: Uint8ClampedArray): string | null {
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  canvas.width = LANDSCAPE_W;
  canvas.height = LANDSCAPE_H;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  // Allocate the ImageData via the canvas (avoids strict-TS friction with
  // `new ImageData(Uint8ClampedArray, ...)` over generic ArrayBufferLike)
  // then copy the composed pixels in.
  const img = ctx.createImageData(LANDSCAPE_W, LANDSCAPE_H);
  img.data.set(rgba);
  ctx.putImageData(img, 0, 0);
  return canvas.toDataURL('image/png');
}

function PanelPixels({ buffer }: { buffer: DisplayBuffer }) {
  // Re-encode only when the panel actually refreshed (generation changed),
  // not every frame. composeRgba runs at most once per panel refresh.
  const dataUrl = useMemo(() => {
    if (buffer.kind !== 'ssd1680_tricolor_290') return null;
    const rgba = composeRgba(buffer.data);
    if (!rgba) return null;
    return rgbaToPngDataUrl(rgba);
  }, [buffer.kind, buffer.generation, buffer.data]);

  if (!dataUrl) return null;
  return (
    <image
      href={dataUrl}
      x={FACE_X}
      y={FACE_Y}
      width={FACE_W}
      height={FACE_H}
      preserveAspectRatio="none"
      // Nearest-neighbour scaling — these are 1bpp ink pixels, blurring them looks wrong.
      style={{ imageRendering: 'pixelated' }}
    />
  );
}

export const epdSsd1680TricolorComponent: ComponentDef = {
  type: 'ssd1680_tricolor_290',
  label: 'E-Paper 2.9" tri-color (SSD1680)',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'VCC',  x: 24,  y: H, side: 'bottom', label: 'VCC' },
    { id: 'GND',  x: 40,  y: H, side: 'bottom', label: 'GND' },
    { id: 'DIN',  x: 56,  y: H, side: 'bottom', label: 'DIN' },
    { id: 'CLK',  x: 72,  y: H, side: 'bottom', label: 'CLK' },
    { id: 'CS',   x: 88,  y: H, side: 'bottom', label: 'CS' },
    { id: 'DC',   x: 104, y: H, side: 'bottom', label: 'DC' },
    { id: 'RST',  x: 120, y: H, side: 'bottom', label: 'RST' },
    { id: 'BUSY', x: 136, y: H, side: 'bottom', label: 'BUSY' },
  ],
  defaultAttrs: {},
  boardIoKind: 'spi_device',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const active = !!state?.active;
    const buffer = state?.displayBuffer;

    return (
      <g>
        <defs>
          <linearGradient id="epd-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1a4a1a" />
            <stop offset="1" stopColor="#0e2e0e" />
          </linearGradient>
          <linearGradient id="epd-paper" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f4f1e8" />
            <stop offset="1" stopColor="#e6e1d3" />
          </linearGradient>
          <linearGradient id="epd-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 8} ry={3.5} fill="#000" opacity={0.3} />

        {/* PCB */}
        <rect width={W} height={H} rx={3} fill="url(#epd-pcb)" stroke={selected ? '#F062B8' : '#0a1f0a'} strokeWidth={selected ? 2.5 : 1} />

        {/* Mounting holes */}
        <circle cx={4}     cy={4}      r={1.6} fill="#0a1f0a" />
        <circle cx={W - 4} cy={4}      r={1.6} fill="#0a1f0a" />
        <circle cx={4}     cy={H - 16} r={1.6} fill="#0a1f0a" />
        <circle cx={W - 4} cy={H - 16} r={1.6} fill="#0a1f0a" />

        {/* Panel face — paper background */}
        <rect
          x={FACE_X}
          y={FACE_Y}
          width={FACE_W}
          height={FACE_H}
          rx={1}
          fill="url(#epd-paper)"
          stroke="#bcb6a3"
          strokeWidth={0.6}
        />

        {/* Live pixels from the simulated framebuffer, if present. Drawn on
            top of the paper background so transparent / off pixels read as
            paper white. */}
        {buffer ? (
          <PanelPixels buffer={buffer} />
        ) : active ? (
          <text
            x={W / 2}
            y={H / 2 - 5}
            textAnchor="middle"
            fill="#1a1a1a"
            fontFamily="'JetBrains Mono', monospace"
            fontSize={5.5}
            fontWeight={600}
          >
            waiting for first refresh...
          </text>
        ) : (
          <>
            <text x={W / 2} y={H / 2 - 9} textAnchor="middle" fill="#a8a290" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={500} letterSpacing="0.04em">
              2.9" tri-color e-paper
            </text>
            <text x={W / 2} y={H / 2 - 1} textAnchor="middle" fill="#bcb6a3" fontFamily="'JetBrains Mono', monospace" fontSize={5}>
              296 × 128 · SSD1680
            </text>
          </>
        )}

        {/* Silkscreen */}
        <text x={W / 2} y={H - 18} textAnchor="middle" fill="rgba(180,255,180,0.5)" fontFamily="'Outfit', sans-serif" fontSize={5} fontWeight={600} letterSpacing="0.08em">
          B / W / R · SPI · 3.3V
        </text>

        {/* Bottom pin pads */}
        {[
          { x: 24,  label: 'VCC',  color: '#FF6B6B' },
          { x: 40,  label: 'GND',  color: '#aaa' },
          { x: 56,  label: 'DIN',  color: '#B07BFF' },
          { x: 72,  label: 'CLK',  color: '#5BD8FF' },
          { x: 88,  label: 'CS',   color: '#3DD68C' },
          { x: 104, label: 'DC',   color: '#5B9DFF' },
          { x: 120, label: 'RST',  color: '#F5B642' },
          { x: 136, label: 'BUSY', color: '#FFE680' },
        ].map((pad) => (
          <g key={pad.label}>
            <rect x={pad.x - 4} y={H - 12} width={8} height={10} fill="url(#epd-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={pad.x} cy={H - 7} r={1.4} fill="#0a1f0a" />
            <text x={pad.x} y={H - 14} textAnchor="middle" fill={pad.color} fontFamily="'JetBrains Mono', monospace" fontSize={4.5} fontWeight={600}>
              {pad.label}
            </text>
          </g>
        ))}

        {selected && (
          <rect width={W} height={H} rx={3} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
