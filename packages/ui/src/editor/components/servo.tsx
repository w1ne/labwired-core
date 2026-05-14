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
    const selected = !!state?.selected;
    const angle = ((state?.angle as number | undefined) ?? parseInt((attrs.angle as string) || '90', 10));
    const armAngleRad = ((angle - 90) * Math.PI) / 180;
    const hubX = W - 24;
    const hubY = H / 2;
    const armLen = 22;
    const ax = hubX + Math.cos(armAngleRad) * armLen;
    const ay = hubY - Math.sin(armAngleRad) * armLen;

    return (
      <g>
        <defs>
          <linearGradient id="servo-body" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#4F90D4" />
            <stop offset="0.5" stopColor="#2D6BB0" />
            <stop offset="1" stopColor="#1A4A82" />
          </linearGradient>
          <linearGradient id="servo-tab" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a6aa0" />
            <stop offset="1" stopColor="#1f3f6a" />
          </linearGradient>
          <radialGradient id="servo-horn" cx="0.4" cy="0.3" r="0.7">
            <stop offset="0" stopColor="#FFFFFF" />
            <stop offset="0.6" stopColor="#DDDDDD" />
            <stop offset="1" stopColor="#888888" />
          </radialGradient>
          <radialGradient id="servo-hub" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor="#FFFFFF" />
            <stop offset="0.5" stopColor="#CCCCCC" />
            <stop offset="1" stopColor="#666666" />
          </radialGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 12} ry={3} fill="#000" opacity={0.4} />

        {/* Mounting tabs (left side) */}
        <rect x={6} y={18} width={10} height={10} rx={1.5} fill="url(#servo-tab)" stroke="#0e2240" strokeWidth={0.6} />
        <circle cx={11} cy={23} r={1.5} fill="#1a1a1a" />
        <rect x={6} y={H - 28} width={10} height={10} rx={1.5} fill="url(#servo-tab)" stroke="#0e2240" strokeWidth={0.6} />
        <circle cx={11} cy={H - 23} r={1.5} fill="#1a1a1a" />

        {/* Main body */}
        <rect x={14} y={6} width={W - 38} height={H - 12} rx={5} fill="url(#servo-body)" stroke={selected ? '#F062B8' : '#0e2240'} strokeWidth={selected ? 2.5 : 1} />

        {/* Top highlight strip */}
        <rect x={16} y={8} width={W - 42} height={4} rx={2} fill="rgba(255,255,255,0.18)" />

        {/* Brand label silkscreen */}
        <text x={(14 + W - 24) / 2} y={H / 2 - 2} textAnchor="middle" fill="rgba(255,255,255,0.6)" fontFamily="'Outfit', sans-serif" fontSize={8} fontWeight={700} letterSpacing="0.08em">
          SERVO
        </text>
        <text x={(14 + W - 24) / 2} y={H / 2 + 8} textAnchor="middle" fill="rgba(255,255,255,0.5)" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          SG90 · 50Hz
        </text>

        {/* Output shaft hub (white plastic) */}
        <circle cx={hubX} cy={hubY} r={14} fill="url(#servo-hub)" stroke="#444" strokeWidth={1} />
        <circle cx={hubX} cy={hubY} r={14} fill="none" stroke="#fff" strokeWidth={0.5} opacity={0.4} />

        {/* Horn arm (rotates with angle) */}
        <g>
          <line x1={hubX} y1={hubY} x2={ax} y2={ay} stroke="#F2F4F9" strokeWidth={6} strokeLinecap="round" />
          <line x1={hubX} y1={hubY} x2={ax} y2={ay} stroke="#888" strokeWidth={1} strokeLinecap="round" />
          <circle cx={ax} cy={ay} r={2.5} fill="#3a3a3a" stroke="#000" strokeWidth={0.5} />
        </g>

        {/* Center screw on hub */}
        <circle cx={hubX} cy={hubY} r={2.5} fill="#3a3a3a" />
        <line x1={hubX - 2} y1={hubY} x2={hubX + 2} y2={hubY} stroke="#fff" strokeWidth={0.6} opacity={0.7} />

        {/* Wire stubs for SIG/VCC/GND */}
        <line x1={2} y1={16} x2={14} y2={16} stroke="#F5B642" strokeWidth={2.2} strokeLinecap="round" />
        <line x1={2} y1={34} x2={14} y2={34} stroke="#F2545B" strokeWidth={2.2} strokeLinecap="round" />
        <line x1={2} y1={52} x2={14} y2={52} stroke="#5a5a5a" strokeWidth={2.2} strokeLinecap="round" />

        {/* Pin labels */}
        <text x={16} y={13} fill="#F5B642" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>SIG</text>
        <text x={16} y={31} fill="#F2545B" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>VCC</text>
        <text x={16} y={49} fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>GND</text>

        {/* Angle label below */}
        <text x={hubX} y={H + 12} textAnchor="middle" fill="#5BD8FF" fontFamily="'JetBrains Mono', monospace" fontSize={9} fontWeight={600}>
          {angle}°
        </text>

        {/* Selection */}
        {selected && (
          <rect x={14} y={6} width={W - 38} height={H - 12} rx={5} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
