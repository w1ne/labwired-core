import type { ComponentDef } from '../types';

const W = 60;
const H = 40;

export const ntcThermistorComponent: ComponentDef = {
  type: 'ntc-thermistor',
  label: 'NTC Thermistor',
  category: 'sensor',
  width: W,
  height: H,
  pins: [
    { id: 'A', x: 0, y: H / 2, side: 'left', label: 'A' },
    { id: 'B', x: W, y: H / 2, side: 'right', label: 'B' },
  ],
  defaultAttrs: { beta: '3950', r0: '10K' },
  boardIoKind: 'adc_input',
  attrFields: [
    { key: 'beta', label: 'Beta coefficient', type: 'text' },
    { key: 'r0', label: 'R0 at 25°C', type: 'text' },
  ],
  render: (_attrs, state) => {
    const selected = state?.selected;
    return (
      <g>
        {/* Left lead */}
        <line x1={0} y1={H / 2} x2={10} y2={H / 2} stroke="#555" strokeWidth={2} />
        {/* Right lead */}
        <line x1={W - 10} y1={H / 2} x2={W} y2={H / 2} stroke="#555" strokeWidth={2} />
        {/* Disc body */}
        <ellipse
          cx={W / 2}
          cy={H / 2}
          rx={18}
          ry={14}
          fill="#C0392B"
          stroke={selected ? '#e83e8c' : '#922B21'}
          strokeWidth={selected ? 2.5 : 1.5}
        />
        {/* NTC label */}
        <text
          x={W / 2}
          y={H / 2 + 1}
          textAnchor="middle"
          dominantBaseline="middle"
          fill="#fff"
          fontFamily="monospace"
          fontSize={8}
          fontWeight="bold"
        >
          NTC
        </text>
        {/* Zigzag symbol at bottom of disc */}
        <path
          d={`M${W / 2 - 6},${H / 2 + 7} l3,-4 l3,4 l3,-4 l3,4`}
          fill="none"
          stroke="#fff"
          strokeWidth={1}
          opacity={0.7}
        />
        {/* ADC count readout */}
        {state?.analogValue !== undefined && (
          <text
            x={W / 2}
            y={H + 12}
            textAnchor="middle"
            fill="#3399ff"
            fontFamily="monospace"
            fontSize={8}
          >
            {state.analogValue}
          </text>
        )}
      </g>
    );
  },
};
