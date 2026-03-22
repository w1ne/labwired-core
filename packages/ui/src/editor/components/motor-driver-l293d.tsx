import type { ComponentDef } from '../types';

const W = 80;
const H = 140;
const PIN_SPACING = 16;

const LEFT_PINS = ['EN1,2', 'IN1', 'OUT1', 'GND', 'GND', 'OUT2', 'IN2', 'VS'];
const RIGHT_PINS = ['VSS', 'IN4', 'OUT4', 'GND', 'GND', 'OUT3', 'IN3', 'EN3,4'];

export const motorDriverL293dComponent: ComponentDef = {
  type: 'l293d',
  label: 'L293D',
  category: 'ic',
  width: W,
  height: H,
  pins: [
    ...LEFT_PINS.map((label, i) => ({
      id: `L${i + 1}`,
      x: 0,
      y: 14 + i * PIN_SPACING,
      side: 'left' as const,
      label,
    })),
    ...RIGHT_PINS.map((label, i) => ({
      id: `R${i + 1}`,
      x: W,
      y: 14 + i * PIN_SPACING,
      side: 'right' as const,
      label,
    })),
  ],
  defaultAttrs: {},
  boardIoKind: 'pwm_output',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    return (
      <g>
        <rect x={8} y={3} width={W - 16} height={H - 6} rx={3}
          fill="#333" stroke={selected ? '#e83e8c' : '#111'} strokeWidth={selected ? 2.5 : 1.5} />
        <circle cx={W / 2} cy={8} r={5} fill="none" stroke="#555" strokeWidth={1} />
        <circle cx={16} cy={16} r={2} fill="#888" />
        {LEFT_PINS.map((label, i) => (
          <text key={`l${i}`} x={14} y={18 + i * PIN_SPACING}
            fill="#888" fontFamily="monospace" fontSize={5.5}>{label}</text>
        ))}
        {RIGHT_PINS.map((label, i) => (
          <text key={`r${i}`} x={W - 14} y={18 + i * PIN_SPACING}
            textAnchor="end" fill="#888" fontFamily="monospace" fontSize={5.5}>{label}</text>
        ))}
        <text x={W / 2} y={H / 2 + 2} textAnchor="middle" fill="#aaa"
          fontFamily="monospace" fontSize={7} transform={`rotate(-90, ${W / 2}, ${H / 2})`}>
          L293D
        </text>
      </g>
    );
  },
};
