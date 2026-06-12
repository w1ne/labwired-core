import type { ComponentDef } from '../types';

const W = 118;
const H = 92;

export const canTransceiverComponent: ComponentDef = {
  type: 'can-transceiver',
  label: 'CAN Transceiver',
  category: 'ic',
  width: W,
  height: H,
  pins: [
    { id: 'TXD', x: 0, y: 24, side: 'left', label: 'TXD' },
    { id: 'RXD', x: 0, y: 44, side: 'left', label: 'RXD' },
    { id: 'VCC', x: 0, y: 64, side: 'left', label: 'VCC' },
    { id: 'GND', x: 0, y: 78, side: 'left', label: 'GND' },
    { id: 'CAN_H', x: W, y: 32, side: 'right', label: 'CAN_H' },
    { id: 'CAN_L', x: W, y: 56, side: 'right', label: 'CAN_L' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const uid = state?.id ?? 'can-xcvr';
    return (
      <g>
        <defs>
          <linearGradient id={`can-xcvr-body-${uid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2c3440" />
            <stop offset="1" stopColor="#121820" />
          </linearGradient>
        </defs>
        <rect
          width={W}
          height={H}
          rx={6}
          fill={`url(#can-xcvr-body-${uid})`}
          stroke={selected ? '#F5B642' : '#0b1017'}
          strokeWidth={selected ? 2.5 : 1.2}
        />
        <text x={W / 2} y={20} textAnchor="middle" fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700}>
          CAN
        </text>
        <text x={W / 2} y={31} textAnchor="middle" fill="#93a4b5" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          TRANSCEIVER
        </text>
        <rect x={30} y={44} width={W - 60} height={18} rx={2} fill="#07111b" stroke="#314154" strokeWidth={1} />
        <text x={W / 2} y={56} textAnchor="middle" fill="#d9f4ff" fontFamily="'JetBrains Mono', monospace" fontSize={7}>
          ISO 11898
        </text>
        {[
          { x: -3, y: 20, label: 'TXD', anchor: 'start' },
          { x: -3, y: 40, label: 'RXD', anchor: 'start' },
          { x: -3, y: 60, label: 'VCC', anchor: 'start' },
          { x: -3, y: 74, label: 'GND', anchor: 'start' },
          { x: W - 6, y: 28, label: 'CAN_H', anchor: 'end' },
          { x: W - 6, y: 52, label: 'CAN_L', anchor: 'end' },
        ].map(({ x, y, label, anchor }) => (
          <g key={label}>
            <rect x={x} y={y} width={9} height={8} fill="#c9a227" stroke="#7a5a1a" strokeWidth={0.3} />
            <text
              x={anchor === 'end' ? x - 4 : x + 13}
              y={y + 6}
              textAnchor={anchor as 'start' | 'end'}
              fill="#f4f7fb"
              fontFamily="'JetBrains Mono', monospace"
              fontSize={5.5}
              fontWeight={600}
            >
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
