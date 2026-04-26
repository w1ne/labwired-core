// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware survival tests: load real compiled binaries and assert the simulator
//! runs without crashing for a meaningful number of cycles.
//!
//! These are the ground truth for CPU correctness — if a real firmware can't
//! survive N cycles, something is broken in the instruction decoder or executor.
//!
//! Each test also asserts that the firmware emits the expected UART bytes,
//! proving the CPU executed real application logic, not just spun in reset loops.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::riscv::RiscV;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::trace::TraceObserver;
use labwired_core::{Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How many cycles a firmware must survive before the test passes.
const SURVIVAL_CYCLES: u32 = 800_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CpuFamily {
    CortexM,
    RiscV,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SurvivalCase {
    name: &'static str,
    core: &'static str,
    family: CpuFamily,
    chip: &'static str,
    system: &'static str,
    fixture: &'static str,
    valid_pc_ranges: &'static [(u32, u32)],
    /// Bytes that must appear somewhere in the UART output after SURVIVAL_CYCLES.
    /// Proves the firmware executed real application logic, not just a reset loop.
    expected_uart_output: &'static [u8],
}

const IMPORTANT_CORES: &[&str] = &[
    "cortex-m0+",
    "cortex-m3",
    "cortex-m4",
    "cortex-m33",
    "rv32i",
];

const SURVIVAL_CASES: &[SurvivalCase] = &[
    SurvivalCase {
        name: "stm32f103_blinky",
        core: "cortex-m3",
        family: CpuFamily::CortexM,
        chip: "stm32f103",
        system: "stm32f103-bare",
        fixture: "stm32f103-blinky.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        // Arduino HAL firmware: setup() prints this via interrupt-driven HardwareSerial.
        expected_uart_output: b"LabWired Playground - Arduino Blink",
    },
    SurvivalCase {
        name: "stm32f401_blinky",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32f401",
        system: "nucleo-f401re",
        fixture: "stm32f401-blinky.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        // Keep this as a control-flow survival check. The current F401 board model
        // does not yet produce deterministic UART bytes end-to-end.
        expected_uart_output: b"",
    },
    SurvivalCase {
        name: "rp2040_demo",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "rp2040",
        system: "rp2040-pico",
        fixture: "rp2040-demo.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x101F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"RP2040_SMOKE_OK\n",
    },
    SurvivalCase {
        name: "nrf52840_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "nrf52840",
        system: "nrf52840-dk",
        fixture: "nrf52840-demo.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"NRF52840_SMOKE_OK\n",
    },
    SurvivalCase {
        name: "nrf52832_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "nrf52832",
        system: "nrf52-dk",
        fixture: "nrf52832-demo.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2000_FFFF)],
        // The nrf52832-demo.elf binary was compiled for nRF52840 (256KB RAM), but the
        // nRF52832 chip config only has 64KB RAM. The initial SP (0x20040000) sits outside
        // the 64KB boundary, making the stack unreliable. UART output is not asserted here.
        expected_uart_output: b"",
    },
    SurvivalCase {
        name: "stm32h563_demo",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "stm32h563",
        system: "nucleo-h563zi-demo",
        fixture: "stm32h563-demo.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x2009_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        name: "riscv_ci_fixture",
        core: "rv32i",
        family: CpuFamily::RiscV,
        chip: "ci-fixture-riscv",
        system: "ci-fixture-riscv-uart1",
        fixture: "riscv-ci-fixture.elf",
        valid_pc_ranges: &[(0x8000_0000, 0x8001_FFFF), (0x8002_0000, 0x8002_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        name: "esp32c3_demo",
        core: "rv32i",
        family: CpuFamily::RiscV,
        chip: "esp32c3",
        system: "esp32c3-devkit",
        fixture: "esp32c3-demo.elf",
        valid_pc_ranges: &[(0x4200_0000, 0x423F_FFFF), (0x3FC8_0000, 0x3FEF_FFFF)],
        expected_uart_output: b"ESP OK\n",
    },
    SurvivalCase {
        // Hardware-validated against real NUCLEO-L476RG silicon: the
        // exact byte stream below was captured from /dev/ttyACM1 with the
        // J-Link OB Virtual COM Port at 115200 baud. The simulator must
        // reproduce it verbatim — drift means a regression in the L4
        // chip config, the FPU implementation, or the Thumb-2 decoder.
        name: "nucleo_l476rg_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output:
            b"L476 SMOKE\r\nDEV=10076415\r\nMUL=60FC303A\r\nFPU=40C8F5C3\r\nDONE\r\n",
    },
    SurvivalCase {
        // SPI1 register-level fidelity. Captured from real silicon — the
        // sim's SPI peripheral matches CR1/CR2/SR latching, CR2 reset
        // value (0x0700 = DS=8-bit on STM32L4), and the no-loopback
        // transmit semantics (SR=0x0002 / DR=0x00 after TX with no
        // slave wired).
        name: "nucleo_l476rg_spi",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-spi.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"SPI1 RESET\r\n\
CR1=0000\r\n\
CR2=0700\r\n\
SR=0002\r\n\
SPI1 CONFIG\r\n\
CR1=033C\r\n\
CR2=1700\r\n\
SR=0002\r\n\
SPI1 ENABLED\r\n\
CR1=037C\r\n\
SR=0002\r\n\
SPI1 AFTER TX\r\n\
SR=0002\r\n\
DR=00\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // STM32L4 RCC PLL bring-up: walks the canonical HAL clock-init
        // sequence — enable HSE, request PLL, switch SYSCLK to PLL.
        // Surfaces the ready-flag state machine: HSEON without HSEBYP
        // never readies (NUCLEO board has ST-LINK MCO not crystal),
        // PLLON gated on source ready, CFGR.SWS only follows SW once
        // the requested source is locked. CR/CFGR/PLLCFGR sequences
        // captured byte-for-byte from real silicon.
        name: "nucleo_l476rg_pll",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-pll.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"CLK START\r\n\
CR=00000063\r\n\
CFGR=00000000\r\n\
HSEON\r\n\
CR=00010063\r\n\
PLLON\r\n\
CR=01010063\r\n\
PLLCFGR=01001403\r\n\
SWITCHED\r\n\
CFGR=00000003\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // STM32L4 secondary peripheral fingerprint: IWDG + WWDG + DAC +
        // RTC. Reset values verified against real silicon (without
        // enabling WWDG clock, so the counter doesn't decrement).
        name: "nucleo_l476rg_misc",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-misc.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"IWDG\r\n\
KR=00000000\r\n\
PR=00000000\r\n\
RLR=00000FFF\r\n\
SR=00000000\r\n\
WWDG\r\n\
CR=0000007F\r\n\
CFR=0000007F\r\n\
SR=00000000\r\n\
DAC\r\n\
CR=00000000\r\n\
SR=00000000\r\n\
RTC\r\n\
CR=00000000\r\n\
ISR=00000027\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // STM32L4 foundational-peripheral reset-value fingerprint:
        // PWR / FLASH / TIM2 / RNG / CRC. Captured from real silicon —
        // every value below is what NUCLEO-L476RG hardware reports
        // immediately after a reset. The PWR / FLASH peripherals in
        // particular are required for HAL-generated firmware to boot
        // (HAL_PWREx_ControlVoltageScaling and the FLASH ACR latency
        // dance both run before SYSCLK is reconfigured).
        name: "nucleo_l476rg_l4periphs",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-l4periphs.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"PWR\r\n\
CR1=00000200\r\n\
CR2=00000000\r\n\
CR3=00008000\r\n\
CR4=00000000\r\n\
SR1=00000000\r\n\
SR2=00000100\r\n\
FLASH\r\n\
ACR=00000600\r\n\
SR=00000000\r\n\
CR=C0000000\r\n\
OPTR=FFEFF8AA\r\n\
TIM2\r\n\
CR1=00000000\r\n\
ARR=FFFFFFFF\r\n\
PSC=00000000\r\n\
CNT=00000000\r\n\
RNG\r\n\
CR=00000000\r\n\
SR=00000000\r\n\
CRC\r\n\
DR=FFFFFFFF\r\n\
CR=00000000\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // Comprehensive end-to-end demo for NUCLEO-L476RG. Built from
        // crates/firmware-l476-demo (Rust, no_std), exercises every
        // peripheral that's been hardware-validated on real silicon —
        // RCC, GPIO, USART, SPI, I2C, ADC, DMA — in one cohesive
        // bring-up sequence. The output stream below is captured
        // byte-for-byte from real NUCLEO-L476RG silicon.
        name: "nucleo_l476rg_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-demo.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"L476-DEMO BOOT\r\n\
DEV=10076415\r\n\
SPI1 OK\r\n\
I2C1 OK\r\n\
ADC1 OK\r\n\
DMA1 OK\r\n\
LED ON\r\n\
LED OFF\r\n\
BTN=1\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // DMA1 channel-1 mem-to-mem fidelity. Captured from real silicon:
        // a 4-byte memory-to-memory transfer (CPAR=0x20000010,
        // CMAR=0x20000020, CNDTR=4, CCR=0x40D3) completes with
        // ISR=0x07 (GIF1 + TCIF1 + HTIF1), CNDTR drained to 0, and
        // CPAR/CMAR REMAIN at their configured base addresses (real DMA
        // uses internal next-address pointers, not the user-facing
        // registers). The sim now reproduces all of that.
        name: "nucleo_l476rg_dma",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-dma.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"DMA1 RESET\r\n\
ISR=00000000\r\n\
CCR1=00000000\r\n\
CNDTR1=00000000\r\n\
CPAR1=00000000\r\n\
CMAR1=00000000\r\n\
DMA1 CONFIG\r\n\
ISR=00000007\r\n\
CCR1=000040D3\r\n\
CNDTR1=00000000\r\n\
CPAR1=20000010\r\n\
CMAR1=20000020\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // ADC1 modern (STM32L4-style) register-level fidelity. Captured
        // from real silicon: CR resets to 0x20000000 (DEEPPWD), CFGR to
        // 0x80000000 (JQDIS), and ADCAL stays set after a calibration
        // request when the ADC has no clock source configured (matches
        // the smoke firmware which only enables AHB2.ADCEN, not CCIPR).
        name: "nucleo_l476rg_adc",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-adc.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"ADC1 RESET\r\n\
ISR=00000000\r\n\
CR=20000000\r\n\
CFGR=80000000\r\n\
ADC1 REGEN\r\n\
CR=10000000\r\n\
ISR=00000000\r\n\
ADC1 CALDONE\r\n\
CR=90000000\r\n\
ISR=00000000\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // I2C1 modern (STM32L4-style) register-level fidelity. Captured
        // from real silicon: ISR resets to 0x00000001 (TXE=1), CR2.START
        // set on master start lights ISR.BUSY (bit 15), and TIMINGR
        // latches 32-bit values. The sim's i2c peripheral with the
        // stm32l4 layout reproduces all 13 lines verbatim.
        name: "nucleo_l476rg_i2c",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-i2c.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"I2C1 RESET\r\n\
CR1=00000000\r\n\
CR2=00000000\r\n\
OAR1=00000000\r\n\
TIMINGR=00000000\r\n\
ISR=00000001\r\n\
I2C1 CONFIG\r\n\
CR1=00000001\r\n\
OAR1=00008084\r\n\
TIMINGR=10805E89\r\n\
ISR=00000001\r\n\
I2C1 START PENDING\r\n\
CR2=000120A0\r\n\
ISR=00008001\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // L4 secondary-peripheral coverage ("round 8"): LPUART1, LPTIM1,
        // EXTI L4 dual-bank layout, QUADSPI, SAI1, USB OTG FS core regs,
        // bxCAN1 INRQ/INAK handshake. Captured byte-for-byte from real
        // NUCLEO-L476RG silicon via J-Link OB Virtual COM Port. The
        // capture surfaced four sim<->silicon divergences that this round
        // also fixed:
        //   - SAI1 ACR1/BCR1 reset is 0x40 (NODIV bit set), not 0.
        //   - USB OTG GINTSTS reset is 0x1400_0020 (NPTXFE|PTXFE|CIDSCHG|
        //     DISCINT — cable disconnected, FIFOs empty), not 0x0400_0001.
        //   - bxCAN MSR after INRQ=1 is 0x0000_0409 (INAK + WKUI + SAMP),
        //     not 0x0000_0C01. INRQ also latches WKUI on real silicon.
        //   - bxCAN MSR reset (before INRQ) is 0x0000_040A (SLAK + SAMP),
        //     not 0x0000_0C02.
        name: "nucleo_l476rg_l4periphs2",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-l4periphs2.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"L4-PERIPHS2\r\n\
LPUART1\r\n\
CR1=00000000\r\n\
ISR=000000C0\r\n\
BRR=00000000\r\n\
LPTIM1\r\n\
ISR=00000018\r\n\
ARR=00001000\r\n\
CMP=00000800\r\n\
EXTI\r\n\
IMR1=00400000\r\n\
PR1 =00400000\r\n\
IMR2=00000008\r\n\
QUADSPI\r\n\
CR =00000000\r\n\
DCR=00000000\r\n\
SR =00000000\r\n\
SAI1\r\n\
GCR =00000000\r\n\
ACR1=00000040\r\n\
BCR1=00000040\r\n\
OTG\r\n\
GUSBCFG=00001440\r\n\
GRSTCTL=80000000\r\n\
GINTSTS=14000020\r\n\
CAN1\r\n\
MCR=00000001\r\n\
MSR=00000409\r\n\
TSR=1C000000\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // CubeMX-style HAL firmware ("round 9"). Mimics the canonical
        // STM32CubeIDE-generated boot sequence end-to-end:
        //
        //   Reset_Handler   -> .data copy + .bss zero, then main()
        //   SystemInit()    -> VTOR relocate + FPU enable (CPACR)
        //   HAL_Init()      -> SysTick @ 1ms (priority 15), TICKINT enabled,
        //                      uwTick++ in SysTick_Handler
        //   SystemClock_    -> PWR.VOSCR=01, FLASH 4WS, MSI->PLL@80MHz,
        //     Config()         CFGR.SW=PLL with SWS-source-lock poll
        //   MX_USART2_      -> PA2 AF7, BRR=694 -> 115200 @ 80 MHz
        //     UART_Init()
        //   loop:           -> 3x TICK print spaced by hal_delay(2)
        //
        // The 4x "HAL BOOT" preamble is a hardware-side defensive measure:
        // J-Link OB CDC bridges can drop the first packet on noisy USB
        // hosts during re-enumeration. The repeat banner gives the host
        // capture window something to latch onto.
        //
        // This trace was hardware-validated against NUCLEO-L476RG silicon
        // (J-Link OB VCP captured via Python select()-based reader at
        // 115200 8N1; raw `cat` drops bytes around USB packet boundaries
        // but a select-driven reader does not). It exercises the full
        // clock-tree state machine (PWR voltage scaling, FLASH ACR
        // latency dance, RCC PLL source-ready / source-lock handshake),
        // interrupt-driven SysTick routing through the user-supplied
        // vector table, and FPU bring-up via CPACR.
        name: "nucleo_l476rg_cubemx_hal",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-cubemx-hal.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"HAL BOOT\r\n\
HAL BOOT\r\n\
HAL BOOT\r\n\
HAL BOOT\r\n\
TICK 1\r\n\
TICK 2\r\n\
TICK 3\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // TIM1 advanced-control bring-up ("round 10"). Programs the
        // canonical centre-aligned PWM init sequence on TIM1 channel 1:
        //   PSC=79, ARR=999  -> 1 kHz @ 80 MHz
        //   RCR=5            -> repetition counter (advanced-only)
        //   CCR1=500         -> 50% duty cycle
        //   CCMR1.OC1M=110   -> PWM mode 1, OC1PE=1 (preload)
        //   CCER=CC1E|CC1NE  -> channel + complementary output
        //   BDTR=MOE|DTG=0x40-> master output enable + dead-time
        //
        // Captured byte-for-byte from real NUCLEO-L476RG silicon. Round 10
        // surfaced and fixed one sim<->silicon delta:
        //   - CCER mask was 0x3333 (CC*E/CC*P only) — correct for general-
        //     purpose timers but wrong for advanced. The CC*NE / CC*NP
        //     complementary-output bits got dropped. Now `advanced=true`
        //     selects mask 0xFFFF.
        name: "nucleo_l476rg_tim1_advanced",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-tim1-advanced.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"TIM1-ADV\r\n\
CR1\r\n\
CR1 =00000001\r\n\
PWM\r\n\
PSC =0000004F\r\n\
ARR =000003E7\r\n\
RCR =00000005\r\n\
CCR1=000001F4\r\n\
CCMR=00000068\r\n\
CCER=00000005\r\n\
BDTR=00008040\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // Round 11: DMA_CSELR (L4 channel selection) + SDMMC1 reset
        // state + EXTI bank-2 IMR2/PR2 latching via SWIER2.
        //
        // Captured byte-for-byte from real NUCLEO-L476RG silicon.
        // Round 11 surfaced and fixed two SDMMC sim<->silicon deltas:
        //   - RSPCMD was being mirrored from CMDINDEX on every CPSMEN
        //     write. Real silicon only updates RESPCMD when a card
        //     actually responds. With no card present, it stays 0.
        //   - STA wrong flag: sim asserted CMDSENT (bit 7), silicon
        //     asserts CTIMEOUT (bit 11) when CLKCR.CLKEN=0 (no SDMMC
        //     clock). The state machine times out before sending
        //     anything. Now sim picks the right flag based on CLKEN.
        name: "nucleo_l476rg_r11",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-r11.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"R11\r\n\
DMA\r\n\
CSELR=05000004\r\n\
SDMMC\r\n\
POWER =00000000\r\n\
CLKCR =00000000\r\n\
CMD   =00000405\r\n\
RSPCMD=00000000\r\n\
STA   =00000800\r\n\
STA-2 =00000800\r\n\
EXTI2\r\n\
IMR2  =00000008\r\n\
PR2   =00000008\r\n\
DONE\r\n",
    },
];

fn workspace_root() -> PathBuf {
    // crates/core → crates → workspace root (core/)
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn fixtures() -> PathBuf {
    workspace_root().join("tests/fixtures")
}

fn chip_config(name: &str) -> PathBuf {
    workspace_root().join("configs/chips").join(format!("{name}.yaml"))
}

fn system_config(name: &str) -> PathBuf {
    workspace_root().join("configs/systems").join(format!("{name}.yaml"))
}

fn load_system(chip_name: &str, system_name: &str) -> (ChipDescriptor, SystemManifest) {
    let chip = ChipDescriptor::from_file(&chip_config(chip_name))
        .unwrap_or_else(|e| panic!("Failed to load chip {chip_name}: {e}"));

    let sys_path = system_config(system_name);
    let mut manifest = SystemManifest::from_file(&sys_path)
        .unwrap_or_else(|e| panic!("Failed to load system {system_name}: {e}"));

    manifest.chip = sys_path.parent().unwrap().join(&manifest.chip)
        .to_str().unwrap().to_string();

    (chip, manifest)
}

fn assert_pc_in_range(pc: u32, cycles: u32, ranges: &[(u32, u32)]) {
    assert!(
        ranges.iter().any(|(start, end)| (*start..=*end).contains(&pc)),
        "PC={:#010x} after {} cycles — jumped to unmapped region",
        pc,
        cycles
    );
}

fn assert_uart_contains(uart_bytes: &[u8], expected: &[u8], name: &str) {
    // Empty expected means "no assertion" — useful for boards with known limitations.
    if expected.is_empty() {
        return;
    }
    assert!(
        uart_bytes.windows(expected.len()).any(|w| w == expected),
        "Board '{}': UART output did not contain expected bytes.\n\
         Expected (escaped): {:?}\n\
         Actual   (escaped): {:?}\n\
         Actual   (utf8):    {}\n",
        name,
        std::str::from_utf8(expected).unwrap_or("<non-utf8>"),
        std::str::from_utf8(uart_bytes).unwrap_or("<non-utf8>"),
        String::from_utf8_lossy(uart_bytes),
    );
}

fn run_survival_case(case: &SurvivalCase) {
    let firmware = fixtures().join(case.fixture);
    let (pc, uart_bytes) = match case.family {
        CpuFamily::CortexM => {
            run_cortex_m_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES)
        }
        CpuFamily::RiscV => run_riscv_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES),
    };

    assert_pc_in_range(pc, SURVIVAL_CYCLES, case.valid_pc_ranges);
    assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
}

/// Run a Cortex-M machine loaded with `firmware_path` for `cycles` steps.
/// Returns `(final_pc, uart_bytes)` so callers can assert correctness.
fn run_cortex_m_firmware(
    chip_name: &str,
    system_name: &str,
    firmware_path: PathBuf,
    cycles: u32,
) -> (u32, Vec<u8>) {
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );

    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.observers.push(trace.clone());

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("Failed to load ELF {:?}: {e}", firmware_path));
    machine.load_firmware(&image)
        .expect("Failed to load firmware into machine");

    let mut last_state: std::collections::VecDeque<(u32, u32, u32)> =
        std::collections::VecDeque::new();
    for step in 0..cycles {
        let pc_before = machine.cpu.get_pc();
        let lr_before = machine.cpu.lr;
        last_state.push_back((step, pc_before, lr_before));
        if last_state.len() > 30 {
            last_state.pop_front();
        }

        machine.step().unwrap_or_else(|e| {
            eprintln!("Last 30 steps before crash:");
            for (s, p, lr) in &last_state {
                eprintln!("  step {:5}: PC={:#010x}  LR={:#010x}", s, p, lr);
            }
            eprintln!("Last instruction traces before crash:");
            for t in trace.take_traces().into_iter().rev().take(24).rev() {
                let lr = t.register_delta.get(&14).map(|(_, new)| *new);
                let sp = t.register_delta.get(&13).map(|(_, new)| *new);
                let pc = t.register_delta.get(&15).map(|(_, new)| *new);
                eprintln!(
                    "  trace pc={:#010x} opcode={:#010x} lr={} sp={} next_pc={}",
                    t.pc,
                    t.instruction,
                    lr.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                    sp.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                    pc.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                );
            }
            panic!(
                "Simulation crashed at step {} (PC={:#010x}): {}",
                step, pc_before, e
            )
        });
    }

    let uart_bytes = uart_sink.lock().unwrap().clone();
    let final_pc = machine.cpu.get_pc();
    (final_pc, uart_bytes)
}

fn run_riscv_firmware(
    chip_name: &str,
    system_name: &str,
    firmware_path: PathBuf,
    cycles: u32,
) -> (u32, Vec<u8>) {
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );

    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);
    let mut machine = Machine::new(RiscV::new(), bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.observers.push(trace.clone());

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("Failed to load ELF {:?}: {e}", firmware_path));
    machine.load_firmware(&image)
        .expect("Failed to load firmware into machine");

    let mut last_pcs: std::collections::VecDeque<(u32, u32)> = std::collections::VecDeque::new();
    for step in 0..cycles {
        let pc_before = machine.cpu.get_pc();
        last_pcs.push_back((step, pc_before));
        if last_pcs.len() > 30 {
            last_pcs.pop_front();
        }

        machine.step().unwrap_or_else(|e| {
            eprintln!("Last 30 steps before crash:");
            for (s, p) in &last_pcs {
                eprintln!("  step {:5}: PC={:#010x}", s, p);
            }
            eprintln!("Last instruction traces before crash:");
            for t in trace.take_traces().into_iter().rev().take(24).rev() {
                let pc = t.register_delta.get(&32).map(|(_, new)| *new);
                eprintln!(
                    "  trace pc={:#010x} opcode={:#010x} next_pc={}",
                    t.pc,
                    t.instruction,
                    pc.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                );
            }
            panic!(
                "Simulation crashed at step {} (PC={:#010x}): {}",
                step, pc_before, e
            )
        });
    }

    let uart_bytes = uart_sink.lock().unwrap().clone();
    (machine.cpu.get_pc(), uart_bytes)
}

#[test]
fn test_stm32f103_blinky_survival() {
    run_survival_case(&SURVIVAL_CASES[0]);
}

#[test]
fn test_stm32f401_blinky_survival() {
    run_survival_case(&SURVIVAL_CASES[1]);
}

#[test]
fn test_rp2040_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[2]);
}

#[test]
fn test_nrf52840_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[3]);
}

#[test]
fn test_nrf52832_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[4]);
}

#[test]
fn test_stm32h563_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[5]);
}

#[test]
fn test_riscv_ci_fixture_survival() {
    run_survival_case(&SURVIVAL_CASES[6]);
}

#[test]
fn test_esp32c3_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[7]);
}

#[test]
fn test_nucleo_l476rg_smoke_survival() {
    run_survival_case(&SURVIVAL_CASES[8]);
}

#[test]
fn test_nucleo_l476rg_spi_survival() {
    run_survival_case(&SURVIVAL_CASES[9]);
}

#[test]
fn test_nucleo_l476rg_pll_survival() {
    run_survival_case(&SURVIVAL_CASES[10]);
}

#[test]
fn test_nucleo_l476rg_misc_survival() {
    run_survival_case(&SURVIVAL_CASES[11]);
}

#[test]
fn test_nucleo_l476rg_l4periphs_survival() {
    run_survival_case(&SURVIVAL_CASES[12]);
}

#[test]
fn test_nucleo_l476rg_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[13]);
}

#[test]
fn test_nucleo_l476rg_dma_survival() {
    run_survival_case(&SURVIVAL_CASES[14]);
}

#[test]
fn test_nucleo_l476rg_adc_survival() {
    run_survival_case(&SURVIVAL_CASES[15]);
}

#[test]
fn test_nucleo_l476rg_i2c_survival() {
    run_survival_case(&SURVIVAL_CASES[16]);
}

#[test]
fn test_nucleo_l476rg_l4periphs2_survival() {
    run_survival_case(&SURVIVAL_CASES[17]);
}

#[test]
fn test_nucleo_l476rg_tim1_advanced_survival() {
    run_survival_case(&SURVIVAL_CASES[19]);
}

#[test]
fn test_nucleo_l476rg_r11_survival() {
    run_survival_case(&SURVIVAL_CASES[20]);
}

#[test]
fn test_nucleo_l476rg_cubemx_hal_survival() {
    // HAL flow needs more cycles than other tests because it spends most
    // of its time in HAL_Delay() polling SysTick (RVR=80_000-1).
    let case = &SURVIVAL_CASES[18];
    let firmware = fixtures().join(case.fixture);
    let (pc, uart_bytes) = run_cortex_m_firmware(
        case.chip, case.system, firmware, SURVIVAL_CYCLES * 4,
    );
    assert_pc_in_range(pc, SURVIVAL_CYCLES * 4, case.valid_pc_ranges);
    assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
}

/// One-shot capture helper: runs the new round-8 ELF through the simulator
/// and prints the UART trace to stdout so a human can audit it before locking
/// the bytes into a survival case. Marked `#[ignore]` so it doesn't run in
/// CI — invoke with `cargo test ... -- --ignored capture_l4periphs2`.
#[test]
#[ignore]
fn capture_l4periphs2_sim_output() {
    let firmware = fixtures().join("nucleo-l476rg-l4periphs2.elf");
    let (_pc, uart) =
        run_cortex_m_firmware("stm32l476", "nucleo-l476rg", firmware, SURVIVAL_CYCLES);
    let s = String::from_utf8_lossy(&uart);
    eprintln!("--- BEGIN UART ---");
    eprintln!("{}", s);
    eprintln!("--- END UART ---");
    eprintln!("escaped: {:?}", s);
}

#[test]
#[ignore]
fn capture_r11_sim_output() {
    let firmware = fixtures().join("nucleo-l476rg-r11.elf");
    let (_pc, uart) =
        run_cortex_m_firmware("stm32l476", "nucleo-l476rg", firmware, SURVIVAL_CYCLES);
    let s = String::from_utf8_lossy(&uart);
    eprintln!("--- BEGIN UART ---");
    eprintln!("{}", s);
    eprintln!("--- END UART ---");
    eprintln!("escaped: {:?}", s);
}

#[test]
#[ignore]
fn capture_tim1_advanced_sim_output() {
    let firmware = fixtures().join("nucleo-l476rg-tim1-advanced.elf");
    let (_pc, uart) =
        run_cortex_m_firmware("stm32l476", "nucleo-l476rg", firmware, SURVIVAL_CYCLES);
    let s = String::from_utf8_lossy(&uart);
    eprintln!("--- BEGIN UART ---");
    eprintln!("{}", s);
    eprintln!("--- END UART ---");
    eprintln!("escaped: {:?}", s);
}

#[test]
#[ignore]
fn capture_cubemx_hal_sim_output() {
    let firmware = fixtures().join("nucleo-l476rg-cubemx-hal.elf");
    let (pc, uart) =
        run_cortex_m_firmware("stm32l476", "nucleo-l476rg", firmware, SURVIVAL_CYCLES * 4);
    let s = String::from_utf8_lossy(&uart);
    eprintln!("--- BEGIN UART (final PC={:#010x}) ---", pc);
    eprintln!("{}", s);
    eprintln!("--- END UART ---");
    eprintln!("escaped: {:?}", s);
}

#[test]
fn test_important_core_regression_matrix_is_complete() {
    for core in IMPORTANT_CORES {
        assert!(
            SURVIVAL_CASES.iter().any(|case| case.core == *core),
            "important core {} is missing from SURVIVAL_CASES",
            core
        );
    }
}
