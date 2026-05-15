import type { ComponentDef } from '../types';

// MAX31855 breakout board SVG component
// Typical PCB: ~64×56 px, green PCB, IC + screw terminal for K-type thermocouple
// Pins on right side: VCC, GND, CS, SCK, DO
const W = 96;
const H = 80;

export const max31855Component: ComponentDef = {
  type: 'max31855',
  label: 'MAX31855',
  category: 'sensor',
  width: W,
  height: H,
  boardIoKind: 'spi_device',
  pins: [
    { id: 'VCC', x: W, y: 12, side: 'right', label: 'VCC' },
    { id: 'GND', x: W, y: 26, side: 'right', label: 'GND' },
    { id: 'CS',  x: W, y: 40, side: 'right', label: 'CS' },
    { id: 'SCK', x: W, y: 54, side: 'right', label: 'SCK' },
    { id: 'DO',  x: W, y: 68, side: 'right', label: 'DO' },
    // Screw terminal on left for K-type thermocouple wires
    { id: 'TC+', x: 0, y: 30, side: 'left', label: 'TC+' },
    { id: 'TC-', x: 0, y: 50, side: 'left', label: 'TC-' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    return (
      <g>
        <defs>
          <linearGradient id="max31855-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2E7D32" />
            <stop offset="1" stopColor="#1B5E20" />
          </linearGradient>
          <pattern id="max31855-dots" x="0" y="0" width="4" height="4" patternUnits="userSpaceOnUse">
            <circle cx={2} cy={2} r={0.3} fill="#0a2d0c" opacity={0.6} />
          </pattern>
          <linearGradient id="max31855-chip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2a2a2a" />
            <stop offset="1" stopColor="#0a0a0a" />
          </linearGradient>
          <linearGradient id="max31855-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 4} ry={3} fill="#000" opacity={0.35} />

        {/* PCB body */}
        <rect
          width={W}
          height={H}
          rx={5}
          fill="url(#max31855-pcb)"
          stroke={selected ? '#F5B642' : '#0d3d0f'}
          strokeWidth={selected ? 2.5 : 1.2}
        />
        <rect width={W} height={H} rx={5} fill="url(#max31855-dots)" opacity={0.5} />

        {/* IC chip — MAX31855 in SOIC-8 or similar */}
        <rect x={28} y={H / 2 - 12} width={28} height={24} rx={2} fill="url(#max31855-chip)" stroke="#000" strokeWidth={0.8} />
        {/* Pin 1 marker */}
        <circle cx={31} cy={H / 2 - 9} r={1.2} fill="#555" />
        <text x={42} y={H / 2 - 2} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={4} fontWeight={600}>
          MAX
        </text>
        <text x={42} y={H / 2 + 4} textAnchor="middle" fill="#bbb" fontFamily="'JetBrains Mono', monospace" fontSize={4} fontWeight={600}>
          31855
        </text>

        {/* Screw terminal block on left for thermocouple */}
        <rect x={2} y={20} width={14} height={40} rx={2} fill="#888" stroke="#444" strokeWidth={0.5} />
        <rect x={4} y={24} width={10} height={14} rx={1} fill="#555" stroke="#333" strokeWidth={0.4} />
        <rect x={4} y={42} width={10} height={14} rx={1} fill="#555" stroke="#333" strokeWidth={0.4} />
        {/* Screw heads */}
        <circle cx={9} cy={31} r={3.5} fill="#777" stroke="#333" strokeWidth={0.4} />
        <line x1={6} y1={31} x2={12} y2={31} stroke="#333" strokeWidth={0.8} />
        <circle cx={9} cy={49} r={3.5} fill="#777" stroke="#333" strokeWidth={0.4} />
        <line x1={6} y1={49} x2={12} y2={49} stroke="#333" strokeWidth={0.8} />

        {/* Silkscreen title */}
        <text x={W / 2 + 6} y={10} textAnchor="middle" fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={6.5} fontWeight={700} letterSpacing="0.03em">
          MAX31855
        </text>
        <text x={W / 2 + 6} y={H - 4} textAnchor="middle" fill="rgba(255,255,255,0.55)" fontFamily="'JetBrains Mono', monospace" fontSize={4}>
          K-TYPE · SPI
        </text>

        {/* Right pads — VCC, GND, CS, SCK, DO */}
        {[
          { y: 8,  label: 'VCC' },
          { y: 22, label: 'GND' },
          { y: 36, label: 'CS' },
          { y: 50, label: 'SCK' },
          { y: 64, label: 'DO' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={W - 6} y={y} width={9} height={8} fill="url(#max31855-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={W - 2} cy={y + 4} r={1.5} fill="#0a0a0a" />
            <text x={W - 10} y={y + 6} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={5.5} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {/* Left pads — TC+, TC- */}
        {[
          { y: 26, label: 'TC+' },
          { y: 46, label: 'TC-' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={-3} y={y} width={9} height={8} fill="url(#max31855-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={2} cy={y + 4} r={1.5} fill="#0a0a0a" />
            <text x={10} y={y + 6} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={5.5} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {/* Thermocouple wire graphic hint */}
        <line x1={0} y1={30} x2={-6} y2={30} stroke="#F5B642" strokeWidth={1.2} strokeDasharray="2,1" />
        <line x1={0} y1={50} x2={-6} y2={50} stroke="#999" strokeWidth={1.2} strokeDasharray="2,1" />

        {/* Selection highlight */}
        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#F5B642" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
