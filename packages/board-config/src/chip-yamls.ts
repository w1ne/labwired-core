/** Chip YAML templates keyed by board name. */
export const CHIP_YAMLS: Record<string, string> = {
  stm32f103: `
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
  - id: "adc1"
    type: "adc"
    base_address: 0x40012400
    size: "1KB"
    irq: 18
`,
  stm32f401: `
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
`,
  // v1 subset — see core/configs/chips/stm32l476.yaml for the full peripheral list
  stm32l476: `
name: "stm32l476rg"
arch: "arm"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
    config:
      profile: "stm32l4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x48000000
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x48000400
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x48000800
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpiod"
    type: "gpio"
    base_address: 0x48000C00
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioe"
    type: "gpio"
    base_address: 0x48001000
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "gpioh"
    type: "gpio"
    base_address: 0x48001C00
    size: "1KB"
    config:
      profile: "stm32v2"
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 38
    config:
      profile: "stm32v2"
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
    config:
      profile: "stm32v2"
  - id: "spi1"
    type: "spi"
    base_address: 0x40013000
    size: "1KB"
    irq: 35
    config:
      profile: "stm32_fifo"
  - id: "i2c1"
    type: "i2c"
    base_address: 0x40005400
    size: "1KB"
    irq: 31
    config:
      profile: "stm32l4"
  - id: "adc1"
    type: "adc"
    base_address: 0x50040000
    size: "1KB"
    irq: 18
    config:
      profile: "stm32l4"
  - id: "dma1"
    type: "dma"
    base_address: 0x40020000
    size: "1KB"
    irq: 11
`,
};
