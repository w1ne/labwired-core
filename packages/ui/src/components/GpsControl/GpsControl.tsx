import { useState } from 'react';

export interface GpsControlProps {
  lat: number;
  lon: number;
  hasFix: boolean;
  onChange: (lat: number, lon: number) => void;
  onFixToggle: (active: boolean) => void;
}

export function GpsControl({ lat, lon, hasFix, onChange, onFixToggle }: GpsControlProps) {
  const [latInput, setLatInput] = useState(lat.toFixed(4));
  const [lonInput, setLonInput] = useState(lon.toFixed(4));
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 gap-2">
        <label className="text-fg-tertiary text-[11px] font-mono">
          Lat
          <input
            type="number"
            value={latInput}
            step="0.0001"
            onChange={(e) => setLatInput(e.target.value)}
            onBlur={() => onChange(parseFloat(latInput) || 0, parseFloat(lonInput) || 0)}
            className="w-full h-7 px-2 mt-1 bg-bg-elevated border border-border rounded text-fg-primary outline-none focus:border-accent font-mono"
          />
        </label>
        <label className="text-fg-tertiary text-[11px] font-mono">
          Lon
          <input
            type="number"
            value={lonInput}
            step="0.0001"
            onChange={(e) => setLonInput(e.target.value)}
            onBlur={() => onChange(parseFloat(latInput) || 0, parseFloat(lonInput) || 0)}
            className="w-full h-7 px-2 mt-1 bg-bg-elevated border border-border rounded text-fg-primary outline-none focus:border-accent font-mono"
          />
        </label>
      </div>
      <button
        type="button"
        onClick={() => onFixToggle(!hasFix)}
        className={
          hasFix
            ? 'w-full h-8 rounded-pill bg-ok/20 text-ok border border-ok/40 text-[11px] font-medium'
            : 'w-full h-8 rounded-pill bg-warn/20 text-warn border border-warn/40 text-[11px] font-medium'
        }
      >
        {hasFix ? '✓ GPS fix' : '✗ No fix'}
      </button>
      <div className="text-fg-tertiary text-[10px] font-mono">
        Streaming NMEA at 2Hz · GGA + RMC sentences
      </div>
    </div>
  );
}
