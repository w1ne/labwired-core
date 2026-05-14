import type { ComponentDef } from '../types';

const W = 80;
const H = 140;
const PIN_SPACING = 16;

const LEFT_PINS = ['EN1,2', 'IN1', 'OUT1', 'GND', 'GND', 'OUT2', 'IN2', 'VS'];
const RIGHT_PINS = ['VSS', 'IN4', 'OUT4', 'GND', 'GND', 'OUT3', 'IN3', 'EN3,4'];

export const motorDriverL293dComponent: ComponentDef = {
  type: 'l293d',
  label: 'L293D',
  category: 'ic',
  width: W,
  height: H,
  pins: [
    ...LEFT_PINS.map((label, i) => ({
      id: `L${i + 1}`,
      x: 0,
      y: 14 + i * PIN_SPACING,
      side: 'left' as const,
      label,
    })),
    ...RIGHT_PINS.map((label, i) => ({
      id: `R${i + 1}`,
      x: W,
      y: 14 + i * PIN_SPACING,
      side: 'right' as const,
      label,
    })),
  ],
  defaultAttrs: {},
  boardIoKind: 'pwm_output',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="l293-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a3a3a" />
            <stop offset="0.5" stopColor="#1c1c1c" />
            <stop offset="1" stopColor="#0a0a0a" />
          </linearGradient>
          <linearGradient id="l293-pin" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f0f0f0" />
            <stop offset="1" stopColor="#888" />
          </linearGradient>
        </defs>

        {/* Pin lead frames on both sides (silver) */}
        {LEFT_PINS.map((_, i) => (
          <rect
            key={`lp-${i}`}
            x={-2}
            y={11 + i * PIN_SPACING}
            width={12}
            height={6}
            fill="url(#l293-pin)"
            stroke="#5a5a5a"
            strokeWidth={0.3}
          />
        ))}
        {RIGHT_PINS.map((_, i) => (
          <rect
            key={`rp-${i}`}
            x={W - 10}
            y={11 + i * PIN_SPACING}
            width={12}
            height={6}
            fill="url(#l293-pin)"
            stroke="#5a5a5a"
            strokeWidth={0.3}
          />
        ))}

        {/* DIP-16 chip body */}
        <rect x={6} y={3} width={W - 12} height={H - 6} rx={2} fill="url(#l293-chip)" stroke={selected ? '#F062B8' : '#000'} strokeWidth={selected ? 2.5 : 1} />

        {/* Notch at top */}
        <circle cx={W / 2} cy={7} r={4} fill="#0a0a0a" />
        <path d={`M ${W / 2 - 4} 7 A 4 4 0 0 1 ${W / 2 + 4} 7`} fill="none" stroke="#444" strokeWidth={0.6} />

        {/* Pin 1 indicator dot */}
        <circle cx={14} cy={16} r={1.5} fill="#666" />

        {/* L293D silkscreen — rotated 90deg for vertical orientation */}
        <text
          x={W / 2}
          y={H / 2}
          textAnchor="middle"
          fill="#bbb"
          fontFamily="'Outfit', sans-serif"
          fontSize={11}
          fontWeight={700}
          letterSpacing="0.1em"
          transform={`rotate(-90, ${W / 2}, ${H / 2})`}
        >
          L293D
        </text>
        <text
          x={W / 2}
          y={H / 2 + 12}
          textAnchor="middle"
          fill="#666"
          fontFamily="'JetBrains Mono', monospace"
          fontSize={5}
          transform={`rotate(-90, ${W / 2}, ${H / 2 + 12})`}
        >
          Push-Pull 4ch
        </text>

        {/* Pin labels */}
        {LEFT_PINS.map((label, i) => (
          <text
            key={`l-${i}`}
            x={14}
            y={17 + i * PIN_SPACING}
            fill="#fff"
            fontFamily="'JetBrains Mono', monospace"
            fontSize={5.5}
          >
            {label}
          </text>
        ))}
        {RIGHT_PINS.map((label, i) => (
          <text
            key={`r-${i}`}
            x={W - 14}
            y={17 + i * PIN_SPACING}
            textAnchor="end"
            fill="#fff"
            fontFamily="'JetBrains Mono', monospace"
            fontSize={5.5}
          >
            {label}
          </text>
        ))}
      </g>
    );
  },
};
