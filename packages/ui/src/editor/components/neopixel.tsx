import type { ComponentDef } from '../types';

const W = 120;
const H = 36;

export const neopixelComponent: ComponentDef = {
  type: 'neopixel',
  label: 'NeoPixel Strip',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: 'DIN', x: 0, y: H / 2, side: 'left', label: 'DIN' },
    { id: 'VCC', x: W / 2 - 14, y: 0, side: 'top', label: 'VCC' },
    { id: 'GND', x: W / 2 + 14, y: 0, side: 'top', label: 'GND' },
    { id: 'DOUT', x: W, y: H / 2, side: 'right', label: 'DOUT' },
  ],
  defaultAttrs: { count: '8' },
  boardIoKind: 'spi_device',
  attrFields: [
    { key: 'count', label: 'LED Count', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = !!state?.selected;
    const active = !!state?.active;
    const count = Math.min(Math.max(parseInt((attrs.count as string) || '8', 10), 1), 8);
    const colors = ['#F2545B', '#F5B642', '#3DD68C', '#5B9DFF', '#F062B8', '#B07BFF', '#5BD8FF', '#FFE680'];
    const ledSize = (W - 16) / count;
    return (
      <g>
        <defs>
          <linearGradient id="neo-strip" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#1f1f1f" />
            <stop offset="0.5" stopColor="#0e0e0e" />
            <stop offset="1" stopColor="#000" />
          </linearGradient>
          <radialGradient id="neo-led-base" cx="0.5" cy="0.5" r="0.6">
            <stop offset="0" stopColor="#f8f8f8" />
            <stop offset="1" stopColor="#888" />
          </radialGradient>
        </defs>

        {/* Drop shadow under strip */}
        <ellipse cx={W / 2} cy={H} rx={W / 2 - 4} ry={2} fill="#000" opacity={0.4} />

        {/* Flexible PCB strip body */}
        <rect x={0} y={3} width={W} height={H - 6} rx={3} fill="url(#neo-strip)" stroke={selected ? '#F062B8' : '#000'} strokeWidth={selected ? 2.5 : 0.8} />

        {/* End connector pads (left + right) */}
        <rect x={0} y={H / 2 - 4} width={3} height={8} fill="#FFE680" stroke="#7a5a1a" strokeWidth={0.3} />
        <rect x={W - 3} y={H / 2 - 4} width={3} height={8} fill="#FFE680" stroke="#7a5a1a" strokeWidth={0.3} />

        {/* WS2812 LED packages */}
        {Array.from({ length: count }, (_, i) => {
          const cx = 8 + ledSize * (i + 0.5);
          const cy = H / 2;
          const color = colors[i % colors.length];
          return (
            <g key={i}>
              {/* LED glow when active */}
              {active && (
                <circle cx={cx} cy={cy} r={ledSize * 0.85} fill={color} opacity={0.35} />
              )}
              {/* White ceramic LED package */}
              <rect x={cx - ledSize / 2 + 1} y={cy - 6} width={ledSize - 2} height={12} rx={1} fill="url(#neo-led-base)" stroke="#444" strokeWidth={0.4} />
              {/* Inner die */}
              <rect x={cx - 3} y={cy - 3} width={6} height={6} fill={active ? color : '#1a1a1a'} opacity={active ? 1 : 0.6} />
              {/* Specular highlight when active */}
              {active && <circle cx={cx - 1} cy={cy - 1.5} r={1} fill="rgba(255,255,255,0.6)" />}
            </g>
          );
        })}

        {/* Solder dots between LEDs (visible trace pattern) */}
        {Array.from({ length: count - 1 }, (_, i) => {
          const dx = 8 + ledSize * (i + 1);
          return <circle key={i} cx={dx} cy={4} r={0.6} fill="#444" opacity={0.7} />;
        })}

        {/* Silkscreen */}
        <text x={5} y={H / 2 - 9} fill="rgba(255,255,255,0.45)" fontFamily="'JetBrains Mono', monospace" fontSize={4.5}>
          WS2812B · {count}px
        </text>

        {/* Pin labels (outside the strip) */}
        <text x={4} y={H / 2 - 7} fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={5}>DIN</text>
        <text x={W - 4} y={H / 2 - 7} textAnchor="end" fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={5}>DOUT</text>
      </g>
    );
  },
};
