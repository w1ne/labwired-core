import type { ComponentDef } from '../types';

const W = 140;
const H = 104;

export const ili9341TftComponent: ComponentDef = {
  type: 'ili9341',
  label: 'ILI9341 TFT 240x320',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'VCC',   x: 16,  y: H, side: 'bottom', label: 'VCC' },
    { id: 'GND',   x: 36,  y: H, side: 'bottom', label: 'GND' },
    { id: 'CS',    x: 56,  y: H, side: 'bottom', label: 'CS' },
    { id: 'RESET', x: 76,  y: H, side: 'bottom', label: 'RST' },
    { id: 'DC',    x: 96,  y: H, side: 'bottom', label: 'DC' },
    { id: 'MOSI',  x: 116, y: H, side: 'bottom', label: 'MOSI' },
    { id: 'SCK',   x: 124, y: H, side: 'bottom', label: 'SCK' },
    { id: 'LED',   x: 132, y: H, side: 'bottom', label: 'LED' },
  ],
  defaultAttrs: {},
  boardIoKind: 'spi_device',
  attrFields: [],
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const active = !!state?.active;

    return (
      <g>
        <defs>
          <linearGradient id="tft-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#B03010" />
            <stop offset="1" stopColor="#7A1F05" />
          </linearGradient>
          <linearGradient id="tft-screen-bg" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#0a0a18" />
            <stop offset="1" stopColor="#040408" />
          </linearGradient>
          <linearGradient id="tft-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 8} ry={4} fill="#000" opacity={0.35} />

        {/* PCB body */}
        <rect width={W} height={H} rx={4} fill="url(#tft-pcb)" stroke={selected ? '#F062B8' : '#4a1005'} strokeWidth={selected ? 2.5 : 1} />

        {/* Subtle texture overlay */}
        <rect width={W} height={H} rx={4} fill="none" stroke="#ffffff" strokeWidth={0.3} opacity={0.08} />

        {/* Mounting holes */}
        <circle cx={6}     cy={6}     r={2} fill="#4a1005" stroke="#2a0a00" strokeWidth={0.5} />
        <circle cx={W - 6} cy={6}     r={2} fill="#4a1005" stroke="#2a0a00" strokeWidth={0.5} />
        <circle cx={6}     cy={H - 18} r={2} fill="#4a1005" stroke="#2a0a00" strokeWidth={0.5} />
        <circle cx={W - 6} cy={H - 18} r={2} fill="#4a1005" stroke="#2a0a00" strokeWidth={0.5} />

        {/* Display glass bezel */}
        <rect x={10} y={8} width={W - 20} height={H - 32} rx={2} fill="#111" stroke="#2a2a3a" strokeWidth={1} />

        {/* Active display area */}
        <rect x={12} y={10} width={W - 24} height={H - 36} rx={1} fill="url(#tft-screen-bg)" />

        {/* Screen content */}
        {active ? (
          <>
            {/* Simulate color bars when display is on */}
            {[0, 1, 2, 3, 4, 5, 6, 7].map((i) => {
              const cssColors = ['#F00', '#0F0', '#00F', '#FF0', '#F0F', '#0FF', '#FFF', '#888'];
              const bw = (W - 24) / 8;
              return (
                <rect
                  key={i}
                  x={12 + i * bw}
                  y={10}
                  width={bw}
                  height={H - 36}
                  fill={cssColors[i]}
                  opacity={0.7}
                />
              );
            })}
            <text x={W / 2} y={H / 2 - 4} textAnchor="middle" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={600} style={{ mixBlendMode: 'difference' }}>
              240×320
            </text>
          </>
        ) : (
          <text x={W / 2} y={H / 2 - 8} textAnchor="middle" fill="#1a1a3a" fontFamily="'JetBrains Mono', monospace" fontSize={8}>
            240×320 TFT
          </text>
        )}

        {/* Silkscreen label */}
        <text x={W / 2} y={H - 18} textAnchor="middle" fill="rgba(255,220,180,0.55)" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={600} letterSpacing="0.06em">
          ILI9341 · RGB565 · SPI
        </text>

        {/* Bottom pin pads */}
        {[
          { x: 16,  label: 'VCC',  color: '#FF6B6B' },
          { x: 36,  label: 'GND',  color: '#aaa' },
          { x: 56,  label: 'CS',   color: '#3DD68C' },
          { x: 76,  label: 'RST',  color: '#F5B642' },
          { x: 96,  label: 'DC',   color: '#5B9DFF' },
          { x: 116, label: 'MOSI', color: '#B07BFF' },
          { x: 124, label: 'SCK',  color: '#5BD8FF' },
          { x: 132, label: 'LED',  color: '#FFE680' },
        ].map((pad) => (
          <g key={pad.label}>
            <rect x={pad.x - 4} y={H - 12} width={8} height={10} fill="url(#tft-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={pad.x} cy={H - 7} r={1.5} fill="#4a1005" />
            <text x={pad.x} y={H - 14} textAnchor="middle" fill={pad.color} fontFamily="'JetBrains Mono', monospace" fontSize={5} fontWeight={600}>
              {pad.label}
            </text>
          </g>
        ))}

        {/* Selection highlight */}
        {selected && (
          <rect width={W} height={H} rx={4} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
