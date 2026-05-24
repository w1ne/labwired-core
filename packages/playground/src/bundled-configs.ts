/**
 * Bundled board configurations for the playground.
 * These manifests are imported directly from core/, so the playground stays
 * aligned with the engine's source-of-truth chip/system definitions.
 */

import chipEsp32 from '../../../core/configs/chips/esp32.yaml?raw';
import chipEsp32c3 from '../../../core/configs/chips/esp32c3.yaml?raw';
import chipEsp32s3 from '../../../core/configs/chips/esp32s3.yaml?raw';
import chipNrf52840 from '../../../core/configs/chips/nrf52840.yaml?raw';
import chipRp2040 from '../../../core/configs/chips/rp2040.yaml?raw';
import chipStm32f103 from '../../../core/configs/chips/stm32f103.yaml?raw';
import chipStm32f401 from '../../../core/configs/chips/stm32f401.yaml?raw';
import chipStm32f401cdu6 from '../../../core/configs/chips/stm32f401cdu6.yaml?raw';
import chipStm32h563 from '../../../core/configs/chips/stm32h563.yaml?raw';
import systemEsp32c3Devkit from '../../../core/configs/systems/esp32c3-devkit.yaml?raw';
import systemEsp32s3Zero from '../../../core/configs/systems/esp32s3-zero.yaml?raw';
import systemNrf52840Dk from '../../../core/configs/systems/nrf52840-dk.yaml?raw';
import systemNucleoF401re from '../../../core/configs/systems/nucleo-f401re.yaml?raw';
import systemNucleoH563zi from '../../../core/configs/systems/nucleo-h563zi-demo.yaml?raw';
import systemRp2040Pico from '../../../core/configs/systems/rp2040-pico.yaml?raw';
import systemStm32f401cdu6Blackpill from '../../../core/configs/systems/stm32f401cdu6-blackpill.yaml?raw';
import systemStm32f103Blinky from '../../../core/examples/demo-blinky/system.yaml?raw';
import systemAdxl345SensorLab from '../../../core/examples/adxl345-sensor-lab/system.yaml?raw';
import systemMpu6050SensorLab from '../../../core/examples/mpu6050-sensor-lab/system.yaml?raw';
import systemBme280WeatherLab from '../../../core/examples/bme280-weather-lab/system.yaml?raw';
import systemMax31855ThermocoupleLab from '../../../core/examples/max31855-thermocouple-lab/system.yaml?raw';
import systemSsd1306HelloLab from '../../../core/examples/ssd1306-hello-lab/system.yaml?raw';
import systemNeo6mGpsLab from '../../../core/examples/neo6m-gps-lab/system.yaml?raw';
import systemNtcThermistorLab from '../../../core/examples/ntc-thermistor-lab/system.yaml?raw';
import systemIli9341TftLab from '../../../core/examples/ili9341-tft-lab/system.yaml?raw';
import systemEpaperTricolorLab from '../../../core/examples/epaper-tricolor-lab/system.yaml?raw';
import systemEsp32EpaperLab from '../../../core/examples/esp32-epaper-lab/system.yaml?raw';
import sourceBlinky from '../../../core/examples/demo-blinky/src/main.rs?raw';
import sourceAdxl345 from '../../../core/examples/adxl345-sensor-lab/src/main.rs?raw';
import sourceMpu6050 from '../../../core/examples/mpu6050-sensor-lab/src/main.rs?raw';
import sourceBme280 from '../../../core/examples/bme280-weather-lab/src/main.rs?raw';
import sourceMax31855 from '../../../core/examples/max31855-thermocouple-lab/src/main.rs?raw';
import sourceSsd1306 from '../../../core/examples/ssd1306-hello-lab/src/main.rs?raw';
import sourceNeo6mGps from '../../../core/examples/neo6m-gps-lab/src/main.rs?raw';
import sourceNtcThermistor from '../../../core/examples/ntc-thermistor-lab/src/main.rs?raw';
import sourceIli9341Tft from '../../../core/examples/ili9341-tft-lab/src/main.rs?raw';
import sourceEpaperTricolor from '../../../core/examples/epaper-tricolor-lab/src/main.rs?raw';
import sourceEsp32Epaper from '../../../core/examples/esp32-epaper-lab/src/main.rs?raw';

/**
 * Board-summary tooltip the playground shows above the canvas. `nextStep`
 * is rendered when the simulation is idle; `nextStepRunning` (optional) is
 * swapped in once the sim is active. Used by every demo — when omitted the
 * playground falls back to a generic "Click Run to start" hint.
 */
export interface BoardSummary {
  title: string;
  description: string;
  nextStep: string;
  nextStepRunning?: string;
}

export interface BoardConfig {
  boardId: string;
  chipId: string;
  name: string;
  description: string;
  arch: string;
  chipYaml: string;
  systemYaml: string;
  demoFirmwarePath?: string;
  mcuComponentType: string;
  /** Raw firmware source code, surfaced in the Dev drawer's Source tab. */
  sourceCode?: string;
  /** Filename shown alongside the Source tab. */
  sourceFilename?: string;
  /**
   * Project shape — drives BoardPicker grouping. `bare` = MCU only (you wire
   * everything). `lab` = full pre-wired project with peripheral on canvas and
   * demo firmware ready to Run.
   */
  kind?: 'bare' | 'lab';
  /**
   * Firmware-runtime quirks the simulator needs to apply before the first
   * `step`. `'esp32-arduino'` installs the heap-caps / timer / lock / WiFi /
   * sendHello / esp_crc8 thunks plus the dual-core handshake refresh that
   * every Arduino-ESP32 (ESP-IDF + Arduino core) firmware needs to reach
   * `app_main`. See `wasm/src/lib.rs::install_esp32_arduino_quirks` for the
   * canonical list.
   */
  quirks?: 'esp32-arduino';
  /**
   * Optional URL of a pre-warmed runtime snapshot (`.lwrs`). When set, the
   * playground fetches this blob right after loading the firmware ELF and
   * calls `simulator.applyRuntimeSnapshot` to skip the cold boot — a
   * 30 s warm-up collapses to one HTTP round-trip plus a few ms of
   * bincode decode. Produce with `labwired-cli snapshot capture`.
   */
  bootSnapshotUrl?: string;
  /**
   * Default scale factor for the lab's main display component (e.g. the
   * SSD1680 e-paper face). Used by `App.tsx` when seeding the diagram —
   * 2x for tiny-font OLED/e-paper panels so text glyphs stay legible.
   */
  panelScale?: number;
  /** Board-aware summary tooltip shown above the canvas. */
  summary?: BoardSummary;
  /** Board-aware "Click Run to start" hint shown next to the SimDock. */
  runHint?: string;
}

const BASE = import.meta.env.BASE_URL;

export const BOARD_CONFIGS: BoardConfig[] = [
  {
    boardId: 'ntc-thermistor-lab',
    chipId: 'stm32f103',
    name: 'NTC Thermistor',
    description: 'STM32F103 + NTC 3950 thermistor on ADC1 ch0. Steinhart-Hart Beta equation in Rust core. Slide the temperature and watch the ADC count change.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemNtcThermistorLab,
    demoFirmwarePath: `${BASE}wasm/demo-ntc-thermistor-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceNtcThermistor,
    sourceFilename: 'ntc-thermistor-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'neo6m-gps-lab',
    chipId: 'stm32f103',
    name: 'NEO-6M GPS',
    description: 'STM32F103 + NEO-6M GPS module over simulated UART. Live NMEA stream injection, all parsing in Rust core.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemNeo6mGpsLab,
    demoFirmwarePath: `${BASE}wasm/demo-neo6m-gps-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceNeo6mGps,
    sourceFilename: 'neo6m-gps-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'ssd1306-hello-lab',
    chipId: 'stm32f103',
    name: 'SSD1306 OLED',
    description: 'STM32F103 + SSD1306 128×64 OLED display over simulated I²C. Live pixel rendering.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemSsd1306HelloLab,
    demoFirmwarePath: `${BASE}wasm/demo-ssd1306-hello-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceSsd1306,
    sourceFilename: 'ssd1306-hello-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'bme280-weather-lab',
    chipId: 'stm32f103',
    name: 'BME280 Weather',
    description: 'STM32F103 + BME280 temperature/humidity/pressure sensor over simulated I²C.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemBme280WeatherLab,
    demoFirmwarePath: `${BASE}wasm/demo-bme280-weather-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceBme280,
    sourceFilename: 'bme280-weather-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'ili9341-tft-lab',
    chipId: 'stm32f103',
    name: 'ILI9341 TFT Color',
    description: 'STM32F103 + ILI9341 240×320 RGB565 TFT display over simulated SPI. Live color framebuffer rendering.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemIli9341TftLab,
    demoFirmwarePath: `${BASE}wasm/demo-ili9341-tft-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceIli9341Tft,
    sourceFilename: 'ili9341-tft-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'epaper-tricolor-lab',
    chipId: 'stm32f103',
    name: 'E-Paper 2.9" Tri-color',
    description: 'STM32F103 + Waveshare 2.9" SSD1680 tri-color e-paper over simulated SPI. Same firmware ELF flashes to a real NUCLEO-F103RB + Waveshare panel for side-by-side digital-twin verification.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemEpaperTricolorLab,
    demoFirmwarePath: `${BASE}wasm/demo-epaper-tricolor-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceEpaperTricolor,
    sourceFilename: 'epaper-tricolor-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'esp32-epaper-lab',
    chipId: 'esp32',
    name: 'ESP32 E-Reader',
    description: 'ESP32-WROOM-32 + Waveshare 2.9" SSD1680 tri-color e-paper over simulated VSPI. Same Rust no_std ELF flashes to a real ESP32 module via espflash for side-by-side digital-twin verification.',
    arch: 'Xtensa LX6',
    chipYaml: chipEsp32,
    systemYaml: systemEsp32EpaperLab,
    demoFirmwarePath: `${BASE}wasm/demo-esp32-epaper-lab.elf`,
    mcuComponentType: 'esp32',
    sourceCode: sourceEsp32Epaper,
    sourceFilename: 'esp32-epaper-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'max31855-thermocouple-lab',
    chipId: 'stm32f103',
    name: 'MAX31855 Thermocouple',
    description: 'STM32F103 + MAX31855 K-type thermocouple interface over simulated SPI. Live temperature reading.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemMax31855ThermocoupleLab,
    demoFirmwarePath: `${BASE}wasm/demo-max31855-thermocouple-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceMax31855,
    sourceFilename: 'max31855-thermocouple-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'mpu6050-sensor-lab',
    chipId: 'stm32f103',
    name: 'MPU6050 IMU',
    description: 'STM32F103 + MPU6050 6-DoF IMU over simulated I²C. Reads accel + gyro.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemMpu6050SensorLab,
    demoFirmwarePath: `${BASE}wasm/demo-mpu6050-sensor-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceMpu6050,
    sourceFilename: 'mpu6050-sensor-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'adxl345-sensor-lab',
    chipId: 'stm32f103',
    name: 'ADXL345 Sensor Lab',
    description: 'Guided STM32F103 + ADXL345 accelerometer lab over simulated I2C.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemAdxl345SensorLab,
    demoFirmwarePath: `${BASE}wasm/demo-adxl345-sensor-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceAdxl345,
    sourceFilename: 'adxl345-sensor-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'stm32f103-blinky',
    chipId: 'stm32f103',
    name: 'STM32F103 Blinky',
    description: 'Classic LED blink on Cortex-M3. Toggles PA5 via GPIO.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemStm32f103Blinky,
    demoFirmwarePath: `${BASE}wasm/demo-blinky.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceBlinky,
    sourceFilename: 'demo-blinky/src/main.rs',
  },
  {
    boardId: 'nucleo-f401re',
    chipId: 'stm32f401',
    name: 'Nucleo-F401RE',
    description: 'STM32F4 Nucleo board with LED on PA5 and user button on PC13.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipStm32f401,
    systemYaml: systemNucleoF401re,
    demoFirmwarePath: `${BASE}wasm/demo-nucleo-f401.elf`,
    mcuComponentType: 'stm32-dev',
  },
  {
    boardId: 'stm32f401cdu6-blackpill',
    chipId: 'stm32f401cdu6',
    name: 'STM32F401CDU6 Black Pill',
    description: 'Compact STM32F401CDU6 Black Pill board with active-low PC13 LED.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipStm32f401cdu6,
    systemYaml: systemStm32f401cdu6Blackpill,
    mcuComponentType: 'stm32-dev',
  },
  {
    boardId: 'nucleo-h563zi',
    chipId: 'stm32h563',
    name: 'Nucleo-H563ZI',
    description: 'STM32H5 Nucleo-144 board with 3 LEDs and a user button.',
    arch: 'ARM Cortex-M33',
    chipYaml: chipStm32h563,
    systemYaml: systemNucleoH563zi,
    mcuComponentType: 'stm32-dev',
  },
  {
    boardId: 'esp32c3-supermini',
    chipId: 'esp32c3',
    name: 'ESP32-C3 Super Mini',
    description: 'Compact RISC-V ESP32-C3 Super Mini board with USB-C. Built-in user LED on GPIO8 (active-low).',
    arch: 'RISC-V',
    chipYaml: chipEsp32c3,
    systemYaml: systemEsp32c3Devkit,
    mcuComponentType: 'esp32-c3-supermini',
  },
  {
    boardId: 'esp32s3-zero',
    chipId: 'esp32s3',
    name: 'ESP32-S3-Zero',
    description: 'Xtensa LX7 dual-core ESP32-S3 with USB-C. RGB LED on GPIO48.',
    arch: 'Xtensa LX7',
    chipYaml: chipEsp32s3,
    systemYaml: systemEsp32s3Zero,
    mcuComponentType: 'esp32-s3-zero',
  },
  {
    boardId: 'rp2040-pico',
    chipId: 'rp2040',
    name: 'Raspberry Pi Pico',
    description: 'RP2040 dual-core ARM Cortex-M0+ board.',
    arch: 'ARM Cortex-M0+',
    chipYaml: chipRp2040,
    systemYaml: systemRp2040Pico,
    mcuComponentType: 'rpi-pico',
  },
  {
    boardId: 'nrf52840-dk',
    chipId: 'nrf52840',
    name: 'nRF52840 DK',
    description: 'Nordic nRF52840 dev kit with BLE.',
    arch: 'ARM Cortex-M4F',
    chipYaml: chipNrf52840,
    systemYaml: systemNrf52840Dk,
    mcuComponentType: 'nrf52840-dk',
  },
];

export const BOARD_CONFIG_MAP = new Map(BOARD_CONFIGS.map((c) => [c.boardId, c]));

export function getBoardConfigForChip(chipId: string): BoardConfig | undefined {
  return BOARD_CONFIGS.find((c) => c.chipId === chipId);
}
