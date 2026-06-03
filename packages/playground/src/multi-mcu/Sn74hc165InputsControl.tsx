export interface Sn74hc165InputsControlProps {
  value: number;
  onChannelChange: (channel: number, high: boolean) => void;
  onByteChange?: (value: number) => void;
}

function clampByte(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(255, Math.round(value)));
}

function formatBinaryByte(value: number): string {
  const bits = clampByte(value).toString(2).padStart(8, '0');
  return `${bits.slice(0, 4)} ${bits.slice(4)}`;
}

export function Sn74hc165InputsControl({
  value,
  onChannelChange,
  onByteChange,
}: Sn74hc165InputsControlProps) {
  const byte = clampByte(value);
  const hex = `0x${byte.toString(16).toUpperCase().padStart(2, '0')}`;

  return (
    <section className="flex flex-col gap-2 rounded-md border border-border bg-bg-canvas p-2">
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="text-[11px] uppercase tracking-wide text-fg-tertiary">Inputs</div>
          <div className="font-mono text-[12px] text-fg-primary">
            <span className="font-semibold">{hex}</span>
            <span className="ml-2 text-fg-secondary">{formatBinaryByte(byte)}</span>
          </div>
        </div>
        {onByteChange && (
          <div className="flex gap-1">
            <button
              type="button"
              className="h-7 rounded border border-border px-2 font-mono text-[10px] text-fg-secondary hover:text-fg-primary"
              onClick={() => onByteChange(0)}
            >
              00
            </button>
            <button
              type="button"
              className="h-7 rounded border border-border px-2 font-mono text-[10px] text-fg-secondary hover:text-fg-primary"
              onClick={() => onByteChange(0xff)}
            >
              FF
            </button>
          </div>
        )}
      </div>

      <div className="grid grid-cols-4 gap-1.5">
        {Array.from({ length: 8 }, (_, channel) => {
          const high = (byte & (1 << channel)) !== 0;
          return (
            <button
              key={channel}
              type="button"
              onClick={() => onChannelChange(channel, !high)}
              className={`h-8 rounded border px-2 font-mono text-[11px] font-semibold ${
                high
                  ? 'border-emerald-500/60 bg-emerald-500/20 text-emerald-300'
                  : 'border-border bg-bg-elevated text-fg-secondary hover:text-fg-primary'
              }`}
            >
              D{channel} {high ? 'HI' : 'LO'}
            </button>
          );
        })}
      </div>
    </section>
  );
}
