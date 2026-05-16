import type { ComponentDef, PinDef } from '../../types';

const W = 90;
const H = 180;
const PIN_SPACING = 16;
const PIN_START_Y = 30;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  // Left side (top → bottom): power rails + GPIO4..GPIO0
  // The ESP32-C3 Super Mini exposes 5V, GND, 3V3 at the top of the left header,
  // followed by GPIO4-GPIO0.
  const leftPower: Array<{ id: string; label: string }> = [
    { id: '5V', label: '5V' },
    { id: 'GND', label: 'GND' },
    { id: '3V3', label: '3V3' },
  ];
  leftPower.forEach((p, i) => {
    pins.push({ id: p.id, x: 0, y: PIN_START_Y + i * PIN_SPACING, side: 'left', label: p.label });
  });
  const leftGpio = [4, 3, 2, 1, 0];
  leftGpio.forEach((gp, i) => {
    pins.push({
      id: `GPIO${gp}`,
      x: 0,
      y: PIN_START_Y + (leftPower.length + i) * PIN_SPACING,
      side: 'left',
      label: `GP${gp}`,
    });
  });

  // Right side (top → bottom): GPIO5..GPIO10, then GPIO20 (RX0) and GPIO21 (TX0)
  const rightPins = [5, 6, 7, 8, 9, 10, 20, 21];
  rightPins.forEach((gp, i) => {
    pins.push({
      id: `GPIO${gp}`,
      x: W,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'right',
      label: `GP${gp}`,
    });
  });

  return pins;
}

const allPins = generatePins();

export const esp32C3SuperMiniComponent: ComponentDef = {
  type: 'esp32-c3-supermini',
  label: 'ESP32-C3 Super Mini',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width={W} height={H} rx={4}
        fill="#1a1a2e" stroke={state?.selected ? '#e83e8c' : '#333'} strokeWidth={state?.selected ? 3 : 2} />
      {/* USB-C connector at top */}
      <rect x={W / 2 - 9} y={0} width={18} height={6} rx={2} fill="#555" />
      {/* ESP32-C3 module silhouette + antenna stub */}
      <rect x={W / 2 - 16} y={H / 2 - 14} width={32} height={28} rx={2}
        fill="#0f0f1a" stroke="#2a2a3a" strokeWidth={1} />
      <rect x={W / 2 - 5} y={10} width={10} height={6} rx={1} fill="#2a2a3a" />
      {/* User LED on GPIO8 (blue, active-low on the Super Mini) */}
      <circle cx={W / 2 + 10} cy={20} r={2.5} fill="#5BD8FF" opacity={0.75} />
      <text x={W / 2} y={H / 2 - 2} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={8} fontWeight={700}>ESP32-C3</text>
      <text x={W / 2} y={H / 2 + 8} textAnchor="middle" fill="rgba(255,255,255,0.55)"
        fontFamily="'JetBrains Mono', monospace" fontSize={5}>SuperMini</text>

      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={5} y={p.y + 3} fill="#888"
          fontFamily="'JetBrains Mono', monospace" fontSize={5}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 5} y={p.y + 3} textAnchor="end" fill="#888"
          fontFamily="'JetBrains Mono', monospace" fontSize={5}>{p.label}</text>
      ))}
    </g>
  ),
};
