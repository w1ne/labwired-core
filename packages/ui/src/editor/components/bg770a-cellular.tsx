import type { ComponentDef } from '../types';

// Quectel BG770A-GL LTE-M / NB-IoT cellular module: ~32×29 LCC package.
// Real silicon ships as an LGA, but rendered here on a small dev breakout PCB
// (~120×46) with a 4-pin header (VCC GND TX RX) and a tiny SMA antenna icon.
const W = 120;
const H = 46;

export const bg770aCellularComponent: ComponentDef = {
  type: 'bg770a-cellular',
  label: 'Quectel BG770A',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'uart_device',
  pins: [
    { id: 'VCC', x: W, y: 8, side: 'right', label: 'VCC' },
    { id: 'GND', x: W, y: 18, side: 'right', label: 'GND' },
    { id: 'TX', x: W, y: 28, side: 'right', label: 'TX' },
    { id: 'RX', x: W, y: 38, side: 'right', label: 'RX' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="bg770a-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1e3a3f" />
            <stop offset="1" stopColor="#0a1a1d" />
          </linearGradient>
          <linearGradient id="bg770a-lcc" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a3a3a" />
            <stop offset="1" stopColor="#0e0e0e" />
          </linearGradient>
          <linearGradient id="bg770a-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
          <pattern id="bg770a-dots" x="0" y="0" width="4" height="4" patternUnits="userSpaceOnUse">
            <circle cx={2} cy={2} r={0.25} fill="#06181a" opacity={0.7} />
          </pattern>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 4} ry={3} fill="#000" opacity={0.3} />

        {/* PCB body */}
        <rect
          width={W}
          height={H}
          rx={4}
          fill="url(#bg770a-pcb)"
          stroke={selected ? '#3DD68C' : '#062023'}
          strokeWidth={selected ? 2.5 : 1}
        />
        <rect width={W} height={H} rx={4} fill="url(#bg770a-dots)" opacity={0.4} />

        {/* LCC module — square dark package in the left half */}
        <rect x={10} y={8} width={36} height={30} rx={1.5} fill="url(#bg770a-lcc)" stroke="#000" strokeWidth={0.6} />
        <circle cx={12} cy={10} r={0.7} fill="#666" />
        <text x={28} y={20} textAnchor="middle" fill="#d6e8ec" fontFamily="'JetBrains Mono', monospace" fontSize={3.6} fontWeight={700}>
          Quectel
        </text>
        <text x={28} y={25} textAnchor="middle" fill="#9ab" fontFamily="'JetBrains Mono', monospace" fontSize={3.4} fontWeight={700}>
          BG770A
        </text>
        <text x={28} y={30} textAnchor="middle" fill="#789" fontFamily="'JetBrains Mono', monospace" fontSize={2.8}>
          LTE-M
        </text>

        {/* Tiny SMA / U.FL antenna pad top-right of module */}
        <circle cx={56} cy={12} r={3.2} fill="#1a1a1a" stroke="#444" strokeWidth={0.4} />
        <circle cx={56} cy={12} r={1} fill="#888" />
        <text x={56} y={22} textAnchor="middle" fill="rgba(255,255,255,0.4)" fontFamily="'JetBrains Mono', monospace" fontSize={2.8}>
          ANT
        </text>

        {/* SIM slot silhouette to the right of module */}
        <rect x={52} y={26} width={26} height={12} rx={0.8} fill="#0e1517" stroke="#2a3a3d" strokeWidth={0.4} />
        <rect x={54} y={28} width={22} height={8} rx={0.4} fill="none" stroke="#3a4a4d" strokeWidth={0.3} strokeDasharray="1,1" />
        <text x={65} y={34} textAnchor="middle" fill="rgba(255,255,255,0.35)" fontFamily="'JetBrains Mono', monospace" fontSize={2.8}>
          SIM
        </text>

        {/* Silkscreen title */}
        <text x={W / 2 - 12} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.65)" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={700}>
          BG770A-GL
        </text>
        <text x={W / 2 + 18} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.35)" fontFamily="'JetBrains Mono', monospace" fontSize={4}>
          AT 115200
        </text>

        {/* Right-side header pads — VCC GND TX RX */}
        {[
          { y: 4, label: 'VCC', color: '#ff6b6b' },
          { y: 14, label: 'GND', color: '#888' },
          { y: 24, label: 'TX', color: '#5bd8ff' },
          { y: 34, label: 'RX', color: '#a3e635' },
        ].map(({ y, label, color }) => (
          <g key={label}>
            <rect x={W - 6} y={y} width={9} height={8} fill="url(#bg770a-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
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
