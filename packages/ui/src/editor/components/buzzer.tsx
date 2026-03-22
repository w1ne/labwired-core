import type { ComponentDef } from '../types';

const W = 60;
const H = 60;

export const buzzerComponent: ComponentDef = {
  type: 'buzzer',
  label: 'Buzzer',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: '+', x: W / 2 - 10, y: H, side: 'bottom', label: '+' },
    { id: '-', x: W / 2 + 10, y: H, side: 'bottom', label: '-' },
  ],
  defaultAttrs: {},
  boardIoKind: 'pwm_output',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    const active = state?.active;
    return (
      <g>
        <circle cx={W / 2} cy={W / 2} r={26}
          fill="#222" stroke={selected ? '#e83e8c' : '#444'} strokeWidth={selected ? 2.5 : 1.5} />
        <circle cx={W / 2} cy={W / 2} r={8}
          fill={active ? '#ffcc00' : '#555'} />
        <text x={W / 2 - 10} y={H - 2} textAnchor="middle" fill="#ff3333" fontFamily="monospace" fontSize={8}>+</text>
        <text x={W / 2 + 10} y={H - 2} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={8}>-</text>
        {active && (
          <>
            <path d={`M${W / 2 + 16},${W / 2 - 6} q6,-6 0,-12`} fill="none" stroke="#ffcc00" strokeWidth={1.5} opacity={0.6} />
            <path d={`M${W / 2 + 22},${W / 2 - 3} q8,-8 0,-16`} fill="none" stroke="#ffcc00" strokeWidth={1.5} opacity={0.4} />
          </>
        )}
        {state?.frequency !== undefined && (
          <text x={W / 2} y={H + 12} textAnchor="middle" fill="#ffcc00"
            fontFamily="monospace" fontSize={8}>{state.frequency}Hz</text>
        )}
      </g>
    );
  },
};
