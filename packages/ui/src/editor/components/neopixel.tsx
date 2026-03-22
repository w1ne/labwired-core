import type { ComponentDef } from '../types';

const W = 120;
const H = 36;

export const neopixelComponent: ComponentDef = {
  type: 'neopixel',
  label: 'NeoPixel Strip',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: 'DIN', x: 0, y: H / 2, side: 'left', label: 'DIN' },
    { id: 'VCC', x: W / 2 - 14, y: 0, side: 'top', label: 'VCC' },
    { id: 'GND', x: W / 2 + 14, y: 0, side: 'top', label: 'GND' },
    { id: 'DOUT', x: W, y: H / 2, side: 'right', label: 'DOUT' },
  ],
  defaultAttrs: { count: '8' },
  boardIoKind: 'spi_device',
  attrFields: [
    { key: 'count', label: 'LED Count', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const count = Math.min(parseInt(attrs.count || '8', 10), 8);
    const colors = ['#ff3333', '#27c93f', '#3399ff', '#ffcc00', '#e83e8c', '#00cccc', '#ff6633', '#9966ff'];
    const spacing = (W - 16) / count;
    return (
      <g>
        <rect x={0} y={3} width={W} height={H - 6} rx={4}
          fill="#1a1a1a" stroke={selected ? '#e83e8c' : '#333'} strokeWidth={selected ? 2.5 : 1} />
        {Array.from({ length: count }, (_, i) => (
          <rect key={i}
            x={8 + i * spacing} y={8} width={spacing - 3} height={H - 16} rx={2}
            fill={state?.active ? colors[i % colors.length] : '#333'}
            opacity={state?.active ? 0.9 : 0.4}
          />
        ))}
      </g>
    );
  },
};
