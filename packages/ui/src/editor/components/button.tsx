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
    const selected = !!state?.selected;
    const pressed = !!state?.active;
    const cx = W / 2;
    const cy = H / 2;

    return (
      <g>
        <defs>
          <radialGradient id="btn-housing" cx="0.5" cy="0.3" r="0.7">
            <stop offset="0" stopColor="#3a3a3a" />
            <stop offset="0.6" stopColor="#1c1c1c" />
            <stop offset="1" stopColor="#0a0a0a" />
          </radialGradient>
          <radialGradient id="btn-actuator" cx="0.4" cy="0.3" r="0.65">
            <stop offset="0" stopColor="#222" />
            <stop offset="0.7" stopColor="#0c0c0c" />
            <stop offset="1" stopColor="#000" />
          </radialGradient>
          <radialGradient id="btn-actuator-pressed" cx="0.4" cy="0.3" r="0.65">
            <stop offset="0" stopColor="#0e0e0e" />
            <stop offset="1" stopColor="#000" />
          </radialGradient>
          <radialGradient id="btn-glow" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor="#3DD68C" stopOpacity={0.55} />
            <stop offset="1" stopColor="#3DD68C" stopOpacity={0} />
          </radialGradient>
        </defs>

        {/* Drop shadow under the body */}
        <ellipse cx={cx} cy={H - 2} rx={W / 2 - 4} ry={3} fill="#000" opacity={0.4} />

        {/* Glow halo when pressed */}
        {pressed && <circle cx={cx} cy={cy} r={W / 2 - 2} fill="url(#btn-glow)" />}

        {/* Tactile switch housing — square dark body */}
        <rect
          x={6}
          y={6}
          width={W - 12}
          height={H - 12}
          rx={3}
          fill="url(#btn-housing)"
          stroke={selected ? '#F062B8' : '#000'}
          strokeWidth={selected ? 2.5 : 1}
        />

        {/* Embossed inset where the actuator sits */}
        <rect x={9} y={9} width={W - 18} height={H - 18} rx={2} fill="#0a0a0a" opacity={0.7} />

        {/* The four solder legs visible at corners (decorative) */}
        <rect x={2} y={cy - 3} width={6} height={6} rx={1} fill="#cfcfcf" stroke="#5a5a5a" strokeWidth={0.4} />
        <rect x={W - 8} y={cy - 3} width={6} height={6} rx={1} fill="#cfcfcf" stroke="#5a5a5a" strokeWidth={0.4} />

        {/* Round actuator button */}
        <circle
          cx={cx}
          cy={cy}
          r={pressed ? 10 : 12}
          fill={pressed ? 'url(#btn-actuator-pressed)' : 'url(#btn-actuator)'}
          stroke="#000"
          strokeWidth={1}
        />

        {/* Top specular highlight on actuator */}
        {!pressed && (
          <ellipse cx={cx - 3} cy={cy - 5} rx={5} ry={2.5} fill="rgba(255,255,255,0.18)" />
        )}

        {/* Ring around actuator for tactile feel */}
        <circle cx={cx} cy={cy} r={pressed ? 10 : 12} fill="none" stroke="#444" strokeWidth={0.6} opacity={0.5} />

        {/* Press indicator dot in center when pressed */}
        {pressed && <circle cx={cx} cy={cy} r={2} fill="#3DD68C" />}

        {/* Pin labels */}
        <text x={10} y={cy + 3} fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={7}>1</text>
        <text x={W - 10} y={cy + 3} textAnchor="end" fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={7}>2</text>

        {/* State label below */}
        <text
          x={cx}
          y={H + 12}
          textAnchor="middle"
          fill={pressed ? '#3DD68C' : '#5A6178'}
          fontFamily="'JetBrains Mono', monospace"
          fontSize={8}
          fontWeight={pressed ? 600 : 400}
        >
          {pressed ? 'PRESSED' : 'PRESS'}
        </text>
      </g>
    );
  },
};
