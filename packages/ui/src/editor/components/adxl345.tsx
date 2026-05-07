import type { ComponentDef } from '../types';

const W = 96;
const H = 64;

export const adxl345Component: ComponentDef = {
  type: 'adxl345',
  label: 'ADXL345',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'i2c_device',
  pins: [
    { id: 'VCC', x: 0, y: 14, side: 'left', label: 'VCC' },
    { id: 'GND', x: 0, y: 30, side: 'left', label: 'GND' },
    { id: 'SDA', x: W, y: 22, side: 'right', label: 'SDA' },
    { id: 'SCL', x: W, y: 42, side: 'right', label: 'SCL' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect
        width={W}
        height={H}
        rx={6}
        fill={state?.selected ? '#fff7fb' : '#f8f9fa'}
        stroke="#111"
        strokeWidth={2}
      />
      <rect x={25} y={18} width={46} height={28} rx={3} fill="#111" />
      <text
        x={W / 2}
        y={35}
        textAnchor="middle"
        fontSize={9}
        fill="#fff"
        fontFamily="monospace"
      >
        ADXL345
      </text>
      <circle cx={10} cy={14} r={3} fill="#e83e8c" />
      <circle cx={10} cy={30} r={3} fill="#444" />
      <circle cx={W - 10} cy={22} r={3} fill="#27c93f" />
      <circle cx={W - 10} cy={42} r={3} fill="#569cd6" />
    </g>
  ),
};
