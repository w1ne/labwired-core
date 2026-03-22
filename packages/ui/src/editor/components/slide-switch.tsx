import type { ComponentDef } from '../types';

const W = 52;
const H = 28;

export const slideSwitchComponent: ComponentDef = {
  type: 'slide-switch',
  label: 'Slide Switch',
  category: 'input',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 8, y: H, side: 'bottom', label: '1' },
    { id: 'COM', x: W / 2, y: H, side: 'bottom', label: 'COM' },
    { id: '2', x: W - 8, y: H, side: 'bottom', label: '2' },
  ],
  defaultAttrs: { position: 'left' },
  boardIoKind: 'button',
  attrFields: [
    { key: 'position', label: 'Position', type: 'select', options: ['left', 'right'] },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const pos = attrs.position === 'right' ? 'right' : 'left';
    const knobX = pos === 'left' ? 14 : W - 14;
    return (
      <g>
        <rect x={2} y={2} width={W - 4} height={H - 4} rx={4}
          fill="#ddd" stroke={selected ? '#e83e8c' : '#888'} strokeWidth={selected ? 2.5 : 1.5} />
        <rect x={10} y={8} width={W - 20} height={8} rx={4} fill="#aaa" />
        <rect x={knobX - 6} y={6} width={12} height={12} rx={3}
          fill="#444" stroke="#222" strokeWidth={0.5} />
        <text x={8} y={H + 10} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={6}>1</text>
        <text x={W - 8} y={H + 10} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={6}>2</text>
      </g>
    );
  },
};
