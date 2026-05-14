interface FeaturedLab {
  id: string;
  name: string;
  chip: string;
  description: string;
  detail: string;
  accent: string;
  icon: string;
}

const FEATURED: FeaturedLab[] = [
  {
    id: 'stm32f103-blinky',
    name: 'STM32F103 Blinky',
    chip: 'Cortex-M3 · 72 MHz',
    description: 'Classic LED blink on PA5. The "hello world" of embedded.',
    detail: 'Bare-metal Rust toggling GPIOA_ODR. ~16k cycles between toggles. Perfect for verifying your toolchain.',
    accent: '#3DD68C',
    icon: '⚡',
  },
  {
    id: 'adxl345-sensor-lab',
    name: 'ADXL345 Tilt Sensor',
    chip: 'STM32F103 + ADXL345 over I²C',
    description: 'Read 3-axis acceleration data from a real I²C device model.',
    detail: 'Demonstrates LabWired\'s peripheral device model: a real ADXL345 register-level implementation responding to I²C reads from your firmware.',
    accent: '#F062B8',
    icon: '📊',
  },
  {
    id: 'mpu6050-sensor-lab',
    name: 'MPU6050 IMU',
    chip: 'STM32F103 + MPU6050 over I²C',
    description: 'Read 6-DoF acceleration and gyroscope data from a real I²C device model.',
    detail: 'Demonstrates LabWired\'s 6-DoF IMU device model: register-level MPU6050 implementation responding to I²C reads from your firmware. WHO_AM_I check + continuous accel/gyro loop.',
    accent: '#B07BFF',
    icon: '🧭',
  },
  {
    id: 'nucleo-f401re',
    name: 'Nucleo-F401RE',
    chip: 'Cortex-M4F · 84 MHz',
    description: 'STM32F4 Nucleo board with LED + user button.',
    detail: 'Higher-performance Cortex-M4 with FPU. Demonstrates LabWired\'s coverage of the STM32F4 family.',
    accent: '#5B9DFF',
    icon: '🔵',
  },
];

export function Gallery() {
  return (
    <div className="min-h-screen bg-bg-base text-fg-primary font-sans">
      {/* Top chrome */}
      <header className="sticky top-0 z-30 h-12 px-6 flex items-center gap-4 bg-[rgba(13,14,18,0.7)] backdrop-blur border-b border-border/60">
        <a href="./" className="flex items-center gap-2 text-fg-primary font-semibold tracking-tight shrink-0">
          <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
            <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
          </svg>
          LabWired
        </a>
        <span className="text-fg-tertiary text-[11px] hidden md:inline tracking-[0.01em]">
          Deterministic firmware simulation
        </span>
        <div className="flex-1" />
        <nav className="flex items-center gap-4 text-[13px]">
          <a href="./" className="text-fg-secondary hover:text-fg-primary transition-colors duration-150">
            Playground
          </a>
          <a href="ci.html" className="text-fg-secondary hover:text-fg-primary transition-colors duration-150">
            For CI
          </a>
          <a
            href="https://github.com/w1ne/labwired"
            target="_blank"
            rel="noopener noreferrer"
            className="text-fg-secondary hover:text-fg-primary transition-colors duration-150"
          >
            GitHub
          </a>
        </nav>
      </header>

      {/* Hero */}
      <section className="px-6 pt-20 pb-12 max-w-[1080px] mx-auto">
        <div className="inline-flex items-center gap-2 text-[11px] uppercase tracking-[0.12em] text-magenta font-semibold mb-5">
          <span className="w-1.5 h-1.5 rounded-full bg-magenta" />
          Featured labs
        </div>
        <h1 className="text-[44px] md:text-[52px] leading-[1.05] font-bold tracking-tight max-w-[22ch]">
          Real firmware. Real silicon parity. One click.
        </h1>
        <p className="text-fg-secondary text-[17px] leading-[1.5] mt-5 max-w-[58ch]">
          Each lab is a working STM32 firmware running cycle-accurate in your browser. Pick one to open in the
          playground — see the PC counter, cycles, and UART output in real time.
        </p>
      </section>

      {/* Cards grid */}
      <section className="px-6 pb-24 max-w-[1080px] mx-auto">
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-5">
          {FEATURED.map((lab) => (
            <a
              key={lab.id}
              href={`./?lab=${encodeURIComponent(lab.id)}`}
              className="group lw-glass p-5 transition-all duration-150 hover:bg-bg-elevated/70 hover:-translate-y-0.5 block"
            >
              {/* Thumbnail */}
              <div
                className="aspect-[16/10] rounded-card mb-4 flex items-center justify-center relative overflow-hidden"
                style={{
                  background: `linear-gradient(135deg, ${lab.accent}22, ${lab.accent}08 60%, transparent)`,
                  border: `1px solid ${lab.accent}33`,
                }}
              >
                <div className="text-[64px] opacity-90" aria-hidden>{lab.icon}</div>
                <div
                  className="absolute inset-0 opacity-30 pointer-events-none"
                  style={{
                    backgroundImage:
                      'radial-gradient(circle at 20% 80%, rgba(255,255,255,0.04) 0, transparent 50%)',
                  }}
                />
              </div>
              <div className="text-fg-tertiary text-[10px] uppercase tracking-[0.1em] font-semibold mb-1">
                {lab.chip}
              </div>
              <h3 className="text-fg-primary text-[17px] font-semibold mb-1.5">{lab.name}</h3>
              <p className="text-fg-secondary text-[13px] leading-[1.5] mb-3">{lab.description}</p>
              <p className="text-fg-tertiary text-[12px] leading-[1.5]">{lab.detail}</p>
              <div
                className="mt-4 text-[12px] font-medium transition-colors duration-150"
                style={{ color: lab.accent }}
              >
                Open in playground →
              </div>
            </a>
          ))}
        </div>

        {/* "Bring your own" card */}
        <div className="lw-glass p-6 mt-5 flex flex-col md:flex-row items-start md:items-center gap-4 justify-between">
          <div>
            <h3 className="text-fg-primary text-[16px] font-semibold mb-1">Bring your own firmware</h3>
            <p className="text-fg-secondary text-[13px] leading-[1.5]">
              Compile locally, drop your <code className="text-fg-primary font-mono text-[12px]">.elf</code> or
              <code className="text-fg-primary font-mono text-[12px]"> .bin</code> into the playground via the
              Upload button. Works against any of the supported STM32 / RP2040 / ESP32 / nRF52 chips.
            </p>
          </div>
          <a
            href="./"
            className="h-9 px-5 rounded-pill bg-accent text-bg-base font-semibold hover:bg-accent-hover transition-colors duration-150 flex items-center shrink-0"
          >
            Open playground →
          </a>
        </div>

        {/* Coming soon */}
        <div className="mt-12 mb-2 text-[11px] uppercase tracking-[0.1em] text-fg-tertiary font-semibold">
          Coming soon
        </div>
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-3 opacity-60">
          {[
            { name: 'BME280 Weather Station', wave: 'Wave 1' },
            { name: 'SSD1306 OLED', wave: 'Wave 2' },
            { name: 'ILI9341 TFT', wave: 'Wave 2' },
            { name: 'NEO-6M GPS', wave: 'Wave 3' },
            { name: 'NTC Thermistor', wave: 'Wave 3' },
          ].map((c) => (
            <div key={c.name} className="px-4 py-3 rounded-card bg-white/[0.03] border border-border/40 text-[13px] flex items-center justify-between">
              <span className="text-fg-secondary">{c.name}</span>
              <span className="text-fg-tertiary text-[10px] uppercase tracking-[0.08em] font-semibold">{c.wave}</span>
            </div>
          ))}
        </div>
      </section>

      {/* Footer */}
      <footer className="px-6 py-10 border-t border-border/60">
        <div className="max-w-[1080px] mx-auto flex flex-wrap items-center justify-between gap-4 text-[12px] text-fg-tertiary">
          <div className="flex items-center gap-2">
            <svg viewBox="0 0 20 20" width="14" height="14" aria-hidden>
              <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
            </svg>
            <span>LabWired · Deterministic firmware simulation</span>
          </div>
          <div className="flex items-center gap-5">
            <a className="hover:text-fg-primary transition-colors" href="./">Playground</a>
            <a className="hover:text-fg-primary transition-colors" href="ci.html">For CI</a>
            <a className="hover:text-fg-primary transition-colors" href="https://github.com/w1ne/labwired" target="_blank" rel="noopener noreferrer">GitHub</a>
            <a className="hover:text-fg-primary transition-colors" href="mailto:hello@labwired.com">Contact</a>
          </div>
        </div>
      </footer>
    </div>
  );
}
