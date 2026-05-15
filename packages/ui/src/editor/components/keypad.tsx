import type { ComponentDef } from '../types';

const W = 88;
const H = 108;

const KEYS = [
  ['1', '2', '3', 'A'],
  ['4', '5', '6', 'B'],
  ['7', '8', '9', 'C'],
  ['*', '0', '#', 'D'],
];

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
    const selected = !!state?.selected;
    const bw = 14;
    const bh = 14;
    const sx = 10;
    const sy = 12;
    const hgap = 4;
    const vgap = 5;

    return (
      <g>
        <defs>
          <linearGradient id="keypad-membrane" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1a1a1a" />
            <stop offset="1" stopColor="#0a0a0a" />
          </linearGradient>
          <radialGradient id="keypad-key" cx="0.4" cy="0.3" r="0.7">
            <stop offset="0" stopColor="#4a4a4a" />
            <stop offset="1" stopColor="#1c1c1c" />
          </radialGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 1} rx={W / 2 - 6} ry={3} fill="#000" opacity={0.35} />

        {/* Membrane body (black plastic) */}
        <rect x={3} y={3} width={W - 6} height={H - 6} rx={5} fill="url(#keypad-membrane)" stroke={selected ? '#F062B8' : '#000'} strokeWidth={selected ? 2.5 : 1} />

        {/* Subtle inner border (overlay laminate edge) */}
        <rect x={5} y={5} width={W - 10} height={H - 10} rx={4} fill="none" stroke="#3a3a3a" strokeWidth={0.5} opacity={0.6} />

        {/* Key buttons */}
        {KEYS.map((row, ri) =>
          row.map((key, ci) => {
            const x = sx + ci * (bw + hgap);
            const y = sy + ri * (bh + vgap);
            return (
              <g key={`${ri}-${ci}`}>
                <rect
                  x={x}
                  y={y}
                  width={bw}
                  height={bh}
                  rx={2.5}
                  fill="url(#keypad-key)"
                  stroke="#000"
                  strokeWidth={0.5}
                />
                <rect x={x + 1.5} y={y + 1} width={bw - 3} height={1.5} rx={1} fill="rgba(255,255,255,0.15)" />
                <text
                  x={x + bw / 2}
                  y={y + bh / 2 + 3.5}
                  textAnchor="middle"
                  fill="#e0e0e0"
                  fontFamily="'JetBrains Mono', monospace"
                  fontSize={9}
                  fontWeight={600}
                >
                  {key}
                </text>
              </g>
            );
          }),
        )}

        {/* Silkscreen */}
        <text x={W / 2} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.35)" fontFamily="'Outfit', sans-serif" fontSize={5.5} fontWeight={600} letterSpacing="0.08em">
          4×4 MATRIX
        </text>
      </g>
    );
  },
};
