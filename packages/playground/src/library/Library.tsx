/**
 * The Library — comprehensive catalog of supported boards and labs.
 * Replaces the older Gallery (featured-labs-only) framing.
 */

interface SupportedBoard {
  chip: string;
  family: string;
  arch: string;
  vendor: string;
  status: 'working-labs' | 'bring-your-own' | 'roadmap';
  notes: string;
  /** Optional deep-link into the playground with this board pre-selected. */
  playgroundBoardId?: string;
}

const SUPPORTED_BOARDS: SupportedBoard[] = [
  {
    chip: 'STM32F103',
    family: 'STM32F1 (Bluepill)',
    arch: 'ARM Cortex-M3',
    vendor: 'STMicroelectronics',
    status: 'working-labs',
    notes: '9 working firmware labs · I²C / SPI / UART / ADC fully wired',
    playgroundBoardId: 'stm32f103-blinky',
  },
  {
    chip: 'STM32F401RE',
    family: 'STM32F4 (Nucleo)',
    arch: 'ARM Cortex-M4F',
    vendor: 'STMicroelectronics',
    status: 'working-labs',
    notes: 'Nucleo-F401RE board · LED + user button demo',
    playgroundBoardId: 'nucleo-f401re',
  },
  {
    chip: 'STM32F401CDU6',
    family: 'STM32F4 (Black Pill)',
    arch: 'ARM Cortex-M4F',
    vendor: 'STMicroelectronics',
    status: 'bring-your-own',
    notes: 'Compact Black Pill board · active-low PC13 LED',
    playgroundBoardId: 'stm32f401cdu6-blackpill',
  },
  {
    chip: 'STM32H563ZI',
    family: 'STM32H5 (Nucleo-144)',
    arch: 'ARM Cortex-M33',
    vendor: 'STMicroelectronics',
    status: 'bring-your-own',
    notes: '3 LEDs + user button · TrustZone-capable Cortex-M33',
    playgroundBoardId: 'nucleo-h563zi',
  },
  {
    chip: 'RP2040',
    family: 'Raspberry Pi Pico',
    arch: 'ARM Cortex-M0+',
    vendor: 'Raspberry Pi',
    status: 'bring-your-own',
    notes: 'Dual-core M0+ · upload your own ELF/UF2',
    playgroundBoardId: 'rp2040-pico',
  },
  {
    chip: 'nRF52840',
    family: 'nRF52840 DK',
    arch: 'ARM Cortex-M4F',
    vendor: 'Nordic Semiconductor',
    status: 'bring-your-own',
    notes: 'Nordic dev kit · BLE-capable target',
    playgroundBoardId: 'nrf52840-dk',
  },
  {
    chip: 'ESP32-C3',
    family: 'ESP32-C3 Super Mini',
    arch: 'RISC-V',
    vendor: 'Espressif',
    status: 'bring-your-own',
    notes: 'Compact RISC-V board · USB-C · built-in LED on GPIO8',
    playgroundBoardId: 'esp32c3-supermini',
  },
  {
    chip: 'ESP32-S3',
    family: 'ESP32-S3-Zero',
    arch: 'Xtensa LX7',
    vendor: 'Espressif',
    status: 'bring-your-own',
    notes: 'Dual-core Xtensa LX7 · RGB LED on GPIO48',
    playgroundBoardId: 'esp32s3-zero',
  },
];

interface FeaturedLab {
  id: string;
  name: string;
  chip: string;
  description: string;
  detail: string;
  accent: string;
  icon: string;
}

const FEATURED_LABS: FeaturedLab[] = [
  {
    id: 'stm32f103-blinky',
    name: 'Blinky',
    chip: 'STM32F103',
    description: 'Classic LED blink on PA5. The "hello world" of embedded.',
    detail: 'Bare-metal Rust toggling GPIOA_ODR. ~16k cycles between toggles. Verifies the toolchain end-to-end.',
    accent: '#3DD68C',
    icon: '⚡',
  },
  {
    id: 'adxl345-sensor-lab',
    name: 'ADXL345 Tilt',
    chip: 'STM32F103 · I²C',
    description: 'Read 3-axis accelerometer data from a real I²C device model.',
    detail: 'Register-level ADXL345 implementation responding to firmware I²C reads.',
    accent: '#F062B8',
    icon: '📊',
  },
  {
    id: 'mpu6050-sensor-lab',
    name: 'MPU6050 IMU',
    chip: 'STM32F103 · I²C',
    description: '6-DoF accelerometer + gyroscope over I²C.',
    detail: 'WHO_AM_I check + continuous accel/gyro loop. Full register state machine in the core.',
    accent: '#B07BFF',
    icon: '🧭',
  },
  {
    id: 'bme280-weather-lab',
    name: 'BME280 Weather',
    chip: 'STM32F103 · I²C',
    description: 'Temperature / humidity / pressure environmental sensor.',
    detail: 'Bosch BME280 with factory calibration coefficients. Firmware runs the full compensation pipeline.',
    accent: '#3DD68C',
    icon: '🌡',
  },
  {
    id: 'ssd1306-hello-lab',
    name: 'OLED Hello',
    chip: 'STM32F103 · I²C',
    description: 'SSD1306 128×64 monochrome OLED with live framebuffer rendering.',
    detail: 'Full GDDRAM + addressing-mode state machine. Pixels render live in the inspector.',
    accent: '#5BD8FF',
    icon: '📺',
  },
  {
    id: 'max31855-thermocouple-lab',
    name: 'MAX31855',
    chip: 'STM32F103 · SPI',
    description: 'K-type thermocouple amplifier — read-only SPI device.',
    detail: 'Demonstrates the SPI device-attach plumbing. 32-bit response with TC + cold-junction temps.',
    accent: '#F5B642',
    icon: '🔥',
  },
  {
    id: 'neo6m-gps-lab',
    name: 'NEO-6M GPS',
    chip: 'STM32F103 · UART',
    description: 'GPS module streaming NMEA sentences over UART RX.',
    detail: 'GGA + RMC sentences with XOR checksum, generated entirely in the Rust core. Firmware echoes the stream.',
    accent: '#B07BFF',
    icon: '📡',
  },
  {
    id: 'ntc-thermistor-lab',
    name: 'NTC Thermistor',
    chip: 'STM32F103 · ADC',
    description: 'Analog temperature sensor with Steinhart-Hart math.',
    detail: '10kΩ NTC + 10kΩ pulldown @ 3.3V. Slider injects °C; core computes mV and ADC count.',
    accent: '#F5B642',
    icon: '🌡️',
  },
  {
    id: 'ili9341-tft-lab',
    name: 'TFT Color',
    chip: 'STM32F103 · SPI',
    description: 'ILI9341 240×320 RGB565 color TFT display.',
    detail: 'Full ILI9341 protocol state machine + 153KB framebuffer + live RGB565 canvas decode.',
    accent: '#F062B8',
    icon: '🎨',
  },
  {
    id: 'nucleo-f401re',
    name: 'Nucleo-F401RE',
    chip: 'STM32F4 · Cortex-M4F',
    description: 'Nucleo dev board with LED + user button.',
    detail: 'Higher-performance Cortex-M4 with FPU. Demonstrates LabWired\'s coverage of the STM32F4 family.',
    accent: '#5B9DFF',
    icon: '🔵',
  },
];

const STATUS_LABEL: Record<SupportedBoard['status'], string> = {
  'working-labs': 'Working labs',
  'bring-your-own': 'Bring your own ELF',
  roadmap: 'Roadmap',
};

const STATUS_COLOR: Record<SupportedBoard['status'], string> = {
  'working-labs': '#3DD68C',
  'bring-your-own': '#5B9DFF',
  roadmap: '#F5B642',
};

export function Library() {
  return (
    <div className="min-h-screen bg-bg-base text-fg-primary font-sans">
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
          <a href="./" className="text-fg-secondary hover:text-fg-primary transition-colors duration-150">Playground</a>
          <a href="ci.html" className="text-fg-secondary hover:text-fg-primary transition-colors duration-150">For CI</a>
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

      <section className="px-6 pt-20 pb-12 max-w-[1120px] mx-auto">
        <div className="inline-flex items-center gap-2 text-[11px] uppercase tracking-[0.12em] text-magenta font-semibold mb-5">
          <span className="w-1.5 h-1.5 rounded-full bg-magenta" />
          The Library
        </div>
        <h1 className="text-[44px] md:text-[52px] leading-[1.05] font-bold tracking-tight max-w-[24ch]">
          Every supported board, every working lab.
        </h1>
        <p className="text-fg-secondary text-[17px] leading-[1.5] mt-5 max-w-[58ch]">
          LabWired covers multiple chip families across ARM Cortex-M, RISC-V, and Xtensa. Pick a
          board to start with a saved workspace, or jump straight into one of the curated labs.
        </p>
      </section>

      <section className="px-6 pb-16 max-w-[1120px] mx-auto">
        <div className="flex items-baseline justify-between mb-5 flex-wrap gap-2">
          <h2 className="text-[20px] font-semibold tracking-tight">Supported boards</h2>
          <div className="text-fg-tertiary text-[12px]">
            {SUPPORTED_BOARDS.length} chips · ARM Cortex-M0+ · M3 · M4 / M4F · M33 · RISC-V · Xtensa LX7
          </div>
        </div>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-3">
          {SUPPORTED_BOARDS.map((b) => (
            <a
              key={b.chip}
              href={b.playgroundBoardId ? `./?lab=${encodeURIComponent(b.playgroundBoardId)}` : './'}
              className="lw-glass p-4 transition-all duration-150 hover:bg-bg-elevated/70 hover:-translate-y-0.5 block"
            >
              <div className="flex items-baseline justify-between mb-2 gap-2">
                <div className="text-fg-primary font-mono font-semibold text-[14px] tracking-tight">
                  {b.chip}
                </div>
                <div
                  className="text-[9px] uppercase tracking-[0.08em] font-semibold px-2 py-0.5 rounded-pill"
                  style={{
                    color: STATUS_COLOR[b.status],
                    background: `${STATUS_COLOR[b.status]}1a`,
                    border: `1px solid ${STATUS_COLOR[b.status]}33`,
                  }}
                >
                  {STATUS_LABEL[b.status]}
                </div>
              </div>
              <div className="text-fg-secondary text-[13px] mb-1">{b.family}</div>
              <div className="text-fg-tertiary text-[11px] font-mono mb-2">
                {b.arch} · {b.vendor}
              </div>
              <div className="text-fg-tertiary text-[12px] leading-[1.5]">{b.notes}</div>
            </a>
          ))}
        </div>
      </section>

      <section className="px-6 pb-24 max-w-[1120px] mx-auto">
        <div className="flex items-baseline justify-between mb-5 flex-wrap gap-2">
          <h2 className="text-[20px] font-semibold tracking-tight">Featured labs</h2>
          <div className="text-fg-tertiary text-[12px]">
            {FEATURED_LABS.length} working firmware demos · click to run
          </div>
        </div>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-5">
          {FEATURED_LABS.map((lab) => (
            <a
              key={lab.id}
              href={`./?lab=${encodeURIComponent(lab.id)}`}
              className="group lw-glass p-5 transition-all duration-150 hover:bg-bg-elevated/70 hover:-translate-y-0.5 block"
            >
              <div
                className="aspect-[16/10] rounded-card mb-4 flex items-center justify-center relative overflow-hidden"
                style={{
                  background: `linear-gradient(135deg, ${lab.accent}22, ${lab.accent}08 60%, transparent)`,
                  border: `1px solid ${lab.accent}33`,
                }}
              >
                <div className="text-[64px] opacity-90" aria-hidden>{lab.icon}</div>
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

        <div className="lw-glass p-6 mt-8 flex flex-col md:flex-row items-start md:items-center gap-4 justify-between">
          <div>
            <h3 className="text-fg-primary text-[16px] font-semibold mb-1">Bring your own firmware</h3>
            <p className="text-fg-secondary text-[13px] leading-[1.5]">
              Compile locally with your existing toolchain. Drop your{' '}
              <code className="text-fg-primary font-mono text-[12px]">.elf</code> /{' '}
              <code className="text-fg-primary font-mono text-[12px]">.bin</code> /{' '}
              <code className="text-fg-primary font-mono text-[12px]">.hex</code> into the playground
              via the Upload button. Works against every supported chip above.
            </p>
          </div>
          <a
            href="./"
            className="h-9 px-5 rounded-pill bg-accent text-bg-base font-semibold hover:bg-accent-hover transition-colors duration-150 flex items-center shrink-0"
          >
            Open playground →
          </a>
        </div>
      </section>

      <footer className="px-6 py-10 border-t border-border/60">
        <div className="max-w-[1120px] mx-auto flex flex-wrap items-center justify-between gap-4 text-[12px] text-fg-tertiary">
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
            <a className="hover:text-fg-primary transition-colors" href="mailto:andrii@shylenko.com">Contact</a>
          </div>
        </div>
      </footer>
    </div>
  );
}
