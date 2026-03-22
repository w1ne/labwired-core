import type { ComponentDef, PinDef } from '../../types';

const W = 180;
const H = 360;
const PIN_SPACING = 16;
const PIN_START_Y = 40;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  // Left side: GPIO0-GPIO19
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

  // Right side: GPIO21-GPIO39
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

  // Power
  pins.push({ id: '3V3', x: W / 2 - 20, y: H, side: 'bottom', label: '3.3V' });
  pins.push({ id: 'GND', x: W / 2 + 20, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const esp32Component: ComponentDef = {
  type: 'esp32',
  label: 'ESP32',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width={W} height={H} rx={6}
        fill="#1e1e28" stroke={state?.selected ? '#e83e8c' : '#333'} strokeWidth={state?.selected ? 3 : 2} />
      {/* Antenna notch */}
      <rect x={W / 2 - 15} y={0} width={30} height={12} rx={2} fill="#444" />
      <text x={W / 2} y={24} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={12} fontWeight={700}>ESP32</text>
      <text x={W / 2} y={34} textAnchor="middle" fill="rgba(255,255,255,0.5)"
        fontFamily="'JetBrains Mono', monospace" fontSize={7}>ESP32-WROOM-32</text>

      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={8} y={p.y + 3} fill="#888"
          fontFamily="'JetBrains Mono', monospace" fontSize={6}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 8} y={p.y + 3} textAnchor="end" fill="#888"
          fontFamily="'JetBrains Mono', monospace" fontSize={6}>{p.label}</text>
      ))}
    </g>
  ),
};
