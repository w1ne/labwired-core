import type { ComponentDef } from '../types';

const W = 88;
const H = 88;
const ROWS = 8;
const COLS = 8;

export const ledMatrixComponent: ComponentDef = {
  type: 'led-matrix',
  label: '8x8 LED Matrix',
  category: 'display',
  width: W,
  height: H,
  pins: [
    ...Array.from({ length: ROWS }, (_, i) => ({
      id: `R${i + 1}`,
      x: 0,
      y: 10 + i * 9,
      side: 'left' as const,
      label: `R${i + 1}`,
    })),
    ...Array.from({ length: COLS }, (_, i) => ({
      id: `C${i + 1}`,
      x: W,
      y: 10 + i * 9,
      side: 'right' as const,
      label: `C${i + 1}`,
    })),
  ],
  defaultAttrs: { color: 'red' },
  boardIoKind: 'spi_device',
  attrFields: [
    { key: 'color', label: 'LED Color', type: 'select', options: ['red', 'green', 'blue'] },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const color = attrs.color || 'red';
    const dotColor = { red: '#661111', green: '#0d4d16', blue: '#0d2d4d' }[color] || '#661111';
    const dotSize = 3.5;
    const sx = 14, sy = 8;
    const gx = (W - 2 * sx) / (COLS - 1);
    const gy = (H - 2 * sy) / (ROWS - 1);
    return (
      <g>
        <rect x={3} y={3} width={W - 6} height={H - 6} rx={4}
          fill="#1a1a1a" stroke={selected ? '#e83e8c' : '#333'} strokeWidth={selected ? 2.5 : 1.5} />
        {Array.from({ length: ROWS }, (_, r) =>
          Array.from({ length: COLS }, (_, c) => (
            <circle key={`${r}-${c}`}
              cx={sx + c * gx} cy={sy + r * gy} r={dotSize}
              fill={dotColor} opacity={0.6}
            />
          ))
        )}
      </g>
    );
  },
};
