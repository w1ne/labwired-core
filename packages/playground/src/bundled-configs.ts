/**
 * Bundled board configurations for the playground.
 * Contains chip YAML + system YAML as string literals for boards
 * that the WASM simulator can actually run.
 *
 * Source: core/configs/chips/ and core/configs/systems/
 */

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
}

// ── Chip YAMLs (from core/configs/chips/) ────────────────────────────

const CHIP_STM32F103 = `\
name: "stm32f103c8"
arch: "arm"
registers_count: 125
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x40010800
    size: "1KB"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x40010C00
    size: "1KB"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x40011000
    size: "1KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
  - id: "i2c1"
    type: "i2c"
    base_address: 0x40005400
    size: "1KB"
    irq: 31
  - id: "i2c2"
    type: "i2c"
    base_address: 0x40005800
    size: "1KB"
    irq: 33
  - id: "afio"
    type: "afio"
    base_address: 0x40010000
    size: "1KB"
  - id: "exti"
    type: "exti"
    base_address: 0x40010400
    size: "1KB"
  - id: "dma1"
    type: "dma"
    base_address: 0x40020000
    size: "1KB"
  - id: "adc1"
    type: "adc"
    base_address: 0x40012400
    size: "1KB"
    irq: 18
`;

const CHIP_STM32F401 = `\
name: "stm32f401re"
arch: "arm"
registers_count: 150
flash:
  base: 0x08000000
  size: "512KB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40023800
    size: "1KB"
    config:
      profile: "stm32f4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x40020000
    size: "1KB"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x40020400
    size: "1KB"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x40020800
    size: "1KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
`;

const CHIP_STM32H563 = `\
name: "stm32h563zi"
arch: "arm"
registers_count: 220
flash:
  base: 0x08000000
  size: "2MB"
ram:
  base: 0x20000000
  size: "640KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x44020C00
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x42020000
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x42020400
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x42020800
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiod"
    type: "gpio"
    base_address: 0x42020C00
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioe"
    type: "gpio"
    base_address: 0x42021000
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiof"
    type: "gpio"
    base_address: 0x42021400
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiog"
    type: "gpio"
    base_address: 0x42021800
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart3"
    type: "uart"
    base_address: 0x40004800
    size: "1KB"
    irq: 60
    config:
      profile: "stm32v2"
  - id: "dma1"
    type: "dma"
    base_address: 0x40020000
    size: "1KB"
`;

const CHIP_ESP32C3 = `\
name: "esp32c3"
arch: "riscv"
registers_count: 50
flash:
  base: 0x42000000
  size: "4MB"
ram:
  base: 0x3FC80000
  size: "400KB"
peripherals:
  - id: "gpio"
    type: "gpio"
    base_address: 0x60004000
    size: "4KB"
  - id: "uart0"
    type: "uart"
    base_address: 0x60000000
    size: "4KB"
  - id: "systick"
    type: "systick"
    base_address: 0x6001F000
`;

const CHIP_RP2040 = `\
name: "rp2040"
arch: "arm"
registers_count: 240
flash:
  base: 0x10000000
  size: "2MB"
ram:
  base: 0x20000000
  size: "256KB"
peripherals:
  - id: "gpio"
    type: "gpio"
    base_address: 0x40014000
    size: "4KB"
  - id: "uart0"
    type: "uart"
    base_address: 0x40034000
    size: "4KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
`;

const CHIP_NRF52840 = `\
name: "nrf52840"
arch: "arm"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "256KB"
peripherals:
  - id: "gpio0"
    type: "gpio"
    base_address: 0x50000000
    size: "4KB"
  - id: "gpio1"
    type: "gpio"
    base_address: 0x50000300
    size: "4KB"
  - id: "uart0"
    type: "uart"
    base_address: 0x40002000
    size: "4KB"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
`;

// ── System YAMLs (from core/configs/systems/) ────────────────────────

const SYS_STM32F103_BLINKY = `\
name: "stm32f103-blinky"
chip: "inline"
board_io:
  - id: "led_pa5"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
`;

const SYS_NUCLEO_F401RE = `\
name: "nucleo-f401re"
chip: "inline"
board_io:
  - id: "led2_pa5"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
  - id: "button_user_pc13"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: true
`;

const SYS_NUCLEO_H563ZI = `\
name: "nucleo-h563zi-demo"
chip: "inline"
board_io:
  - id: "led_green_pb0"
    kind: "led"
    peripheral: "gpiob"
    pin: 0
    signal: "output"
    active_high: true
  - id: "led_orange_pf4"
    kind: "led"
    peripheral: "gpiof"
    pin: 4
    signal: "output"
    active_high: true
  - id: "led_red_pg4"
    kind: "led"
    peripheral: "gpiog"
    pin: 4
    signal: "output"
    active_high: true
  - id: "button_blue_pc13"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: true
`;

const SYS_ESP32C3_DEVKIT = `\
name: "esp32c3-devkit"
chip: "inline"
board_io:
  - id: "status_led"
    kind: "led"
    peripheral: "gpio"
    pin: 8
    signal: "output"
    active_high: true
`;

const SYS_RP2040_PICO = `\
name: "rp2040-pico"
chip: "inline"
board_io:
  - id: "led_gp25"
    kind: "led"
    peripheral: "gpio"
    pin: 25
    signal: "output"
    active_high: true
`;

const SYS_NRF52840_DK = `\
name: "nrf52840-dk"
chip: "inline"
board_io:
  - id: "led1_p0_13"
    kind: "led"
    peripheral: "gpio0"
    pin: 13
    signal: "output"
    active_high: true
  - id: "button1_p0_11"
    kind: "button"
    peripheral: "gpio0"
    pin: 11
    signal: "input"
    active_high: true
`;

// ── Board Configs ────────────────────────────────────────────────────

const BASE = import.meta.env.BASE_URL;

export const BOARD_CONFIGS: BoardConfig[] = [
  {
    boardId: 'stm32f103-blinky',
    chipId: 'stm32f103',
    name: 'STM32F103 Blinky',
    description: 'Classic LED blink on Cortex-M3. Toggles PA5 via GPIO.',
    arch: 'ARM Cortex-M3',
    chipYaml: CHIP_STM32F103,
    systemYaml: SYS_STM32F103_BLINKY,
    demoFirmwarePath: `${BASE}wasm/demo-blinky.bin`,
    mcuComponentType: 'stm32-dev',
  },
  {
    boardId: 'nucleo-f401re',
    chipId: 'stm32f401',
    name: 'Nucleo-F401RE',
    description: 'STM32F4 Nucleo board with LED on PA5 and user button on PC13.',
    arch: 'ARM Cortex-M4',
    chipYaml: CHIP_STM32F401,
    systemYaml: SYS_NUCLEO_F401RE,
    demoFirmwarePath: `${BASE}wasm/demo-nucleo-f401.elf`,
    mcuComponentType: 'stm32-dev',
  },
  {
    boardId: 'nucleo-h563zi',
    chipId: 'stm32h563',
    name: 'Nucleo-H563ZI',
    description: 'STM32H5 Nucleo-144 board with 3 LEDs and a user button.',
    arch: 'ARM Cortex-M33',
    chipYaml: CHIP_STM32H563,
    systemYaml: SYS_NUCLEO_H563ZI,
    mcuComponentType: 'stm32-dev',
  },
  {
    boardId: 'esp32c3-devkit',
    chipId: 'esp32c3',
    name: 'ESP32-C3 DevKit',
    description: 'RISC-V based ESP32-C3 with WiFi/BLE. Status LED on GPIO8.',
    arch: 'RISC-V',
    chipYaml: CHIP_ESP32C3,
    systemYaml: SYS_ESP32C3_DEVKIT,
    mcuComponentType: 'esp32',
  },
  {
    boardId: 'rp2040-pico',
    chipId: 'rp2040',
    name: 'Raspberry Pi Pico',
    description: 'RP2040 dual-core ARM Cortex-M0+ board. LED on GP25.',
    arch: 'ARM Cortex-M0+',
    chipYaml: CHIP_RP2040,
    systemYaml: SYS_RP2040_PICO,
    mcuComponentType: 'rpi-pico',
  },
  {
    boardId: 'nrf52840-dk',
    chipId: 'nrf52840',
    name: 'nRF52840 DK',
    description: 'Nordic nRF52840 dev kit with BLE. LED on P0.13, button on P0.11.',
    arch: 'ARM Cortex-M4F',
    chipYaml: CHIP_NRF52840,
    systemYaml: SYS_NRF52840_DK,
    mcuComponentType: 'nrf52840-dk',
  },
];

export const BOARD_CONFIG_MAP = new Map(BOARD_CONFIGS.map((c) => [c.boardId, c]));

export function getBoardConfigForChip(chipId: string): BoardConfig | undefined {
  return BOARD_CONFIGS.find((c) => c.chipId === chipId);
}
