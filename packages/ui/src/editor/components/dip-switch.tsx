import type { ComponentDef } from '../types';

const W = 64;
const H = 40;

export const dipSwitchComponent: ComponentDef = {
  type: 'dip-switch',
  label: 'DIP Switch',
  category: 'input',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 8, y: H, side: 'bottom', label: '1' },
    { id: '2', x: 24, y: H, side: 'bottom', label: '2' },
    { id: '3', x: 40, y: H, side: 'bottom', label: '3' },
    { id: '4', x: 56, y: H, side: 'bottom', label: '4' },
    { id: 'C1', x: 8, y: 0, side: 'top', label: 'C1' },
    { id: 'C2', x: 24, y: 0, side: 'top', label: 'C2' },
    { id: 'C3', x: 40, y: 0, side: 'top', label: 'C3' },
    { id: 'C4', x: 56, y: 0, side: 'top', label: 'C4' },
  ],
  defaultAttrs: {},
  boardIoKind: 'button',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    return (
      <g>
        <rect x={0} y={3} width={W} height={H - 6} rx={3}
          fill="#cc2222" stroke={selected ? '#e83e8c' : '#881111'} strokeWidth={selected ? 2.5 : 1.5} />
        {[8, 24, 40, 56].map((x, i) => (
          <g key={i}>
            <rect x={x - 4} y={8} width={8} height={22} rx={1} fill="#fff" opacity={0.2} />
            <rect x={x - 3.5} y={8} width={7} height={11} rx={1} fill="#eee" />
          </g>
        ))}
        <text x={W / 2} y={H + 10} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={6}>DIP-4</text>
      </g>
    );
  },
};
