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
          className={clsx(
            'h-9 px-3 rounded-pill text-xs font-medium flex items-center gap-2',
            'transition-colors duration-micro',
            lab.locked
              ? 'bg-bg-surface/50 border border-border text-fg-tertiary hover:text-fg-secondary'
              : 'bg-bg-surface border border-border text-fg-primary hover:border-accent hover:text-accent'
          )}
        >
          <span aria-hidden>{lab.icon}</span>
          {lab.name}
          {lab.locked && lab.comingIn && (
            <span className="text-fg-tertiary text-[10px] uppercase tracking-wider ml-1">{lab.comingIn}</span>
          )}
        </button>
      ))}
    </div>
  );
}
