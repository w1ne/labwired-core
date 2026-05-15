import type { ComponentDef } from '../types';

const W = 96;
const H = 64;

export const adxl345Component: ComponentDef = {
  type: 'adxl345',
  label: 'ADXL345',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'i2c_device',
  pins: [
    { id: 'VCC', x: 0, y: 14, side: 'left', label: 'VCC' },
    { id: 'GND', x: 0, y: 30, side: 'left', label: 'GND' },
    { id: 'SDA', x: W, y: 22, side: 'right', label: 'SDA' },
    { id: 'SCL', x: W, y: 42, side: 'right', label: 'SCL' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="adxl-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#7B2C5F" />
            <stop offset="1" stopColor="#4A1A3A" />
          </linearGradient>
          <pattern id="adxl-dots" x="0" y="0" width="4" height="4" patternUnits="userSpaceOnUse">
            <circle cx={2} cy={2} r={0.3} fill="#2a0a20" opacity={0.6} />
          </pattern>
          <linearGradient id="adxl-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0a0a0a" />
          </linearGradient>
          <linearGradient id="adxl-pad" x1="0" y1="0" x2="0" y2="1">
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
          fill="url(#adxl-pcb)"
          stroke={selected ? '#F062B8' : '#1a0814'}
          strokeWidth={selected ? 2.5 : 1.2}
        />
        <rect width={W} height={H} rx={5} fill="url(#adxl-dots)" opacity={0.5} />

        {/* MCU chip in center — LGA-14 package */}
        <rect x={W / 2 - 12} y={H / 2 - 8} width={24} height={16} rx={1.5} fill="url(#adxl-chip)" stroke="#000" strokeWidth={0.8} />
        <circle cx={W / 2 - 9} cy={H / 2 - 5} r={1} fill="#666" />
        <text x={W / 2} y={H / 2 + 1} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={4.5} fontWeight={600}>
          ADXL345
        </text>
        <text x={W / 2} y={H / 2 + 6} textAnchor="middle" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={3.5}>
          BCCZ
        </text>

        {/* Decap caps near chip */}
        <rect x={W / 2 + 16} y={H / 2 - 5} width={3} height={6} fill="#888" stroke="#444" strokeWidth={0.3} />
        <rect x={W / 2 - 19} y={H / 2 - 5} width={3} height={6} fill="#888" stroke="#444" strokeWidth={0.3} />

        {/* Silkscreen title */}
        <text x={W / 2} y={10} textAnchor="middle" fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={7} fontWeight={700} letterSpacing="0.05em">
          ADXL345
        </text>
        <text x={W / 2} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.55)" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          3-AXIS · I²C 0x53
        </text>

        {/* Left pads — VCC, GND */}
        <rect x={-3} y={10} width={9} height={8} fill="url(#adxl-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={2} cy={14} r={1.5} fill="#0a0a0a" />
        <text x={10} y={16} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>VCC</text>

        <rect x={-3} y={26} width={9} height={8} fill="url(#adxl-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={2} cy={30} r={1.5} fill="#0a0a0a" />
        <text x={10} y={32} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>GND</text>

        {/* Right pads — SDA, SCL */}
        <rect x={W - 6} y={18} width={9} height={8} fill="url(#adxl-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={W - 2} cy={22} r={1.5} fill="#0a0a0a" />
        <text x={W - 10} y={24} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>SDA</text>

        <rect x={W - 6} y={38} width={9} height={8} fill="url(#adxl-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={W - 2} cy={42} r={1.5} fill="#0a0a0a" />
        <text x={W - 10} y={44} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>SCL</text>

        {/* Selection highlight */}
        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
