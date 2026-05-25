import type { ComponentDef, PinDef } from '../../types';

const W = 280;
const H = 440;
const PIN_SPACING = 16;
const PIN_START_Y = 92;

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

export const nucleoF401reComponent: ComponentDef = {
  type: 'nucleo-f401re',
  label: 'NUCLEO-F401RE',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const stLinkH = 76;
    return (
      <g>
        <ellipse cx={W / 2} cy={H + 6} rx={W / 2 - 8} ry={5} fill="#000" opacity={0.3} />

        <defs>
          <linearGradient id="nuc64-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f5f5f0" />
            <stop offset="1" stopColor="#e3e0d6" />
          </linearGradient>
          <linearGradient id="nuc64-stlink" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#ecebe5" />
            <stop offset="1" stopColor="#d8d4c8" />
          </linearGradient>
          <linearGradient id="nuc64-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0e0e0e" />
          </linearGradient>
          <linearGradient id="nuc64-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#C49A3A" />
          </linearGradient>
        </defs>

        <rect
          width={W}
          height={H}
          rx={6}
          fill="url(#nuc64-pcb)"
          stroke={selected ? '#F062B8' : '#5a5648'}
          strokeWidth={selected ? 3 : 1}
        />

        {/* ST-LINK area */}
        <rect x={4} y={4} width={W - 8} height={stLinkH} fill="url(#nuc64-stlink)" stroke="#bdb7a8" strokeWidth={0.5} />
        <rect x={0} y={4 + stLinkH} width={W} height={1} fill="#9c9686" />

        {/* USB Mini-B */}
        <rect x={W / 2 - 18} y={-6} width={36} height={14} rx={1.5} fill="#a8aab0" stroke="#3a3a3a" strokeWidth={1} />
        <rect x={W / 2 - 14} y={-3} width={28} height={6} fill="#2a2a2a" />

        {/* ST-LINK MCU */}
        <rect x={28} y={26} width={32} height={32} rx={1} fill="url(#nuc64-chip)" stroke="#000" strokeWidth={0.5} />
        <text x={44} y={46} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          STLINK
        </text>
        <text x={44} y={54} textAnchor="middle" fill="#777" fontFamily="'JetBrains Mono', monospace" fontSize={5}>
          V2-1
        </text>

        {/* COM LED */}
        <circle cx={76} cy={42} r={2.8} fill="#1a3a1a" stroke="#000" strokeWidth={0.4} />
        <circle cx={76} cy={42} r={1.6} fill="#3DD68C" opacity={0.85} />

        <text x={W - 12} y={18} textAnchor="end" fill="#444" fontFamily="'Outfit', sans-serif" fontSize={8} fontWeight={700}>
          ST-LINK / V2-1
        </text>
        <text x={W - 12} y={28} textAnchor="end" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          on-board debugger
        </text>

        {/* Target MCU LQFP-64 */}
        <rect x={W / 2 - 44} y={H / 2 - 44} width={88} height={88} rx={2} fill="url(#nuc64-chip)" stroke="#000" strokeWidth={1} />
        <rect x={W / 2 - 42} y={H / 2 - 42} width={84} height={2} fill="#3a3a3a" opacity={0.5} />
        <circle cx={W / 2 - 36} cy={H / 2 - 36} r={2.5} fill="#666" />
        <text x={W / 2} y={H / 2 - 8} textAnchor="middle" fill="#ddd" fontFamily="'Outfit', sans-serif" fontSize={10} fontWeight={700}>
          STM32F401
        </text>
        <text x={W / 2} y={H / 2 + 4} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={7}>
          RET6
        </text>
        <text x={W / 2} y={H / 2 + 16} textAnchor="middle" fill="#999" fontFamily="'JetBrains Mono', monospace" fontSize={6.5}>
          ARM Cortex-M4F
        </text>
        <text x={W / 2} y={H / 2 + 26} textAnchor="middle" fill="#777" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          LQFP-64
        </text>

        {/* LD2 user LED on PA5 */}
        <circle cx={W - 60} cy={H - 84} r={4} fill="#222" stroke="#000" strokeWidth={0.4} />
        <circle cx={W - 60} cy={H - 84} r={2.4} fill="#3DD68C" opacity={0.85} />
        <text x={W - 60} y={H - 70} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>LD2</text>

        {/* User button B1 (blue) */}
        <rect x={W - 60} y={H - 60} width={32} height={22} rx={2} fill="#1F5A82" stroke="#0E3552" strokeWidth={0.7} />
        <circle cx={W - 44} cy={H - 49} r={6} fill="#2A6F9C" stroke="#0E3552" strokeWidth={0.5} />
        <text x={W - 44} y={H - 28} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>USER</text>

        {/* Reset button */}
        <rect x={20} y={H - 60} width={28} height={16} rx={2} fill="#202020" stroke="#000" strokeWidth={0.5} />
        <circle cx={34} cy={H - 52} r={5} fill="#555" stroke="#2a2a2a" strokeWidth={0.5} />
        <text x={34} y={H - 32} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>NRST</text>

        <text
          x={W / 2}
          y={stLinkH + 22}
          textAnchor="middle"
          fill="#3a342a"
          fontFamily="'Outfit', sans-serif"
          fontSize={13}
          fontWeight={700}
          letterSpacing="0.08em"
        >
          NUCLEO-F401RE
        </text>
        <text
          x={W / 2}
          y={stLinkH + 34}
          textAnchor="middle"
          fill="#7a7363"
          fontFamily="'JetBrains Mono', monospace"
          fontSize={7}
        >
          MB1136 · STM32F4 Nucleo-64
        </text>

        {/* Morpho headers - GPIO pin pads */}
        {allPins.filter((p) => p.side === 'left').map((p) => (
          <g key={p.id}>
            <rect x={-3} y={p.y - 4} width={12} height={8} fill="url(#nuc64-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={3} cy={p.y} r={1.5} fill="#0a0a0a" />
            <text x={14} y={p.y + 2.5} fill="#3a342a" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}
        {allPins.filter((p) => p.side === 'right').map((p) => (
          <g key={p.id}>
            <rect x={W - 9} y={p.y - 4} width={12} height={8} fill="url(#nuc64-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={W - 3} cy={p.y} r={1.5} fill="#0a0a0a" />
            <text x={W - 14} y={p.y + 2.5} textAnchor="end" fill="#3a342a" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}

        {/* Morpho silkscreen labels */}
        <text x={4} y={PIN_START_Y - 6} fill="#7a7363" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          CN7
        </text>
        <text x={W - 4} y={PIN_START_Y - 6} textAnchor="end" fill="#7a7363" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          CN10
        </text>

        {/* Power pads */}
        <rect x={W / 2 - 28} y={H - 8} width={14} height={10} fill="#FF6B6B" stroke="#7a1a1a" strokeWidth={0.4} />
        <text x={W / 2 - 21} y={H - 12} textAnchor="middle" fill="#FF6666" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          3V3
        </text>
        <rect x={W / 2 + 14} y={H - 8} width={14} height={10} fill="#2a2a2a" stroke="#000" strokeWidth={0.4} />
        <text x={W / 2 + 21} y={H - 12} textAnchor="middle" fill="#666" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          GND
        </text>

        {selected && (
          <rect width={W} height={H} rx={6} fill="none" stroke="#F062B8" strokeWidth={3} opacity={0.85} />
        )}
      </g>
    );
  },
};
