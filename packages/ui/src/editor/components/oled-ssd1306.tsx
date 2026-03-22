import type { ComponentDef } from '../types';

const W = 140;
const H = 84;

export const oledSsd1306Component: ComponentDef = {
  type: 'oled-ssd1306',
  label: 'OLED 128x64',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'GND', x: 22, y: H, side: 'bottom', label: 'GND' },
    { id: 'VCC', x: 50, y: H, side: 'bottom', label: 'VCC' },
    { id: 'SCL', x: 78, y: H, side: 'bottom', label: 'SCL' },
    { id: 'SDA', x: 106, y: H, side: 'bottom', label: 'SDA' },
  ],
  defaultAttrs: {},
  boardIoKind: 'i2c_device',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    const text = state?.displayText;
    return (
      <g>
        <rect x={0} y={0} width={W} height={H} rx={4}
          fill="#1a2a4a" stroke={selected ? '#e83e8c' : '#0d1a30'} strokeWidth={selected ? 2.5 : 1.5} />
        <rect x={8} y={6} width={W - 16} height={H - 24} rx={2}
          fill="#000" stroke="#222" strokeWidth={0.5} />
        {text ? (
          <text x={W / 2} y={28} textAnchor="middle" fill="#00aaff"
            fontFamily="monospace" fontSize={10}>{text.slice(0, 20)}</text>
        ) : (
          <>
            <text x={W / 2} y={28} textAnchor="middle" fill="#00aaff"
              fontFamily="monospace" fontSize={10}>128x64</text>
            <text x={W / 2} y={44} textAnchor="middle" fill="#00aaff"
              fontFamily="monospace" fontSize={8}>OLED</text>
          </>
        )}
        <text x={22} y={H - 4} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={6}>GND</text>
        <text x={50} y={H - 4} textAnchor="middle" fill="#ff3333" fontFamily="monospace" fontSize={6}>VCC</text>
        <text x={78} y={H - 4} textAnchor="middle" fill="#569cd6" fontFamily="monospace" fontSize={6}>SCL</text>
        <text x={106} y={H - 4} textAnchor="middle" fill="#569cd6" fontFamily="monospace" fontSize={6}>SDA</text>
      </g>
    );
  },
};
