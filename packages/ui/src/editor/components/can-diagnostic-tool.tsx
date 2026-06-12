import type { ComponentDef } from '../types';

const W = 138;
const H = 96;

export const canDiagnosticToolComponent: ComponentDef = {
  type: 'can-diagnostic-tool',
  label: 'UDS Tester',
  category: 'tool',
  width: W,
  height: H,
  pins: [
    { id: 'CAN_H', x: 0, y: 34, side: 'left', label: 'CAN_H' },
    { id: 'CAN_L', x: 0, y: 54, side: 'left', label: 'CAN_L' },
    { id: 'GND', x: 0, y: 74, side: 'left', label: 'GND' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const uid = state?.id ?? 'uds-tester';
    return (
      <g>
        <defs>
          <linearGradient id={`uds-tool-body-${uid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#27313d" />
            <stop offset="1" stopColor="#111820" />
          </linearGradient>
        </defs>
        <rect
          width={W}
          height={H}
          rx={7}
          fill={`url(#uds-tool-body-${uid})`}
          stroke={selected ? '#F5B642' : '#0b1017'}
          strokeWidth={selected ? 2.5 : 1.2}
        />
        <rect x={14} y={12} width={W - 28} height={24} rx={3} fill="#07111b" stroke="#314154" strokeWidth={1} />
        <text x={20} y={27} fill="#d9f4ff" fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={700}>
          UDS TESTER
        </text>
        <circle cx={W - 25} cy={51} r={13} fill="#0a121c" stroke="#3a4a5e" strokeWidth={1.4} />
        <circle cx={W - 25} cy={51} r={8} fill="#1a2636" stroke="#2a3a4e" strokeWidth={0.8} />
        <circle cx={W - 29} cy={48} r={1.5} fill="#f3cf65" />
        <circle cx={W - 21} cy={48} r={1.5} fill="#f3cf65" />
        <circle cx={W - 25} cy={56} r={1.5} fill="#f3cf65" />
        {[
          { y: 30, label: 'CAN_H' },
          { y: 50, label: 'CAN_L' },
          { y: 70, label: 'GND' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={-3} y={y} width={9} height={8} fill="#c9a227" stroke="#7a5a1a" strokeWidth={0.3} />
            <text x={10} y={y + 6} fill="#f4f7fb" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>
              {label}
            </text>
          </g>
        ))}
        <text x={14} y={H - 10} fill="#7f93a7" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          DIAG CLIENT
        </text>
        {selected && (
          <rect width={W} height={H} rx={7} fill="none" stroke="#F5B642" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
