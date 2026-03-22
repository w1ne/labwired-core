import type { ComponentDef } from '../types';

const W = 220;
const H = 110;

export const lcd1602Component: ComponentDef = {
  type: 'lcd1602',
  label: 'LCD 16x2',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'VSS', x: 0, y: 16, side: 'left', label: 'VSS' },
    { id: 'VDD', x: 0, y: 32, side: 'left', label: 'VDD' },
    { id: 'V0', x: 0, y: 48, side: 'left', label: 'V0' },
    { id: 'RS', x: 0, y: 64, side: 'left', label: 'RS' },
    { id: 'RW', x: 0, y: 80, side: 'left', label: 'RW' },
    { id: 'E', x: 0, y: 96, side: 'left', label: 'E' },
    { id: 'D4', x: W, y: 16, side: 'right', label: 'D4' },
    { id: 'D5', x: W, y: 32, side: 'right', label: 'D5' },
    { id: 'D6', x: W, y: 48, side: 'right', label: 'D6' },
    { id: 'D7', x: W, y: 64, side: 'right', label: 'D7' },
    { id: 'BLA', x: W, y: 80, side: 'right', label: 'BLA' },
    { id: 'BLK', x: W, y: 96, side: 'right', label: 'BLK' },
  ],
  defaultAttrs: { text: 'Hello World!' },
  boardIoKind: 'i2c_device',
  attrFields: [
    { key: 'text', label: 'Display Text', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const text = state?.displayText || attrs.text || 'Hello World!';
    const line1 = text.slice(0, 16).padEnd(16);
    const line2 = text.slice(16, 32).padEnd(16);
    return (
      <g>
        <rect x={0} y={0} width={W} height={H} rx={5}
          fill="#1c6b3c" stroke={selected ? '#e83e8c' : '#0d4d1e'} strokeWidth={selected ? 2.5 : 1.5} />
        <rect x={28} y={14} width={W - 56} height={H - 28} rx={3}
          fill="#2a5a1a" stroke="#1a3a0a" strokeWidth={1} />
        <rect x={34} y={20} width={W - 68} height={H - 40} rx={2}
          fill="#5cb85c" />
        <text x={40} y={44} fill="#1a3a0a"
          fontFamily="'JetBrains Mono', monospace" fontSize={13} letterSpacing={2}>
          {line1}
        </text>
        <text x={40} y={68} fill="#1a3a0a"
          fontFamily="'JetBrains Mono', monospace" fontSize={13} letterSpacing={2}>
          {line2}
        </text>
      </g>
    );
  },
};
