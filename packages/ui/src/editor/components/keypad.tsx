import type { ComponentDef } from '../types';

const W = 88;
const H = 108;

export const keypadComponent: ComponentDef = {
  type: 'keypad',
  label: '4x4 Keypad',
  category: 'input',
  width: W,
  height: H,
  pins: [
    { id: 'R1', x: 0, y: 16, side: 'left', label: 'R1' },
    { id: 'R2', x: 0, y: 38, side: 'left', label: 'R2' },
    { id: 'R3', x: 0, y: 60, side: 'left', label: 'R3' },
    { id: 'R4', x: 0, y: 82, side: 'left', label: 'R4' },
    { id: 'C1', x: W, y: 16, side: 'right', label: 'C1' },
    { id: 'C2', x: W, y: 38, side: 'right', label: 'C2' },
    { id: 'C3', x: W, y: 60, side: 'right', label: 'C3' },
    { id: 'C4', x: W, y: 82, side: 'right', label: 'C4' },
  ],
  defaultAttrs: {},
  boardIoKind: 'button',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    const keys = [
      ['1', '2', '3', 'A'],
      ['4', '5', '6', 'B'],
      ['7', '8', '9', 'C'],
      ['*', '0', '#', 'D'],
    ];
    const bw = 16, bh = 16, sx = 10, sy = 10, gap = 3;
    return (
      <g>
        <rect x={3} y={3} width={W - 6} height={H - 6} rx={6}
          fill="#f0f0f0" stroke={selected ? '#e83e8c' : '#888'} strokeWidth={selected ? 2.5 : 1.5} />
        {keys.map((row, ri) =>
          row.map((key, ci) => (
            <g key={`${ri}-${ci}`}>
              <rect
                x={sx + ci * (bw + gap)} y={sy + ri * (bh + gap + 4)}
                width={bw} height={bh} rx={3}
                fill="#ddd" stroke="#999" strokeWidth={0.5}
              />
              <text
                x={sx + ci * (bw + gap) + bw / 2}
                y={sy + ri * (bh + gap + 4) + bh / 2 + 4}
                textAnchor="middle" fill="#333" fontFamily="monospace" fontSize={9}
              >
                {key}
              </text>
            </g>
          ))
        )}
      </g>
    );
  },
};
