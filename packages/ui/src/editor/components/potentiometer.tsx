import type { ComponentDef } from '../types';

const W = 72;
const H = 72;

export const potentiometerComponent: ComponentDef = {
  type: 'potentiometer',
  label: 'Potentiometer',
  category: 'input',
  width: W,
  height: H,
  pins: [
    { id: '1', x: 0, y: H - 12, side: 'left', label: '1' },
    { id: 'W', x: W / 2, y: 0, side: 'top', label: 'W' },
    { id: '2', x: W, y: H - 12, side: 'right', label: '2' },
  ],
  defaultAttrs: { value: '10K' },
  boardIoKind: 'adc_input',
  attrFields: [
    { key: 'value', label: 'Resistance', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = !!state?.selected;
    const value = (attrs.value as string) || '10K';
    const cx = W / 2;
    const cy = H / 2 + 4;
    const analogRaw = (state?.analogValue as number) ?? 2048;
    const hasValue = state?.analogValue !== undefined;
    const angleDeg = (analogRaw / 4095) * 270 - 135;
    const pct = Math.round((analogRaw / 4095) * 100);
    return (
      <g>
        <defs>
          <radialGradient id="pot-knob" cx="0.35" cy="0.3" r="0.85">
            <stop offset="0" stopColor="#F2F4F9" />
            <stop offset="0.5" stopColor="#A8AEBE" />
            <stop offset="1" stopColor="#3a3f4a" />
          </radialGradient>
          <radialGradient id="pot-base" cx="0.5" cy="0.3" r="0.7">
            <stop offset="0" stopColor="#888" />
            <stop offset="1" stopColor="#3a3a3a" />
          </radialGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={cx} cy={cy + 28} rx={26} ry={4} fill="#000" opacity={0.4} />

        {/* Metal base ring */}
        <circle cx={cx} cy={cy} r={30} fill="url(#pot-base)" stroke="#1a1a1a" strokeWidth={1} />
        <circle cx={cx} cy={cy} r={30} fill="none" stroke="#fff" strokeWidth={0.4} opacity={0.2} />

        {/* Tick marks (every 30deg from -135 to 135) */}
        {[-135, -90, -45, 0, 45, 90, 135].map((tick) => {
          const rad = ((tick - 90) * Math.PI) / 180;
          const ix = cx + 26 * Math.cos(rad);
          const iy = cy + 26 * Math.sin(rad);
          const ox = cx + 30 * Math.cos(rad);
          const oy = cy + 30 * Math.sin(rad);
          const major = tick % 90 === 0;
          return (
            <line
              key={tick}
              x1={ix}
              y1={iy}
              x2={ox}
              y2={oy}
              stroke={major ? '#F2F4F9' : '#9098a8'}
              strokeWidth={major ? 1.2 : 0.6}
              opacity={0.6}
            />
          );
        })}

        {/* Knob — cylindrical with rotation */}
        <g transform={`rotate(${angleDeg}, ${cx}, ${cy})`}>
          <circle cx={cx} cy={cy} r={22} fill="url(#pot-knob)" stroke="#1a1a1a" strokeWidth={0.8} />
          {/* Knurled-edge hint — small dashes around perimeter */}
          {Array.from({ length: 24 }, (_, i) => {
            const a = (i / 24) * Math.PI * 2;
            const r1 = 19, r2 = 22;
            return (
              <line
                key={i}
                x1={cx + r1 * Math.cos(a)}
                y1={cy + r1 * Math.sin(a)}
                x2={cx + r2 * Math.cos(a)}
                y2={cy + r2 * Math.sin(a)}
                stroke="#3a3f4a"
                strokeWidth={0.5}
                opacity={0.4}
              />
            );
          })}
          {/* Inner indicator notch */}
          <rect x={cx - 1.5} y={cy - 20} width={3} height={10} rx={1} fill="#F062B8" />
          {/* Knob top specular */}
          <ellipse cx={cx - 6} cy={cy - 8} rx={8} ry={5} fill="rgba(255,255,255,0.35)" />
        </g>

        {/* Center cap */}
        <circle cx={cx} cy={cy} r={5} fill="#1a1a1a" stroke="#0a0a0a" strokeWidth={0.5} />
        <circle cx={cx - 1.5} cy={cy - 1.5} r={1.5} fill="rgba(255,255,255,0.25)" />

        {/* Pin labels */}
        <text x={8} y={H - 10} fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={7}>1</text>
        <text x={cx} y={9} textAnchor="middle" fill="#F062B8" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>W</text>
        <text x={W - 8} y={H - 10} textAnchor="end" fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={7}>2</text>

        {/* Resistance + value labels */}
        <text x={cx} y={H + 12} textAnchor="middle" fill="#9098a8" fontFamily="'JetBrains Mono', monospace" fontSize={8}>
          {value}
        </text>
        {hasValue && (
          <text x={cx} y={H + 22} textAnchor="middle" fill="#5B9DFF" fontFamily="'JetBrains Mono', monospace" fontSize={8} fontWeight={600}>
            {pct}% · {analogRaw}
          </text>
        )}

        {/* Selection */}
        {selected && (
          <circle cx={cx} cy={cy} r={34} fill="none" stroke="#F062B8" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
