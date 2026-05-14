import type { ComponentDef } from '../types';

const W = 60;
const H = 80;

export const dht22Component: ComponentDef = {
  type: 'dht22',
  label: 'DHT22 Sensor',
  category: 'sensor',
  width: W,
  height: H,
  pins: [
    { id: 'VCC', x: 10, y: H, side: 'bottom', label: 'VCC' },
    { id: 'DATA', x: W / 2, y: H, side: 'bottom', label: 'DATA' },
    { id: 'GND', x: W - 10, y: H, side: 'bottom', label: 'GND' },
  ],
  defaultAttrs: { temperature: '25', humidity: '50' },
  boardIoKind: 'button',
  attrFields: [
    { key: 'temperature', label: 'Temp (°C)', type: 'text' },
    { key: 'humidity', label: 'Humidity (%)', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = !!state?.selected;
    const temp = (attrs.temperature as string) || '25';
    const hum = (attrs.humidity as string) || '50';
    return (
      <g>
        <defs>
          <linearGradient id="dht-body" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#F5F5F5" />
            <stop offset="1" stopColor="#CCCCCC" />
          </linearGradient>
        </defs>

        {/* Drop shadow */}
        <ellipse cx={W / 2} cy={H - 4} rx={W / 2 - 6} ry={3} fill="#000" opacity={0.35} />

        {/* Plastic housing */}
        <rect x={4} y={2} width={W - 8} height={H - 12} rx={4} fill="url(#dht-body)" stroke={selected ? '#F062B8' : '#888'} strokeWidth={selected ? 2.5 : 1} />

        {/* Vent grille for humidity sensing */}
        {[18, 26, 34, 42, 50].map((y) => (
          <rect key={y} x={12} y={y} width={W - 24} height={2} rx={1} fill="#bbb" />
        ))}

        {/* DHT22 silkscreen */}
        <text x={W / 2} y={14} textAnchor="middle" fill="#1a1a1a" fontFamily="'Outfit', sans-serif" fontSize={8} fontWeight={700} letterSpacing="0.08em">
          DHT22
        </text>
        <text x={W / 2} y={62} textAnchor="middle" fill="#1a1a1a" fontFamily="'JetBrains Mono', monospace" fontSize={4.5} opacity={0.7}>
          AM2302
        </text>

        {/* Status readouts (live values overlay) */}
        <text x={W / 2} y={H - 18} textAnchor="middle" fill="#F2545B" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>
          {temp}°C
        </text>
        <text x={W / 2} y={H - 10} textAnchor="middle" fill="#5B9DFF" fontFamily="'JetBrains Mono', monospace" fontSize={7} fontWeight={600}>
          {hum}%
        </text>

        {/* Pin leads */}
        <line x1={10} y1={H - 12} x2={10} y2={H - 1} stroke="#aaa" strokeWidth={1.5} />
        <line x1={W / 2} y1={H - 12} x2={W / 2} y2={H - 1} stroke="#aaa" strokeWidth={1.5} />
        <line x1={W - 10} y1={H - 12} x2={W - 10} y2={H - 1} stroke="#aaa" strokeWidth={1.5} />
      </g>
    );
  },
};
