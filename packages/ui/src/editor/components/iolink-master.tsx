import type { ComponentDef } from '../types';

// IO-Link master peer — drives the IO-Link device firmware over the UART line
// and displays the cyclic process data it reads back. Attaches as a UART device.
// Live state: `state.displayText` = link state ("OPERATE"/"STARTUP"),
// `state.analogValue` = latest process-data input byte.
const W = 120;
const H = 92;

export const iolinkMasterComponent: ComponentDef = {
  type: 'iolink-master',
  label: 'IO-Link Master',
  category: 'ic',
  width: W,
  height: H,
  // UART peer — typed loosely like neo6m-gps (not i2c_device / spi_device).
  boardIoKind: 'uart_device' as never,
  pins: [
    { id: 'TX', x: 0, y: 30, side: 'left', label: 'TX' },
    { id: 'RX', x: 0, y: 50, side: 'left', label: 'RX' },
    { id: 'L+', x: 0, y: 70, side: 'left', label: 'L+' },
  ],
  defaultAttrs: {},
  render: (_attrs, state) => {
    const selected = !!state?.selected;
    const uid = state?.id ?? 'iolm';
    const link = (state?.displayText ?? '').toUpperCase();
    const operate = link.includes('OPERATE');
    const pd = Math.max(0, Math.min(255, Math.round(state?.analogValue ?? 0)));
    const accent = operate ? '#37d67a' : '#6b7280';
    return (
      <g>
        <defs>
          <linearGradient id={`iolm-body-${uid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#2b3b52" />
            <stop offset="1" stopColor="#16202e" />
          </linearGradient>
        </defs>

        <ellipse cx={W / 2} cy={H + 2} rx={W / 2 - 6} ry={3} fill="#000" opacity={0.35} />

        {/* Module housing */}
        <rect
          width={W}
          height={H}
          rx={7}
          fill={`url(#iolm-body-${uid})`}
          stroke={selected ? '#F5B642' : '#0c1622'}
          strokeWidth={selected ? 2.5 : 1.2}
        />

        {/* Title */}
        <text x={14} y={18} fill="#fff" fontFamily="'Outfit', sans-serif" fontSize={9} fontWeight={700} letterSpacing="0.03em">
          IO-Link
        </text>
        <text x={14} y={29} fill="rgba(255,255,255,0.55)" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          MASTER
        </text>

        {/* M12 port (right) */}
        <circle cx={W - 22} cy={26} r={13} fill="#0a121c" stroke="#3a4a5e" strokeWidth={1.5} />
        <circle cx={W - 22} cy={26} r={8} fill="#1a2636" stroke="#2a3a4e" strokeWidth={0.8} />
        {[
          [0, -4],
          [-3.5, 2],
          [3.5, 2],
        ].map(([dx, dy], i) => (
          <circle key={i} cx={W - 22 + dx} cy={26 + dy} r={1.4} fill="#c9a227" />
        ))}

        {/* Link-state lamp + label */}
        <circle cx={18} cy={H - 30} r={4.5} fill={accent} stroke="#0a0a0a" strokeWidth={0.6} />
        {operate && <circle cx={18} cy={H - 30} r={4.5} fill={accent} opacity={0.5} />}
        <text x={28} y={H - 27} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6.5} fontWeight={600}>
          {link || 'OFFLINE'}
        </text>

        {/* Process-data readout: 8 bits + hex */}
        <text x={14} y={H - 12} fill="rgba(255,255,255,0.6)" fontFamily="'JetBrains Mono', monospace" fontSize={5.5}>
          PD
        </text>
        {Array.from({ length: 8 }, (_, i) => {
          const bit = 7 - i; // MSB first
          const high = (pd & (1 << bit)) !== 0;
          return (
            <rect
              key={i}
              x={28 + i * 8}
              y={H - 18}
              width={6}
              height={7}
              rx={1}
              fill={high ? accent : '#243040'}
              stroke={high ? '#9affc4' : '#11202e'}
              strokeWidth={0.5}
            />
          );
        })}
        <text x={W - 8} y={H - 12} textAnchor="end" fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={6}>
          0x{pd.toString(16).toUpperCase().padStart(2, '0')}
        </text>

        {/* Left pads */}
        {[
          { y: 26, label: 'TX' },
          { y: 46, label: 'RX' },
          { y: 66, label: 'L+' },
        ].map(({ y, label }) => (
          <g key={label}>
            <rect x={-3} y={y} width={9} height={8} fill="#c9a227" stroke="#7a5a1a" strokeWidth={0.3} />
            <text x={10} y={y + 6} fill="#fff" fontFamily="'JetBrains Mono', monospace" fontSize={5} fontWeight={500}>
              {label}
            </text>
          </g>
        ))}

        {selected && (
          <rect width={W} height={H} rx={7} fill="none" stroke="#F5B642" strokeWidth={2.5} opacity={0.85} />
        )}
      </g>
    );
  },
};
