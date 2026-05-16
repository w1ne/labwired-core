import type { ComponentDef } from '../types';

const W = 160;
const H = 78;

export const epdSsd1680TricolorComponent: ComponentDef = {
  type: 'ssd1680_tricolor_290',
  label: 'E-Paper 2.9" tri-color (SSD1680)',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'VCC',  x: 24,  y: H, side: 'bottom', label: 'VCC' },
    { id: 'GND',  x: 40,  y: H, side: 'bottom', label: 'GND' },
    { id: 'DIN',  x: 56,  y: H, side: 'bottom', label: 'DIN' },
    { id: 'CLK',  x: 72,  y: H, side: 'bottom', label: 'CLK' },
    { id: 'CS',   x: 88,  y: H, side: 'bottom', label: 'CS' },
    { id: 'DC',   x: 104, y: H, side: 'bottom', label: 'DC' },
    { id: 'RST',  x: 120, y: H, side: 'bottom', label: 'RST' },
    { id: 'BUSY', x: 136, y: H, side: 'bottom', label: 'BUSY' },
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
          <linearGradient id="epd-pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1a4a1a" />
            <stop offset="1" stopColor="#0e2e0e" />
          </linearGradient>
          <linearGradient id="epd-paper" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#f4f1e8" />
            <stop offset="1" stopColor="#e6e1d3" />
          </linearGradient>
          <linearGradient id="epd-pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#FFE680" />
            <stop offset="1" stopColor="#B0871A" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H + 4} rx={W / 2 - 8} ry={3.5} fill="#000" opacity={0.3} />

        {/* PCB */}
        <rect width={W} height={H} rx={3} fill="url(#epd-pcb)" stroke={selected ? '#F062B8' : '#0a1f0a'} strokeWidth={selected ? 2.5 : 1} />

        {/* Mounting holes */}
        <circle cx={4}     cy={4}     r={1.6} fill="#0a1f0a" />
        <circle cx={W - 4} cy={4}     r={1.6} fill="#0a1f0a" />
        <circle cx={4}     cy={H - 16} r={1.6} fill="#0a1f0a" />
        <circle cx={W - 4} cy={H - 16} r={1.6} fill="#0a1f0a" />

        {/* E-paper face (the visible panel) — wider than it is tall to suggest 296×128 landscape */}
        <rect x={8} y={6} width={W - 16} height={H - 30} rx={1} fill="url(#epd-paper)" stroke="#bcb6a3" strokeWidth={0.6} />

        {/* Active content hint: faint placeholder text + a single red dot for tri-color hint */}
        {active ? (
          <>
            <rect x={12} y={10} width={W - 24} height={9} fill="#1a1a1a" opacity={0.85} rx={1} />
            <text x={W / 2} y={16.5} textAnchor="middle" fill="#f4f1e8" fontFamily="'JetBrains Mono', monospace" fontSize={6} fontWeight={700} letterSpacing="0.05em">
              AGENT REQUEST
            </text>
            <text x={14} y={28} fill="#1a1a1a" fontFamily="'Outfit', sans-serif" fontSize={5.5} fontWeight={600}>
              session: cli-7f3a
            </text>
            <text x={14} y={36} fill="#1a1a1a" fontFamily="'Outfit', sans-serif" fontSize={5}>
              run npm install on host
            </text>
            <circle cx={W - 18} cy={32} r={3.5} fill="#c41e1e" />
            <text x={W - 18} y={34} textAnchor="middle" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={4.5} fontWeight={700}>!</text>
            <text x={14} y={45} fill="#666" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
              [Y] approve   [N] deny
            </text>
          </>
        ) : (
          <>
            <text x={W / 2} y={H / 2 - 9} textAnchor="middle" fill="#a8a290" fontFamily="'Outfit', sans-serif" fontSize={6} fontWeight={500} letterSpacing="0.04em">
              2.9" tri-color e-paper
            </text>
            <text x={W / 2} y={H / 2 - 1} textAnchor="middle" fill="#bcb6a3" fontFamily="'JetBrains Mono', monospace" fontSize={5}>
              296 × 128 · SSD1680
            </text>
          </>
        )}

        {/* Silkscreen */}
        <text x={W / 2} y={H - 18} textAnchor="middle" fill="rgba(180,255,180,0.5)" fontFamily="'Outfit', sans-serif" fontSize={5} fontWeight={600} letterSpacing="0.08em">
          B / W / R · SPI · 3.3V
        </text>

        {/* Bottom pin pads */}
        {[
          { x: 24,  label: 'VCC',  color: '#FF6B6B' },
          { x: 40,  label: 'GND',  color: '#aaa' },
          { x: 56,  label: 'DIN',  color: '#B07BFF' },
          { x: 72,  label: 'CLK',  color: '#5BD8FF' },
          { x: 88,  label: 'CS',   color: '#3DD68C' },
          { x: 104, label: 'DC',   color: '#5B9DFF' },
          { x: 120, label: 'RST',  color: '#F5B642' },
          { x: 136, label: 'BUSY', color: '#FFE680' },
        ].map((pad) => (
          <g key={pad.label}>
            <rect x={pad.x - 4} y={H - 12} width={8} height={10} fill="url(#epd-pad)" stroke="#7a5a1a" strokeWidth={0.3} />
            <circle cx={pad.x} cy={H - 7} r={1.4} fill="#0a1f0a" />
            <text x={pad.x} y={H - 14} textAnchor="middle" fill={pad.color} fontFamily="'JetBrains Mono', monospace" fontSize={4.5} fontWeight={600}>
              {pad.label}
            </text>
          </g>
        ))}

        {selected && (
          <rect width={W} height={H} rx={3} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
