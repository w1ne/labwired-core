import type { ComponentDef, PinDef } from '../../types';

const W = 280;
const H = 340;
const SIDE_PIN_SPACING = 17;
const SIDE_PIN_START_Y = 48;
const TOP_PIN_SPACING = 16;
const TOP_PIN_START_X = 18;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  for (let i = 0; i <= 15; i++) {
    pins.push({
      id: `PA${i}`,
      x: 0,
      y: SIDE_PIN_START_Y + i * SIDE_PIN_SPACING,
      side: 'left',
      label: `PA${i}`,
    });
    pins.push({
      id: `PB${i}`,
      x: W,
      y: SIDE_PIN_START_Y + i * SIDE_PIN_SPACING,
      side: 'right',
      label: `PB${i}`,
    });
    pins.push({
      id: `PC${i}`,
      x: TOP_PIN_START_X + i * TOP_PIN_SPACING,
      y: 0,
      side: 'top',
      label: `PC${i}`,
    });
  }

  pins.push({ id: 'VCC', x: W / 2 - 20, y: H, side: 'bottom', label: 'VCC' });
  pins.push({ id: 'GND', x: W / 2 + 20, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const stm32DevComponent: ComponentDef = {
  type: 'stm32-dev',
  label: 'STM32 Dev Board',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        {/* Drop shadow under the PCB */}
        <ellipse cx={W / 2} cy={H + 6} rx={W / 2 - 8} ry={6} fill="#000" opacity={0.35} />

        {/* PCB body — classic Bluepill teal/blue gradient with subtle silkscreen texture */}
        <defs>
          <linearGradient id="stm-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1F5A82" />
            <stop offset="0.5" stopColor="#164566" />
            <stop offset="1" stopColor="#0E3552" />
          </linearGradient>
          <pattern id="stm-dots" x="0" y="0" width="6" height="6" patternUnits="userSpaceOnUse">
            <circle cx={3} cy={3} r={0.35} fill="#0a2440" opacity={0.7} />
          </pattern>
          <linearGradient id="stm-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0e0e0e" />
          </linearGradient>
          <linearGradient id="stm-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#C49A3A" />
          </linearGradient>
        </defs>

        <rect
          width={W}
          height={H}
          rx={10}
          fill="url(#stm-pcb)"
          stroke={selected ? '#F062B8' : '#06192a'}
          strokeWidth={selected ? 3 : 1.5}
        />
        <rect width={W} height={H} rx={10} fill="url(#stm-dots)" opacity={0.4} />

        {/* USB Mini connector at top-center */}
        <rect x={W / 2 - 26} y={-10} width={52} height={22} rx={2} fill="#a8aab0" stroke="#3a3a3a" strokeWidth={1.2} />
        <rect x={W / 2 - 22} y={-6} width={44} height={12} rx={1} fill="#2a2a2a" />
        <rect x={W / 2 - 14} y={-3} width={28} height={6} fill="#101010" />

        {/* MCU chip — LQFP with pin 1 marker */}
        <rect x={W / 2 - 36} y={H / 2 - 36} width={72} height={72} rx={2} fill="url(#stm-chip)" stroke="#000" strokeWidth={1} />
        {/* Faint chip surface highlight */}
        <rect x={W / 2 - 34} y={H / 2 - 34} width={68} height={2} fill="#3a3a3a" opacity={0.5} />
        <circle cx={W / 2 - 28} cy={H / 2 - 28} r={2.5} fill="#666" />
        <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#bbb" fontFamily="'Outfit', sans-serif" fontSize={8.5} fontWeight={600}>
          STM32F103
        </text>
        <text x={W / 2} y={H / 2 + 6} textAnchor="middle" fill="#888" fontFamily="'JetBrains Mono', monospace" fontSize={6.5}>
          C8T6
        </text>
        <text x={W / 2} y={H / 2 + 18} textAnchor="middle" fill="#666" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          ARM Cortex-M3
        </text>

        {/* Status LED on PC13 (top-right, near PC13 pin) */}
        <circle cx={W - 38} cy={28} r={4} fill="#1a3a1a" stroke="#000" strokeWidth={0.5} />
        <circle cx={W - 38} cy={28} r={2.5} fill="#3DD68C" opacity={0.85} />
        <text x={W - 38} y={45} textAnchor="middle" fill="#ddd" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>PC13</text>

        {/* Reset button */}
        <rect x={W - 56} y={H - 60} width={34} height={16} rx={2} fill="#202020" stroke="#000" strokeWidth={0.8} />
        <circle cx={W - 39} cy={H - 52} r={5} fill="#555" stroke="#2a2a2a" strokeWidth={0.8} />
        <circle cx={W - 39} cy={H - 52} r={2.5} fill="#1a1a1a" />
        <text x={W - 39} y={H - 30} textAnchor="middle" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6}>NRST</text>

        {/* BOOT0 jumper */}
        <rect x={26} y={H - 60} width={20} height={16} fill="#3a3a3a" stroke="#1a1a1a" strokeWidth={0.6} />
        <rect x={28} y={H - 58} width={6} height={4} fill="#FFD700" />
        <rect x={38} y={H - 58} width={6} height={4} fill="#FFD700" />
        <rect x={28} y={H - 50} width={16} height={3} fill="#444" />
        <text x={36} y={H - 30} textAnchor="middle" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6}>BOOT0</text>

        {/* SWD header (top-right corner) */}
        <rect x={W - 70} y={H - 30} width={60} height={14} fill="#1a1a1a" stroke="#000" strokeWidth={0.5} />
        {[0, 1, 2, 3].map((i) => (
          <circle key={i} cx={W - 62 + i * 14} cy={H - 23} r={2} fill="url(#stm-pad)" stroke="#2a2a2a" strokeWidth={0.4} />
        ))}
        <text x={W - 40} y={H - 6} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>SWD</text>

        {/* Silkscreen title */}
        <text
          x={W / 2}
          y={26}
          textAnchor="middle"
          fill="#ffffff"
          fontFamily="'Outfit', sans-serif"
          fontSize={13}
          fontWeight={700}
          letterSpacing="0.06em"
        >
          BLUEPILL
        </text>

        {/* GPIO pin pads with through-hole — left side */}
        {allPins.filter((p) => p.side === 'left').map((p) => (
          <g key={p.id}>
            <rect x={-3} y={p.y - 4} width={12} height={8} fill="url(#stm-pad)" stroke="#7a5a1a" strokeWidth={0.4} />
            <circle cx={3} cy={p.y} r={1.7} fill="#0a0a0a" />
            <text x={14} y={p.y + 2.5} fill="#ffffff" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={500}>{p.label}</text>
          </g>
        ))}
        {/* Right side */}
        {allPins.filter((p) => p.side === 'right').map((p) => (
          <g key={p.id}>
            <rect x={W - 9} y={p.y - 4} width={12} height={8} fill="url(#stm-pad)" stroke="#7a5a1a" strokeWidth={0.4} />
            <circle cx={W - 3} cy={p.y} r={1.7} fill="#0a0a0a" />
            <text x={W - 14} y={p.y + 2.5} textAnchor="end" fill="#ffffff" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={500}>{p.label}</text>
          </g>
        ))}
        {/* Top side */}
        {allPins.filter((p) => p.side === 'top').map((p) => (
          <g key={p.id}>
            <rect x={p.x - 4} y={-3} width={8} height={12} fill="url(#stm-pad)" stroke="#7a5a1a" strokeWidth={0.4} />
            <circle cx={p.x} cy={3} r={1.7} fill="#0a0a0a" />
            <text x={p.x} y={20} textAnchor="middle" fill="#ffffff" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>{p.label}</text>
          </g>
        ))}

        {/* Bottom power pads */}
        <rect x={W / 2 - 26} y={H - 8} width={12} height={10} fill="#FF6B6B" stroke="#7a1a1a" strokeWidth={0.4} />
        <text x={W / 2 - 20} y={H - 12} textAnchor="middle" fill="#FF9999" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>VCC</text>
        <rect x={W / 2 + 14} y={H - 8} width={12} height={10} fill="#2a2a2a" stroke="#000" strokeWidth={0.4} />
        <text x={W / 2 + 20} y={H - 12} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>GND</text>

        {/* Selection highlight */}
        {selected && (
          <rect width={W} height={H} rx={10} fill="none" stroke="#F062B8" strokeWidth={3} opacity={0.85} />
        )}
      </g>
    );
  },
};
