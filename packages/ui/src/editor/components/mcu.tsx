import type { ComponentDef, PinDef } from '../types';

const MCU_WIDTH = 200;
const MCU_HEIGHT = 280;
const PIN_SPACING = 20;
const PIN_START_Y = 40;

/** Generate GPIO pins along the left and right edges of the MCU. */
function generateMcuPins(): PinDef[] {
  const pins: PinDef[] = [];
  const ports = [
    { prefix: 'PA', count: 16, side: 'left' as const },
    { prefix: 'PB', count: 16, side: 'right' as const },
  ];

  for (const port of ports) {
    for (let i = 0; i < Math.min(port.count, 12); i++) {
      pins.push({
        id: `${port.prefix}${i}`,
        x: port.side === 'left' ? 0 : MCU_WIDTH,
        y: PIN_START_Y + i * PIN_SPACING,
        side: port.side,
        label: `${port.prefix}${i}`,
      });
    }
  }

  // Power/GND pins at bottom
  pins.push({ id: 'VCC', x: MCU_WIDTH / 2 - 20, y: MCU_HEIGHT, side: 'bottom', label: 'VCC' });
  pins.push({ id: 'GND', x: MCU_WIDTH / 2 + 20, y: MCU_HEIGHT, side: 'bottom', label: 'GND' });

  return pins;
}

export const mcuComponent: ComponentDef = {
  type: 'mcu',
  label: 'MCU',
  category: 'mcu',
  width: MCU_WIDTH,
  height: MCU_HEIGHT,
  pins: generateMcuPins(),
  defaultAttrs: {},
  render: (_attrs, state) => (
    <g>
      <rect
        width={MCU_WIDTH}
        height={MCU_HEIGHT}
        rx={8}
        fill="#1e1e28"
        stroke={state?.selected ? '#e83e8c' : '#000'}
        strokeWidth={state?.selected ? 3 : 2}
      />
      {/* Chip label */}
      <text x={MCU_WIDTH / 2} y={20} textAnchor="middle" fill="#fff"
        fontFamily="'Outfit', sans-serif" fontSize={14} fontWeight={700}>
        STM32
      </text>
      {/* Pin labels - left side (Port A) */}
      {generateMcuPins()
        .filter((p) => p.side === 'left')
        .map((p) => (
          <text key={p.id} x={8} y={p.y + 4} fill="#888"
            fontFamily="'JetBrains Mono', monospace" fontSize={8}>
            {p.label}
          </text>
        ))}
      {/* Pin labels - right side (Port B) */}
      {generateMcuPins()
        .filter((p) => p.side === 'right')
        .map((p) => (
          <text key={p.id} x={MCU_WIDTH - 8} y={p.y + 4} textAnchor="end" fill="#888"
            fontFamily="'JetBrains Mono', monospace" fontSize={8}>
            {p.label}
          </text>
        ))}
      {/* Power labels */}
      <text x={MCU_WIDTH / 2 - 20} y={MCU_HEIGHT - 8} textAnchor="middle" fill="#ff3333"
        fontFamily="'JetBrains Mono', monospace" fontSize={8}>VCC</text>
      <text x={MCU_WIDTH / 2 + 20} y={MCU_HEIGHT - 8} textAnchor="middle" fill="#888"
        fontFamily="'JetBrains Mono', monospace" fontSize={8}>GND</text>
    </g>
  ),
};
