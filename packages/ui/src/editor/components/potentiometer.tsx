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
    const selected = state?.selected;
    const value = attrs.value || '10K';
    const cx = W / 2;
    const cy = H / 2 + 4;
    const analogRaw = state?.analogValue ?? 2048;
    const hasValue = state?.analogValue !== undefined;
    // Map 0..4095 → -135°..+135°
    const angleDeg = (analogRaw / 4095) * 270 - 135;
    const pct = Math.round((analogRaw / 4095) * 100);
    return (
      <g>
        <circle cx={cx} cy={cy} r={28}
          fill="#f8f9fa" stroke={selected ? '#e83e8c' : '#000'} strokeWidth={selected ? 2.5 : 1.5} />
        {/* Tick marks at min/max/center */}
        {[-135, 0, 135].map((tick) => {
          const rad = (tick - 90) * Math.PI / 180;
          const ix = cx + 24 * Math.cos(rad);
          const iy = cy + 24 * Math.sin(rad);
          const ox = cx + 28 * Math.cos(rad);
          const oy = cy + 28 * Math.sin(rad);
          return <line key={tick} x1={ix} y1={iy} x2={ox} y2={oy} stroke="#ccc" strokeWidth={1} />;
        })}
        {/* Rotated arm + arrow */}
        <g transform={`rotate(${angleDeg}, ${cx}, ${cy})`}>
          <line x1={cx} y1={10} x2={cx} y2={cy}
            stroke="#e83e8c" strokeWidth={2.5} />
          <polygon points={`${cx},10 ${cx - 5},20 ${cx + 5},20`}
            fill="#e83e8c" />
        </g>
        {/* Pin labels */}
        <text x={6} y={H} fill="#888" fontFamily="monospace" fontSize={8}>1</text>
        <text x={cx} y={H} textAnchor="middle" fill="#888" fontFamily="monospace" fontSize={8}>W</text>
        <text x={W - 6} y={H} textAnchor="end" fill="#888" fontFamily="monospace" fontSize={8}>2</text>
        {/* Resistance label */}
        <text x={cx} y={cy + 6} textAnchor="middle" fill="#444"
          fontFamily="'JetBrains Mono', monospace" fontSize={9}>{value}</text>
        {/* Analog value + percentage */}
        {hasValue ? (
          <>
            <text x={cx} y={H + 12} textAnchor="middle" fill="#3399ff"
              fontFamily="monospace" fontSize={9}>{analogRaw}</text>
            <text x={cx} y={H + 22} textAnchor="middle" fill="#3399ff"
              fontFamily="monospace" fontSize={8}>{pct}%</text>
          </>
        ) : (
          <text x={cx} y={H + 14} textAnchor="middle" fill="#aaa"
            fontFamily="monospace" fontSize={7}>&#x1F5B1; scroll</text>
        )}
      </g>
    );
  },
};
