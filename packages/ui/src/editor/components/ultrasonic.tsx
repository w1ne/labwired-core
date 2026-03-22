import type { ComponentDef } from '../types';

const W = 88;
const H = 52;

export const ultrasonicComponent: ComponentDef = {
  type: 'ultrasonic',
  label: 'HC-SR04',
  category: 'sensor',
  width: W,
  height: H,
  pins: [
    { id: 'VCC', x: 14, y: H, side: 'bottom', label: 'VCC' },
    { id: 'TRIG', x: 32, y: H, side: 'bottom', label: 'TRIG' },
    { id: 'ECHO', x: 56, y: H, side: 'bottom', label: 'ECHO' },
    { id: 'GND', x: 74, y: H, side: 'bottom', label: 'GND' },
  ],
  defaultAttrs: { distance: '100' },
  boardIoKind: 'button',
  attrFields: [
    { key: 'distance', label: 'Distance (cm)', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const dist = attrs.distance || '100';
    return (
      <g>
        <rect x={3} y={3} width={W - 6} height={H - 8} rx={4}
          fill="#1a6aaa" stroke={selected ? '#e83e8c' : '#0d4060'} strokeWidth={selected ? 2.5 : 1.5} />
        <circle cx={24} cy={22} r={14} fill="#ccc" stroke="#999" strokeWidth={1} />
        <circle cx={24} cy={22} r={8} fill="#ddd" />
        <circle cx={W - 24} cy={22} r={14} fill="#ccc" stroke="#999" strokeWidth={1} />
        <circle cx={W - 24} cy={22} r={8} fill="#ddd" />
        <rect x={W / 2 - 5} y={8} width={10} height={5} rx={1} fill="#888" />
        <text x={W / 2} y={38} textAnchor="middle" fill="#fff"
          fontFamily="monospace" fontSize={8}>{dist}cm</text>
      </g>
    );
  },
};
