import clsx from 'clsx';

export interface StarterLab {
  id: string;
  name: string;
  icon: string;
  locked: boolean;
  comingIn?: string;
}

export const STARTER_LABS: StarterLab[] = [
  { id: 'stm32f103-blinky', name: 'Blinky', icon: '⚡', locked: false },
  { id: 'adxl345-sensor-lab', name: 'ADXL345 Tilt', icon: '📊', locked: false },
  { id: 'bme280-weather', name: 'BME280 Weather', icon: '🌡', locked: true, comingIn: 'Wave 2' },
  { id: 'oled-hello', name: 'OLED Hello', icon: '📺', locked: true, comingIn: 'Wave 2' },
  { id: 'gps-trail', name: 'GPS Trail', icon: '📡', locked: true, comingIn: 'Wave 3' },
  { id: 'tft-demo', name: 'TFT Demo', icon: '🎨', locked: true, comingIn: 'Wave 2' },
];

export interface ChipRowProps {
  onPick: (labId: string) => void;
  onLocked: (labId: string) => void;
}

export function ChipRow({ onPick, onLocked }: ChipRowProps) {
  return (
    <div className="flex flex-wrap gap-2 justify-center max-w-[640px] mx-auto">
      {STARTER_LABS.map((lab) => (
        <button
          key={lab.id}
          type="button"
          onClick={() => (lab.locked ? onLocked(lab.id) : onPick(lab.id))}
          style={{ borderRadius: 999 }}
          className={clsx(
            'h-10 px-4 text-[13px] font-medium inline-flex items-center gap-2',
            'transition-all duration-150 outline-none border-0',
            'focus-visible:ring-2 focus-visible:ring-accent/60',
            lab.locked
              ? 'bg-white/[0.04] text-fg-tertiary hover:bg-white/[0.07] hover:text-fg-secondary'
              : 'bg-white/[0.06] text-fg-primary hover:bg-white/[0.10] hover:-translate-y-[1px] active:translate-y-0'
          )}
        >
          <span className="text-base leading-none" aria-hidden>{lab.icon}</span>
          <span>{lab.name}</span>
          {lab.locked && lab.comingIn && (
            <span
              style={{ borderRadius: 4 }}
              className="text-fg-tertiary text-[9px] uppercase tracking-[0.08em] font-semibold ml-0.5 px-1.5 py-0.5 bg-white/[0.04]"
            >
              {lab.comingIn}
            </span>
          )}
        </button>
      ))}
    </div>
  );
}
