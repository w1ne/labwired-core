import type { ComponentDef } from '../types';

const W = 96;
const H = 64;

export const bme280Component: ComponentDef = {
  type: 'bme280',
  label: 'BME280',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'i2c_device',
  pins: [
    { id: 'VCC', x: 0, y: 10, side: 'left', label: 'VCC' },
    { id: 'GND', x: 0, y: 22, side: 'left', label: 'GND' },
    { id: 'SCL', x: 0, y: 34, side: 'left', label: 'SCL' },
    { id: 'SDA', x: 0, y: 46, side: 'left', label: 'SDA' },
    { id: 'CSB', x: W, y: 10, side: 'right', label: 'CSB' },
    { id: 'SDO', x: W, y: 22, side: 'right', label: 'SDO' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="bme-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#7B3DB5" />
            <stop offset="1" stopColor="#3A1060" />
          </linearGradient>
          <pattern id="bme-dots" x="0" y="0" width="4" height="4" patternUnits="userSpaceOnUse">
            <circle cx={2} cy={2} r={0.3} fill="#1a0a40" opacity={0.6} />
          </pattern>
          <linearGradient id="bme-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0a0a0a" />
          </linearGradient>
          <linearGradient id="bme-pad" x1="0" y1="0" x2="0" y2="1">
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
          fill="url(#bme-pcb)"
          stroke={selected ? '#3DD68C' : '#1a0840'}
          strokeWidth={selected ? 2.5 : 1.2}
        />
        <rect width={W} height={H} rx={5} fill="url(#bme-dots)" opacity={0.5} />

        {/* IC chip in center — LGA-8 package */}
        <rect x={W / 2 - 10} y={H / 2 - 8} width={20} height={16} rx={1} fill="url(#bme-chip)" stroke="#000" strokeWidth={0.8} />
        <circle cx={W / 2 - 7} cy={H / 2 - 5} r={0.8} fill="#666" />
        <text x={W / 2} y={H / 2} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={4.5} fontWeight={600}>
          BME280
        </text>
        <text x={W / 2} y={H / 2 + 5} textAnchor="middle" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={3.5}>
          Bosch
        </text>

        {/* Decap caps near chip */}
        <rect x={W / 2 + 14} y={H / 2 - 4} width={3} height={5} fill="#888" stroke="#444" strokeWidth={0.3} />
        <rect x={W / 2 - 17} y={H / 2 - 4} width={3} height={5} fill="#888" stroke="#444" strokeWidth={0.3} />

        {/* Silkscreen title */}
        <text x={W / 2} y={10} textAnchor="middle" fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={7} fontWeight={700} letterSpacing="0.05em">
          BME280
        </text>
        <text x={W / 2} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.55)" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          T/H/P · I²C 0x76
        </text>

        {/* Left pads — VCC, GND, SCL, SDA */}
        <rect x={-3} y={6} width={9} height={8} fill="url(#bme-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={2} cy={10} r={1.5} fill="#0a0a0a" />
        <text x={10} y={12} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>VCC</text>

        <rect x={-3} y={18} width={9} height={8} fill="url(#bme-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={2} cy={22} r={1.5} fill="#0a0a0a" />
        <text x={10} y={24} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>GND</text>

        <rect x={-3} y={30} width={9} height={8} fill="url(#bme-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={2} cy={34} r={1.5} fill="#0a0a0a" />
        <text x={10} y={36} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>SCL</text>

        <rect x={-3} y={42} width={9} height={8} fill="url(#bme-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={2} cy={46} r={1.5} fill="#0a0a0a" />
        <text x={10} y={48} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>SDA</text>

        {/* Right pads — CSB, SDO */}
        <rect x={W - 6} y={6} width={9} height={8} fill="url(#bme-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={W - 2} cy={10} r={1.5} fill="#0a0a0a" />
        <text x={W - 10} y={12} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>CSB</text>

        <rect x={W - 6} y={18} width={9} height={8} fill="url(#bme-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
        <circle cx={W - 2} cy={22} r={1.5} fill="#0a0a0a" />
        <text x={W - 10} y={24} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>SDO</text>

        {/* Selection highlight */}
        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#3DD68C" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
