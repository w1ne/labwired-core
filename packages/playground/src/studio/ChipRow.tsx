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
  { id: 'mpu6050-sensor-lab', name: 'MPU6050 IMU', icon: '🧭', locked: false },
  { id: 'bme280-weather-lab', name: 'BME280 Weather', icon: '🌡', locked: false },
  { id: 'ssd1306-hello-lab', name: 'OLED Hello', icon: '📺', locked: false },
  { id: 'max31855-thermocouple-lab', name: 'MAX31855 Thermocouple', icon: '🔥', locked: false },
  { id: 'neo6m-gps-lab', name: 'NEO-6M GPS', icon: '📡', locked: false },
  { id: 'nucleo-f401re', name: 'Nucleo F4', icon: '🔵', locked: false },
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
