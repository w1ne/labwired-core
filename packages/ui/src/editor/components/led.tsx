import type { ComponentDef } from '../types';

const W = 60;
const H = 80;

export const ledComponent: ComponentDef = {
  type: 'led',
  label: 'LED',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: 'A', x: W / 2, y: 0, side: 'top', label: 'A' },
    { id: 'C', x: W / 2, y: H, side: 'bottom', label: 'C' },
  ],
  defaultAttrs: { color: 'red' },
  boardIoKind: 'led',
  attrFields: [
    {
      key: 'color',
      label: 'Color',
      type: 'select',
      options: ['red', 'green', 'blue', 'yellow', 'white'],
    },
  ],
  render: (attrs, state) => {
    const color = attrs.color || 'red';
    const colorMap: Record<string, string> = {
      red: '#ff3333', green: '#27c93f', blue: '#3399ff',
      yellow: '#ffcc00', white: '#ffffff',
    };
    const fill = colorMap[color] || color;
    const darkFill = state?.active ? fill : darken(fill, 0.6);
    const selected = state?.selected;
    const active = !!state?.active;

    return (
      <g>
        {/* Body */}
        <rect x={6} y={16} width={W - 12} height={H - 32} rx={6}
          fill="#f8f9fa" stroke={selected ? '#e83e8c' : '#000'} strokeWidth={selected ? 2.5 : 1.5} />
        {/* Outer glow — large faint halo for realistic bloom */}
        {active && (
          <circle cx={W / 2} cy={H / 2} r={30} fill={fill} opacity={0.15} />
        )}
        {/* LED glow */}
        {active && (
          <circle cx={W / 2} cy={H / 2} r={22} fill={fill} opacity={0.3} />
        )}
        {/* LED circle */}
        <circle cx={W / 2} cy={H / 2} r={14}
          fill={darkFill} stroke="#000" strokeWidth={1} />
        {/* Specular highlight — brighter center for pulsing hint */}
        {active && (
          <>
            <circle cx={W / 2 - 4} cy={H / 2 - 4} r={4} fill="rgba(255,255,255,0.5)" />
            <circle cx={W / 2} cy={H / 2} r={5} fill="rgba(255,255,255,0.25)" />
          </>
        )}
        {/* Labels */}
        <text x={W / 2} y={12} textAnchor="middle" fill="#888"
          fontFamily="monospace" fontSize={8}>A</text>
        <text x={W / 2} y={H - 4} textAnchor="middle" fill="#888"
          fontFamily="monospace" fontSize={8}>C</text>
        {/* ON / OFF status */}
        <text x={W / 2} y={H + 12} textAnchor="middle"
          fill={active ? '#27c93f' : '#888'} fontFamily="monospace" fontSize={9}
          fontWeight={active ? 'bold' : 'normal'}>{active ? 'ON' : 'OFF'}</text>
        {/* Analog percentage as secondary info */}
        {state?.analogValue !== undefined && (
          <text x={W / 2} y={H + 22} textAnchor="middle" fill="#888"
            fontFamily="monospace" fontSize={7}>{Math.round(state.analogValue / 40.95)}%</text>
        )}
      </g>
    );
  },
};

function darken(hex: string, amount: number): string {
  const num = parseInt(hex.replace('#', ''), 16);
  const r = Math.max(0, ((num >> 16) & 0xff) * (1 - amount));
  const g = Math.max(0, ((num >> 8) & 0xff) * (1 - amount));
  const b = Math.max(0, (num & 0xff) * (1 - amount));
  return `rgb(${Math.round(r)},${Math.round(g)},${Math.round(b)})`;
}
