import type { ComponentDef } from '../types';

const W = 60;
const H = 80;

interface LedColor {
  body: string;
  bodyDim: string;
  glow: string;
  highlight: string;
}

const LED_COLORS: Record<string, LedColor> = {
  red: { body: '#F2545B', bodyDim: '#7a2226', glow: 'rgba(242,84,91,0.65)', highlight: '#FF9FA3' },
  green: { body: '#3DD68C', bodyDim: '#19663F', glow: 'rgba(61,214,140,0.65)', highlight: '#A8F0CB' },
  blue: { body: '#5B9DFF', bodyDim: '#24487A', glow: 'rgba(91,157,255,0.65)', highlight: '#B8D6FF' },
  yellow: { body: '#F5B642', bodyDim: '#7a5a18', glow: 'rgba(245,182,66,0.65)', highlight: '#FDE3A8' },
  white: { body: '#F2F4F9', bodyDim: '#777a85', glow: 'rgba(242,244,249,0.55)', highlight: '#FFFFFF' },
};

export const ledComponent: ComponentDef = {
  type: 'led',
  label: 'LED',
  category: 'output',
  width: W,
  height: H,
  pins: [
    { id: 'A', x: W / 2, y: 0, side: 'top', label: 'A' },
    { id: 'C', x: W / 2, y: H, side: 'bottom', label: 'C' },
  ],
  defaultAttrs: { color: 'red' },
  boardIoKind: 'led',
  attrFields: [
    {
      key: 'color',
      label: 'Color',
      type: 'select',
      options: ['red', 'green', 'blue', 'yellow', 'white'],
    },
  ],
  render: (attrs, state) => {
    const colorKey = (attrs.color as string) || 'red';
    const c = LED_COLORS[colorKey] ?? LED_COLORS.red;
    const active = !!state?.active;
    const selected = !!state?.selected;
    const cx = W / 2;
    const cy = H / 2;
    const r = 16;
    const gradId = `led-grad-${colorKey}`;
    const glowId = `led-glow-${colorKey}`;

    return (
      <g>
        <defs>
          <radialGradient id={gradId} cx="0.35" cy="0.3" r="0.8">
            <stop offset="0" stopColor={c.highlight} stopOpacity={active ? 1 : 0.7} />
            <stop offset="0.4" stopColor={active ? c.body : c.bodyDim} />
            <stop offset="1" stopColor={active ? c.bodyDim : '#1a1a1a'} />
          </radialGradient>
          <radialGradient id={glowId} cx="0.5" cy="0.5" r="0.5">
            <stop offset="0" stopColor={c.body} stopOpacity={0.55} />
            <stop offset="1" stopColor={c.body} stopOpacity={0} />
          </radialGradient>
        </defs>

        {/* Glow halo when active */}
        {active && <circle cx={cx} cy={cy} r={30} fill={`url(#${glowId})`} />}

        {/* Anode lead (top) */}
        <line x1={cx} y1={4} x2={cx} y2={cy - r + 2} stroke="#b0b0b0" strokeWidth={2} strokeLinecap="round" />
        <line x1={cx + 0.5} y1={4} x2={cx + 0.5} y2={cy - r + 2} stroke="#666" strokeWidth={0.5} />

        {/* Cathode lead (bottom) — slightly thicker/darker by convention */}
        <line x1={cx} y1={cy + r - 2} x2={cx} y2={H - 4} stroke="#888" strokeWidth={2.2} strokeLinecap="round" />
        <line x1={cx + 0.5} y1={cy + r - 2} x2={cx + 0.5} y2={H - 4} stroke="#555" strokeWidth={0.5} />

        {/* LED body — round translucent dome */}
        <circle cx={cx} cy={cy} r={r} fill={`url(#${gradId})`} stroke="#0a0a0a" strokeWidth={1.2} />

        {/* Internal die / chip visible through the dome */}
        <rect x={cx - 3} y={cy + 2} width={6} height={4} fill={active ? c.highlight : '#5a5a5a'} opacity={active ? 0.95 : 0.5} />

        {/* Bond wire (very thin line from die toward anode) */}
        <line x1={cx} y1={cy + 3} x2={cx - 6} y2={cy - 4} stroke={active ? c.highlight : '#888'} strokeWidth={0.6} opacity={0.7} />

        {/* Specular highlight on the dome */}
        <ellipse cx={cx - 5} cy={cy - 6} rx={4.5} ry={3} fill="rgba(255,255,255,0.55)" />
        <circle cx={cx - 7} cy={cy - 8} r={1.5} fill="rgba(255,255,255,0.85)" />

        {/* Pin labels */}
        <text x={cx - 8} y={9} textAnchor="middle" fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={7}>A</text>
        <text x={cx - 8} y={H - 2} textAnchor="middle" fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={7}>C</text>

        {/* Component label below */}
        <text x={cx} y={H + 12} textAnchor="middle" fill={active ? '#3DD68C' : '#5A6178'} fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={active ? 600 : 400}>
          {active ? 'ON' : 'OFF'}
        </text>

        {/* Selection ring */}
        {selected && (
          <circle cx={cx} cy={cy} r={r + 4} fill="none" stroke="#F062B8" strokeWidth={2} opacity={0.85} />
        )}
      </g>
    );
  },
};
