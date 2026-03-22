import type { ComponentDef } from '../types';

const W = 80;
const H = 32;

export const resistorComponent: ComponentDef = {
  type: 'resistor',
  label: 'Resistor',
  category: 'passive',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 0, y: H / 2, side: 'left', label: '1' },
    { id: '2', x: W, y: H / 2, side: 'right', label: '2' },
  ],
  defaultAttrs: { value: '220' },
  attrFields: [
    { key: 'value', label: 'Resistance (Ω)', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const value = attrs.value || '220';
    return (
      <g>
        <line x1={0} y1={H / 2} x2={14} y2={H / 2} stroke="#444" strokeWidth={2} />
        <line x1={W - 14} y1={H / 2} x2={W} y2={H / 2} stroke="#444" strokeWidth={2} />
        <polyline
          points={`14,${H / 2} 20,${H / 2 - 8} 26,${H / 2 + 8} 32,${H / 2 - 8} 38,${H / 2 + 8} 44,${H / 2 - 8} 50,${H / 2 + 8} 56,${H / 2 - 8} 62,${H / 2 + 8} 66,${H / 2}`}
          fill="none"
          stroke={selected ? '#e83e8c' : '#000'}
          strokeWidth={selected ? 2.5 : 2}
          strokeLinejoin="round"
        />
        <text x={W / 2} y={H / 2 - 12} textAnchor="middle" fill="#444"
          fontFamily="'JetBrains Mono', monospace" fontSize={9}>
          {value}Ω
        </text>
      </g>
    );
  },
};
