import type { ComponentDef } from '../types';

// M12 A-coded 5-pin IO-Link field connector (the panel/field side of an IO-Link
// device). Pin 1 = L+ (24V bus power), pin 3 = L- (0V), pin 4 = C/Q (the IO-Link
// communication line, driven by the MCU UART through an IO-Link PHY). Passive
// connector — no sim state.
const W = 96;
const H = 96;

export const m12IoLinkComponent: ComponentDef = {
  type: 'm12-iolink',
  label: 'M12 IO-Link',
  category: 'ic',
  width: W,
  height: H,
  boardIoKind: 'uart_device',
  pins: [
    { id: 'CQ', x: 0, y: 30, side: 'left', label: 'C/Q' },
    { id: 'L+', x: 0, y: 50, side: 'left', label: 'L+' },
    { id: 'L-', x: 0, y: 70, side: 'left', label: 'L-' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const cx = W / 2 + 12;
    const cy = H / 2;
    // 5-pin A-coded layout: 4 around + 1 center.
    const pins5: Array<[number, number]> = [
      [0, -8],
      [-7.5, -1],
      [7.5, -1],
      [-4.5, 7],
      [4.5, 7],
    ];
    return (
      <g>
        <defs>
          <linearGradient id="m12-body" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a4453" />
            <stop offset="1" stopColor="#1c232e" />
          </linearGradient>
          <radialGradient id="m12-face" cx="0.4" cy="0.35" r="0.8">
            <stop offset="0" stopColor="#cfd6de" />
            <stop offset="0.7" stopColor="#8a939e" />
            <stop offset="1" stopColor="#454c56" />
          </radialGradient>
        </defs>

        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 6} ry={3} fill="#000" opacity={0.35} />

        {/* Housing */}
        <rect
          width={W}
          height={H}
          rx={7}
          fill="url(#m12-body)"
          stroke={selected ? '#F5B642' : '#10161e'}
          strokeWidth={selected ? 2.5 : 1.2}
        />

        {/* Title */}
        <text x={12} y={18} fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700} letterSpacing="0.03em">
          M12
        </text>
        <text x={12} y={28} fill="rgba(255,255,255,0.55)" fontFamily="'JetBrains Mono', monospace" fontSize={5}>
          IO-LINK
        </text>

        {/* Knurled M12 thread ring + face */}
        <circle cx={cx} cy={cy} r={20} fill="#0a0f15" stroke="#2a3340" strokeWidth={1.2} />
        {Array.from({ length: 24 }, (_, i) => {
          const a = (i / 24) * Math.PI * 2;
          return (
            <line
              key={i}
              x1={cx + Math.cos(a) * 18}
              y1={cy + Math.sin(a) * 18}
              x2={cx + Math.cos(a) * 20}
              y2={cy + Math.sin(a) * 20}
              stroke="#39424f"
              strokeWidth={0.8}
            />
          );
        })}
        <circle cx={cx} cy={cy} r={15} fill="url(#m12-face)" stroke="#39424f" strokeWidth={1} />

        {/* 5 gold contacts (A-coded) */}
        {pins5.map(([dx, dy], i) => (
          <circle key={i} cx={cx + dx} cy={cy + dy} r={2.2} fill="#d8b13a" stroke="#7a5a1a" strokeWidth={0.4} />
        ))}

        {/* Left pads — C/Q, L+, L- */}
        {[
          { y: 26, label: 'C/Q' },
          { y: 46, label: 'L+' },
          { y: 66, label: 'L-' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={-3} y={y} width={9} height={8} fill="#c9a227" stroke="#7a5a1a" strokeWidth={0.3} />
            <text x={10} y={y + 6} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={5.5} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {selected && (
          <rect width={W} height={H} rx={7} fill="none" stroke="#F5B642" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
