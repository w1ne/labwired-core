import { useMemo } from 'react';
import type { ComponentDef, DisplayBuffer } from '../types';

const W = 124;
const H = 96;

// Visible LCD glass area inside the PCB silkscreen.
const FACE_X = 12;
const FACE_Y = 12;
const FACE_W = W - 24; // 100
const FACE_H = H - 40; // 56

// PCD8544 native resolution.
const LCD_W = 84;
const LCD_H = 48;

/**
 * Decode a 504-byte PCD8544 framebuffer into LCD_W×LCD_H RGBA. Lit pixels are
 * drawn as dark ink; unlit pixels are left transparent so the green glass
 * background shows through. Byte layout: 84 cols × 6 banks, bank-major;
 * pixel (x, y) is bit `(y & 7)` of byte `[(y >> 3) * 84 + x]`, 1 = on/dark.
 */
function pcd8544Rgba(data: Uint8Array): Uint8ClampedArray | null {
  if (data.length < (LCD_W * LCD_H) / 8) return null;
  const out = new Uint8ClampedArray(LCD_W * LCD_H * 4);
  for (let y = 0; y < LCD_H; y++) {
    for (let x = 0; x < LCD_W; x++) {
      const byte = data[(y >> 3) * LCD_W + x];
      const on = (byte & (1 << (y & 7))) !== 0;
      const off = (y * LCD_W + x) * 4;
      if (on) {
        // Dark bluish-grey ink, like the real reflective LCD segments.
        out[off] = 32; out[off + 1] = 41; out[off + 2] = 28; out[off + 3] = 255;
      } else {
        out[off + 3] = 0; // transparent — glass shows through
      }
    }
  }
  return out;
}

/** Convert RGBA pixels to a PNG data URL via an off-screen canvas. */
function rgbaToPngDataUrl(rgba: Uint8ClampedArray): string | null {
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  canvas.width = LCD_W;
  canvas.height = LCD_H;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const img = ctx.createImageData(LCD_W, LCD_H);
  img.data.set(rgba);
  ctx.putImageData(img, 0, 0);
  return canvas.toDataURL('image/png');
}

function ScreenPixels({ buffer }: { buffer: DisplayBuffer }) {
  // Re-encode only when the frame actually changed (generation bumps on a
  // real pixel change — see useSimulationLoop's pcd8544 poll branch).
  const dataUrl = useMemo(() => {
    if (buffer.kind !== 'pcd8544') return null;
    const rgba = pcd8544Rgba(buffer.data);
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
      // Nearest-neighbour — 1bpp pixels should stay crisp, not blur.
      style={{ imageRendering: 'pixelated' }}
    />
  );
}

/** Nokia 5110 (PCD8544) 84×48 monochrome SPI LCD module. */
export const pcd8544Component: ComponentDef = {
  type: 'pcd8544',
  label: 'Nokia 5110',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'RST', x: 12, y: H, side: 'bottom', label: 'RST' },
    { id: 'CE', x: 28, y: H, side: 'bottom', label: 'CE' },
    { id: 'DC', x: 44, y: H, side: 'bottom', label: 'DC' },
    { id: 'DIN', x: 60, y: H, side: 'bottom', label: 'DIN' },
    { id: 'CLK', x: 76, y: H, side: 'bottom', label: 'CLK' },
    { id: 'VCC', x: 96, y: H, side: 'bottom', label: 'VCC' },
    { id: 'GND', x: 112, y: H, side: 'bottom', label: 'GND' },
  ],
  defaultAttrs: {},
  boardIoKind: 'spi_device',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const active = !!state?.active;
    const buffer = state?.displayBuffer;
    const hasFrame = buffer?.kind === 'pcd8544';
    return (
      <g>
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 8} ry={4} fill="#000" opacity={0.4} />
        {/* PCB */}
        <rect
          width={W}
          height={H}
          rx={5}
          fill="#27506b"
          stroke={selected ? '#F062B8' : '#0a1a22'}
          strokeWidth={selected ? 2.5 : 1}
        />
        {/* Greenish LCD glass */}
        <rect
          x={FACE_X}
          y={FACE_Y}
          width={FACE_W}
          height={FACE_H}
          rx={2}
          fill={active || hasFrame ? '#c2d3a6' : '#9fb288'}
          stroke="#3a4a2a"
          strokeWidth={1.5}
        />
        {/* Live framebuffer drawn over the glass when the sim is running;
            otherwise a static label. */}
        {hasFrame ? (
          <ScreenPixels buffer={buffer} />
        ) : active ? (
          <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#2e3a26" fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={600}>
            84 × 48
          </text>
        ) : (
          <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#52613f" fontFamily="'JetBrains Mono', monospace" fontSize={8}>
            NOKIA 5110
          </text>
        )}
        {/* Silkscreen */}
        <text x={W / 2} y={H - 16} textAnchor="middle" fill="rgba(255,255,255,0.55)" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={600} letterSpacing="0.05em">
          PCD8544 · SPI
        </text>
        {/* Header pads */}
        {[
          { x: 12 }, { x: 28 }, { x: 44 }, { x: 60 }, { x: 76 }, { x: 96 }, { x: 112 },
        ].map((p, i) => (
          <rect key={i} x={p.x - 3} y={H - 11} width={6} height={9} fill="#d9b24a" stroke="#7a5a1a" strokeWidth={0.3} />
        ))}
        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
