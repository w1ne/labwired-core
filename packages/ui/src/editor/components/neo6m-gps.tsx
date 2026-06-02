import type { ComponentDef } from '../types';

// NEO-6M GPS breakout: ~120×40 PCB, typically blue, 4-pin header (VCC GND TX RX)
// plus an active patch antenna connector on the back edge.
const W = 120;
const H = 44;

export const neo6mGpsComponent: ComponentDef = {
  type: 'neo6m-gps',
  label: 'NEO-6M GPS',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'uart_device',
  pins: [
    { id: 'VCC', x: W, y: 8,  side: 'right', label: 'VCC' },
    { id: 'GND', x: W, y: 18, side: 'right', label: 'GND' },
    { id: 'TX',  x: W, y: 28, side: 'right', label: 'TX' },
    { id: 'RX',  x: W, y: 38, side: 'right', label: 'RX' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="neo-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2B5FCB" />
            <stop offset="1" stopColor="#0E2E7A" />
          </linearGradient>
          <linearGradient id="neo-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#080808" />
          </linearGradient>
          <linearGradient id="neo-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
          <pattern id="neo-dots" x="0" y="0" width="4" height="4" patternUnits="userSpaceOnUse">
            <circle cx={2} cy={2} r={0.25} fill="#0a1a50" opacity={0.7} />
          </pattern>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 4} ry={3} fill="#000" opacity={0.3} />

        {/* PCB body */}
        <rect
          width={W}
          height={H}
          rx={4}
          fill="url(#neo-pcb)"
          stroke={selected ? '#3DD68C' : '#0a1870'}
          strokeWidth={selected ? 2.5 : 1}
        />
        <rect width={W} height={H} rx={4} fill="url(#neo-dots)" opacity={0.4} />

        {/* u-blox NEO-6M IC — square black package in the left half */}
        <rect x={12} y={8} width={28} height={28} rx={1.5} fill="url(#neo-chip)" stroke="#000" strokeWidth={0.6} />
        <circle cx={14} cy={10} r={0.7} fill="#555" />
        <text x={26} y={21} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={3.5} fontWeight={700}>
          u-blox
        </text>
        <text x={26} y={26} textAnchor="middle" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={3}>
          NEO-6M
        </text>
        <text x={26} y={31} textAnchor="middle" fill="#666" fontFamily="'JetBrains Mono', monospace" fontSize={2.5}>
          GPS
        </text>

        {/* Patch antenna connector (ceramic block top-right) */}
        <rect x={52} y={4} width={28} height={20} rx={2} fill="#1a1a1a" stroke="#333" strokeWidth={0.5} />
        <rect x={56} y={7} width={20} height={14} rx={1} fill="#222" stroke="#555" strokeWidth={0.3} />
        <line x1={66} y1={7} x2={66} y2={21} stroke="#444" strokeWidth={0.4} />
        <line x1={56} y1={14} x2={76} y2={14} stroke="#444" strokeWidth={0.4} />
        <text x={66} y={30} textAnchor="middle" fill="rgba(255,255,255,0.4)" fontFamily="'JetBrains Mono', monospace" fontSize={3}>
          ANT
        </text>

        {/* Crystal oscillator (small rectangle) */}
        <rect x={48} y={28} width={10} height={6} rx={1} fill="#888" stroke="#555" strokeWidth={0.3} />

        {/* Silkscreen title */}
        <text x={W / 2 - 12} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.65)" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={700}>
          NEO-6M GPS
        </text>
        <text x={W / 2 + 16} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.35)" fontFamily="'JetBrains Mono', monospace" fontSize={4}>
          9600 baud
        </text>

        {/* Right-side header pads — VCC GND TX RX */}
        {[
          { y: 4,  label: 'VCC', color: '#ff6b6b' },
          { y: 14, label: 'GND', color: '#888' },
          { y: 24, label: 'TX',  color: '#5bd8ff' },
          { y: 34, label: 'RX',  color: '#a3e635' },
        ].map(({ y, label, color }) => (
          <g key={label}>
            <rect x={W - 6} y={y} width={9} height={8} fill="url(#neo-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={W - 2} cy={y + 4} r={1.4} fill="#0a0a0a" />
            <text x={W - 10} y={y + 6} textAnchor="end" fill={color} fontFamily="'JetBrains Mono', monospace" fontSize={5.5} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {/* Selection highlight */}
        {selected && (
          <rect width={W} height={H} rx={4} fill="none" stroke="#3DD68C" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
