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
  render: (_attrs, state) => (
    <g>
      <rect
        width={W}
        height={H}
        rx={10}
        fill="#1e1e28"
        stroke={state?.selected ? '#e83e8c' : '#000'}
        strokeWidth={state?.selected ? 3 : 2}
      />
      <rect x={22} y={34} width={W - 44} height={H - 90} rx={8} fill="#273043" stroke="#3f4c66" strokeWidth={1.5} />
      <text x={W / 2} y={28} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={14} fontWeight={700}>
        STM32 Dev Board
      </text>
      <text x={W / 2} y={H - 22} textAnchor="middle" fill="rgba(255,255,255,0.55)"
        fontFamily="'JetBrains Mono', monospace" fontSize={9}>
        PA/PB/PC GPIO breakout
      </text>

      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={8} y={p.y + 3} fill="#8aa0ff"
          fontFamily="'JetBrains Mono', monospace" fontSize={7}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 8} y={p.y + 3} textAnchor="end" fill="#8aa0ff"
          fontFamily="'JetBrains Mono', monospace" fontSize={7}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'top').map((p) => (
        <text key={p.id} x={p.x} y={12} textAnchor="middle" fill="#7ec8a5"
          fontFamily="'JetBrains Mono', monospace" fontSize={6}>{p.label}</text>
      ))}
      <text x={W / 2 - 20} y={H - 8} textAnchor="middle" fill="#ff6666"
        fontFamily="'JetBrains Mono', monospace" fontSize={8}>VCC</text>
      <text x={W / 2 + 20} y={H - 8} textAnchor="middle" fill="#a0a0a0"
        fontFamily="'JetBrains Mono', monospace" fontSize={8}>GND</text>
    </g>
  ),
};
