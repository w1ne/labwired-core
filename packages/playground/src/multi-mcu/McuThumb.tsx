// Tiny SVG board thumbnail (PCB + chip silhouette + pins +
// status LED) for the MCU strip tiles. Keeps the strip compact but
// recognisable — each tile reads as a piece of hardware rather
// than text.
import type { ChipSession } from './ChipsProvider';

interface McuThumbProps {
  session: ChipSession;
  width: number;
  height: number;
}

interface FamilyVisual {
  pcb: string;
  chip: string;
  label: string;
}

function pickFamily(boardId: string): FamilyVisual {
  const id = boardId.toLowerCase();
  if (id.includes('nrf52840')) return { pcb: '#2b1f6e', chip: '#1a103f', label: 'nRF52840' };
  if (id.includes('stm32f4')) return { pcb: '#0e3b18', chip: '#0a2811', label: 'STM32F4' };
  if (id.includes('stm32') || id.includes('blinky') || id.includes('bluepill'))
    return { pcb: '#0e2a72', chip: '#091a4a', label: 'STM32F103' };
  if (id.includes('rp2040') || id.includes('pico'))
    return { pcb: '#3a1135', chip: '#1f0a1c', label: 'RP2040' };
  if (id.includes('esp32')) return { pcb: '#1a1a1a', chip: '#0f0f0f', label: 'ESP32' };
  return { pcb: '#1f2a44', chip: '#11182a', label: 'MCU' };
}

function ledState(session: ChipSession): { fill: string; opacity: number } {
  if (session.bridge) return { fill: '#33dd66', opacity: 0.95 };
  if (session.source) return { fill: '#e8c842', opacity: 0.75 };
  return { fill: '#555', opacity: 0.6 };
}

export function McuThumb({ session, width, height }: McuThumbProps) {
  const family = pickFamily(session.board.boardId);
  const led = ledState(session);
  return (
    <svg viewBox={`0 0 ${width} ${height}`} width={width} height={height} style={{ display: 'block' }}>
      <rect
        x={2}
        y={2}
        width={width - 4}
        height={height - 4}
        rx={3}
        fill={family.pcb}
        stroke="rgba(0,0,0,0.4)"
        strokeWidth={0.5}
      />
      {/* Pin headers — gold dots top + bottom */}
      {Array.from({ length: 8 }).map((_, i) => {
        const px = 8 + i * ((width - 16) / 7);
        return (
          <g key={i}>
            <rect x={px - 1} y={4} width={2} height={2.5} fill="#caa64a" />
            <rect x={px - 1} y={height - 6.5} width={2} height={2.5} fill="#caa64a" />
          </g>
        );
      })}
      {/* Chip body */}
      <rect
        x={width * 0.25}
        y={height * 0.3}
        width={width * 0.5}
        height={height * 0.42}
        rx={1.5}
        fill={family.chip}
        stroke="rgba(0,0,0,0.5)"
        strokeWidth={0.5}
      />
      {/* Status LED */}
      <circle cx={width - 6} cy={6} r={2} fill={led.fill} opacity={led.opacity}>
        {led.fill === '#33dd66' && (
          <animate attributeName="opacity" values="0.6;1;0.6" dur="1.2s" repeatCount="indefinite" />
        )}
      </circle>
    </svg>
  );
}
