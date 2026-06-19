/**
 * The Library — comprehensive catalog of supported boards and labs.
 * Replaces the older Gallery (featured-labs-only) framing.
 *
 * Styled to match the LabWired marketing landing (light / neo-brutalist).
 * Token overrides live in ../ci/ci-light.css, imported via library.entry.tsx.
 */

import { GlobalLogo, GlobalNav } from '../components/GlobalNav';
import { GlobalFooter } from '../components/GlobalFooter';
import { BOARD_CONFIGS } from '../bundled-configs';

// The Library is served on labwired.com (marketing), but the playground lives on
// app.labwired.com — so tiles must link there absolutely. A relative ./?lab=
// resolves to the marketing origin (which has no playground), opening the
// homepage / last-cached project instead of the lab. Same-origin in dev.
const PLAYGROUND_URL = import.meta.env.DEV ? '' : 'https://app.labwired.com';

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

// Library-only presentation for a featured lab. The lab's name, description
// and existence come from BOARD_CONFIGS (single source of truth); this only
// curates which labs are featured (and their order) plus pure decoration.
interface FeaturedLabPresentation {
  id: string;
  chip: string;
  detail: string;
  accent: string;
  icon: string;
}

const CURATED_LABS: FeaturedLabPresentation[] = [
  {
    id: 'stm32f103-blinky',
    chip: 'STM32F103',
    detail: 'Bare-metal Rust toggling GPIOA_ODR. ~16k cycles between toggles. Verifies the toolchain end-to-end.',
    accent: '#27c93f',
    icon: '⚡',
  },
  {
    id: 'adxl345-sensor-lab',
    chip: 'STM32F103 · I²C',
    detail: 'Register-level ADXL345 implementation responding to firmware I²C reads.',
    accent: '#d63384',
    icon: '📊',
  },
  {
    id: 'mpu6050-sensor-lab',
    chip: 'STM32F103 · I²C',
    detail: 'WHO_AM_I check + continuous accel/gyro loop. Full register state machine in the core.',
    accent: '#7e3ff2',
    icon: '🧭',
  },
  {
    id: 'vl53l1x-tof-lab',
    chip: 'STM32F103 · I²C',
    detail: 'Laser time-of-flight ranging: MODEL_ID check, start ranging, poll data-ready, read range in mm and trip a NEAR/FAR flag.',
    accent: '#0d9488',
    icon: '📏',
  },
  {
    id: 'bme280-weather-lab',
    chip: 'STM32F103 · I²C',
    detail: 'Bosch BME280 with factory calibration coefficients. Firmware runs the full compensation pipeline.',
    accent: '#27c93f',
    icon: '🌡',
  },
  {
    id: 'ssd1306-hello-lab',
    chip: 'STM32F103 · I²C',
    detail: 'Full GDDRAM + addressing-mode state machine. Pixels render live in the inspector.',
    accent: '#0056b3',
    icon: '📺',
  },
  {
    id: 'max31855-thermocouple-lab',
    chip: 'STM32F103 · SPI',
    detail: 'Demonstrates the SPI device-attach plumbing. 32-bit response with TC + cold-junction temps.',
    accent: '#ffbd2e',
    icon: '🔥',
  },
  {
    id: 'neo6m-gps-lab',
    chip: 'STM32F103 · UART',
    detail: 'GGA + RMC sentences with XOR checksum, generated entirely in the Rust core. Firmware echoes the stream.',
    accent: '#7e3ff2',
    icon: '📡',
  },
  {
    id: 'quectel-bg770a-lab',
    chip: 'STM32F103 · UART',
    detail: 'Byte-exact V.250 + Quectel +QI*/+QMT*/+QHTTP*/+QGPS*/+QSSL* state machines, validated against real BG770A-GL hardware captures. Firmware sends AT commands, modem replies stream back over UART2.',
    accent: '#3ec1d3',
    icon: '📶',
  },
  {
    id: 'ntc-thermistor-lab',
    chip: 'STM32F103 · ADC',
    detail: '10kΩ NTC + 10kΩ pulldown @ 3.3V. Slider injects °C; core computes mV and ADC count.',
    accent: '#ffbd2e',
    icon: '🌡️',
  },
  {
    id: 'ili9341-tft-lab',
    chip: 'STM32F103 · SPI',
    detail: 'Full ILI9341 protocol state machine + 153KB framebuffer + live RGB565 canvas decode.',
    accent: '#d63384',
    icon: '🎨',
  },
  {
    id: 'nucleo-f401re',
    chip: 'STM32F4 · Cortex-M4F',
    detail: 'Higher-performance Cortex-M4 with FPU. Demonstrates LabWired\'s coverage of the STM32F4 family.',
    accent: '#0056b3',
    icon: '🔵',
  },
  {
    id: 'labwired-ereader',
    chip: 'ESP32-WROOM-32 · Xtensa LX6',
    detail: 'GxEPD2 + Adafruit_GFX + FreeRTOS on dual-core Xtensa. The exact same .elf flashes to physical hardware via espflash.',
    accent: '#d63384',
    icon: '📖',
  },
  {
    id: 'esp32-epaper-lab',
    chip: 'ESP32-WROOM-32 · Xtensa LX6',
    detail: 'ESP32 VSPI to SSD1680 with full controller state machine. Same ELF runs in the sim and flashes to a real ESP32 module.',
    accent: '#7e3ff2',
    icon: '🖼',
  },
  {
    id: 'epaper-tricolor-lab',
    chip: 'STM32F103 · Cortex-M3',
    detail: 'Same SSD1680 model as the ESP32 lab, exercised from a different MCU + arch. Side-by-side digital-twin verification.',
    accent: '#27c93f',
    icon: '📰',
  },
  {
    id: 'nokia5110-invaders-lab',
    chip: 'STM32L476 · Cortex-M4',
    detail: 'The full PCD8544 framebuffer model — the bus resolves the D/C pin to its driving GPIO ODR address at attach time, then commands vs data are decoded purely from SPI transactions.',
    accent: '#ffbd2e',
    icon: '🕹️',
  },
  {
    id: 'al2205-iolink-dido',
    chip: 'STM32L476 · Cortex-M4',
    detail: 'Full IO-Link master state machine in Rust core (wake-up → startup → operate cycles, m-sequence types, CRC6). A 74HC165 shift register surfaces the field-side switch state for the firmware to read over SPI.',
    accent: '#3ec1d3',
    icon: '🔌',
  },
];

// Featured labs = curated presentation joined to BOARD_CONFIGS. Names,
// descriptions and links come from BOARD_CONFIGS, so a lab can never drift from
// the real board, and a tile can never deep-link to a board that doesn't exist.
export const FEATURED_LABS = CURATED_LABS.flatMap((p) => {
  const cfg = BOARD_CONFIGS.find((b) => b.boardId === p.id);
  if (!cfg) return [];
  return [{ ...p, name: cfg.name, description: cfg.description }];
});

const STATUS_LABEL: Record<SupportedBoard['status'], string> = {
  'working-labs': 'Working labs',
  'bring-your-own': 'Bring your own ELF',
  roadmap: 'Roadmap',
};

const STATUS_COLOR: Record<SupportedBoard['status'], string> = {
  'working-labs': '#27c93f',
  'bring-your-own': '#0056b3',
  roadmap: '#ffbd2e',
};

export function Library() {
  return (
    <div className="min-h-screen bg-bg-base text-fg-primary font-sans">
      <header className="lw-chrome">
        <GlobalLogo />
        <span className="text-fg-tertiary text-[12px] hidden md:inline tracking-[0.01em]">
          Deterministic firmware simulation
        </span>
        <div className="flex-1" />
        <GlobalNav active="library" />
      </header>

      <section className="px-6 pt-20 pb-12 max-w-[1120px] mx-auto">
        <div className="lw-kicker-pill mb-6">
          <span className="lw-kicker-dot" />
          The Library
        </div>
        <h1 className="text-[44px] md:text-[56px] leading-[1.05] font-bold tracking-tight max-w-[24ch] text-fg-primary">
          Every supported board,{' '}
          <span className="text-accent">every working lab.</span>
        </h1>
        <p className="text-fg-secondary text-[18px] leading-[1.5] mt-6 max-w-[58ch]">
          LabWired covers multiple chip families across ARM Cortex-M, RISC-V, and Xtensa. Pick a
          board to start with a saved workspace, or jump straight into one of the curated labs.
        </p>
      </section>

      <section className="px-6 pb-16 max-w-[1120px] mx-auto">
        <div className="flex items-baseline justify-between mb-6 flex-wrap gap-2">
          <h2 className="text-[24px] font-bold tracking-tight text-fg-primary">Supported boards</h2>
          <div className="text-fg-tertiary text-[12px] font-medium">
            {SUPPORTED_BOARDS.length} chips · ARM Cortex-M0+ · M3 · M4 / M4F · M33 · RISC-V · Xtensa LX7
          </div>
        </div>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-5">
          {SUPPORTED_BOARDS.map((b) => (
            <a
              key={b.chip}
              href={b.playgroundBoardId ? `${PLAYGROUND_URL}/?lab=${encodeURIComponent(b.playgroundBoardId)}` : `${PLAYGROUND_URL}/`}
              className="block bg-white border-2 border-[#1a1a1a] rounded-[10px] p-5 shadow-[5px_5px_0_#1a1a1a] transition-all duration-150 hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-[7px_7px_0_#1a1a1a]"
            >
              <div className="flex items-baseline justify-between mb-2 gap-2">
                <div className="text-fg-primary font-mono font-bold text-[15px] tracking-tight">
                  {b.chip}
                </div>
                <div
                  className="text-[9px] uppercase tracking-[0.1em] font-bold px-2 py-0.5 rounded-pill"
                  style={{
                    color: STATUS_COLOR[b.status],
                    background: `${STATUS_COLOR[b.status]}1a`,
                    border: `1.5px solid ${STATUS_COLOR[b.status]}`,
                  }}
                >
                  {STATUS_LABEL[b.status]}
                </div>
              </div>
              <div className="text-fg-primary text-[13px] mb-1 font-semibold">{b.family}</div>
              <div className="text-fg-tertiary text-[11px] font-mono mb-2">
                {b.arch} · {b.vendor}
              </div>
              <div className="text-fg-secondary text-[12.5px] leading-[1.5]">{b.notes}</div>
            </a>
          ))}
        </div>
      </section>

      <section className="px-6 pb-24 max-w-[1120px] mx-auto">
        <div className="flex items-baseline justify-between mb-6 flex-wrap gap-2">
          <h2 className="text-[24px] font-bold tracking-tight text-fg-primary">Featured labs</h2>
          <div className="text-fg-tertiary text-[12px] font-medium">
            {FEATURED_LABS.length} working firmware demos · click to run
          </div>
        </div>

        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-6">
          {FEATURED_LABS.map((lab) => (
            <a
              key={lab.id}
              href={`${PLAYGROUND_URL}/?lab=${encodeURIComponent(lab.id)}`}
              className="group block bg-white border-2 border-[#1a1a1a] rounded-[10px] p-5 shadow-[5px_5px_0_#1a1a1a] transition-all duration-150 hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-[7px_7px_0_#1a1a1a]"
            >
              <div
                className="aspect-[16/10] rounded-[8px] mb-4 flex items-center justify-center relative overflow-hidden border-2 border-[#1a1a1a]"
                style={{
                  background: `linear-gradient(135deg, ${lab.accent}33, ${lab.accent}11 60%, #ffffff)`,
                }}
              >
                <div className="text-[64px] opacity-90" aria-hidden>{lab.icon}</div>
              </div>
              <div className="text-fg-tertiary text-[10px] uppercase tracking-[0.12em] font-bold mb-1">
                {lab.chip}
              </div>
              <h3 className="text-fg-primary text-[18px] font-bold mb-1.5">{lab.name}</h3>
              <p className="text-fg-secondary text-[13px] leading-[1.5] mb-3">{lab.description}</p>
              <p className="text-fg-tertiary text-[12px] leading-[1.5]">{lab.detail}</p>
              <div
                className="mt-4 text-[13px] font-semibold transition-colors duration-150"
                style={{ color: lab.accent }}
              >
                Open in playground →
              </div>
            </a>
          ))}
        </div>

        <div className="mt-10 bg-white border-2 border-[#1a1a1a] rounded-[10px] p-7 shadow-[5px_5px_0_#1a1a1a] flex flex-col md:flex-row items-start md:items-center gap-5 justify-between">
          <div>
            <h3 className="text-fg-primary text-[18px] font-bold mb-1">Bring your own firmware</h3>
            <p className="text-fg-secondary text-[13.5px] leading-[1.55] max-w-[58ch]">
              Compile locally with your existing toolchain. Drop your{' '}
              <code className="text-fg-primary font-mono text-[12px] px-1.5 py-0.5 bg-[#f8f9fa] border border-[#d6d8dc] rounded">.elf</code> /{' '}
              <code className="text-fg-primary font-mono text-[12px] px-1.5 py-0.5 bg-[#f8f9fa] border border-[#d6d8dc] rounded">.bin</code> /{' '}
              <code className="text-fg-primary font-mono text-[12px] px-1.5 py-0.5 bg-[#f8f9fa] border border-[#d6d8dc] rounded">.hex</code> into the playground
              via the Upload button. Works against every supported chip above.
            </p>
          </div>
          <a href="./" className="lw-cta-primary shrink-0">
            Open playground &rarr;
          </a>
        </div>
      </section>

      <GlobalFooter />
    </div>
  );
}
