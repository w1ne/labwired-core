import type { ComponentDef } from '../types';

const W = 64;
const H = 64;

export const rotaryEncoderComponent: ComponentDef = {
  type: 'rotary-encoder',
  label: 'Rotary Encoder',
  category: 'input',
  width: W,
  height: H,
  pins: [
    { id: 'CLK', x: 0, y: 14, side: 'left', label: 'CLK' },
    { id: 'DT', x: 0, y: 32, side: 'left', label: 'DT' },
    { id: 'SW', x: 0, y: 50, side: 'left', label: 'SW' },
    { id: 'VCC', x: W, y: 22, side: 'right', label: 'VCC' },
    { id: 'GND', x: W, y: 42, side: 'right', label: 'GND' },
  ],
  defaultAttrs: {},
  boardIoKind: 'button',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    return (
      <g>
        <rect x={3} y={3} width={W - 6} height={H - 6} rx={6}
          fill="#1a3a6a" stroke={selected ? '#e83e8c' : '#0d2040'} strokeWidth={selected ? 2.5 : 1.5} />
        <circle cx={W / 2} cy={H / 2} r={20} fill="#888" stroke="#555" strokeWidth={1.5} />
        <circle cx={W / 2} cy={H / 2} r={14} fill="#aaa" stroke="#888" strokeWidth={1} />
        <line x1={W / 2} y1={H / 2 - 14} x2={W / 2} y2={H / 2 - 6}
          stroke="#333" strokeWidth={2.5} strokeLinecap="round" />
        <text x={10} y={18} fill="#569cd6" fontFamily="monospace" fontSize={6}>CLK</text>
        <text x={10} y={36} fill="#569cd6" fontFamily="monospace" fontSize={6}>DT</text>
        <text x={10} y={54} fill="#569cd6" fontFamily="monospace" fontSize={6}>SW</text>
      </g>
    );
  },
};
