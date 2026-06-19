/**
 * Bundled board configurations for the playground.
 * These manifests are imported directly from core/, so the playground stays
 * aligned with the engine's source-of-truth chip/system definitions.
 */

import chipEsp32 from '../../../core/configs/chips/esp32.yaml?raw';
import chipEsp32c3 from '../../../core/configs/chips/esp32c3.yaml?raw';
import chipEsp32s3 from '../../../core/configs/chips/esp32s3.yaml?raw';
import chipNrf52840 from '../../../core/configs/chips/nrf52840.yaml?raw';
import chipNrf52840Onboarding from '../../../core/configs/chips/onboarding/nrf52840.yaml?raw';
import chipRp2040 from '../../../core/configs/chips/rp2040.yaml?raw';
import chipStm32f103 from '../../../core/configs/chips/stm32f103.yaml?raw';
import chipStm32f401 from '../../../core/configs/chips/stm32f401.yaml?raw';
import chipStm32f401cdu6 from '../../../core/configs/chips/stm32f401cdu6.yaml?raw';
import chipStm32h563 from '../../../core/configs/chips/stm32h563.yaml?raw';
import chipStm32l476 from '../../../core/configs/chips/stm32l476.yaml?raw';
import systemEsp32c3Devkit from '../../../core/configs/systems/esp32c3-devkit.yaml?raw';
import systemEsp32s3Zero from '../../../core/configs/systems/esp32s3-zero.yaml?raw';
import systemNrf52840Dk from '../../../core/configs/systems/nrf52840-dk.yaml?raw';
import systemNrf52840Onboarding from '../../../core/configs/systems/onboarding/nrf52840.yaml?raw';
import systemNrf52840Proximity from '../../../core/examples/nrf52840-proximity-lab/system.yaml?raw';
import systemNucleoF401re from '../../../core/configs/systems/nucleo-f401re.yaml?raw';
import systemNucleoH563zi from '../../../core/configs/systems/nucleo-h563zi-demo.yaml?raw';
import systemNucleoL476rg from '../../../core/configs/systems/nucleo-l476rg.yaml?raw';
import systemRp2040Pico from '../../../core/configs/systems/rp2040-pico.yaml?raw';
import systemStm32f401cdu6Blackpill from '../../../core/configs/systems/stm32f401cdu6-blackpill.yaml?raw';
import systemStm32f103Blinky from '../../../core/examples/demo-blinky/system.yaml?raw';
import systemAdxl345SensorLab from '../../../core/examples/adxl345-sensor-lab/system.yaml?raw';
import systemMpu6050SensorLab from '../../../core/examples/mpu6050-sensor-lab/system.yaml?raw';
import systemBme280WeatherLab from '../../../core/examples/bme280-weather-lab/system.yaml?raw';
import systemMax31855ThermocoupleLab from '../../../core/examples/max31855-thermocouple-lab/system.yaml?raw';
import systemSsd1306HelloLab from '../../../core/examples/ssd1306-hello-lab/system.yaml?raw';
import systemNokia5110InvadersLab from '../../../core/examples/nokia5110-invaders-lab/system.yaml?raw';
import systemNeo6mGpsLab from '../../../core/examples/neo6m-gps-lab/system.yaml?raw';
import systemQuectelBg770aLab from '../../../core/examples/quectel-bg770a-lab/system.yaml?raw';
import systemNtcThermistorLab from '../../../core/examples/ntc-thermistor-lab/system.yaml?raw';
import systemIli9341TftLab from '../../../core/examples/ili9341-tft-lab/system.yaml?raw';
import systemEpaperTricolorLab from '../../../core/examples/epaper-tricolor-lab/system.yaml?raw';
import systemEsp32EpaperLab from '../../../core/examples/esp32-epaper-lab/system.yaml?raw';
import systemAl2205IolinkDido from '../../../core/examples/al2205-iolink-dido/system.yaml?raw';
import systemH563UdsEcu from '../../../core/examples/h563-uds-ecu/system.yaml?raw';
import systemF103UdsEcu from '../../../core/examples/f103-uds-ecu/system.yaml?raw';
import sourceBlinky from '../../../core/examples/demo-blinky/src/main.rs?raw';
import sourceAl2205IolinkDido from '../../../core/examples/al2205-iolink-dido/firmware/main.c?raw';
import sourceH563UdsEcu from '../../../core/examples/h563-uds-ecu/firmware/main.c?raw';
import sourceF103UdsEcu from '../../../core/examples/f103-uds-ecu/firmware/main.c?raw';
import sourceAdxl345 from '../../../core/examples/adxl345-sensor-lab/src/main.rs?raw';
import sourceMpu6050 from '../../../core/examples/mpu6050-sensor-lab/src/main.rs?raw';
import sourceBme280 from '../../../core/examples/bme280-weather-lab/src/main.rs?raw';
import sourceMax31855 from '../../../core/examples/max31855-thermocouple-lab/src/main.rs?raw';
import sourceSsd1306 from '../../../core/examples/ssd1306-hello-lab/src/main.rs?raw';
import sourceNokia5110Invaders from '../../../core/examples/nokia5110-invaders-lab/src/main.rs?raw';
import sourceNrf52840Proximity from '../../../core/crates/firmware-nrf52840-proximity/src/main.rs?raw';
import sourceNeo6mGps from '../../../core/examples/neo6m-gps-lab/src/main.rs?raw';
import sourceQuectelBg770a from '../../../core/examples/quectel-bg770a-lab/src/main.rs?raw';
import sourceNtcThermistor from '../../../core/examples/ntc-thermistor-lab/src/main.rs?raw';
import sourceIli9341Tft from '../../../core/examples/ili9341-tft-lab/src/main.rs?raw';
import sourceEpaperTricolor from '../../../core/examples/epaper-tricolor-lab/src/main.rs?raw';
import sourceEsp32Epaper from '../../../core/examples/esp32-epaper-lab/src/main.rs?raw';
import systemEsp32WroomEpaper from '../../../core/configs/systems/esp32-wroom-epaper.yaml?raw';
import sourceLabwiredEreader from '../../../core/examples/labwired-ereader-arduino/labwired-ereader.ino?raw';

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
  quirks?: 'esp32-arduino' | 'arduino-esp32-autodiscover';
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
  /** Hidden from user-facing board lists (still resolvable by boardId — e.g. firmware for a sub-part of a multi-board lab). */
  hidden?: boolean;
  /**
   * Instruments to auto-open when this board loads, so a shared lab shows its
   * output immediately instead of a blank canvas. Currently honors
   * `'logic-analyzer'` (opens the analyzer panel).
   */
  openInstruments?: Array<'logic-analyzer'>;
  /**
   * Run from the bundled system YAML, not one regenerated from the diagram.
   * Needed when the lab's system YAML declares virtual devices the diagram
   * emitter can't reproduce — e.g. the multi-frame `uds-tester` external device,
   * which the diagram path would emit as a single-frame `can-diagnostic-tester`
   * instead, so the CAN exchange (and its frames) never happens in-browser.
   */
  preferBundledSystem?: boolean;
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
    boardId: 'quectel-bg770a-lab',
    chipId: 'stm32f103',
    name: 'Quectel BG770A Cellular',
    description: 'STM32F103 + Quectel BG770A-GL LTE-M / NB-IoT modem over simulated UART. Byte-exact AT command surface, V.250 echo/CMEE modes, MQTT/HTTP/GPS state machines.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemQuectelBg770aLab,
    demoFirmwarePath: `${BASE}wasm/demo-quectel-bg770a-lab.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceQuectelBg770a,
    sourceFilename: 'quectel-bg770a-lab/src/main.rs',
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
    boardId: 'nokia5110-invaders-lab',
    chipId: 'stm32l476',
    name: 'Nokia 5110 Breakout',
    description:
      'STM32L476 Breakout on a Nokia 5110 (PCD8544 84×48 LCD) over SPI, with an HC-SR04 ultrasonic sensor — adjust Distance (cm) to steer the paddle.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipStm32l476,
    systemYaml: systemNokia5110InvadersLab,
    demoFirmwarePath: `${BASE}wasm/demo-nokia5110-invaders-lab.elf`,
    mcuComponentType: 'nucleo-l476rg',
    sourceCode: sourceNokia5110Invaders,
    sourceFilename: 'nokia5110-invaders-lab/src/main.rs',
    kind: 'lab',
  },
  {
    boardId: 'al2205-iolink-dido',
    chipId: 'stm32l476',
    name: 'IO-Link DI Device (AL2205)',
    description:
      'STM32L476 running the real iolinki IO-Link device stack as firmware: 8 digital inputs via a 74HC165 shift register, read over SPI and published as IO-Link process data to a master peer. Toggle the inputs and watch the process data update at the master.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipStm32l476,
    systemYaml: systemAl2205IolinkDido,
    demoFirmwarePath: `${BASE}wasm/demo-al2205-iolink-dido.elf`,
    mcuComponentType: 'nucleo-l476rg',
    sourceCode: sourceAl2205IolinkDido,
    sourceFilename: 'al2205-iolink-dido/firmware/main.c',
    kind: 'lab',
  },
  {
    boardId: 'stm32h5-uds-ecu',
    chipId: 'stm32h563',
    name: 'STM32H5 UDS ECU',
    description:
      'STM32H5 ECU example on the H563 model: virtual diagnostic tester, reusable CAN transceiver block, UDSLib ISO-TP over CAN-FD, and ReadDataByIdentifier 0xF190.',
    arch: 'ARM Cortex-M33',
    chipYaml: chipStm32h563,
    systemYaml: systemH563UdsEcu,
    demoFirmwarePath: `${BASE}wasm/demo-stm32h5-uds-ecu.elf`,
    mcuComponentType: 'nucleo-h563zi',
    sourceCode: sourceH563UdsEcu,
    sourceFilename: 'h563-uds-ecu/firmware/main.c',
    kind: 'lab',
    summary: {
      title: 'STM32H5 UDS ECU',
      description: 'A minimal diagnostic ECU driven by a reusable virtual CAN diagnostic tester over the FDCAN model.',
      nextStep: 'Click Run to let the tester send UDS 0x22 F190 and watch the logic analyzer decode CAN_H/CAN_L traffic.',
      nextStepRunning: 'Running UDS over the simulated CAN bus.',
    },
    runHint: 'Click Run - the virtual tester sends 0x22 F190 and the ECU answers with VIN DID 0xF190.',
  },
  {
    boardId: 'f103-uds-ecu',
    chipId: 'stm32f103',
    name: 'STM32F103 bxCAN UDS ECU (fixed)',
    description:
      'STM32F103 ECU on a real two-node CAN bus: a virtual UDS tester drives a multi-frame SecurityAccess request and real UDSLib (normal-mode bxCAN, valid timing + acceptance filter) reassembles it and answers. The working build of udslib issue #29.',
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemF103UdsEcu,
    demoFirmwarePath: `${BASE}wasm/demo-f103-uds-ecu.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceF103UdsEcu,
    sourceFilename: 'f103-uds-ecu/firmware/main.c',
    kind: 'lab',
    hidden: true,
    openInstruments: ['logic-analyzer'],
    preferBundledSystem: true,
    summary: {
      title: 'STM32F103 bxCAN UDS ECU (fixed)',
      description:
        'Real UDSLib on the F103 bxCAN model reassembling a multi-frame (FF+CF) SecurityAccess request over a real two-node classical-CAN bus.',
      nextStep:
        'Click Run - the virtual tester sends the FirstFrame + ConsecutiveFrame and the ECU answers; watch the logic analyzer decode the full UDS exchange.',
      nextStepRunning: 'Running multi-frame UDS over the simulated two-node bxCAN bus.',
    },
    runHint:
      'Click Run - the FF (27 01 ...) + CF reassemble and the ECU replies with the SecurityAccess seed (67 01 DE AD BE EF).',
  },
  {
    boardId: 'f103-uds-ecu-broken',
    chipId: 'stm32f103',
    name: 'STM32F103 bxCAN UDS ECU (broken)',
    description:
      "The BROKEN build that reproduces udslib issue #29 live: the ECU's ISO-TP N_Cr timer is armed from a clock reading 0 while uds_tp_isotp_process() is fed a real tick, so the multi-frame session is torn down right after the FirstFrame. The tester sends FF (ECU answers FlowControl) then CF — but the CF is dropped and the ECU never answers.",
    arch: 'ARM Cortex-M3',
    chipYaml: chipStm32f103,
    systemYaml: systemF103UdsEcu,
    demoFirmwarePath: `${BASE}wasm/demo-f103-uds-ecu-broken.elf`,
    mcuComponentType: 'stm32-dev',
    sourceCode: sourceF103UdsEcu,
    sourceFilename: 'f103-uds-ecu/firmware/main.c (-DBROKEN_NCR)',
    kind: 'lab',
    hidden: true,
    openInstruments: ['logic-analyzer'],
    preferBundledSystem: true,
    summary: {
      title: 'STM32F103 bxCAN UDS ECU (broken)',
      description:
        'The N_Cr clock-mismatch bug from udslib issue #29, reproduced live: FirstFrame + FlowControl, then the ConsecutiveFrame is dropped and the ECU goes silent.',
      nextStep:
        'Click Run - the analyzer shows the FF and the FlowControl, then no response. Compare with the (fixed) lab where the seed comes back.',
      nextStepRunning: 'Reproducing the dropped-consecutive-frame bug on the two-node bxCAN bus.',
    },
    runHint:
      'Click Run - watch the FF + FlowControl appear, then silence (the multi-frame is dropped). This is the reporter’s "no response" bug.',
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
    // Superseded by labwired-ereader (Arduino-ESP32 + GxEPD2); kept resolvable
    // by boardId for legacy URLs but hidden from all user-facing pickers.
    hidden: true,
  },
  {
    boardId: 'esp32-epaper-lab',
    chipId: 'esp32',
    name: 'ESP32 + E-Paper (Rust)',
    description: 'ESP32-WROOM-32 + Waveshare 2.9" SSD1680 tri-color e-paper over simulated VSPI. Pure-Rust no_std implementation. Same ELF flashes to a real ESP32 module via espflash for side-by-side digital-twin verification.',
    arch: 'Xtensa LX6',
    chipYaml: chipEsp32,
    systemYaml: systemEsp32EpaperLab,
    demoFirmwarePath: `${BASE}wasm/demo-esp32-epaper-lab.elf`,
    mcuComponentType: 'esp32',
    sourceCode: sourceEsp32Epaper,
    sourceFilename: 'esp32-epaper-lab/src/main.rs',
    kind: 'lab',
    // Superseded by labwired-ereader (Arduino-ESP32 + GxEPD2); kept resolvable
    // by boardId for legacy URLs but hidden from all user-facing pickers.
    hidden: true,
  },
  {
    // `labwired-ereader.ino` compiled against Arduino-ESP32 + GxEPD2 +
    // Adafruit_GFX. Same .elf espflash'es to a real ESP32-WROOM-32 +
    // Waveshare 2.9" tri-color panel; runs unmodified in the sim via the
    // `arduino-esp32-autodiscover` quirks pipeline (resolves thunk PCs
    // from ELF symbols at runtime; attaches a UC8151D panel model).
    boardId: 'labwired-ereader',
    chipId: 'esp32',
    name: 'ESP32 E-Reader',
    description: 'ESP32-WROOM-32 + Waveshare 2.9" tri-color e-paper. Arduino sketch (GxEPD2 + Adafruit_GFX) — same .elf flashes to physical hardware.',
    arch: 'Xtensa LX6',
    chipYaml: chipEsp32,
    systemYaml: systemEsp32WroomEpaper,
    demoFirmwarePath: `${BASE}wasm/demo-labwired-ereader.elf`,
    // Pre-warmed snapshot disabled — see labwired-core#122. The CLI
    // snapshot captures the panel in its post-DRF state, which the
    // ereader firmware fills with white (refresh+clear cycle in
    // loop()). The live cold-boot path actually paints; snapshot resume
    // just shows a blank panel until the next paint cycle, which the
    // sketch doesn't trigger in its idle loop.
    mcuComponentType: 'esp32',
    sourceCode: sourceLabwiredEreader,
    sourceFilename: 'labwired-ereader-arduino/labwired-ereader.ino',
    kind: 'lab',
    quirks: 'arduino-esp32-autodiscover',
    panelScale: 2,
    summary: {
      title: 'ESP32 E-Reader',
      description: 'Arduino sketch running through the Arduino-ESP32 + GxEPD2 + FreeRTOS stack. Same .elf that flashes to physical hardware via espflash.',
      nextStep: 'Click Run — panel paints over ~2 min of cold-boot.',
      nextStepRunning: 'Running.',
    },
    runHint: 'Click Run — panel paints over ~2 min of cold-boot.',
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
    mcuComponentType: 'nucleo-f401re',
  },
  {
    boardId: 'stm32f401cdu6-blackpill',
    chipId: 'stm32f401cdu6',
    name: 'STM32F401CDU6 Black Pill',
    description: 'Compact STM32F401CDU6 Black Pill board with active-low PC13 LED.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipStm32f401cdu6,
    systemYaml: systemStm32f401cdu6Blackpill,
    mcuComponentType: 'stm32-blackpill',
  },
  {
    boardId: 'nucleo-h563zi',
    chipId: 'stm32h563',
    name: 'Nucleo-H563ZI',
    description: 'STM32H5 Nucleo-144 board with 3 LEDs and a user button.',
    arch: 'ARM Cortex-M33',
    chipYaml: chipStm32h563,
    systemYaml: systemNucleoH563zi,
    mcuComponentType: 'nucleo-h563zi',
  },
  {
    boardId: 'nucleo-l476rg',
    chipId: 'stm32l476',
    name: 'Nucleo-L476RG',
    description: 'STM32L4 Nucleo-64 board with LD2 on PA5 and user button on PC13.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipStm32l476,
    systemYaml: systemNucleoL476rg,
    mcuComponentType: 'nucleo-l476rg',
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
  {
    boardId: 'nrf52840-onboarding',
    chipId: 'nrf52840-onboarding',
    name: 'nRF52840',
    description: 'Nordic nRF52840 with the full 22-peripheral onboarding surface — TIMER, RTC, RNG, CLOCK, GPIOTE, PPI, WDT, RADIO + Easy DMA, USBD, CRYPTOCELL, FICR, NVMC, and more. Validated against real XIAO nRF52840 silicon (22/22 MODELLED).',
    arch: 'ARM Cortex-M4F',
    chipYaml: chipNrf52840Onboarding,
    systemYaml: systemNrf52840Onboarding,
    mcuComponentType: 'nrf52840-dk',
    hidden: true,
  },
  {
    boardId: 'nrf52840-ble-sensor',
    chipId: 'nrf52840',
    name: 'nRF52840 BLE Sensor',
    description: 'Nordic nRF52840 broadcasting an incrementing reading over the BLE 1 Mbit PHY (2442 MHz) into the shared virtual air. Add the BLE Collector to the same canvas and Run both to watch them talk. The same .elf flashes to real nRF silicon (ST-Link parity-proven).',
    arch: 'ARM Cortex-M4F',
    chipYaml: chipNrf52840Onboarding,
    systemYaml: systemNrf52840Onboarding,
    demoFirmwarePath: `${BASE}wasm/demo-nrf52840-ble-sensor.elf`,
    hidden: true,
    mcuComponentType: 'nrf52840-dk',
    kind: 'lab',
  },
  {
    boardId: 'nrf52840-ble-collector',
    chipId: 'nrf52840',
    name: 'nRF52840 BLE Collector',
    description: 'Nordic nRF52840 receiving BLE frames from the BLE Sensor over the shared virtual air, recording the latest reading, length, and CRC status. The same .elf flashes to real nRF silicon.',
    arch: 'ARM Cortex-M4F',
    chipYaml: chipNrf52840Onboarding,
    systemYaml: systemNrf52840Onboarding,
    demoFirmwarePath: `${BASE}wasm/demo-nrf52840-ble-collector.elf`,
    hidden: true,
    mcuComponentType: 'nrf52840-dk',
    kind: 'lab',
  },
  {
    boardId: 'nrf52840-ble-lab',
    chipId: 'nrf52840',
    name: 'nRF52840 BLE Lab (2 boards)',
    description: 'Two nRF52840s on one canvas: a Sensor advertising an incrementing reading over the BLE 1 Mbit PHY and a Collector receiving it — both over the shared virtual air. Run the Sensor, select the Collector and Run it too, then open the Packet Analyzer (Tools) to watch the frames cross.',
    arch: 'ARM Cortex-M4F',
    chipYaml: chipNrf52840Onboarding,
    systemYaml: systemNrf52840Onboarding,
    mcuComponentType: 'nrf52840-dk',
    kind: 'lab',
    runHint: 'Run the Sensor, then click the Collector and Run it too — open the Analyzer (toolbar) to watch frames.',
  },
  {
    boardId: 'nrf52840-proximity-lab',
    chipId: 'nrf52840',
    name: 'nRF52840 Proximity + BLE',
    description:
      'Nordic nRF52840 reading an HC-SR04 ultrasonic sensor over GPIO (TRIG P0.04, ECHO P0.05), raising an ALARM LED on P0.06 when the target comes within 50 cm, and broadcasting {distance, in-range} over the BLE 1 Mbit PHY. Drag Distance (cm) and watch the LED and the BLE packets follow. The same .elf flashes to real nRF silicon.',
    arch: 'ARM Cortex-M4',
    chipYaml: chipNrf52840,
    systemYaml: systemNrf52840Proximity,
    demoFirmwarePath: `${BASE}wasm/demo-nrf52840-proximity.elf`,
    mcuComponentType: 'nrf52840-dk',
    sourceCode: sourceNrf52840Proximity,
    sourceFilename: 'firmware-nrf52840-proximity/src/main.rs',
    kind: 'lab',
    runHint: 'Run, then drag Distance (cm): under 50 cm the ALARM LED lights. Open the Air Tracer · BLE (Tools) to see the distance broadcast over BLE.',
  },
];

export const BOARD_CONFIG_MAP = new Map(BOARD_CONFIGS.map((c) => [c.boardId, c]));

/** Boards shown in the palette "Boards" picker: bare MCUs only — labs are
 *  Examples (see STARTER_LABS), not boards to build on. */
export function pickerBoards(configs: BoardConfig[] = BOARD_CONFIGS): BoardConfig[] {
  return configs.filter((b) => !b.hidden && b.kind !== 'lab');
}

export function getBoardConfigForChip(chipId: string): BoardConfig | undefined {
  return BOARD_CONFIGS.find((c) => c.chipId === chipId);
}
