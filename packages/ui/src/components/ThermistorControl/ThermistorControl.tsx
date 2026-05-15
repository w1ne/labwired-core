export interface ThermistorControlProps {
  temperatureC: number;
  dividerMv?: number;
  adcCount?: number;
  onChange: (temperatureC: number) => void;
}

export function ThermistorControl({
  temperatureC,
  dividerMv,
  adcCount,
  onChange,
}: ThermistorControlProps) {
  return (
    <div className="space-y-3">
      <label className="block">
        <div className="flex items-center justify-between text-fg-tertiary text-[11px] font-mono mb-1">
          <span>Temperature</span>
          <span className="text-fg-primary">{temperatureC.toFixed(1)} °C</span>
        </div>
        <input
          type="range"
          min={-20}
          max={100}
          step={0.5}
          value={temperatureC}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          className="w-full accent-magenta"
        />
        <div className="flex justify-between text-fg-tertiary text-[10px] font-mono mt-1">
          <span>-20°C</span>
          <span>40°C</span>
          <span>100°C</span>
        </div>
      </label>
      {dividerMv !== undefined && adcCount !== undefined && (
        <div className="grid grid-cols-2 gap-2 text-[11px] font-mono">
          <div className="bg-bg-elevated rounded p-2">
            <div className="text-fg-tertiary text-[9px] uppercase tracking-wider mb-0.5">Divider</div>
            <div className="text-fg-primary">{dividerMv} mV</div>
          </div>
          <div className="bg-bg-elevated rounded p-2">
            <div className="text-fg-tertiary text-[9px] uppercase tracking-wider mb-0.5">ADC count</div>
            <div className="text-fg-primary">{adcCount} / 4095</div>
          </div>
        </div>
      )}
    </div>
  );
}
