import type { ComponentDef } from '../types';

const W = 56;
const H = 56;

export const pirSensorComponent: ComponentDef = {
  type: 'pir-sensor',
  label: 'PIR Sensor',
  category: 'sensor',
  width: W,
  height: H,
  pins: [
    { id: 'VCC', x: 0, y: 16, side: 'left', label: 'VCC' },
    { id: 'OUT', x: 0, y: H / 2, side: 'left', label: 'OUT' },
    { id: 'GND', x: 0, y: H - 16, side: 'left', label: 'GND' },
  ],
  defaultAttrs: {},
  boardIoKind: 'button',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    const active = state?.active;
    return (
      <g>
        <circle cx={W / 2} cy={H / 2} r={24}
          fill="#1a6a3a" stroke={selected ? '#e83e8c' : '#0d4d1e'} strokeWidth={selected ? 2.5 : 1.5} />
        <circle cx={W / 2} cy={H / 2} r={16}
          fill={active ? 'rgba(255,204,0,0.3)' : '#f8f8f8'} stroke="#ccc" strokeWidth={1} />
        <circle cx={W / 2} cy={H / 2} r={6} fill="#ddd" stroke="#bbb" strokeWidth={0.5} />
        {active && (
          <circle cx={W / 2} cy={H / 2} r={22} fill="none"
            stroke="#ffcc00" strokeWidth={1.5} strokeDasharray="4,4" opacity={0.6} />
        )}
        <text x={W / 2} y={H + 10} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={6}>PIR</text>
      </g>
    );
  },
};
