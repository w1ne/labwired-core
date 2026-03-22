import type { ComponentDef, PinDef } from '../../types';

const W = 160;
const H = 340;
const PIN_SPACING = 16;
const PIN_START_Y = 35;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  // Left side: GP0-GP15
  for (let i = 0; i <= 15; i++) {
    pins.push({
      id: `GP${i}`,
      x: 0,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'left',
      label: `GP${i}`,
    });
  }

  // Right side: GP16-GP28
  for (let i = 16; i <= 28; i++) {
    pins.push({
      id: `GP${i}`,
      x: W,
      y: PIN_START_Y + (i - 16) * PIN_SPACING,
      side: 'right',
      label: `GP${i}`,
    });
  }

  // Power
  pins.push({ id: '3V3', x: W / 2 - 15, y: H, side: 'bottom', label: '3V3' });
  pins.push({ id: 'GND', x: W / 2 + 15, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const rpiPicoComponent: ComponentDef = {
  type: 'rpi-pico',
  label: 'RPi Pico',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width={W} height={H} rx={6}
        fill="#2d8040" stroke={state?.selected ? '#e83e8c' : '#1a5c2a'} strokeWidth={state?.selected ? 3 : 2} />
      {/* USB connector */}
      <rect x={W / 2 - 10} y={-4} width={20} height={8} rx={2} fill="#888" />
      <text x={W / 2} y={20} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={11} fontWeight={700}>RPi Pico</text>
      <text x={W / 2} y={30} textAnchor="middle" fill="rgba(255,255,255,0.5)"
        fontFamily="'JetBrains Mono', monospace" fontSize={7}>RP2040</text>

      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={6} y={p.y + 3} fill="#cfc"
          fontFamily="'JetBrains Mono', monospace" fontSize={6}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 6} y={p.y + 3} textAnchor="end" fill="#cfc"
          fontFamily="'JetBrains Mono', monospace" fontSize={6}>{p.label}</text>
      ))}
    </g>
  ),
};
