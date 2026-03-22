import type { ComponentDef } from '../types';

const W = 64;
const H = 80;

export const rgbLedComponent: ComponentDef = {
  type: 'rgb-led',
  label: 'RGB LED',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: 'R', x: 10, y: 0, side: 'top', label: 'R' },
    { id: 'G', x: W / 2, y: 0, side: 'top', label: 'G' },
    { id: 'B', x: W - 10, y: 0, side: 'top', label: 'B' },
    { id: 'GND', x: W / 2, y: H, side: 'bottom', label: 'GND' },
  ],
  defaultAttrs: {},
  boardIoKind: 'led',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    const active = state?.active;
    return (
      <g>
        <rect x={6} y={16} width={W - 12} height={H - 32} rx={6}
          fill="#f8f9fa" stroke={selected ? '#e83e8c' : '#000'} strokeWidth={selected ? 2.5 : 1.5} />
        <circle cx={16} cy={H / 2} r={10} fill={active ? '#ff3333' : '#661111'} stroke="#000" strokeWidth={0.5} />
        <circle cx={W / 2} cy={H / 2} r={10} fill={active ? '#27c93f' : '#0d4d16'} stroke="#000" strokeWidth={0.5} />
        <circle cx={W - 16} cy={H / 2} r={10} fill={active ? '#3399ff' : '#0d2d4d'} stroke="#000" strokeWidth={0.5} />
        <text x={10} y={12} textAnchor="middle" fill="#ff3333" fontFamily="monospace" fontSize={7}>R</text>
        <text x={W / 2} y={12} textAnchor="middle" fill="#27c93f" fontFamily="monospace" fontSize={7}>G</text>
        <text x={W - 10} y={12} textAnchor="middle" fill="#3399ff" fontFamily="monospace" fontSize={7}>B</text>
        <text x={W / 2} y={H - 4} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={7}>GND</text>
      </g>
    );
  },
};
