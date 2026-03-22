import type { ComponentDef } from '../types';

const W = 100;
const H = 68;

export const servoComponent: ComponentDef = {
  type: 'servo',
  label: 'Servo Motor',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: 'SIG', x: 0, y: 16, side: 'left', label: 'SIG' },
    { id: 'VCC', x: 0, y: 34, side: 'left', label: 'VCC' },
    { id: 'GND', x: 0, y: 52, side: 'left', label: 'GND' },
  ],
  defaultAttrs: { angle: '90' },
  boardIoKind: 'pwm_output',
  attrFields: [
    { key: 'angle', label: 'Angle (0-180)', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const angle = state?.angle ?? parseInt(attrs.angle || '90', 10);
    const armAngle = ((angle - 90) * Math.PI) / 180;
    const cx = W - 22, cy = H / 2;
    const armLen = 20;
    const ax = cx + Math.cos(armAngle) * armLen;
    const ay = cy - Math.sin(armAngle) * armLen;
    return (
      <g>
        <rect x={12} y={6} width={W - 34} height={H - 12} rx={6}
          fill="#2a4a8a" stroke={selected ? '#e83e8c' : '#1a2a4a'} strokeWidth={selected ? 2.5 : 1.5} />
        <rect x={6} y={20} width={10} height={8} rx={2} fill="#2a4a8a" stroke="#1a2a4a" strokeWidth={0.5} />
        <rect x={6} y={H - 28} width={10} height={8} rx={2} fill="#2a4a8a" stroke="#1a2a4a" strokeWidth={0.5} />
        <circle cx={cx} cy={cy} r={12} fill="#ddd" stroke="#888" strokeWidth={1.5} />
        <line x1={cx} y1={cy} x2={ax} y2={ay} stroke="#333" strokeWidth={4} strokeLinecap="round" />
        <circle cx={ax} cy={ay} r={3} fill="#333" />
        <text x={16} y={22} fill="#ffcc00" fontFamily="monospace" fontSize={7}>SIG</text>
        <text x={16} y={40} fill="#ff3333" fontFamily="monospace" fontSize={7}>VCC</text>
        <text x={16} y={58} fill="#888" fontFamily="monospace" fontSize={7}>GND</text>
        <text x={cx} y={H + 12} textAnchor="middle" fill="#569cd6"
          fontFamily="monospace" fontSize={9}>{angle}°</text>
      </g>
    );
  },
};
