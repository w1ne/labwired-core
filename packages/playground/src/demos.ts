/**
 * Demo project definitions for the playground.
 * YAML configs are inlined since the WASM simulator parses them from strings.
 * Firmware ELF binaries are loaded at runtime from the /wasm/ assets directory.
 */

export interface DemoProject {
  id: string;
  name: string;
  chip: string;
  description: string;
  systemYaml: string;
  chipYaml: string;
  firmwarePath: string;
  firmware: Uint8Array; // Populated at runtime
}

// -- STM32F103 Blinky Demo --

const BLINKY_SYSTEM_YAML = `
name: "demo-blinky-board"
chip: "inline"
board_io:
  - id: "led_pa5"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
`;

const STM32F103_CHIP_YAML = `
name: "stm32f103c8"
arch: "arm"
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
`;

// -- Nucleo-F401RE Demo (LED + Button) --

const F401_SYSTEM_YAML = `
name: "nucleo-f401re-example"
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

const STM32F401_CHIP_YAML = `
name: "stm32f401re"
arch: "arm"
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

/**
 * Load firmware binary from a URL path.
 */
async function loadFirmware(path: string): Promise<Uint8Array> {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`Failed to load firmware: ${path}`);
  const buffer = await response.arrayBuffer();
  return new Uint8Array(buffer);
}

/**
 * Available demo projects. Firmware is lazy-loaded.
 */
export const DEMO_PROJECTS: DemoProject[] = [
  {
    id: 'blinky-stm32f103',
    name: 'STM32F103 Blinky',
    chip: 'STM32F103',
    description: 'Classic LED blink on Cortex-M3. Toggles PA5 via GPIO ODR.',
    systemYaml: BLINKY_SYSTEM_YAML,
    chipYaml: STM32F103_CHIP_YAML,
    firmwarePath: `${import.meta.env.BASE_URL}wasm/demo-blinky.bin`,
    firmware: new Uint8Array(0), // Loaded lazily
  },
  {
    id: 'nucleo-f401re',
    name: 'Nucleo-F401RE (LED + Button)',
    chip: 'STM32F401',
    description: 'LED and user button on Nucleo board. Press the button to interact!',
    systemYaml: F401_SYSTEM_YAML,
    chipYaml: STM32F401_CHIP_YAML,
    firmwarePath: `${import.meta.env.BASE_URL}wasm/demo-blinky.bin`, // Reuse blinky firmware for now
    firmware: new Uint8Array(0),
  },
];

/**
 * Ensure firmware is loaded for a demo project.
 */
export async function ensureFirmwareLoaded(project: DemoProject): Promise<DemoProject> {
  if (project.firmware.length > 0) return project;
  const firmware = await loadFirmware(project.firmwarePath);
  return { ...project, firmware };
}
