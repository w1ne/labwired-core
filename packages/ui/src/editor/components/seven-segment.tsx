import type { ComponentDef } from '../types';

const W = 80;
const H = 110;

export const sevenSegmentComponent: ComponentDef = {
  type: 'seven-segment',
  label: '7-Segment',
  category: 'display',
  width: W,
  height: H,
  pins: [
    { id: 'A', x: 0, y: 14, side: 'left', label: 'A' },
    { id: 'B', x: 0, y: 30, side: 'left', label: 'B' },
    { id: 'C', x: 0, y: 46, side: 'left', label: 'C' },
    { id: 'D', x: 0, y: 62, side: 'left', label: 'D' },
    { id: 'E', x: W, y: 14, side: 'right', label: 'E' },
    { id: 'F', x: W, y: 30, side: 'right', label: 'F' },
    { id: 'G', x: W, y: 46, side: 'right', label: 'G' },
    { id: 'DP', x: W, y: 62, side: 'right', label: 'DP' },
    { id: 'COM', x: W / 2, y: H, side: 'bottom', label: 'COM' },
  ],
  defaultAttrs: { color: 'red' },
  boardIoKind: 'spi_device',
  attrFields: [
    { key: 'color', label: 'Color', type: 'select', options: ['red', 'green', 'blue', 'yellow'] },
  ],
  render: (attrs, state) => {
    const selected = state?.selected;
    const color = attrs.color || 'red';
    const litColor = { red: '#ff3333', green: '#27c93f', blue: '#3399ff', yellow: '#ffcc00' }[color] || '#ff3333';
    const dimColor = '#331111';
    const sx = 20, sy = 14, sw = 32, sh = 5, sv = 28;

    // Segment map: which segments (a-g) are on for each digit
    //   a = top, b = top-right, c = bottom-right, d = bottom,
    //   e = bottom-left, f = top-left, g = middle
    const digitSegments: Record<string, boolean[]> = {
      '0': [true,  true,  true,  true,  true,  true,  false],
      '1': [false, true,  true,  false, false, false, false],
      '2': [true,  true,  false, true,  true,  false, true],
      '3': [true,  true,  true,  true,  false, false, true],
      '4': [false, true,  true,  false, false, true,  true],
      '5': [true,  false, true,  true,  false, true,  true],
      '6': [true,  false, true,  true,  true,  true,  true],
      '7': [true,  true,  true,  false, false, false, false],
      '8': [true,  true,  true,  true,  true,  true,  true],
      '9': [true,  true,  true,  true,  false, true,  true],
    };

    const displayChar = state?.displayText?.[0];
    // If we have a recognized digit, use its segments; otherwise show all dim ('8')
    const segs = (displayChar && digitSegments[displayChar]) || null;
    const seg = (index: number) =>
      segs ? (segs[index] ? litColor : dimColor) : dimColor;

    return (
      <g>
        <rect x={3} y={3} width={W - 6} height={H - 6} rx={5}
          fill="#1a1a1a" stroke={selected ? '#e83e8c' : '#333'} strokeWidth={selected ? 2.5 : 1.5} />
        {/* a — top */}
        <rect x={sx} y={sy} width={sw} height={sh} rx={1} fill={seg(0)} />
        {/* b — top-right */}
        <rect x={sx + sw - sh} y={sy} width={sh} height={sv} rx={1} fill={seg(1)} />
        {/* c — bottom-right */}
        <rect x={sx + sw - sh} y={sy + sv} width={sh} height={sv} rx={1} fill={seg(2)} />
        {/* d — bottom */}
        <rect x={sx} y={sy + sv * 2 - sh} width={sw} height={sh} rx={1} fill={seg(3)} />
        {/* e — bottom-left */}
        <rect x={sx} y={sy + sv} width={sh} height={sv} rx={1} fill={seg(4)} />
        {/* f — top-left */}
        <rect x={sx} y={sy} width={sh} height={sv} rx={1} fill={seg(5)} />
        {/* g — middle */}
        <rect x={sx} y={sy + sv - sh / 2} width={sw} height={sh} rx={1} fill={seg(6)} />
        {/* Decimal point */}
        <circle cx={sx + sw + 8} cy={sy + sv * 2 - sh} r={3} fill={dimColor} />
        {/* Display text label */}
        <text x={W / 2} y={H - 10} textAnchor="middle" fill={segs ? litColor : '#666'}
          fontFamily="monospace" fontSize={8}>{displayChar ?? '7-SEG'}</text>
      </g>
    );
  },
};
