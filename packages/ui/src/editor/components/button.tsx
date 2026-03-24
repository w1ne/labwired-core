import type { ComponentDef } from '../types';

const W = 64;
const H = 64;

export const buttonComponent: ComponentDef = {
  type: 'button',
  label: 'Push Button',
  category: 'input',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 0, y: H / 2, side: 'left', label: '1' },
    { id: '2', x: W, y: H / 2, side: 'right', label: '2' },
  ],
  defaultAttrs: {},
  boardIoKind: 'button',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = state?.selected;
    const pressed = state?.active;

    // Button cap geometry: slightly smaller and shifted down when pressed
    const capX = pressed ? 16 : 14;
    const capY = pressed ? 16 : 14;
    const capW = pressed ? W - 32 : W - 28;
    const capH = pressed ? H - 32 : H - 28;

    return (
      <g>
        <defs>
          {/* 3D shadow filter for the housing */}
          <filter id="btn-shadow" x="-10%" y="-10%" width="130%" height="140%">
            <feDropShadow dx={0} dy={2} stdDeviation={2.5} floodColor="#000" floodOpacity={0.35} />
          </filter>
          {/* Green glow filter for active state */}
          <filter id="btn-glow" x="-30%" y="-30%" width="160%" height="160%">
            <feGaussianBlur in="SourceGraphic" stdDeviation={3} />
          </filter>
          {/* Slight inner-shadow on the cap to fake depth */}
          <filter id="cap-shadow" x="-10%" y="-10%" width="130%" height="140%">
            <feDropShadow dx={0} dy={pressed ? 0.5 : 2} stdDeviation={pressed ? 0.5 : 1.5}
              floodColor="#000" floodOpacity={pressed ? 0.5 : 0.4} />
          </filter>
        </defs>

        {/* Green glow ring when active */}
        {pressed && (
          <rect x={1} y={1} width={W - 2} height={H - 2} rx={10}
            fill="none" stroke="#27c93f" strokeWidth={3} filter="url(#btn-glow)" />
        )}

        {/* Housing / body with 3D shadow */}
        <rect x={3} y={3} width={W - 6} height={H - 6} rx={8}
          fill="#f0f1f3" stroke={selected ? '#e83e8c' : (pressed ? '#27c93f' : '#555')}
          strokeWidth={selected ? 2.5 : (pressed ? 2 : 1.5)}
          filter="url(#btn-shadow)" />

        {/* Slight top-edge highlight for 3D bevel */}
        <rect x={5} y={4} width={W - 10} height={(H - 6) / 2} rx={6}
          fill="url(#none)" opacity={0}/>
        <line x1={7} y1={5} x2={W - 7} y2={5} stroke="#fff" strokeWidth={1} opacity={0.5} strokeLinecap="round" />

        {/* Button cap with depth shadow */}
        <rect x={capX} y={capY} width={capW} height={capH} rx={pressed ? 5 : 6}
          fill={pressed ? '#222' : '#6b6b6b'} stroke={pressed ? '#111' : '#444'} strokeWidth={1}
          filter="url(#cap-shadow)" />

        {/* Cap top highlight (specular) when not pressed */}
        {!pressed && (
          <line x1={18} y1={17} x2={W - 18} y2={17} stroke="#aaa" strokeWidth={1} opacity={0.7} strokeLinecap="round" />
        )}

        {/* Pin labels */}
        <text x={W / 2} y={-6} textAnchor="middle" fill="#d9e3f0"
          fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700}>BUTTON</text>
        <text x={8} y={H / 2 + 4} fill="#888" fontFamily="monospace" fontSize={8}>1</text>
        <text x={W - 8} y={H / 2 + 4} textAnchor="end" fill="#888" fontFamily="monospace" fontSize={8}>2</text>

        {/* Status label: "PRESS" hint when idle, "ON" when active */}
        <text x={W / 2} y={H / 2 + 3} textAnchor="middle"
          fill={pressed ? '#27c93f' : '#ccc'}
          fontFamily="monospace" fontSize={pressed ? 9 : 7} fontWeight="bold">
          {pressed ? 'ON' : 'PRESS'}
        </text>
      </g>
    );
  },
};
