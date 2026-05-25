import type { ComponentDef, PinDef } from '../../types';

const W = 320;
const H = 520;
const PIN_SPACING = 14;
const PIN_START_Y = 100;
const MORPHO_LEFT_X = -3;
const MORPHO_RIGHT_X = W + 3;

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

export const nucleoH563ziComponent: ComponentDef = {
  type: 'nucleo-h563zi',
  label: 'NUCLEO-H563ZI',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const stLinkY = 4;
    const stLinkH = 80;
    return (
      <g>
        <ellipse cx={W / 2} cy={H + 6} rx={W / 2 - 8} ry={6} fill="#000" opacity={0.3} />

        <defs>
          <linearGradient id="nuc144-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f5f5f0" />
            <stop offset="1" stopColor="#e3e0d6" />
          </linearGradient>
          <linearGradient id="nuc144-stlink" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#ecebe5" />
            <stop offset="1" stopColor="#d8d4c8" />
          </linearGradient>
          <linearGradient id="nuc144-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0e0e0e" />
          </linearGradient>
          <linearGradient id="nuc144-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#C49A3A" />
          </linearGradient>
        </defs>

        <rect
          width={W}
          height={H}
          rx={6}
          fill="url(#nuc144-pcb)"
          stroke={selected ? '#F062B8' : '#5a5648'}
          strokeWidth={selected ? 3 : 1}
        />

        {/* ST-LINK section divider */}
        <rect x={0} y={stLinkY + stLinkH} width={W} height={1} fill="#9c9686" />
        <rect x={0} y={stLinkY + stLinkH - 1} width={W} height={2} fill="#fff" opacity={0.4} />

        {/* ST-LINK board area */}
        <rect x={4} y={stLinkY} width={W - 8} height={stLinkH} fill="url(#nuc144-stlink)" stroke="#bdb7a8" strokeWidth={0.5} />

        {/* ST-LINK USB Mini connector */}
        <rect x={W / 2 - 18} y={-6} width={36} height={14} rx={1.5} fill="#a8aab0" stroke="#3a3a3a" strokeWidth={1} />
        <rect x={W / 2 - 14} y={-3} width={28} height={6} fill="#2a2a2a" />

        {/* ST-LINK MCU */}
        <rect x={32} y={stLinkY + 28} width={36} height={36} rx={1} fill="url(#nuc144-chip)" stroke="#000" strokeWidth={0.6} />
        <text x={50} y={stLinkY + 50} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          STLINK
        </text>
        <text x={50} y={stLinkY + 58} textAnchor="middle" fill="#777" fontFamily="'JetBrains Mono', monospace" fontSize={5}>
          V3E
        </text>

        {/* COM LED next to st-link */}
        <circle cx={90} cy={stLinkY + 46} r={3} fill="#1a3a1a" stroke="#000" strokeWidth={0.4} />
        <circle cx={90} cy={stLinkY + 46} r={1.8} fill="#3DD68C" opacity={0.85} />
        <text x={90} y={stLinkY + 64} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={5}>COM</text>

        {/* ST-LINK label */}
        <text x={W - 16} y={stLinkY + 18} textAnchor="end" fill="#444" fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700} letterSpacing="0.05em">
          ST-LINK / V3E
        </text>
        <text x={W - 16} y={stLinkY + 30} textAnchor="end" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          MB1361 debugger
        </text>

        {/* Target MCU (LQFP-176 in center) */}
        <rect x={W / 2 - 56} y={H / 2 - 56} width={112} height={112} rx={2} fill="url(#nuc144-chip)" stroke="#000" strokeWidth={1} />
        <rect x={W / 2 - 54} y={H / 2 - 54} width={108} height={2} fill="#3a3a3a" opacity={0.5} />
        <circle cx={W / 2 - 46} cy={H / 2 - 46} r={3} fill="#666" />
        <text x={W / 2} y={H / 2 - 12} textAnchor="middle" fill="#ddd" fontFamily="'Outfit', sans-serif" fontSize={11} fontWeight={700}>
          STM32H563
        </text>
        <text x={W / 2} y={H / 2} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={7.5}>
          ZIT6
        </text>
        <text x={W / 2} y={H / 2 + 14} textAnchor="middle" fill="#999" fontFamily="'JetBrains Mono', monospace" fontSize={7}>
          ARM Cortex-M33
        </text>
        <text x={W / 2} y={H / 2 + 26} textAnchor="middle" fill="#777" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          LQFP-144
        </text>

        {/* Status LEDs (LD1 green, LD2 yellow, LD3 red) */}
        {[
          { x: 60, color: '#3DD68C', label: 'LD1' },
          { x: 100, color: '#F0E040', label: 'LD2' },
          { x: 140, color: '#F04040', label: 'LD3' },
        ].map((led) => (
          <g key={led.label}>
            <circle cx={led.x} cy={H - 96} r={4} fill="#222" stroke="#000" strokeWidth={0.4} />
            <circle cx={led.x} cy={H - 96} r={2.4} fill={led.color} opacity={0.85} />
            <text x={led.x} y={H - 82} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
              {led.label}
            </text>
          </g>
        ))}

        {/* User button B1 (blue) */}
        <rect x={W - 70} y={H - 110} width={36} height={26} rx={2} fill="#1F5A82" stroke="#0E3552" strokeWidth={0.8} />
        <circle cx={W - 52} cy={H - 97} r={7} fill="#2A6F9C" stroke="#0E3552" strokeWidth={0.6} />
        <text x={W - 52} y={H - 76} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={6}>USER</text>

        {/* Reset button */}
        <rect x={28} y={H - 60} width={28} height={16} rx={2} fill="#202020" stroke="#000" strokeWidth={0.6} />
        <circle cx={42} cy={H - 52} r={5} fill="#555" stroke="#2a2a2a" strokeWidth={0.5} />
        <text x={42} y={H - 32} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={6}>NRST</text>

        {/* Ethernet RJ45 (right side mid) */}
        <rect x={W - 36} y={H / 2 + 80} width={32} height={48} rx={1} fill="#3a3a3a" stroke="#000" strokeWidth={0.6} />
        <rect x={W - 34} y={H / 2 + 82} width={28} height={20} fill="#222" />
        <text x={W - 20} y={H / 2 + 138} textAnchor="middle" fill="#444" fontFamily="'JetBrains Mono', monospace" fontSize={6}>ETH</text>

        {/* Silkscreen header label */}
        <text
          x={W / 2}
          y={stLinkY + stLinkH + 18}
          textAnchor="middle"
          fill="#3a342a"
          fontFamily="'Outfit', sans-serif"
          fontSize={14}
          fontWeight={700}
          letterSpacing="0.08em"
        >
          NUCLEO-H563ZI
        </text>
        <text
          x={W / 2}
          y={stLinkY + stLinkH + 32}
          textAnchor="middle"
          fill="#7a7363"
          fontFamily="'JetBrains Mono', monospace"
          fontSize={7.5}
        >
          MB1404 · STM32H5 Nucleo-144
        </text>

        {/* Morpho headers (CN11/CN12) - GPIO pin pads on long sides */}
        {allPins.filter((p) => p.side === 'left').map((p) => (
          <g key={p.id}>
            <rect x={MORPHO_LEFT_X} y={p.y - 3.5} width={11} height={7} fill="url(#nuc144-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={MORPHO_LEFT_X + 5.5} cy={p.y} r={1.4} fill="#0a0a0a" />
            <text x={14} y={p.y + 2} fill="#3a342a" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}
        {allPins.filter((p) => p.side === 'right').map((p) => (
          <g key={p.id}>
            <rect x={MORPHO_RIGHT_X - 11} y={p.y - 3.5} width={11} height={7} fill="url(#nuc144-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={MORPHO_RIGHT_X - 5.5} cy={p.y} r={1.4} fill="#0a0a0a" />
            <text x={W - 14} y={p.y + 2} textAnchor="end" fill="#3a342a" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}

        {/* CN11/CN12 silkscreen labels */}
        <text x={4} y={PIN_START_Y - 6} fill="#7a7363" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          CN11
        </text>
        <text x={W - 4} y={PIN_START_Y - 6} textAnchor="end" fill="#7a7363" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          CN12
        </text>

        {/* Power pads bottom */}
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
