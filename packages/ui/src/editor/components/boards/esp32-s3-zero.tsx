import type { ComponentDef, PinDef } from '../../types';

const W = 120;
const H = 280;
const PIN_SPACING = 16;
const PIN_START_Y = 32;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  // Left side: GPIO1-GPIO14
  const leftPins = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14];
  for (let i = 0; i < leftPins.length; i++) {
    pins.push({
      id: `GPIO${leftPins[i]}`,
      x: 0,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'left',
      label: `GP${leftPins[i]}`,
    });
  }

  // Right side: GPIO15-GPIO21, GPIO35-GPIO38, GPIO47-GPIO48
  const rightPins = [15, 16, 17, 18, 21, 35, 36, 37, 38, 47, 48];
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
  pins.push({ id: '3V3', x: W / 2 - 15, y: H, side: 'bottom', label: '3.3V' });
  pins.push({ id: 'GND', x: W / 2 + 15, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const esp32S3ZeroComponent: ComponentDef = {
  type: 'esp32-s3-zero',
  label: 'ESP32-S3-Zero',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width={W} height={H} rx={4}
        fill="#1a1a2e" stroke={state?.selected ? '#e83e8c' : '#333'} strokeWidth={state?.selected ? 3 : 2} />
      {/* USB-C connector */}
      <rect x={W / 2 - 8} y={0} width={16} height={6} rx={2} fill="#555" />
      {/* RGB LED indicator */}
      <circle cx={W / 2} cy={14} r={3} fill="#2ecc71" opacity={0.6} />
      <text x={W / 2} y={24} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700}>ESP32-S3</text>

      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={6} y={p.y + 3} fill="#888"
          fontFamily="'JetBrains Mono', monospace" fontSize={5}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 6} y={p.y + 3} textAnchor="end" fill="#888"
          fontFamily="'JetBrains Mono', monospace" fontSize={5}>{p.label}</text>
      ))}
    </g>
  ),
};
