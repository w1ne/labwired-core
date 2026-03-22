import type { ComponentDef, PinDef } from '../../types';

const W = 180;
const H = 320;
const PIN_SPACING = 16;
const PIN_START_Y = 40;

function generatePins(): PinDef[] {
  const pins: PinDef[] = [];

  // Left side: P0.00-P0.15
  for (let i = 0; i <= 15; i++) {
    pins.push({
      id: `P0.${String(i).padStart(2, '0')}`,
      x: 0,
      y: PIN_START_Y + i * PIN_SPACING,
      side: 'left',
      label: `P0.${String(i).padStart(2, '0')}`,
    });
  }

  // Right side: P0.16-P0.31
  for (let i = 16; i <= 31; i++) {
    pins.push({
      id: `P0.${i}`,
      x: W,
      y: PIN_START_Y + (i - 16) * PIN_SPACING,
      side: 'right',
      label: `P0.${i}`,
    });
  }

  // Power
  pins.push({ id: 'VDD', x: W / 2 - 15, y: H, side: 'bottom', label: 'VDD' });
  pins.push({ id: 'GND', x: W / 2 + 15, y: H, side: 'bottom', label: 'GND' });

  return pins;
}

const allPins = generatePins();

export const nrf52840DkComponent: ComponentDef = {
  type: 'nrf52840-dk',
  label: 'nRF52840 DK',
  category: 'mcu',
  width: W,
  height: H,
  pins: allPins,
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect width={W} height={H} rx={6}
        fill="#1e2848" stroke={state?.selected ? '#e83e8c' : '#2a3a6e'} strokeWidth={state?.selected ? 3 : 2} />
      <text x={W / 2} y={22} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={11} fontWeight={700}>nRF52840 DK</text>
      <text x={W / 2} y={34} textAnchor="middle" fill="rgba(255,255,255,0.5)"
        fontFamily="'JetBrains Mono', monospace" fontSize={7}>Nordic Semi</text>

      {allPins.filter((p) => p.side === 'left').map((p) => (
        <text key={p.id} x={6} y={p.y + 3} fill="#88a"
          fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>{p.label}</text>
      ))}
      {allPins.filter((p) => p.side === 'right').map((p) => (
        <text key={p.id} x={W - 6} y={p.y + 3} textAnchor="end" fill="#88a"
          fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>{p.label}</text>
      ))}
    </g>
  ),
};
