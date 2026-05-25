import type { ComponentDef, PinDef } from '../../types';

const W = 180;
const H = 460;
const PIN_SPACING = 14;
const PIN_START_Y = 56;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  for (let i = 0; i < 16; i++) {
    pins.push({
      id: `PA${i}`,
      x: 0,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'left',
      label: `PA${i}`,
    });
    pins.push({
      id: `PB${i}`,
      x: W,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'right',
      label: `PB${i}`,
    });
  }

  pins.push({ id: 'VCC', x: W / 2 - 20, y: H, side: 'bottom', label: '3V3' });
  pins.push({ id: 'GND', x: W / 2 + 20, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const stm32BlackpillComponent: ComponentDef = {
  type: 'stm32-blackpill',
  label: 'STM32 Black Pill',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <ellipse cx={W / 2} cy={H + 6} rx={W / 2 - 8} ry={5} fill="#000" opacity={0.35} />

        <defs>
          <linearGradient id="bp-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#161616" />
            <stop offset="1" stopColor="#0c0c0c" />
          </linearGradient>
          <pattern id="bp-dots" x="0" y="0" width="6" height="6" patternUnits="userSpaceOnUse">
            <circle cx={3} cy={3} r={0.3} fill="#2a2a2a" opacity={0.6} />
          </pattern>
          <linearGradient id="bp-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0e0e0e" />
          </linearGradient>
          <linearGradient id="bp-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#C49A3A" />
          </linearGradient>
        </defs>

        <rect
          width={W}
          height={H}
          rx={6}
          fill="url(#bp-pcb)"
          stroke={selected ? '#F062B8' : '#000'}
          strokeWidth={selected ? 3 : 1}
        />
        <rect width={W} height={H} rx={6} fill="url(#bp-dots)" opacity={0.5} />

        {/* USB-C connector at top */}
        <rect x={W / 2 - 22} y={-8} width={44} height={18} rx={6} fill="#9a9da3" stroke="#3a3a3a" strokeWidth={1} />
        <rect x={W / 2 - 18} y={-4} width={36} height={10} rx={5} fill="#2a2a2a" />
        <rect x={W / 2 - 12} y={-2} width={24} height={6} rx={3} fill="#101010" />

        {/* MCU LQFP-48 */}
        <rect x={W / 2 - 32} y={H / 2 - 32} width={64} height={64} rx={1.5} fill="url(#bp-chip)" stroke="#000" strokeWidth={1} />
        <rect x={W / 2 - 30} y={H / 2 - 30} width={60} height={2} fill="#3a3a3a" opacity={0.5} />
        <circle cx={W / 2 - 24} cy={H / 2 - 24} r={2.5} fill="#666" />
        <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#ddd" fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700}>
          STM32F401
        </text>
        <text x={W / 2} y={H / 2 + 4} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={6.5}>
          CCU6 / CDU6
        </text>
        <text x={W / 2} y={H / 2 + 14} textAnchor="middle" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          ARM Cortex-M4F
        </text>
        <text x={W / 2} y={H / 2 + 24} textAnchor="middle" fill="#666" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          LQFP-48
        </text>

        {/* PC13 LED (active-low) */}
        <circle cx={W - 22} cy={36} r={3.5} fill="#1a1a1a" stroke="#000" strokeWidth={0.5} />
        <circle cx={W - 22} cy={36} r={2} fill="#3DD68C" opacity={0.85} />
        <text x={W - 22} y={50} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>PC13</text>

        {/* NRST button */}
        <rect x={20} y={H - 50} width={22} height={16} rx={2} fill="#1a1a1a" stroke="#000" strokeWidth={0.5} />
        <circle cx={31} cy={H - 42} r={4.5} fill="#444" stroke="#1a1a1a" strokeWidth={0.5} />
        <text x={31} y={H - 22} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>NRST</text>

        {/* KEY (BOOT0) button */}
        <rect x={W - 42} y={H - 50} width={22} height={16} rx={2} fill="#1a1a1a" stroke="#000" strokeWidth={0.5} />
        <circle cx={W - 31} cy={H - 42} r={4.5} fill="#444" stroke="#1a1a1a" strokeWidth={0.5} />
        <text x={W - 31} y={H - 22} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>KEY</text>

        {/* Silkscreen branding */}
        <text
          x={W / 2}
          y={28}
          textAnchor="middle"
          fill="#ffffff"
          fontFamily="'Outfit', sans-serif"
          fontSize={11}
          fontWeight={700}
          letterSpacing="0.1em"
        >
          BLACK PILL
        </text>
        <text
          x={W / 2}
          y={42}
          textAnchor="middle"
          fill="#888"
          fontFamily="'JetBrains Mono', monospace"
          fontSize={6}
        >
          WeAct · F401
        </text>

        {/* GPIO pin pads */}
        {allPins.filter((p) => p.side === 'left').map((p) => (
          <g key={p.id}>
            <rect x={-3} y={p.y - 3.5} width={12} height={7} fill="url(#bp-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={3} cy={p.y} r={1.4} fill="#0a0a0a" />
            <text x={14} y={p.y + 2} fill="#ffffff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}
        {allPins.filter((p) => p.side === 'right').map((p) => (
          <g key={p.id}>
            <rect x={W - 9} y={p.y - 3.5} width={12} height={7} fill="url(#bp-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={W - 3} cy={p.y} r={1.4} fill="#0a0a0a" />
            <text x={W - 14} y={p.y + 2} textAnchor="end" fill="#ffffff" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}

        {/* Power pads */}
        <rect x={W / 2 - 28} y={H - 8} width={14} height={10} fill="#FF6B6B" stroke="#7a1a1a" strokeWidth={0.4} />
        <text x={W / 2 - 21} y={H - 12} textAnchor="middle" fill="#FF9999" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          3V3
        </text>
        <rect x={W / 2 + 14} y={H - 8} width={14} height={10} fill="#2a2a2a" stroke="#000" strokeWidth={0.4} />
        <text x={W / 2 + 21} y={H - 12} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          GND
        </text>

        {selected && (
          <rect width={W} height={H} rx={6} fill="none" stroke="#F062B8" strokeWidth={3} opacity={0.85} />
        )}
      </g>
    );
  },
};
