import type { ComponentDef } from '../types';

const W = 44;
const H = 60;

export const dht22Component: ComponentDef = {
  type: 'dht22',
  label: 'DHT22 Sensor',
  category: 'sensor',
  width: W,
  height: H,
  pins: [
    { id: 'VCC', x: 6, y: H, side: 'bottom', label: 'VCC' },
    { id: 'DATA', x: W / 2, y: H, side: 'bottom', label: 'DATA' },
    { id: 'GND', x: W - 6, y: H, side: 'bottom', label: 'GND' },
  ],
  defaultAttrs: { temperature: '25', humidity: '50' },
  boardIoKind: 'button',
  attrFields: [
    { key: 'temperature', label: 'Temp (°C)', type: 'text' },
    { key: 'humidity', label: 'Humidity (%)', type: 'text' },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const temp = attrs.temperature || '25';
    const hum = attrs.humidity || '50';
    return (
      <g>
        <rect x={2} y={2} width={W - 4} height={H - 8} rx={4}
          fill="#f8f8f8" stroke={selected ? '#e83e8c' : '#ccc'} strokeWidth={selected ? 2.5 : 1.5} />
        {[14, 20, 26, 32].map((y) => (
          <line key={y} x1={10} y1={y} x2={W - 10} y2={y} stroke="#ddd" strokeWidth={0.5} />
        ))}
        <text x={W / 2} y={12} textAnchor="middle" fill="#333"
          fontFamily="monospace" fontSize={7} fontWeight={700}>DHT22</text>
        <text x={W / 2} y={38} textAnchor="middle" fill="#666"
          fontFamily="monospace" fontSize={8}>{temp}°C</text>
        <text x={W / 2} y={48} textAnchor="middle" fill="#569cd6"
          fontFamily="monospace" fontSize={7}>{hum}%</text>
      </g>
    );
  },
};
