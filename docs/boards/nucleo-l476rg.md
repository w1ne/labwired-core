# NUCLEO-L476RG

[![tier: hardware-validated](https://img.shields.io/badge/tier-hardware--validated-brightgreen)](#)

The NUCLEO-L476RG (STM32L476RG, Cortex-M4F, 1 MB flash, 96 KB SRAM1) is
LabWired's reference hardware-validated board. Every peripheral the
simulator claims to support has been exercised against real silicon
and the sim is locked to reproduce the captured UART byte stream.

For full build/run instructions, see
[`examples/nucleo-l476rg/README.md`](../../examples/nucleo-l476rg/README.md).
For the full bug-discovery audit trail and per-peripheral fidelity
notes, see
[`examples/nucleo-l476rg/VALIDATION.md`](../../examples/nucleo-l476rg/VALIDATION.md).

## Status at a glance

| Aspect                  | Status                                          |
|-------------------------|-------------------------------------------------|
| Chip yaml               | `configs/chips/stm32l476.yaml`                  |
| System yaml             | `configs/systems/nucleo-l476rg.yaml`            |
| Reference firmware      | `crates/firmware-l476-demo/`                    |
| Survival tests          | 6 (`smoke`, `spi`, `i2c`, `adc`, `dma`, `demo`) |
| Hardware-validated      | Yes — full UART byte-for-byte parity            |

## Peripherals

| Peripheral | Status      | Notes                                                |
|------------|-------------|------------------------------------------------------|
| Cortex-M4F | ✅ full     | Thumb-2 + VFPv4 single-precision                     |
| SysTick    | ✅          | system-exception (15) path, bypasses NVIC            |
| RCC        | ✅          | `Stm32L4` profile (AHB1ENR @0x48, AHB2ENR @0x4C, APB1ENR1 @0x58, APB2ENR @0x60) |
| GPIO       | ✅          | A,B,C,D,E,H — `Stm32V2` MODER/AFR layout             |
| USART2     | ✅          | `Stm32V2`, 115200 8N1 byte-for-byte                  |
| SPI1/2/3   | ✅          | CR2 reset 0x0700, no auto-loopback                   |
| I2C1/2/3   | ✅          | `Stm32L4` modern layout                              |
| ADC1       | ✅          | `Stm32L4` layout, DEEPPWD/ADVREGEN bring-up          |
| DMA1/2     | ✅          | Mem-to-mem CMAR→CPAR, GIF/HTIF/TCIF                  |
| **PWR**    | ✅          | CR1/CR2/CR3/CR4 + SR1/SR2 + SCR + PUCRx/PDCRx        |
| **FLASH**  | ✅          | ACR latency, KEYR/OPTKEYR unlock, CR LOCK/OPTLOCK    |
| **TIM2/3/4/5/6/7** | ✅  | TIM2/5 are 32-bit (ARR reset 0xFFFFFFFF); 3/4/6/7 16-bit |
| **RNG**    | ✅          | xorshift32 LFSR, deterministic per-seed              |
| **CRC**    | ✅          | CRC-32, Ethernet poly 0x04C11DB7, DR/INIT/POL        |
| DBGMCU     | ✅          | IDCODE = 0x10076415                                  |
| NVIC       | ✅          | ISER/ISPR routing, low IRQs route correctly         |

## Pin map (UM1724)

| Signal       | Pin   | Alt-fn | Wired to                              |
|--------------|-------|--------|---------------------------------------|
| USART2_TX    | PA2   | AF7    | ST-LINK / J-Link OB Virtual COM Port  |
| USART2_RX    | PA3   | AF7    | (same)                                |
| LD2 LED      | PA5   | output | green user LED, active high           |
| B1 button    | PC13  | input  | blue user button, active low          |
| SPI1_MISO    | PA6   | AF5    | header CN5 D12                        |
| SPI1_MOSI    | PA7   | AF5    | header CN5 D11                        |
| I2C1_SCL     | PB6   | AF4    | header CN10 D5                        |
| I2C1_SDA     | PB7   | AF4    | header CN7 D7                         |

## Run the demo

```bash
cargo build --release -p firmware-l476-demo --target thumbv7em-none-eabihf
cargo run --release -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabihf/release/firmware-l476-demo \
  --system configs/systems/nucleo-l476rg.yaml
```

Output (sim and real silicon both produce this exactly):

```
L476-DEMO BOOT
DEV=10076415
SPI1 OK
I2C1 OK
ADC1 OK
DMA1 OK
LED ON
LED OFF
BTN=1
DONE
```
