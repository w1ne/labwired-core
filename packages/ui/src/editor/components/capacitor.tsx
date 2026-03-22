import type { ComponentDef } from '../types';

const W = 44;
const H = 32;

export const capacitorComponent: ComponentDef = {
  type: 'capacitor',
  label: 'Capacitor',
  category: 'passive',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 0, y: H / 2, side: 'left', label: '+' },
    { id: '2', x: W, y: H / 2, side: 'right', label: '-' },
  ],
  defaultAttrs: { value: '100nF' },
  attrFields: [
    { key: 'value', label: 'Capacitance', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const value = attrs.value || '100nF';
    const strokeColor = selected ? '#e83e8c' : '#000';
    const sw = selected ? 2.5 : 2;
    return (
      <g>
        <line x1={0} y1={H / 2} x2={W / 2 - 4} y2={H / 2} stroke="#444" strokeWidth={2} />
        <line x1={W / 2 + 4} y1={H / 2} x2={W} y2={H / 2} stroke="#444" strokeWidth={2} />
        <line x1={W / 2 - 4} y1={H / 2 - 10} x2={W / 2 - 4} y2={H / 2 + 10}
          stroke={strokeColor} strokeWidth={sw} />
        <line x1={W / 2 + 4} y1={H / 2 - 10} x2={W / 2 + 4} y2={H / 2 + 10}
          stroke={strokeColor} strokeWidth={sw} />
        <text x={W / 2} y={H / 2 - 12} textAnchor="middle" fill="#444"
          fontFamily="'JetBrains Mono', monospace" fontSize={8}>{value}</text>
      </g>
    );
  },
};
