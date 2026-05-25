import type { ComponentDef, PinDef } from '../../types';

const W = 180;
const H = 360;
const PIN_SPACING = 16;
const PIN_START_Y = 78;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  const leftPins = [0, 1, 2, 3, 4, 5, 12, 13, 14, 15, 16, 17, 18, 19];
  for (let i = 0; i < leftPins.length; i++) {
    pins.push({
      id: `GPIO${leftPins[i]}`,
      x: 0,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'left',
      label: `GP${leftPins[i]}`,
    });
  }

  const rightPins = [21, 22, 23, 25, 26, 27, 32, 33, 34, 35, 36, 39];
  for (let i = 0; i < rightPins.length; i++) {
    pins.push({
      id: `GPIO${rightPins[i]}`,
      x: W,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'right',
      label: `GP${rightPins[i]}`,
    });
  }

  pins.push({ id: '3V3', x: W / 2 - 20, y: H, side: 'bottom', label: '3.3V' });
  pins.push({ id: 'GND', x: W / 2 + 20, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const esp32Component: ComponentDef = {
  type: 'esp32',
  label: 'ESP32-WROOM-32',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const moduleY = 8;
    const moduleH = 156;
    return (
      <g>
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 6} ry={4} fill="#000" opacity={0.3} />

        <defs>
          <linearGradient id="esp-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#0e1a0e" />
            <stop offset="1" stopColor="#070d07" />
          </linearGradient>
          <pattern id="esp-dots" x="0" y="0" width="5" height="5" patternUnits="userSpaceOnUse">
            <circle cx={2.5} cy={2.5} r={0.25} fill="#1a2a1a" opacity={0.6} />
          </pattern>
          <linearGradient id="esp-rfcan" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#d4d6d8" />
            <stop offset="0.5" stopColor="#a8aab0" />
            <stop offset="1" stopColor="#80828a" />
          </linearGradient>
          <linearGradient id="esp-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#C49A3A" />
          </linearGradient>
        </defs>

        <rect
          width={W}
          height={H}
          rx={6}
          fill="url(#esp-pcb)"
          stroke={selected ? '#F062B8' : '#000'}
          strokeWidth={selected ? 3 : 1}
        />
        <rect width={W} height={H} rx={6} fill="url(#esp-dots)" opacity={0.5} />

        {/* USB Micro-B at top */}
        <rect x={W / 2 - 16} y={-6} width={32} height={12} rx={1.5} fill="#a8aab0" stroke="#3a3a3a" strokeWidth={0.8} />
        <rect x={W / 2 - 12} y={-3} width={24} height={6} fill="#2a2a2a" />

        {/* WROOM-32 module (silver RF can) */}
        <rect x={16} y={moduleY} width={W - 32} height={moduleH} rx={2.5} fill="url(#esp-rfcan)" stroke="#5a5d62" strokeWidth={0.8} />
        <rect x={18} y={moduleY + 2} width={W - 36} height={1} fill="#fff" opacity={0.5} />
        <rect x={18} y={moduleY + moduleH - 3} width={W - 36} height={1} fill="#000" opacity={0.2} />

        {/* Antenna trace */}
        <rect x={W / 2 - 30} y={moduleY + 6} width={60} height={22} fill="none" stroke="#5a5d62" strokeWidth={0.6} strokeDasharray="1.6 1.6" opacity={0.55} />
        <text x={W / 2} y={moduleY + 20} textAnchor="middle" fill="#3a3d42" fontFamily="'JetBrains Mono', monospace" fontSize={4.8}>
          PCB ANT
        </text>

        {/* Module branding */}
        <text x={W / 2} y={moduleY + 50} textAnchor="middle" fill="#1a1d22" fontFamily="'Outfit', sans-serif" fontSize={12} fontWeight={700} letterSpacing="0.06em">
          ESP32
        </text>
        <text x={W / 2} y={moduleY + 64} textAnchor="middle" fill="#3a3d42" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>
          WROOM-32
        </text>
        <text x={W / 2} y={moduleY + 76} textAnchor="middle" fill="#5a5d62" fontFamily="'JetBrains Mono', monospace" fontSize={5}>
          Espressif Systems
        </text>
        <text x={W / 2} y={moduleY + 96} textAnchor="middle" fill="#3a3d42" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          Xtensa LX6 · 240 MHz
        </text>
        <text x={W / 2} y={moduleY + 108} textAnchor="middle" fill="#5a5d62" fontFamily="'JetBrains Mono', monospace" fontSize={4.8}>
          4 MB · 520 KB SRAM
        </text>
        <text x={W / 2} y={moduleY + 120} textAnchor="middle" fill="#5a5d62" fontFamily="'JetBrains Mono', monospace" fontSize={4.8}>
          Wi-Fi · BT 4.2 BLE
        </text>

        {/* FCC mark */}
        <text x={W / 2 - 28} y={moduleY + moduleH - 10} textAnchor="middle" fill="#5a5d62" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          FCC
        </text>
        <text x={W / 2 + 28} y={moduleY + moduleH - 10} textAnchor="middle" fill="#5a5d62" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          CE
        </text>

        {/* EN button */}
        <rect x={10} y={moduleY + moduleH + 8} width={18} height={12} rx={1.5} fill="#1a1a1a" stroke="#000" strokeWidth={0.4} />
        <circle cx={19} cy={moduleY + moduleH + 14} r={3} fill="#444" />
        <text x={19} y={moduleY + moduleH + 28} textAnchor="middle" fill="#9c9" fontFamily="'JetBrains Mono', monospace" fontSize={4.8}>EN</text>

        {/* BOOT button */}
        <rect x={W - 28} y={moduleY + moduleH + 8} width={18} height={12} rx={1.5} fill="#1a1a1a" stroke="#000" strokeWidth={0.4} />
        <circle cx={W - 19} cy={moduleY + moduleH + 14} r={3} fill="#444" />
        <text x={W - 19} y={moduleY + moduleH + 28} textAnchor="middle" fill="#9c9" fontFamily="'JetBrains Mono', monospace" fontSize={4.8}>BOOT</text>

        {/* Power LED */}
        <circle cx={W / 2} cy={moduleY + moduleH + 14} r={2.4} fill="#1a3a1a" stroke="#000" strokeWidth={0.3} />
        <circle cx={W / 2} cy={moduleY + moduleH + 14} r={1.3} fill="#FF4444" opacity={0.85} />

        {/* GPIO pin pads */}
        {allPins.filter((p) => p.side === 'left').map((p) => (
          <g key={p.id}>
            <rect x={-3} y={p.y - 3} width={11} height={6} fill="url(#esp-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={2.5} cy={p.y} r={1.2} fill="#0a0a0a" />
            <text x={12} y={p.y + 2} fill="#cdc" fontFamily="'JetBrains Mono', monospace" fontSize={5.5} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}
        {allPins.filter((p) => p.side === 'right').map((p) => (
          <g key={p.id}>
            <rect x={W - 8} y={p.y - 3} width={11} height={6} fill="url(#esp-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={W - 2.5} cy={p.y} r={1.2} fill="#0a0a0a" />
            <text x={W - 12} y={p.y + 2} textAnchor="end" fill="#cdc" fontFamily="'JetBrains Mono', monospace" fontSize={5.5} fontWeight={500}>
              {p.label}
            </text>
          </g>
        ))}

        {/* Power pads */}
        <rect x={W / 2 - 24} y={H - 8} width={12} height={9} fill="#FF6B6B" stroke="#7a1a1a" strokeWidth={0.4} />
        <text x={W / 2 - 18} y={H - 12} textAnchor="middle" fill="#FF9999" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>
          3V3
        </text>
        <rect x={W / 2 + 12} y={H - 8} width={12} height={9} fill="#2a2a2a" stroke="#000" strokeWidth={0.4} />
        <text x={W / 2 + 18} y={H - 12} textAnchor="middle" fill="#aaa" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={600}>
          GND
        </text>

        {selected && (
          <rect width={W} height={H} rx={6} fill="none" stroke="#F062B8" strokeWidth={3} opacity={0.85} />
        )}
      </g>
    );
  },
};
