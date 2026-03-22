import type { ComponentDef, PinDef } from '../../types';

const W = 220;
const H = 300;
const PIN_SPACING = 18;
const PIN_START_Y = 40;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  // Digital pins D0-D13 (right side)
  for (let i = 0; i <= 13; i++) {
    pins.push({
      id: `D${i}`,
      x: W,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'right',
      label: `D${i}`,
    });
  }

  // Analog pins A0-A5 (left side)
  for (let i = 0; i <= 5; i++) {
    pins.push({
      id: `A${i}`,
      x: 0,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'left',
      label: `A${i}`,
    });
  }

  // Power pins
  pins.push({ id: '5V', x: 0, y: PIN_START_Y + 7 * PIN_SPACING, side: 'left', label: '5V' });
  pins.push({ id: '3V3', x: 0, y: PIN_START_Y + 8 * PIN_SPACING, side: 'left', label: '3.3V' });
  pins.push({ id: 'GND', x: W / 2, y: H, side: 'bottom', label: 'GND' });
  pins.push({ id: 'VIN', x: 0, y: PIN_START_Y + 9 * PIN_SPACING, side: 'left', label: 'VIN' });

  return pins;
}

const allPins = generatePins();

export const arduinoUnoComponent: ComponentDef = {
  type: 'arduino-uno',
  label: 'Arduino Uno',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width={W} height={H} rx={8}
        fill="#00687c" stroke={state?.selected ? '#e83e8c' : '#005c6e'} strokeWidth={state?.selected ? 3 : 2} />
      <text x={W / 2} y={20} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={13} fontWeight={700}>Arduino Uno</text>
      <text x={W / 2} y={32} textAnchor="middle" fill="rgba(255,255,255,0.5)"
        fontFamily="'JetBrains Mono', monospace" fontSize={8}>ATmega328P</text>

      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 8} y={p.y + 4} textAnchor="end" fill="#aad"
          fontFamily="'JetBrains Mono', monospace" fontSize={7}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={8} y={p.y + 4} fill="#aad"
          fontFamily="'JetBrains Mono', monospace" fontSize={7}>{p.label}</text>
      ))}
    </g>
  ),
};
