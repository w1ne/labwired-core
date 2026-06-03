import type { ComponentDef } from '../types';

const W = 132;
const H = 112;

const CHANNELS = ['CH0', 'CH1', 'CH2', 'CH3'];

export const logicAnalyzerComponent: ComponentDef = {
  type: 'logic-analyzer',
  label: 'Logic Analyzer',
  category: 'tool',
  width: W,
  height: H,
  pins: [
    ...CHANNELS.map((id, index) => ({
      id,
      x: 0,
      y: 32 + index * 17,
      side: 'left' as const,
      label: id,
      probe: true,
    })),
    { id: 'GND', x: W / 2, y: H, side: 'bottom' as const, label: 'GND', probe: true },
  ],
  defaultAttrs: { decoder: 'raw' },
  attrFields: [
    {
      key: 'decoder',
      label: 'Decoder',
      type: 'select',
      options: ['raw', 'iolink', 'uart', 'spi'],
      defaultValue: 'raw',
    },
  ],
  render: (attrs, state) => {
    const selected = !!state?.selected;
    const uid = state?.id ?? 'logic-analyzer';
    const decoder = (attrs.decoder ?? 'raw').toUpperCase();
    return (
      <g>
        <defs>
          <linearGradient id={`la-body-${uid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#263241" />
            <stop offset="1" stopColor="#111923" />
          </linearGradient>
        </defs>
        <rect
          width={W}
          height={H}
          rx={7}
          fill={`url(#la-body-${uid})`}
          stroke={selected ? '#F5B642' : '#0b1017'}
          strokeWidth={selected ? 2.5 : 1.2}
        />
        <rect x={14} y={12} width={W - 28} height={22} rx={3} fill="#08111b" stroke="#314154" strokeWidth={1} />
        <text x={20} y={26} fill="#d9f4ff" fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={700}>
          LOGIC ANALYZER
        </text>
        <text x={W - 16} y={49} textAnchor="end" fill="#8aa1b6" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          {decoder}
        </text>
        {CHANNELS.map((channel, index) => {
          const y = 32 + index * 17;
          return (
            <g key={channel}>
              <rect x={-3} y={y - 4} width={10} height={8} rx={1} fill="#c9a227" stroke="#725717" strokeWidth={0.4} />
              <text x={14} y={y + 2.5} fill="#f4f7fb" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>
                {channel}
              </text>
              <path
                d={`M 43 ${y + 1} h 10 l 5 -7 l 5 14 l 5 -8 h 16`}
                fill="none"
                stroke={index === 0 ? '#37d67a' : index === 1 ? '#5bd8ff' : index === 2 ? '#ffd166' : '#ef476f'}
                strokeWidth={1.4}
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </g>
          );
        })}
        <text x={W / 2} y={H - 9} textAnchor="middle" fill="#6f8193" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          PROBE INPUTS
        </text>
        <rect x={W / 2 - 5} y={H - 3} width={10} height={6} rx={1} fill="#636b75" stroke="#202833" strokeWidth={0.4} />
        {selected && (
          <rect width={W} height={H} rx={7} fill="none" stroke="#F5B642" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
