import type { ComponentDef } from '../types';

const W = 64;
const H = 32;

export const diodeComponent: ComponentDef = {
  type: 'diode',
  label: 'Diode',
  category: 'passive',
  width: W,
  height: H,
  pins: [
    { id: 'A', x: 0, y: H / 2, side: 'left', label: 'A' },
    { id: 'C', x: W, y: H / 2, side: 'right', label: 'C' },
  ],
  defaultAttrs: { type: '1N4148' },
  attrFields: [
    { key: 'type', label: 'Part Number', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const strokeColor = selected ? '#e83e8c' : '#000';
    const sw = selected ? 2.5 : 2;
    const cx = W / 2, cy = H / 2;
    return (
      <g>
        <line x1={0} y1={cy} x2={cx - 10} y2={cy} stroke="#444" strokeWidth={2} />
        <line x1={cx + 10} y1={cy} x2={W} y2={cy} stroke="#444" strokeWidth={2} />
        <polygon points={`${cx - 10},${cy - 9} ${cx - 10},${cy + 9} ${cx + 8},${cy}`}
          fill="none" stroke={strokeColor} strokeWidth={sw} strokeLinejoin="round" />
        <line x1={cx + 8} y1={cy - 9} x2={cx + 8} y2={cy + 9}
          stroke={strokeColor} strokeWidth={sw} />
        <rect x={cx + 8} y={cy - 9} width={4} height={18} fill="#444" opacity={0.3} />
        <text x={cx} y={cy - 11} textAnchor="middle" fill="#444"
          fontFamily="'JetBrains Mono', monospace" fontSize={7}>{attrs.type || '1N4148'}</text>
      </g>
    );
  },
};
