import type { ComponentDef } from '../types';

const W = 56;
const H = 64;

export const transistorComponent: ComponentDef = {
  type: 'transistor',
  label: 'Transistor',
  category: 'passive',
  width: W,
  height: H,
  pins: [
    { id: 'B', x: 0, y: H / 2, side: 'left', label: 'B' },
    { id: 'C', x: W, y: 10, side: 'right', label: 'C' },
    { id: 'E', x: W, y: H - 10, side: 'right', label: 'E' },
  ],
  defaultAttrs: { type: 'NPN', part: '2N2222' },
  attrFields: [
    { key: 'type', label: 'Type', type: 'select', options: ['NPN', 'PNP'] },
    { key: 'part', label: 'Part Number', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const isNPN = attrs.type !== 'PNP';
    const cx = W / 2, cy = H / 2;
    return (
      <g>
        <circle cx={cx} cy={cy} r={22}
          fill="#f8f9fa" stroke={selected ? '#e83e8c' : '#000'} strokeWidth={selected ? 2.5 : 1.5} />
        <line x1={0} y1={cy} x2={cx - 8} y2={cy} stroke="#444" strokeWidth={2} />
        <line x1={cx - 8} y1={cy - 10} x2={cx - 8} y2={cy + 10} stroke="#444" strokeWidth={2} />
        <line x1={cx - 8} y1={cy - 6} x2={cx + 10} y2={cy - 16} stroke="#444" strokeWidth={2} />
        <line x1={cx + 10} y1={cy - 16} x2={W} y2={10} stroke="#444" strokeWidth={2} />
        <line x1={cx - 8} y1={cy + 6} x2={cx + 10} y2={cy + 16} stroke="#444" strokeWidth={2} />
        <line x1={cx + 10} y1={cy + 16} x2={W} y2={H - 10} stroke="#444" strokeWidth={2} />
        {isNPN ? (
          <polygon points={`${cx + 10},${cy + 16} ${cx + 4},${cy + 10} ${cx + 6},${cy + 18}`}
            fill="#444" />
        ) : (
          <polygon points={`${cx - 6},${cy - 8} ${cx + 2},${cy - 12} ${cx},${cy - 2}`}
            fill="#444" />
        )}
        <text x={6} y={cy - 6} fill="#888" fontFamily="monospace" fontSize={7}>B</text>
        <text x={W - 6} y={16} textAnchor="end" fill="#888" fontFamily="monospace" fontSize={7}>C</text>
        <text x={W - 6} y={H - 6} textAnchor="end" fill="#888" fontFamily="monospace" fontSize={7}>E</text>
        <text x={cx} y={H + 10} textAnchor="middle" fill="#444"
          fontFamily="monospace" fontSize={7}>{attrs.part || '2N2222'}</text>
      </g>
    );
  },
};
