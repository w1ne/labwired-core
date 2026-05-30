import type { ComponentDef } from '../types';

const W = 124;
const H = 96;

/** Nokia 5110 (PCD8544) 84×48 monochrome SPI LCD module. */
export const pcd8544Component: ComponentDef = {
  type: 'pcd8544',
  label: 'Nokia 5110',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'RST', x: 12, y: H, side: 'bottom', label: 'RST' },
    { id: 'CE', x: 28, y: H, side: 'bottom', label: 'CE' },
    { id: 'DC', x: 44, y: H, side: 'bottom', label: 'DC' },
    { id: 'DIN', x: 60, y: H, side: 'bottom', label: 'DIN' },
    { id: 'CLK', x: 76, y: H, side: 'bottom', label: 'CLK' },
    { id: 'VCC', x: 96, y: H, side: 'bottom', label: 'VCC' },
    { id: 'GND', x: 112, y: H, side: 'bottom', label: 'GND' },
  ],
  defaultAttrs: {},
  boardIoKind: 'spi_device',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const active = !!state?.active;
    return (
      <g>
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 8} ry={4} fill="#000" opacity={0.4} />
        {/* PCB */}
        <rect
          width={W}
          height={H}
          rx={5}
          fill="#27506b"
          stroke={selected ? '#F062B8' : '#0a1a22'}
          strokeWidth={selected ? 2.5 : 1}
        />
        {/* Greenish LCD glass */}
        <rect x={12} y={12} width={W - 24} height={H - 40} rx={2} fill={active ? '#c2d3a6' : '#9fb288'} stroke="#3a4a2a" strokeWidth={1.5} />
        {active ? (
          <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#2e3a26" fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={600}>
            84 × 48
          </text>
        ) : (
          <text x={W / 2} y={H / 2 - 6} textAnchor="middle" fill="#52613f" fontFamily="'JetBrains Mono', monospace" fontSize={8}>
            NOKIA 5110
          </text>
        )}
        {/* Silkscreen */}
        <text x={W / 2} y={H - 16} textAnchor="middle" fill="rgba(255,255,255,0.55)" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={600} letterSpacing="0.05em">
          PCD8544 · SPI
        </text>
        {/* Header pads */}
        {[
          { x: 12 }, { x: 28 }, { x: 44 }, { x: 60 }, { x: 76 }, { x: 96 }, { x: 112 },
        ].map((p, i) => (
          <rect key={i} x={p.x - 3} y={H - 11} width={6} height={9} fill="#d9b24a" stroke="#7a5a1a" strokeWidth={0.3} />
        ))}
        {selected && (
          <rect width={W} height={H} rx={5} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
