import type { ComponentDef } from '../types';

// 74HC165 — 8-bit parallel-in / serial-out shift register, used as a digital
// INPUT expander. The 8 parallel channels (D0..D7) are read serially over SPI.
// Live state: `state.analogValue` carries the current input byte (bit i = Dn);
// each channel pad lights when its bit is high.
const W = 116;
const H = 104;

export const sn74hc165Component: ComponentDef = {
  type: 'sn74hc165',
  label: '74HC165',
  category: 'ic',
  width: W,
  height: H,
  boardIoKind: 'spi_device',
  pins: [
    // SPI / control side (right)
    { id: 'VCC', x: W, y: 12, side: 'right', label: 'VCC' },
    { id: 'GND', x: W, y: 28, side: 'right', label: 'GND' },
    { id: 'CLK', x: W, y: 44, side: 'right', label: 'CLK' },
    { id: 'QH', x: W, y: 60, side: 'right', label: 'QH' },
    { id: 'SH_LD', x: W, y: 76, side: 'right', label: 'SH/LD' },
    // 8 parallel digital inputs (left): D0..D7
    { id: 'D0', x: 0, y: 14, side: 'left', label: 'D0' },
    { id: 'D1', x: 0, y: 25, side: 'left', label: 'D1' },
    { id: 'D2', x: 0, y: 36, side: 'left', label: 'D2' },
    { id: 'D3', x: 0, y: 47, side: 'left', label: 'D3' },
    { id: 'D4', x: 0, y: 58, side: 'left', label: 'D4' },
    { id: 'D5', x: 0, y: 69, side: 'left', label: 'D5' },
    { id: 'D6', x: 0, y: 80, side: 'left', label: 'D6' },
    { id: 'D7', x: 0, y: 91, side: 'left', label: 'D7' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const uid = state?.id ?? 'sn74hc165';
    const inputs = Math.max(0, Math.min(255, Math.round(state?.analogValue ?? 0)));
    return (
      <g>
        <defs>
          <linearGradient id={`hc165-body-${uid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a3a3f" />
            <stop offset="1" stopColor="#161618" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 6} ry={3} fill="#000" opacity={0.35} />

        {/* DIP body */}
        <rect
          width={W}
          height={H}
          rx={6}
          fill={`url(#hc165-body-${uid})`}
          stroke={selected ? '#F5B642' : '#000'}
          strokeWidth={selected ? 2.5 : 1}
        />
        {/* Pin-1 notch */}
        <path d={`M ${W / 2 - 7} 0 A 7 7 0 0 0 ${W / 2 + 7} 0 Z`} fill="#0a0a0a" />

        {/* Silkscreen */}
        <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={11} fontWeight={700} letterSpacing="0.04em">
          74HC165
        </text>
        <text x={W / 2} y={H / 2 + 8} textAnchor="middle" fill="rgba(255,255,255,0.5)" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          PISO · SPI-IN
        </text>

        {/* 8 input channel indicators (left), lit per `inputs` bits */}
        {Array.from({ length: 8 }, (_, ch) => {
          const y = 10 + ch * 11;
          const high = (inputs & (1 << ch)) !== 0;
          return (
            <g key={ch}>
              <rect x={6} y={y} width={9} height={8} rx={1.5} fill={high ? '#37d67a' : '#243024'} stroke={high ? '#9affc4' : '#11331c'} strokeWidth={0.6} />
              {high && <rect x={6} y={y} width={9} height={8} rx={1.5} fill="#37d67a" opacity={0.5} />}
            </g>
          );
        })}

        {/* Right control pads */}
        {[
          { y: 8, label: 'VCC' },
          { y: 24, label: 'GND' },
          { y: 40, label: 'CLK' },
          { y: 56, label: 'QH' },
          { y: 72, label: 'SH/LD' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={W - 6} y={y} width={9} height={8} fill="#c9a227" stroke="#7a5a1a" strokeWidth={0.3} />
            <text x={W - 10} y={y + 6} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={5} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {selected && (
          <rect width={W} height={H} rx={6} fill="none" stroke="#F5B642" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
