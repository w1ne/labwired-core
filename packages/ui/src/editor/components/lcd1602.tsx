import type { ComponentDef } from '../types';

const W = 220;
const H = 110;

export const lcd1602Component: ComponentDef = {
  type: 'lcd1602',
  label: 'LCD 16x2',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'VSS', x: 0, y: 16, side: 'left', label: 'VSS' },
    { id: 'VDD', x: 0, y: 32, side: 'left', label: 'VDD' },
    { id: 'V0', x: 0, y: 48, side: 'left', label: 'V0' },
    { id: 'RS', x: 0, y: 64, side: 'left', label: 'RS' },
    { id: 'RW', x: 0, y: 80, side: 'left', label: 'RW' },
    { id: 'E', x: 0, y: 96, side: 'left', label: 'E' },
    { id: 'D4', x: W, y: 16, side: 'right', label: 'D4' },
    { id: 'D5', x: W, y: 32, side: 'right', label: 'D5' },
    { id: 'D6', x: W, y: 48, side: 'right', label: 'D6' },
    { id: 'D7', x: W, y: 64, side: 'right', label: 'D7' },
    { id: 'BLA', x: W, y: 80, side: 'right', label: 'BLA' },
    { id: 'BLK', x: W, y: 96, side: 'right', label: 'BLK' },
  ],
  defaultAttrs: { text: 'Hello World!' },
  boardIoKind: 'i2c_device',
  attrFields: [
    { key: 'text', label: 'Display Text', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = !!state?.selected;
    const text = (state?.displayText as string | undefined) || (attrs.text as string) || 'Hello World!';
    const line1 = text.slice(0, 16).padEnd(16);
    const line2 = text.slice(16, 32).padEnd(16);
    return (
      <g>
        <defs>
          <linearGradient id="lcd-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1F7A3A" />
            <stop offset="1" stopColor="#0F4A20" />
          </linearGradient>
          <pattern id="lcd-pcb-dots" x="0" y="0" width="5" height="5" patternUnits="userSpaceOnUse">
            <circle cx={2.5} cy={2.5} r={0.3} fill="#0a3a18" opacity={0.6} />
          </pattern>
          <linearGradient id="lcd-screen" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#7CC97C" />
            <stop offset="1" stopColor="#5EB05E" />
          </linearGradient>
          <linearGradient id="lcd-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 3} rx={W / 2 - 10} ry={4} fill="#000" opacity={0.35} />

        {/* PCB body */}
        <rect width={W} height={H} rx={5} fill="url(#lcd-pcb)" stroke={selected ? '#F062B8' : '#082010'} strokeWidth={selected ? 2.5 : 1} />
        <rect width={W} height={H} rx={5} fill="url(#lcd-pcb-dots)" opacity={0.5} />

        {/* Mounting holes */}
        {[[8, 8], [W - 8, 8], [8, H - 8], [W - 8, H - 8]].map(([x, y], i) => (
          <circle key={i} cx={x} cy={y} r={2.5} fill="#0a0a0a" stroke="#444" strokeWidth={0.5} />
        ))}

        {/* Display bezel (dark frame) */}
        <rect x={22} y={14} width={W - 44} height={H - 28} rx={2} fill="#1a3a1a" stroke="#0a1a08" strokeWidth={1} />
        {/* Display surface (green LCD) */}
        <rect x={28} y={20} width={W - 56} height={H - 40} fill="url(#lcd-screen)" />

        {/* Per-character grid effect — faint vertical lines */}
        {Array.from({ length: 17 }, (_, i) => (
          <line
            key={i}
            x1={28 + i * ((W - 56) / 16)}
            y1={20}
            x2={28 + i * ((W - 56) / 16)}
            y2={H - 20}
            stroke="#5a8a5a"
            strokeWidth={0.3}
            opacity={0.4}
          />
        ))}
        <line x1={28} y1={20 + (H - 40) / 2} x2={W - 28} y2={20 + (H - 40) / 2} stroke="#5a8a5a" strokeWidth={0.3} opacity={0.4} />

        {/* Display text in characteristic LCD pixelated font */}
        <text x={34} y={42} fill="#0F2F0F" fontFamily="'JetBrains Mono', monospace" fontSize={12.5} fontWeight={700} letterSpacing="0.18em">
          {line1}
        </text>
        <text x={34} y={68} fill="#0F2F0F" fontFamily="'JetBrains Mono', monospace" fontSize={12.5} fontWeight={700} letterSpacing="0.18em">
          {line2}
        </text>

        {/* Silkscreen brand */}
        <text x={W - 22} y={H - 4} textAnchor="end" fill="rgba(255,255,255,0.55)" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={600} letterSpacing="0.04em">
          16×2 HD44780
        </text>

        {/* Pin pads on both sides */}
        {[16, 32, 48, 64, 80, 96].map((y) => (
          <g key={`l-${y}`}>
            <rect x={-3} y={y - 3} width={8} height={6} fill="url(#lcd-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={1} cy={y} r={1.3} fill="#0a0a0a" />
          </g>
        ))}
        {[16, 32, 48, 64, 80, 96].map((y) => (
          <g key={`r-${y}`}>
            <rect x={W - 5} y={y - 3} width={8} height={6} fill="url(#lcd-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={W - 1} cy={y} r={1.3} fill="#0a0a0a" />
          </g>
        ))}
      </g>
    );
  },
};
