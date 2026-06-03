// Compact glyph icons for the component palette. Each is a 20×20 SVG that
// reads at small sizes — silhouettes over realism. Falls back to a category
// glyph when a specific type isn't registered.

import type { PaletteCategory } from './PaletteDrawer';

const stroke = 'currentColor';

// ─── MCU boards ────────────────────────────────────────────────────────────
const ArduinoUnoIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="5" width="16" height="10" rx="1" fill="#1e6091" />
    <rect x="14" y="7" width="3" height="6" rx="0.3" fill="#0f0f12" />
    <circle cx="4.5" cy="7" r="0.5" fill="#ffd166" />
    <circle cx="4.5" cy="13" r="0.5" fill="#ffd166" />
    <rect x="8" y="9" width="4" height="2" fill="#0f0f12" />
  </svg>
);
const Stm32Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="2" width="14" height="16" rx="1" fill="#2563eb" />
    <rect x="7" y="8" width="6" height="4" rx="0.4" fill="#0f0f12" />
    <g fill="#fbbf24">
      <rect x="4" y="4" width="1" height="1" />
      <rect x="4" y="6" width="1" height="1" />
      <rect x="4" y="8" width="1" height="1" />
      <rect x="4" y="10" width="1" height="1" />
      <rect x="15" y="4" width="1" height="1" />
      <rect x="15" y="6" width="1" height="1" />
      <rect x="15" y="8" width="1" height="1" />
      <rect x="15" y="10" width="1" height="1" />
    </g>
  </svg>
);
const Esp32Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="4" width="16" height="12" rx="1" fill="#111418" stroke="#374151" strokeWidth="0.5" />
    <rect x="6" y="7" width="8" height="6" fill="#9ca3af" />
    <text x="10" y="11.5" textAnchor="middle" fontSize="3" fill="#0f0f12" fontFamily="monospace" fontWeight="bold">ESP</text>
  </svg>
);
const Esp32C3Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="4" y="3" width="12" height="14" rx="1" fill="#111418" stroke="#6b7280" strokeWidth="0.5" />
    <rect x="6" y="6" width="8" height="5" fill="#9ca3af" />
    <text x="10" y="10" textAnchor="middle" fontSize="2.5" fill="#0f0f12" fontFamily="monospace" fontWeight="bold">C3</text>
  </svg>
);
const Esp32S3Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="4" y="3" width="12" height="14" rx="1" fill="#111418" stroke="#6b7280" strokeWidth="0.5" />
    <rect x="6" y="6" width="8" height="5" fill="#9ca3af" />
    <text x="10" y="10" textAnchor="middle" fontSize="2.5" fill="#0f0f12" fontFamily="monospace" fontWeight="bold">S3</text>
  </svg>
);
const RpiPicoIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="5" width="16" height="10" rx="1.5" fill="#14532d" />
    <rect x="7" y="8" width="6" height="4" rx="0.3" fill="#0f0f12" />
    <g fill="#facc15"><rect x="3.5" y="7" width="0.6" height="6" /><rect x="15.5" y="7" width="0.6" height="6" /></g>
  </svg>
);
const Nrf52840Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="14" height="14" rx="1" fill="#1e40af" />
    <circle cx="10" cy="10" r="3" fill="#0f0f12" />
    <text x="10" y="11.2" textAnchor="middle" fontSize="2.6" fill="#fff" fontFamily="monospace" fontWeight="bold">nRF</text>
  </svg>
);
const McuIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="4" y="4" width="12" height="12" rx="1" fill="#1f2937" />
    <rect x="6" y="6" width="8" height="8" rx="0.4" fill="#374151" />
    <g fill="#9ca3af"><rect x="3" y="6" width="1" height="0.8" /><rect x="3" y="9" width="1" height="0.8" /><rect x="3" y="12" width="1" height="0.8" /><rect x="16" y="6" width="1" height="0.8" /><rect x="16" y="9" width="1" height="0.8" /><rect x="16" y="12" width="1" height="0.8" /></g>
  </svg>
);

// ─── Outputs ───────────────────────────────────────────────────────────────
const LedIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="9" r="5" fill="#ef4444" stroke="#7f1d1d" strokeWidth="0.5" />
    <ellipse cx="8.5" cy="7" rx="1.2" ry="0.7" fill="#fecaca" opacity="0.8" />
    <line x1="8.5" y1="14" x2="8.5" y2="18" stroke={stroke} strokeWidth="0.7" />
    <line x1="11.5" y1="14" x2="11.5" y2="18" stroke={stroke} strokeWidth="0.7" />
  </svg>
);
const RgbLedIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="9" r="5" fill="url(#rgbGrad)" stroke="#374151" strokeWidth="0.5" />
    <defs>
      <linearGradient id="rgbGrad" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stopColor="#ef4444" />
        <stop offset="0.5" stopColor="#10b981" />
        <stop offset="1" stopColor="#3b82f6" />
      </linearGradient>
    </defs>
    <line x1="7" y1="14" x2="7" y2="18" stroke={stroke} strokeWidth="0.5" />
    <line x1="9" y1="14" x2="9" y2="18" stroke={stroke} strokeWidth="0.5" />
    <line x1="11" y1="14" x2="11" y2="18" stroke={stroke} strokeWidth="0.5" />
    <line x1="13" y1="14" x2="13" y2="18" stroke={stroke} strokeWidth="0.5" />
  </svg>
);
const BuzzerIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="10" r="6" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    <circle cx="10" cy="10" r="2" fill="#9ca3af" />
    <circle cx="10" cy="10" r="0.6" fill="#0f0f12" />
  </svg>
);
const ServoIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="6" width="11" height="8" rx="0.5" fill="#1e3a8a" />
    <circle cx="14" cy="10" r="3" fill="#1e40af" stroke="#3b82f6" strokeWidth="0.5" />
    <rect x="13.4" y="6.5" width="1.2" height="7" rx="0.3" fill="#fff" opacity="0.85" />
  </svg>
);
const NeopixelIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="1" y="7" width="18" height="6" rx="0.5" fill="#0f0f12" stroke="#374151" strokeWidth="0.5" />
    <circle cx="4" cy="10" r="1.6" fill="#ef4444" />
    <circle cx="8" cy="10" r="1.6" fill="#10b981" />
    <circle cx="12" cy="10" r="1.6" fill="#3b82f6" />
    <circle cx="16" cy="10" r="1.6" fill="#f59e0b" />
  </svg>
);

// ─── Inputs ────────────────────────────────────────────────────────────────
const ButtonIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="14" height="14" rx="1.5" fill="#1f2937" stroke="#6b7280" strokeWidth="0.6" />
    <circle cx="10" cy="10" r="4" fill="#10b981" stroke="#065f46" strokeWidth="0.5" />
    <circle cx="10" cy="10" r="2.2" fill="#34d399" />
  </svg>
);
const PotentiometerIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="14" height="14" rx="1" fill="#1e3a8a" />
    <circle cx="10" cy="10" r="5" fill="#3b82f6" stroke="#1e40af" strokeWidth="0.5" />
    <line x1="10" y1="10" x2="13.5" y2="6.5" stroke="#fff" strokeWidth="1.2" strokeLinecap="round" />
  </svg>
);
const SlideSwitchIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="6" width="14" height="8" rx="0.5" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    <rect x="11" y="7" width="5" height="6" rx="0.3" fill="#9ca3af" />
  </svg>
);
const DipSwitchIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="5" width="16" height="10" rx="0.5" fill="#ef4444" />
    <g fill="#fff">
      <rect x="3.5" y="6.5" width="2" height="3" />
      <rect x="6.5" y="6.5" width="2" height="3" />
      <rect x="9.5" y="10.5" width="2" height="3" />
      <rect x="12.5" y="6.5" width="2" height="3" />
      <rect x="15.5" y="10.5" width="0" height="0" />
      <rect x="15.5" y="6.5" width="0.5" height="3" />
    </g>
  </svg>
);
const RotaryEncoderIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="10" r="7" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    <circle cx="10" cy="10" r="4" fill="#374151" />
    <g stroke="#9ca3af" strokeWidth="0.6">
      <line x1="10" y1="3" x2="10" y2="4.5" />
      <line x1="10" y1="15.5" x2="10" y2="17" />
      <line x1="3" y1="10" x2="4.5" y2="10" />
      <line x1="15.5" y1="10" x2="17" y2="10" />
    </g>
    <line x1="10" y1="10" x2="13" y2="7" stroke="#fff" strokeWidth="1" strokeLinecap="round" />
  </svg>
);
const KeypadIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="2" width="16" height="16" rx="1" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    <g fill="#374151">
      {[4, 8.5, 13].flatMap((x) => [4, 8.5, 13, 15.5].map((y) => `${x},${y}`)).map((p) => {
        const [x, y] = p.split(',').map(Number);
        return <rect key={p} x={x - 1.2} y={y - 1.2} width="2.4" height="2.4" rx="0.3" />;
      })}
    </g>
  </svg>
);

// ─── Sensors ───────────────────────────────────────────────────────────────
const Dht22Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="4" y="2" width="12" height="14" rx="1" fill="#fff" stroke="#9ca3af" strokeWidth="0.5" />
    <g fill="#9ca3af">
      <circle cx="6.5" cy="5" r="0.6" /><circle cx="9" cy="5" r="0.6" /><circle cx="11.5" cy="5" r="0.6" /><circle cx="14" cy="5" r="0.6" />
      <circle cx="6.5" cy="8" r="0.6" /><circle cx="9" cy="8" r="0.6" /><circle cx="11.5" cy="8" r="0.6" /><circle cx="14" cy="8" r="0.6" />
    </g>
    <text x="10" y="14" textAnchor="middle" fontSize="3" fill="#0f0f12" fontFamily="monospace" fontWeight="bold">22</text>
  </svg>
);
const PirIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="9" r="6" fill="#fff" stroke="#9ca3af" strokeWidth="0.5" />
    <circle cx="10" cy="9" r="3" fill="#1f2937" />
    <circle cx="9" cy="8" r="0.8" fill="#9ca3af" opacity="0.8" />
  </svg>
);
const UltrasonicIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="5" width="16" height="10" rx="0.5" fill="#1e40af" />
    <circle cx="6" cy="10" r="3" fill="#9ca3af" />
    <circle cx="6" cy="10" r="2" fill="#374151" />
    <circle cx="14" cy="10" r="3" fill="#9ca3af" />
    <circle cx="14" cy="10" r="2" fill="#374151" />
  </svg>
);
const LdrIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="10" r="6" fill="#fbbf24" stroke="#92400e" strokeWidth="0.5" />
    <path d="M10 4v-2 M10 18v-2 M4 10h-2 M18 10h-2 M5.5 5.5l-1.4-1.4 M15.9 15.9l-1.4-1.4 M5.5 14.5l-1.4 1.4 M15.9 4.1l-1.4 1.4" stroke="#92400e" strokeWidth="0.6" />
    <path d="M7 9l1.5 1 -1 1.5 1.5 1 -1 1.5" fill="none" stroke="#374151" strokeWidth="0.5" />
  </svg>
);
const ImuIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="14" height="14" rx="1" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    <line x1="10" y1="10" x2="14" y2="10" stroke="#ef4444" strokeWidth="0.8" markerEnd="url(#arr-r)" />
    <line x1="10" y1="10" x2="10" y2="6" stroke="#10b981" strokeWidth="0.8" markerEnd="url(#arr-g)" />
    <line x1="10" y1="10" x2="7.5" y2="12.5" stroke="#3b82f6" strokeWidth="0.8" />
    <defs>
      <marker id="arr-r" viewBox="0 0 4 4" refX="3" refY="2" markerWidth="3" markerHeight="3" orient="auto"><path d="M0,0 L4,2 0,4 z" fill="#ef4444" /></marker>
      <marker id="arr-g" viewBox="0 0 4 4" refX="3" refY="2" markerWidth="3" markerHeight="3" orient="auto"><path d="M0,0 L4,2 0,4 z" fill="#10b981" /></marker>
    </defs>
  </svg>
);
const BmeIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="14" height="14" rx="1" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    <rect x="7" y="6" width="6" height="6" rx="0.5" fill="#9ca3af" />
    <line x1="10" y1="13" x2="10" y2="16" stroke="#ef4444" strokeWidth="0.6" />
    <circle cx="10" cy="16.5" r="0.8" fill="#ef4444" />
  </svg>
);
const ThermocoupleIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="10" height="14" rx="0.5" fill="#7c2d12" stroke="#92400e" strokeWidth="0.5" />
    <text x="8" y="11.5" textAnchor="middle" fontSize="3.5" fill="#fbbf24" fontFamily="monospace" fontWeight="bold">TC</text>
    <line x1="13" y1="7" x2="18" y2="7" stroke="#ef4444" strokeWidth="0.8" />
    <line x1="13" y1="11" x2="18" y2="11" stroke="#3b82f6" strokeWidth="0.8" />
  </svg>
);
const GpsIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="10" r="7" fill="none" stroke="#10b981" strokeWidth="1" />
    <ellipse cx="10" cy="10" rx="7" ry="3" fill="none" stroke="#10b981" strokeWidth="0.6" />
    <line x1="3" y1="10" x2="17" y2="10" stroke="#10b981" strokeWidth="0.6" />
    <line x1="10" y1="3" x2="10" y2="17" stroke="#10b981" strokeWidth="0.6" />
    <circle cx="10" cy="10" r="1.2" fill="#10b981" />
  </svg>
);
const CellularModemIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="6" y="4" width="8" height="11" rx="1" fill="#0f0f12" stroke="#3ec1d3" strokeWidth="0.6" />
    <rect x="7.2" y="5.4" width="5.6" height="3" rx="0.3" fill="#3ec1d3" opacity="0.6" />
    <line x1="10" y1="2" x2="10" y2="4" stroke="#3ec1d3" strokeWidth="0.8" strokeLinecap="round" />
    <path d="M 7.5 1.5 Q 10 0 12.5 1.5" stroke="#3ec1d3" strokeWidth="0.5" fill="none" opacity="0.7" />
    <path d="M 6.5 0.5 Q 10 -1 13.5 0.5" stroke="#3ec1d3" strokeWidth="0.5" fill="none" opacity="0.4" />
    <circle cx="8.5" cy="11.5" r="0.6" fill="#fff" />
    <circle cx="11.5" cy="11.5" r="0.6" fill="#fff" />
    <line x1="8" y1="17" x2="8" y2="19" stroke="#9ca3af" strokeWidth="0.5" />
    <line x1="12" y1="17" x2="12" y2="19" stroke="#9ca3af" strokeWidth="0.5" />
    <line x1="10" y1="15" x2="10" y2="19" stroke="#9ca3af" strokeWidth="0.5" />
  </svg>
);
const NtcIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="9" y="2" width="2" height="12" rx="1" fill="#fff" stroke="#6b7280" strokeWidth="0.5" />
    <circle cx="10" cy="15.5" r="3" fill="#ef4444" stroke="#7f1d1d" strokeWidth="0.5" />
    <line x1="10" y1="4" x2="10" y2="13" stroke="#ef4444" strokeWidth="1" />
  </svg>
);

// ─── Displays ──────────────────────────────────────────────────────────────
const SevenSegIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="2" width="14" height="16" rx="0.5" fill="#0f0f12" stroke="#374151" strokeWidth="0.5" />
    <g stroke="#ef4444" strokeWidth="1.5" strokeLinecap="round" fill="none">
      <line x1="7" y1="5" x2="13" y2="5" />
      <line x1="7" y1="10" x2="13" y2="10" />
      <line x1="7" y1="15" x2="13" y2="15" />
      <line x1="7" y1="5" x2="7" y2="10" />
      <line x1="13" y1="5" x2="13" y2="10" />
      <line x1="7" y1="10" x2="7" y2="15" />
      <line x1="13" y1="10" x2="13" y2="15" />
    </g>
  </svg>
);
const Lcd1602Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="4" width="16" height="12" rx="0.5" fill="#16a34a" stroke="#14532d" strokeWidth="0.5" />
    <rect x="3.5" y="6" width="13" height="3.2" fill="#86efac" />
    <rect x="3.5" y="10.4" width="13" height="3.2" fill="#86efac" />
    <g fill="#14532d">
      <rect x="4" y="7" width="0.8" height="1.2" /><rect x="5.5" y="7" width="0.8" height="1.2" /><rect x="7" y="7" width="0.8" height="1.2" /><rect x="8.5" y="7" width="0.8" height="1.2" />
    </g>
  </svg>
);
const OledIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="3" width="16" height="14" rx="0.8" fill="#0f0f12" stroke="#3b82f6" strokeWidth="0.7" />
    <rect x="4" y="5" width="12" height="10" fill="#1e40af" />
    <g fill="#60a5fa">
      <rect x="5" y="6" width="2" height="1" /><rect x="5" y="8" width="4" height="1" /><rect x="5" y="10" width="3" height="1" /><rect x="5" y="12" width="5" height="1" />
    </g>
  </svg>
);
const LedMatrixIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="2" width="16" height="16" rx="0.5" fill="#1f2937" stroke="#6b7280" strokeWidth="0.5" />
    {Array.from({ length: 5 }).flatMap((_, r) =>
      Array.from({ length: 5 }).map((_, c) => (
        <circle key={`${r}-${c}`} cx={4 + c * 3} cy={4 + r * 3} r="0.9" fill="#ef4444" opacity={(r + c) % 2 === 0 ? 0.9 : 0.4} />
      ))
    )}
  </svg>
);
const Ili9341Icon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="1" y="2" width="18" height="16" rx="0.8" fill="#0f0f12" stroke="#6b7280" strokeWidth="0.5" />
    <rect x="2.5" y="3.5" width="15" height="13" fill="url(#tftG)" />
    <defs>
      <linearGradient id="tftG" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stopColor="#3b82f6" />
        <stop offset="0.5" stopColor="#8b5cf6" />
        <stop offset="1" stopColor="#ec4899" />
      </linearGradient>
    </defs>
  </svg>
);
const EpaperIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="2" y="2" width="16" height="16" rx="0.5" fill="#fafafa" stroke="#9ca3af" strokeWidth="0.5" />
    <g fill="#0f0f12">
      <rect x="4" y="4" width="12" height="1.2" />
      <rect x="4" y="6.5" width="8" height="0.8" />
      <rect x="4" y="8.5" width="10" height="0.8" />
    </g>
    <rect x="4" y="11" width="6" height="5" fill="#ef4444" />
  </svg>
);

// ─── Passives & ICs ────────────────────────────────────────────────────────
const ResistorIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="8" width="14" height="4" rx="0.5" fill="#d1bd8a" />
    <line x1="0" y1="10" x2="3" y2="10" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="17" y1="10" x2="20" y2="10" stroke="#9ca3af" strokeWidth="0.8" />
    <rect x="5" y="8" width="1" height="4" fill="#7f1d1d" />
    <rect x="8" y="8" width="1" height="4" fill="#0ea5e9" />
    <rect x="11" y="8" width="1" height="4" fill="#10b981" />
    <rect x="14" y="8" width="1" height="4" fill="#fbbf24" />
  </svg>
);
const CapacitorIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <line x1="0" y1="10" x2="8" y2="10" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="12" y1="10" x2="20" y2="10" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="8" y1="5" x2="8" y2="15" stroke="#9ca3af" strokeWidth="1.4" />
    <path d="M12 5 Q14 10 12 15" fill="none" stroke="#9ca3af" strokeWidth="1.4" />
  </svg>
);
const DiodeIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <line x1="0" y1="10" x2="20" y2="10" stroke="#9ca3af" strokeWidth="0.8" />
    <polygon points="6,5 13,10 6,15" fill="#9ca3af" />
    <line x1="13" y1="5" x2="13" y2="15" stroke="#9ca3af" strokeWidth="1.4" />
  </svg>
);
const TransistorIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <circle cx="10" cy="10" r="7" fill="none" stroke="#9ca3af" strokeWidth="0.6" />
    <line x1="6" y1="6" x2="6" y2="14" stroke="#9ca3af" strokeWidth="1.4" />
    <line x1="6" y1="9" x2="13" y2="6" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="6" y1="11" x2="13" y2="14" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="13" y1="6" x2="13" y2="3" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="13" y1="14" x2="13" y2="17" stroke="#9ca3af" strokeWidth="0.8" />
    <line x1="3" y1="10" x2="6" y2="10" stroke="#9ca3af" strokeWidth="0.8" />
  </svg>
);
const ChipIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="4" y="4" width="12" height="12" rx="0.5" fill="#0f0f12" stroke="#6b7280" strokeWidth="0.5" />
    <circle cx="6" cy="6" r="0.6" fill="#9ca3af" />
    <g fill="#9ca3af">
      <rect x="2" y="5.5" width="2" height="1" /><rect x="2" y="8" width="2" height="1" /><rect x="2" y="10.5" width="2" height="1" /><rect x="2" y="13" width="2" height="1" />
      <rect x="16" y="5.5" width="2" height="1" /><rect x="16" y="8" width="2" height="1" /><rect x="16" y="10.5" width="2" height="1" /><rect x="16" y="13" width="2" height="1" />
    </g>
  </svg>
);
const MotorDriverIcon = () => (
  <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
    <rect x="3" y="3" width="14" height="14" rx="0.5" fill="#0f0f12" stroke="#6b7280" strokeWidth="0.5" />
    <text x="10" y="13" textAnchor="middle" fontSize="6" fill="#fbbf24" fontFamily="monospace" fontWeight="bold">M</text>
  </svg>
);

// ─── Category fallbacks ────────────────────────────────────────────────────
const CategoryFallback = ({ category }: { category: PaletteCategory }) => {
  const color =
    category === 'i2c'
      ? '#10b981'
      : category === 'spi'
        ? '#3b82f6'
        : category === 'uart'
          ? '#a855f7'
          : category === 'analog'
            ? '#f59e0b'
            : category === 'gpio'
              ? '#ec4899'
              : category === 'tools'
                ? '#38bdf8'
                : '#9ca3af';
  return (
    <svg viewBox="0 0 20 20" width="20" height="20" aria-hidden>
      <rect x="4" y="4" width="12" height="12" rx="2" fill={color} opacity="0.18" stroke={color} strokeWidth="1" />
      <circle cx="10" cy="10" r="2" fill={color} />
    </svg>
  );
};

const ICONS: Record<string, () => React.JSX.Element> = {
  mcu: McuIcon,
  'arduino-uno': ArduinoUnoIcon,
  'stm32-dev': Stm32Icon,
  esp32: Esp32Icon,
  'esp32-c3-supermini': Esp32C3Icon,
  'esp32-s3-zero': Esp32S3Icon,
  'rpi-pico': RpiPicoIcon,
  'nrf52840-dk': Nrf52840Icon,

  led: LedIcon,
  'rgb-led': RgbLedIcon,
  buzzer: BuzzerIcon,
  servo: ServoIcon,
  neopixel: NeopixelIcon,

  button: ButtonIcon,
  potentiometer: PotentiometerIcon,
  'slide-switch': SlideSwitchIcon,
  'dip-switch': DipSwitchIcon,
  'rotary-encoder': RotaryEncoderIcon,
  keypad: KeypadIcon,

  dht22: Dht22Icon,
  'pir-sensor': PirIcon,
  ultrasonic: UltrasonicIcon,
  ldr: LdrIcon,
  adxl345: ImuIcon,
  mpu6050: ImuIcon,
  bme280: BmeIcon,
  max31855: ThermocoupleIcon,
  'neo6m-gps': GpsIcon,
  'bg770a-cellular': CellularModemIcon,
  'ntc-thermistor': NtcIcon,

  'seven-segment': SevenSegIcon,
  lcd1602: Lcd1602Icon,
  'oled-ssd1306': OledIcon,
  'led-matrix': LedMatrixIcon,
  ili9341: Ili9341Icon,
  'ili9341-tft': Ili9341Icon,
  'epd-ssd1680-tricolor': EpaperIcon,

  resistor: ResistorIcon,
  capacitor: CapacitorIcon,
  diode: DiodeIcon,
  transistor: TransistorIcon,
  'shift-register-74hc595': ChipIcon,
  'motor-driver-l293d': MotorDriverIcon,
};

export function getComponentIcon(type: string, category: PaletteCategory): React.ReactNode {
  const Specific = ICONS[type];
  if (Specific) return <Specific />;
  return <CategoryFallback category={category} />;
}
