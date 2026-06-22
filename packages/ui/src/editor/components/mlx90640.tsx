import type { ComponentDef } from '../types';

// MLX90640 32x24 far-infrared thermal camera on the Adafruit STEMMA breakout.
// I2C device (fixed addr 0x33). Rendered structurally for the read-only board
// view; the sim does not model it (no IR scene), so it carries no live state.
const W = 104;
const H = 64;

export const mlx90640Component: ComponentDef = {
  type: 'mlx90640',
  label: 'MLX90640',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'i2c_device',
  pins: [
    { id: 'VCC', x: 0, y: 10, side: 'left', label: 'VIN' },
    { id: 'GND', x: 0, y: 22, side: 'left', label: 'GND' },
    { id: 'SCL', x: 0, y: 34, side: 'left', label: 'SCL' },
    { id: 'SDA', x: 0, y: 46, side: 'left', label: 'SDA' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="mlx-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#15324a" />
            <stop offset="1" stopColor="#0a1c2c" />
          </linearGradient>
          <radialGradient id="mlx-can" cx="0.4" cy="0.35" r="0.75">
            <stop offset="0" stopColor="#e8edf2" />
            <stop offset="0.6" stopColor="#9aa6b2" />
            <stop offset="1" stopColor="#4a525c" />
          </radialGradient>
          <linearGradient id="mlx-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 4} ry={3} fill="#000" opacity={0.35} />

        {/* PCB body */}
        <rect
          width={W}
          height={H}
          rx={5}
          fill="url(#mlx-pcb)"
          stroke={selected ? '#3DD68C' : '#08161f'}
          strokeWidth={selected ? 2.5 : 1.2}
        />

        {/* Silver IR sensor can (the metal cylinder with the IR window) */}
        <circle cx={W / 2 + 14} cy={H / 2} r={15} fill="url(#mlx-can)" stroke="#3a424c" strokeWidth={1} />
        <circle cx={W / 2 + 14} cy={H / 2} r={9} fill="#1a2028" stroke="#566069" strokeWidth={0.8} />
        <circle cx={W / 2 + 11} cy={H / 2 - 3} r={2.5} fill="#2c343d" opacity={0.8} />

        {/* Silkscreen */}
        <text x={W / 2 - 22} y={20} textAnchor="middle" fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={8} fontWeight={700} letterSpacing="0.04em">
          MLX
        </text>
        <text x={W / 2 - 22} y={31} textAnchor="middle" fill="rgba(255,255,255,0.7)" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          90640
        </text>
        <text x={W / 2 - 22} y={H - 14} textAnchor="middle" fill="rgba(255,255,255,0.55)" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          32x24 IR
        </text>
        <text x={W / 2 - 22} y={H - 7} textAnchor="middle" fill="rgba(255,255,255,0.5)" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          I²C 0x33
        </text>

        {/* Left pads — VIN, GND, SCL, SDA */}
        {[
          { y: 6, label: 'VIN' },
          { y: 18, label: 'GND' },
          { y: 30, label: 'SCL' },
          { y: 42, label: 'SDA' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={-3} y={y} width={9} height={8} fill="url(#mlx-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={2} cy={y + 4} r={1.5} fill="#0a0a0a" />
            <text x={10} y={y + 6} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#3DD68C" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
