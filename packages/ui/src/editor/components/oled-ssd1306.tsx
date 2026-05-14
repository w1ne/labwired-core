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
    const selected = !!state?.selected;
    const active = !!state?.active;
    const text = state?.displayText as string | undefined;
    return (
      <g>
        <defs>
          <linearGradient id="oled-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1A1A1A" />
            <stop offset="1" stopColor="#0A0A0A" />
          </linearGradient>
          <pattern id="oled-pcb-dots" x="0" y="0" width="4" height="4" patternUnits="userSpaceOnUse">
            <circle cx={2} cy={2} r={0.3} fill="#222" opacity={0.6} />
          </pattern>
          <linearGradient id="oled-screen" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#000" />
            <stop offset="1" stopColor="#050505" />
          </linearGradient>
          <linearGradient id="oled-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 8} ry={4} fill="#000" opacity={0.4} />

        {/* PCB body */}
        <rect width={W} height={H} rx={5} fill="url(#oled-pcb)" stroke={selected ? '#F062B8' : '#000'} strokeWidth={selected ? 2.5 : 1} />
        <rect width={W} height={H} rx={5} fill="url(#oled-pcb-dots)" opacity={0.6} />

        {/* Mounting holes at corners */}
        <circle cx={5} cy={5} r={2} fill="#0a0a0a" stroke="#444" strokeWidth={0.5} />
        <circle cx={W - 5} cy={5} r={2} fill="#0a0a0a" stroke="#444" strokeWidth={0.5} />
        <circle cx={5} cy={H - 22} r={2} fill="#0a0a0a" stroke="#444" strokeWidth={0.5} />
        <circle cx={W - 5} cy={H - 22} r={2} fill="#0a0a0a" stroke="#444" strokeWidth={0.5} />

        {/* OLED glass display area */}
        <rect x={12} y={10} width={W - 24} height={H - 38} rx={1} fill="url(#oled-screen)" stroke="#2a2a2a" strokeWidth={1} />
        {/* Subtle inner border for glass edge */}
        <rect x={13} y={11} width={W - 26} height={H - 40} rx={1} fill="none" stroke="#1a1a1a" strokeWidth={0.5} />

        {/* Display content */}
        {text ? (
          <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#5BD8FF" fontFamily="'JetBrains Mono', monospace" fontSize={9} fontWeight={500}>
            {text.slice(0, 22)}
          </text>
        ) : active ? (
          <>
            <text x={W / 2} y={H / 2 - 10} textAnchor="middle" fill="#5BD8FF" fontFamily="'JetBrains Mono', monospace" fontSize={10} fontWeight={600}>
              LabWired
            </text>
            <text x={W / 2} y={H / 2} textAnchor="middle" fill="#5BD8FF" fontFamily="'JetBrains Mono', monospace" fontSize={7} opacity={0.7}>
              128 x 64 · I²C
            </text>
            {/* Fake pixel-row decoration */}
            <rect x={20} y={H / 2 + 8} width={W - 40} height={2} fill="#5BD8FF" opacity={0.4} />
            <rect x={20} y={H / 2 + 14} width={W - 60} height={2} fill="#5BD8FF" opacity={0.25} />
          </>
        ) : (
          <text x={W / 2} y={H / 2 - 4} textAnchor="middle" fill="#1a3a4a" fontFamily="'JetBrains Mono', monospace" fontSize={9}>
            128 x 64 OLED
          </text>
        )}

        {/* Silkscreen — bottom-row labels */}
        <text x={W / 2} y={H - 18} textAnchor="middle" fill="rgba(255,255,255,0.5)" fontFamily="'Outfit', sans-serif" fontSize={6.5} fontWeight={600} letterSpacing="0.06em">
          SSD1306 · I²C 0x3C
        </text>

        {/* Bottom pads */}
        {[
          { x: 22, label: 'GND', color: '#aaa' },
          { x: 50, label: 'VCC', color: '#FF6B6B' },
          { x: 78, label: 'SCL', color: '#5B9DFF' },
          { x: 106, label: 'SDA', color: '#3DD68C' },
        ].map((pad) => (
          <g key={pad.label}>
            <rect x={pad.x - 4} y={H - 12} width={8} height={10} fill="url(#oled-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={pad.x} cy={H - 7} r={1.5} fill="#0a0a0a" />
            <text x={pad.x} y={H - 14} textAnchor="middle" fill={pad.color} fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>
              {pad.label}
            </text>
          </g>
        ))}

        {/* Selection */}
        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
