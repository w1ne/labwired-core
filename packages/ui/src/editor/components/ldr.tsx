import type { ComponentDef } from '../types';

const W = 40;
const H = 40;

export const ldrComponent: ComponentDef = {
  type: 'ldr',
  label: 'Photoresistor',
  category: 'sensor',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 0, y: H / 2, side: 'left', label: '1' },
    { id: '2', x: W, y: H / 2, side: 'right', label: '2' },
  ],
  defaultAttrs: { value: '10K' },
  boardIoKind: 'adc_input',
  attrFields: [
    { key: 'value', label: 'Resistance', type: 'text' },
  ],
  render: (_attrs, state) => {
    const selected = state?.selected;
    return (
      <g>
        <line x1={0} y1={H / 2} x2={8} y2={H / 2} stroke="#444" strokeWidth={2} />
        <line x1={W - 8} y1={H / 2} x2={W} y2={H / 2} stroke="#444" strokeWidth={2} />
        <circle cx={W / 2} cy={H / 2} r={14}
          fill="#8B4513" stroke={selected ? '#e83e8c' : '#5C3317'} strokeWidth={selected ? 2.5 : 1.5} />
        <path d={`M${W / 2 - 6},${H / 2 - 4} l4,8 l4,-8 l4,8`}
          fill="none" stroke="#daa520" strokeWidth={1.2} />
        <line x1={W / 2 - 10} y1={6} x2={W / 2 - 4} y2={10} stroke="#ffcc00" strokeWidth={1} />
        <line x1={W / 2 + 2} y1={3} x2={W / 2 + 5} y2={9} stroke="#ffcc00" strokeWidth={1} />
        {state?.analogValue !== undefined && (
          <text x={W / 2} y={H + 12} textAnchor="middle" fill="#3399ff"
            fontFamily="monospace" fontSize={8}>{state.analogValue}</text>
        )}
      </g>
    );
  },
};
