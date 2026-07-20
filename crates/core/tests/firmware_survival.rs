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

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::riscv::RiscV;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::trace::TraceObserver;
use labwired_core::{Cpu, Machine};
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
        // Unmodified Zephyr 3.7 hello_world for nucleo_f401re. Drives the kernel
        // from Cortex SysTick (not an SoC timer), so it exercises the SysTick
        // count-down/reload path and the USART2 console end-to-end.
        name: "stm32f401_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32f401",
        system: "nucleo-f401re",
        fixture: "stm32f401-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"Hello World! nucleo_f401re",
    },
    // Stock Zephyr 3.7 hello_world across the STM32 families that drive the
    // kernel tick from Cortex SysTick. Each exercises that family's RCC
    // ready-bit path (CR oscillators + CSR LSI) and the modern USART TEACK
    // handshake end-to-end. F1 additionally covers the legacy USART + CSR LSI.
    SurvivalCase {
        name: "stm32f103_zephyr",
        core: "cortex-m3",
        family: CpuFamily::CortexM,
        chip: "stm32f103",
        system: "nucleo-f103rb-epaper",
        fixture: "stm32f103-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0801_FFFF), (0x2000_0000, 0x2000_4FFF)],
        expected_uart_output: b"Hello World! nucleo_f103rb",
    },
    SurvivalCase {
        name: "stm32l073_zephyr",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "stm32l073",
        system: "nucleo-l073rz",
        fixture: "stm32l073-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0802_FFFF), (0x2000_0000, 0x2000_4FFF)],
        expected_uart_output: b"Hello World! nucleo_l073rz",
    },
    SurvivalCase {
        name: "stm32l476_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "stm32l476-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_7FFF)],
        expected_uart_output: b"Hello World! nucleo_l476rg",
    },
    SurvivalCase {
        name: "stm32g474_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32g474re",
        system: "nucleo_g474re",
        fixture: "stm32g474-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"Hello World! nucleo_g474re",
    },
    SurvivalCase {
        name: "stm32h563_zephyr",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "stm32h563",
        system: "nucleo-h563zi-demo",
        fixture: "stm32h563-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x200A_0000)],
        expected_uart_output: b"Hello World! nucleo_h563zi",
    },
    SurvivalCase {
        // Dual-core (M4 + M0+): exercises the HSEM inter-core lock (granted to
        // CPU1) and the classic RCC BDCR LSE path.
        name: "stm32wb55_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32wb55",
        system: "mb1355c",
        fixture: "stm32wb55-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"Hello World! nucleo_wb55rg",
    },
    SurvivalCase {
        // Cortex-M33: exercises the WBA-specific RCC (CFGR1@0x1C, BDCR1@0xF0,
        // the 0x28 request/ack) and the PWR VOSR voltage-ready handshake.
        name: "stm32wba52_zephyr",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "stm32wba52",
        system: "nucleo_wba52cg",
        fixture: "stm32wba52-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"Hello World! nucleo_wba52cg",
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
        // Unmodified Zephyr 3.7 hello_world built for `rpi_pico`: exercises the
        // boot2 vector relocation, atomic register aliases, the clock/reset
        // bring-up (RESET_DONE / XOSC / PLL / CLOCKS) and the PL011 console.
        name: "rp2040_zephyr_hello",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "rp2040",
        system: "rp2040-pico",
        fixture: "rp2040-zephyr-hello.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x101F_FFFF), (0x2000_0000, 0x2004_1FFF)],
        expected_uart_output: b"Hello World! rpi_pico",
    },
    SurvivalCase {
        // Plain Arduino sketch built with the Arduino Mbed-OS RP2040 core (board
        // `pico`). Exercises the boot2 / XIP bring-up: the pico-sdk runtime keeps
        // a RAM copy of the stage-2 bootloader (flash_enable_xip_via_boot2) and
        // re-runs it to configure XIP_SSI (0x18000000) + the QSPI flash before
        // execute-in-place from the 0x10000000 window. Without the XIP_SSI model
        // this faulted on the first SSI status poll at 0x18000028.
        name: "rp2040_arduino_serial",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "rp2040",
        system: "rp2040-pico",
        fixture: "rp2040-arduino-serial.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x101F_FFFF), (0x2000_0000, 0x2004_1FFF)],
        expected_uart_output: b"verdict=GOOD rp2040",
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
        // Plain Arduino sketch built with the Adafruit nRF52 core (board
        // `nrf52840_dk`). Serial.println drives the legacy UART personality of
        // UART0/UARTE0 (ENABLE=4): it writes each byte to TXD (0x51C) and spins
        // on EVENTS_TXDRDY (0x11C) until the shifter reports ready. Without the
        // legacy-TXD model that poll never exits, so no bytes are emitted.
        name: "nrf52840_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "nrf52840",
        system: "nrf52840-dk",
        fixture: "nrf52840-arduino-serial.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"verdict=GOOD nrf52",
    },
    SurvivalCase {
        // NXP KW41Z (Cortex-M0+ BLE + 802.15.4). Bare-metal smoke firmware
        // (crates/firmware-kw41z-demo) brings up LPUART0 the way the NXP HAL
        // does — enable CTRL.TE, poll STAT.TDRE, write DATA — and prints the
        // banner below. Exercises the Kinetis LPUART register layout end to
        // end: DATA writes reach the TX sink and STAT reports TDRE/TC.
        name: "kw41z_smoke",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "mkw41z4",
        system: "frdm-kw41z",
        fixture: "kw41z-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_8000, 0x2001_8000)],
        expected_uart_output: b"KW41Z_SMOKE_OK\n",
    },
    SurvivalCase {
        // NXP KW41Z running REAL, unmodified NXP MCUXpresso vendor code
        // (crates/firmware-kw41z-nxp): the genuine system_MKW41Z4.c SystemInit,
        // the verbatim FRDM-KW41Z BOARD_BootClockRUN (RfOscInit + fsl_clock.c
        // CLOCK_SetFeeMode → 40 MHz FEE), and fsl_lpuart.c LPUART_WriteBlocking.
        // Booting this end to end proves the behavioural MCG/RSIM/SIM models
        // satisfy the vendor clock-bring-up spin loops (RSIM RF_OSC_READY,
        // MCG_S IREFST/CLKST/OSCINIT0) instead of hanging. The deterministic
        // register-level twin of this is tests/kw41z_clock_boot.rs.
        name: "kw41z_nxp",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "mkw41z4",
        system: "frdm-kw41z",
        fixture: "kw41z-nxp.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_8000, 0x2001_8000)],
        expected_uart_output: b"KW41Z_NXP_OK\n",
    },
    SurvivalCase {
        // NXP KW41Z running REAL, unmodified upstream Zephyr v3.7 hello_world
        // built for board frdm_kw41z. Boots through the genuine Zephyr Kinetis
        // clock_control (MCG FEE bring-up) and the LPUART0 console, then prints
        // the banner over LPUART0. Proves the behavioural MCG/RSIM/LPUART models
        // satisfy upstream Zephyr's boot path, not just the NXP vendor HAL.
        name: "kw41z_zephyr",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "mkw41z4",
        system: "frdm-kw41z",
        fixture: "kw41z-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_8000, 0x2001_8000)],
        expected_uart_output: b"Hello World! frdm_kw41z",
    },
    SurvivalCase {
        // NXP KW41Z running REAL, unmodified upstream Zephyr v3.7 — the stock
        // samples/sensor/fxos8700 built for frdm_kw41z (hybrid accel+mag, polled).
        // This is a CowManager-style livestock activity node: the genuine Zephyr
        // `fxos8700` sensor driver probes WHOAMI, runs the standby→config→active
        // bring-up and burst-reads OUT_X/Y/Z over I2C1, then prints accel/mag/temp.
        // Booting it end to end exercises the interrupt-driven Kinetis I2C master
        // (peripherals/i2c.rs KinetisI2c, IRQ 9) against the on-board FXOS8700
        // device model (peripherals/components/fxos8700.rs). The "AX=" banner only
        // prints once a real sample has been fetched, so it proves the full
        // I2C transaction + sensor path, not just survival.
        name: "kw41z_zephyr_fxos8700",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "mkw41z4",
        system: "frdm-kw41z",
        fixture: "kw41z-zephyr-fxos8700.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_8000, 0x2001_8000)],
        expected_uart_output: b"AX=",
    },
    SurvivalCase {
        // KW41Z "cattle activity tag": bare-metal firmware (firmware-kw41z-lcd)
        // reads the FXOS8700 over the Kinetis I2C and renders a 3-axis activity
        // bar-graph onto a Nokia-5110 (PCD8544) LCD over the Kinetis DSPI, D/C
        // driven from GPIOC. Exercises KinetisI2c + KinetisDspi + KinetisGpio +
        // the PCD8544 model end to end. The framebuffer render is asserted
        // separately in test_kw41z_lcd_renders_screen.
        name: "kw41z_lcd_activity",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "mkw41z4",
        system: "frdm-kw41z-lcd",
        fixture: "kw41z-lcd-activity.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_8000, 0x2001_8000)],
        expected_uart_output: b"KW41Z_LCD_OK",
    },
    SurvivalCase {
        // Nordic nRF5340 APPLICATION core (Cortex-M33) running REAL, unmodified
        // upstream Zephyr v3.7 hello_world, built for board
        // nrf5340dk/nrf5340/cpuapp. Boots through the genuine Zephyr nRF
        // clock_control (HFCLK/LFCLK start + poll) and nrf_rtc_timer init, then
        // prints the banner over the UARTE0 EasyDMA console. Proves the shared
        // Nordic CLOCK / UARTE / RTC behavioural models satisfy the nRF5340 boot
        // spin-loops at the 0x50000000 non-secure peripheral alias. Fixture is
        // rebuilt via crates/firmware-nrf5340-zephyr/build.sh.
        name: "nrf5340_zephyr",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "nrf5340",
        system: "nrf5340dk",
        fixture: "nrf5340-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2007_FFFF)],
        expected_uart_output: b"Hello World! nrf5340dk/nrf5340/cpuapp",
    },
    // nRF54L15: RRAM-based (NVM at 0x0, 1524 KB) rather than flash, and the
    // 256 KB SRAM puts the initial SP at 0x2004_0000. The PC range covers
    // RRAM; the firmware spins in main after the banner.
    SurvivalCase {
        name: "nrf54l15_smoke",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "nrf54l15",
        system: "nrf54l15dk",
        fixture: "nrf54l15-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0017_CFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"nRF54L15 boot OK",
    },
    // Unmodified upstream Zephyr v4.4 hello_world for nrf54l15dk/nrf54l15/cpuapp.
    // This is the tier marker: the profile satisfies the real nrfx/Zephyr boot
    // path (TAMPC approtect gate, nRF54L CLOCK XO/LFCLK, GRTC, and the
    // nRF54L-generation UARTE with its DMA.TX cluster), not just firmware
    // written against the simulator's own assumptions.
    SurvivalCase {
        name: "nrf54l15_zephyr",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "nrf54l15",
        system: "nrf54l15dk",
        fixture: "nrf54l15-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0017_CFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"Hello World! nrf54l15dk/nrf54l15/cpuapp",
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
        // Plain Arduino sketch (Serial.begin + Serial.println) built by the
        // STM32 Arduino core for nucleo_l476rg. Its SystemClock_Config brings up
        // PLLSAI1 for the 48 MHz clock domain and spins on RCC_CR.PLLSAI1RDY
        // (bit 27) before the first print — exercises the L4 RCC SAI-PLL ready
        // path (RM0351 §6.4.1). Prints the banner below to USART2 (PA2/PA3).
        name: "nucleo_l476rg_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"verdict=GOOD stm32",
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
    SurvivalCase {
        // Round 12: COMP1/COMP2 + TSC + FMC register-state.
        //
        // Captured byte-for-byte from real NUCLEO-L476RG silicon.
        // Round 12 surfaced and fixed three sim<->silicon deltas:
        //   - COMP CSR.VALUE bit (30) reflects comparator output. With
        //     EN=1 on a NUCLEO with floating analog inputs, silicon
        //     settles to VALUE=1. Sim now mirrors EN -> VALUE.
        //   - TSC.ISR after START asserts BOTH EOAF + MCEF on this
        //     board (no real touch sensor wired -> max-counter-error).
        //     Sim previously only set EOAF.
        //   - TSC.IOGCSR.GxS bits stay CLEAR when MCEF fires (group
        //     didn't complete normally). Sim previously mirrored GxE.
        name: "nucleo_l476rg_r12",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-r12.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"R12\r\n\
COMP\r\n\
CSR1=40400031\r\n\
CSR2=00000000\r\n\
TSC\r\n\
CR  =00000001\r\n\
ISR =00000003\r\n\
GCSR=00000005\r\n\
FMC\r\n\
BCR1=000030DB\r\n\
BTR1=0FFFFFFF\r\n\
PCR =00000018\r\n\
SR  =00000040\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // F407 survival-trace smoke — the first F4-family entry in the
        // rotation. Lands with the *simulator-produced* output as the
        // assertion; running this on real F407 silicon (see
        // examples/nucleo-f407-i2c/ORACLE_CAPTURE.md → "Smoke trace"
        // section) will either confirm it byte-for-byte or surface a
        // sim↔silicon divergence to investigate. Iterate by capturing,
        // diffing, fixing the simulator, and re-running.
        //
        // DEV=10016413: F4 DBGMCU IDCODE (DEV_ID 0x413, REV_ID 0x1001).
        // Confirmed by openocd `device id` readout from the user's
        // STM32F407 silicon — see examples/nucleo-f407-i2c/VALIDATION.md
        // Round 1.
        // MUL=369D0368: 0x12345678 * 3 (low 32 bits) — Thumb-2 MUL.W
        // check that pins the decoder.
        name: "nucleo_f407_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32f407",
        system: "nucleo-f407",
        fixture: "nucleo-f407-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"F407 SMOKE\r\nDEV=10016413\r\nMUL=369D0368\r\nDONE\r\n",
    },
    SurvivalCase {
        // F407 Round 2 — I²C1 register-init + START + no-slave address
        // phase + STOP. With `nucleo-f407.yaml::external_devices: []`
        // and no chip wired to the Discovery's PB6/PB7, neither sim
        // nor silicon should ACK the address — sim's AddressPending
        // tick currently sets ADDR unconditionally, real silicon sets
        // SR1.AF instead. Capture is the witness; the divergence (if
        // any) lands as a sim fix.
        //
        // expected_uart_output below is the *simulator-produced* trace;
        // it gets re-locked against silicon when the capture session
        // surfaces the divergence (or confirms a match).
        name: "nucleo_f407_i2c",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32f407",
        system: "nucleo-f407",
        fixture: "nucleo-f407-i2c.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"I2C INIT\r\n\
CR1=00000001\r\n\
CR2=00000010\r\n\
CCR=00000050\r\n\
TRISE=00000011\r\n\
OAR1=00000000\r\n\
SR1=00000000\r\n\
SR2=00000000\r\n\
I2C START\r\n\
SR1=00000001\r\n\
I2C ADDR\r\n\
SR1=00000400\r\n\
SR2=00000003\r\n\
I2C STOP\r\n\
SR1=00000400\r\n\
SR2=00000000\r\n\
DONE\r\n",
    },
    SurvivalCase {
        // Hardware-validated against real NUCLEO-L073RZ silicon (Cortex-M0+,
        // ST-LINK V2 over SWD): the byte stream below was captured from the
        // Virtual COM Port and matches the simulator verbatim. It locks the
        // L0 chip config, the dedicated `stm32l0` RCC layout (CLK=00000004
        // proves the SW->SWS clock-switch readback), the CRC peripheral
        // (B874177A — read off silicon too), DMA mem-to-mem, and the M0+
        // (ARMv6-M) decoder. Drift means a regression in any of those.
        // Appended at the end so the index-based test fns above stay aligned.
        name: "nucleo_l073rz_smoke",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "stm32l073",
        system: "nucleo-l073rz",
        fixture: "nucleo-l073rz-demo.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0802_FFFF), (0x2000_0000, 0x2000_4FFF)],
        expected_uart_output: b"DEV=20086447\nCLK=00000004\nCRC=B874177A\nDMA=OK\n",
    },
];

fn workspace_root() -> PathBuf {
    // crates/core → crates → workspace root (core/)
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixtures() -> PathBuf {
    workspace_root().join("tests/fixtures")
}

fn chip_config(name: &str) -> PathBuf {
    workspace_root()
        .join("configs/chips")
        .join(format!("{name}.yaml"))
}

fn system_config(name: &str) -> PathBuf {
    workspace_root()
        .join("configs/systems")
        .join(format!("{name}.yaml"))
}

fn load_system(chip_name: &str, system_name: &str) -> (ChipDescriptor, SystemManifest) {
    let chip = ChipDescriptor::from_file(chip_config(chip_name))
        .unwrap_or_else(|e| panic!("Failed to load chip {chip_name}: {e}"));

    let sys_path = system_config(system_name);
    let mut manifest = SystemManifest::from_file(&sys_path)
        .unwrap_or_else(|e| panic!("Failed to load system {system_name}: {e}"));

    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();

    (chip, manifest)
}

fn assert_pc_in_range(pc: u32, cycles: u32, ranges: &[(u32, u32)]) {
    assert!(
        ranges
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&pc)),
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
    let cycles = case_cycles(case);
    let (pc, uart_bytes) = match case.family {
        CpuFamily::CortexM => run_cortex_m_firmware(case.chip, case.system, firmware, cycles),
        CpuFamily::RiscV => run_riscv_firmware(case.chip, case.system, firmware, cycles),
    };

    assert_pc_in_range(pc, cycles, case.valid_pc_ranges);
    assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
}

/// Per-case cycle budget. Most firmwares emit their banner within the default
/// window; the RP2040 Arduino Mbed-OS sketch prints over **USB CDC**, and its
/// `loop()` only sends once per `delay(200)` — so it needs a wider window for a
/// loop iteration to land after the simulated host finishes USB enumeration and
/// asserts CDC DTR.
fn case_cycles(case: &SurvivalCase) -> u32 {
    if case.name == "rp2040_arduino_serial" {
        2_000_000
    } else {
        SURVIVAL_CYCLES
    }
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
    let mut bus =
        SystemBus::from_config(&chip, &manifest).expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.observers.push(trace.clone());

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("Failed to load ELF {:?}: {e}", firmware_path));
    machine
        .load_firmware(&image)
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
                    lr.map(|v| format!("{v:#010x}"))
                        .unwrap_or_else(|| "-".to_string()),
                    sp.map(|v| format!("{v:#010x}"))
                        .unwrap_or_else(|| "-".to_string()),
                    pc.map(|v| format!("{v:#010x}"))
                        .unwrap_or_else(|| "-".to_string()),
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
    let mut bus =
        SystemBus::from_config(&chip, &manifest).expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);
    let mut machine = Machine::new(RiscV::new(), bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.observers.push(trace.clone());

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("Failed to load ELF {:?}: {e}", firmware_path));
    machine
        .load_firmware(&image)
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
                    pc.map(|v| format!("{v:#010x}"))
                        .unwrap_or_else(|| "-".to_string()),
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

/// Lookup a `SurvivalCase` by name. Panics immediately if the name is not
/// found, so index-drift bugs fail at the test boundary rather than silently
/// running the wrong case.
fn case_by_name(name: &str) -> &'static SurvivalCase {
    SURVIVAL_CASES
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no SurvivalCase named {name:?}"))
}

#[test]
fn test_stm32f103_blinky_survival() {
    run_survival_case(case_by_name("stm32f103_blinky"));
}

#[test]
fn test_stm32f401_blinky_survival() {
    run_survival_case(case_by_name("stm32f401_blinky"));
}

#[test]
fn test_stm32f401_zephyr_survival() {
    run_survival_case(case_by_name("stm32f401_zephyr"));
}

#[test]
fn test_stm32f103_zephyr_survival() {
    run_survival_case(case_by_name("stm32f103_zephyr"));
}

#[test]
fn test_stm32l073_zephyr_survival() {
    run_survival_case(case_by_name("stm32l073_zephyr"));
}

#[test]
fn test_stm32l476_zephyr_survival() {
    run_survival_case(case_by_name("stm32l476_zephyr"));
}

#[test]
fn test_stm32g474_zephyr_survival() {
    run_survival_case(case_by_name("stm32g474_zephyr"));
}

#[test]
fn test_stm32h563_zephyr_survival() {
    run_survival_case(case_by_name("stm32h563_zephyr"));
}

#[test]
fn test_stm32wb55_zephyr_survival() {
    run_survival_case(case_by_name("stm32wb55_zephyr"));
}

#[test]
fn test_stm32wba52_zephyr_survival() {
    run_survival_case(case_by_name("stm32wba52_zephyr"));
}

#[test]
fn test_rp2040_demo_survival() {
    run_survival_case(case_by_name("rp2040_demo"));
}

#[test]
fn test_rp2040_zephyr_hello_survival() {
    run_survival_case(case_by_name("rp2040_zephyr_hello"));
}

/// Arduino Mbed-OS RP2040 serial sketch — full end-to-end over **USB CDC**.
///
/// This is the deepest RP2040 path modelled: it exercises the boot2/XIP bring-up
/// (`XIP_SSI` + SIO spinlocks + TBMAN), the RTX kernel tick (real `systick` +
/// TIMER alarms), and the whole USB device stack. The sketch's default `Serial`
/// is USB CDC, so `verdict=GOOD rp2040` is only emitted once the device has been
/// enumerated by a host and the CDC terminal (DTR) is up. The `rp2040_usb`
/// peripheral supplies both the device controller and a simulated host that
/// enumerates the device and asserts DTR, then captures the sketch's bulk-IN
/// bytes into the UART sink.
///
/// Uses a wider cycle budget (see `case_cycles`) because the sketch's `loop()`
/// only transmits once per `delay(200)`, so a loop iteration must land after the
/// simulated host finishes enumeration.
#[test]
fn test_rp2040_arduino_serial_survival() {
    run_survival_case(case_by_name("rp2040_arduino_serial"));
}

#[test]
fn test_nrf52840_demo_survival() {
    run_survival_case(case_by_name("nrf52840_demo"));
}

#[test]
fn test_nrf52840_arduino_serial_survival() {
    run_survival_case(case_by_name("nrf52840_arduino_serial"));
}

#[test]
fn test_nrf52832_demo_survival() {
    run_survival_case(case_by_name("nrf52832_demo"));
}

#[test]
fn test_nrf5340_zephyr_survival() {
    run_survival_case(case_by_name("nrf5340_zephyr"));
}

#[test]
fn test_nrf54l15_smoke_survival() {
    run_survival_case(case_by_name("nrf54l15_smoke"));
}

/// End-to-end proof that GPIO reaches the pin, not just that the CPU survived.
///
/// This exists because the UART banner alone does NOT catch the most likely
/// nRF54L15 profile bug. A Nordic GPIO devicetree node points at the OUT
/// register (peripheral base + 0x500), so mapping the DT address as the
/// peripheral base puts every GPIO register 0x500 too high. UART still works,
/// the banner still prints, the survival test still passes — and the LED
/// silently never lights. That mistake was made and caught here.
#[test]
fn test_nrf54l15_lights_dk_led0() {
    use labwired_core::Bus;

    // DK LED0 is P2.09 (board DT nrf54l15dk_common.dtsi, GPIO_ACTIVE_HIGH).
    // Mapped P2 base = MDK NRF_P2_S_BASE (0x5005_0400) - 0x504, so the gpio
    // model's nRF52-relative offsets land on the real registers: OUT ends up at
    // 0x5005_0400 and DIR at 0x5005_0410, which is where the MDK puts them on
    // this family (NRF_GPIO_Type has OUT at +0x000 here, unlike nRF52/nRF5340).
    const GPIO_P2: u64 = 0x5004_FEFC;
    const GPIO_OUT: u64 = 0x504;
    const GPIO_DIR: u64 = 0x514;
    const LED0: u32 = 1 << 9;

    let (chip, manifest) = load_system("nrf54l15", "nrf54l15dk");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image =
        labwired_loader::load_elf(&fixtures().join("nrf54l15-smoke.elf")).expect("load smoke elf");
    machine.load_firmware(&image).expect("load fw");
    for _ in 0..SURVIVAL_CYCLES {
        if machine.step().is_err() {
            break;
        }
    }

    let dir = machine.bus.read_u32(GPIO_P2 + GPIO_DIR).expect("read DIR");
    let out = machine.bus.read_u32(GPIO_P2 + GPIO_OUT).expect("read OUT");

    assert_ne!(
        dir & LED0,
        0,
        "P2.09 was never configured as an output (DIRSET did not land)"
    );
    assert_ne!(
        out & LED0,
        0,
        "P2.09 is an output but was never driven high — DK LED0 stayed dark"
    );
}

#[test]
fn test_nrf54l15_zephyr_survival() {
    run_survival_case(case_by_name("nrf54l15_zephyr"));
}

#[test]
fn test_kw41z_smoke_survival() {
    run_survival_case(case_by_name("kw41z_smoke"));
}

#[test]
fn test_kw41z_nxp_survival() {
    run_survival_case(case_by_name("kw41z_nxp"));
}

#[test]
fn test_kw41z_zephyr_survival() {
    run_survival_case(case_by_name("kw41z_zephyr"));
}

#[test]
fn test_kw41z_lcd_activity_survival() {
    run_survival_case(case_by_name("kw41z_lcd_activity"));
}

/// End-to-end proof that the activity bar-graph reaches the screen: boot the
/// firmware, then read back the PCD8544 model's framebuffer and confirm the
/// display was turned on and real pixels were drawn — i.e. the FXOS8700 read
/// (Kinetis I2C), the DSPI master, the GPIO D/C latch and the display model all
/// cooperated, not just that the CPU survived.
#[test]
fn test_kw41z_lcd_renders_screen() {
    use labwired_core::peripherals::components::Pcd8544;
    use labwired_core::peripherals::spi::Spi;

    let (chip, manifest) = load_system("mkw41z4", "frdm-kw41z-lcd");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&fixtures().join("kw41z-lcd-activity.elf"))
        .expect("load lcd elf");
    machine.load_firmware(&image).expect("load fw");
    for _ in 0..SURVIVAL_CYCLES {
        if machine.step().is_err() {
            break;
        }
    }

    let lcd = machine
        .bus
        .peripherals
        .iter()
        .filter_map(|p| p.dev.as_any().and_then(|a| a.downcast_ref::<Spi>()))
        .flat_map(|spi| spi.attached_devices.iter())
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Pcd8544>()))
        .expect("PCD8544 attached to an SPI bus");

    assert!(lcd.display_on(), "PCD8544 display was never turned on");
    let fb = lcd.framebuffer();
    let lit = fb.iter().filter(|&&b| b != 0).count();
    assert!(
        lit > 0,
        "PCD8544 framebuffer is blank — no cow was rendered"
    );

    // ASCII snapshot of the 84x48 Nokia-5110 screen (bank-major, 8 px/byte).
    // Printed BEFORE the feature assertions below so a failure still leaves
    // the rendered cow visible in the test output.
    eprintln!("┌{}┐", "─".repeat(84));
    for bank in 0..6 {
        for sub in 0..8 {
            let mut row = String::with_capacity(84);
            for x in 0..84 {
                let byte = fb[bank * 84 + x];
                row.push(if (byte >> sub) & 1 != 0 { '#' } else { ' ' });
            }
            eprintln!("│{row}│");
        }
    }
    eprintln!("└{}┘", "─".repeat(84));
    eprintln!("PCD8544 rendered: {lit} non-blank framebuffer bytes, display ON — cute cow face");

    // Pixel-level helper matching the firmware's own bank-major addressing
    // (fb[(y/8)*84 + x], bit y%8), so we can assert on specific cow features
    // rather than just "something is lit".
    let px = |x: usize, y: usize| -> bool { (fb[(y / 8) * 84 + x] >> (y % 8)) & 1 != 0 };

    // Head outline: the ellipse's left/right extremes at its vertical center
    // (cx=42, calm/grazing cy=22, rx=25) must be lit. The sensor's idle sway
    // peaks below the ACTIVE threshold, so the boot pose is always the calm
    // grazing one.
    assert!(px(17, 22) || px(18, 22), "cow head left edge missing");
    assert!(px(66, 22) || px(67, 22), "cow head right edge missing");
    // Muzzle nostrils (mx=42, my=31, offsets ±5,-1): two solid dots.
    assert!(px(37, 30), "left nostril missing");
    assert!(px(47, 30), "right nostril missing");
    // The whole cow reads as more than a couple of bars: expect a healthy
    // number of lit framebuffer bytes across the face + banner + meter.
    assert!(
        lit > 150,
        "framebuffer has too few non-blank bytes ({lit}) for a full cow face"
    );

    // The bottom-edge activity meter track (row 46) must span the full width.
    for x in 0..84 {
        assert!(px(x, 46), "activity meter track missing at column {x}");
    }
}

/// The demo's whole point, locked in end to end: driving the (interactive)
/// FXOS8700 must change the rendered cow MACROSCOPICALLY. This guards against
/// the "demo looks dead" regression class — the calm→active flip moves the
/// head, swaps the banner for an inverted MOO! band, adds motion lines and a
/// chunky meter, so a hard tilt must change hundreds of pixels, not a few.
#[test]
fn test_kw41z_lcd_cow_reacts_to_tilt() {
    use labwired_core::peripherals::components::{Fxos8700, Pcd8544};
    use labwired_core::peripherals::i2c::I2c;
    use labwired_core::peripherals::spi::Spi;

    let (chip, manifest) = load_system("mkw41z4", "frdm-kw41z-lcd");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&fixtures().join("kw41z-lcd-activity.elf"))
        .expect("load lcd elf");
    machine.load_firmware(&image).expect("load fw");

    let grab_fb = |machine: &Machine<_>| -> Vec<u8> {
        machine
            .bus
            .peripherals
            .iter()
            .filter_map(|p| p.dev.as_any().and_then(|a| a.downcast_ref::<Spi>()))
            .flat_map(|spi| spi.attached_devices.iter())
            .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Pcd8544>()))
            .expect("PCD8544 attached to an SPI bus")
            .framebuffer()
            .to_vec()
    };

    // Boot to the calm grazing pose (idle sensor sway stays below threshold).
    for _ in 0..SURVIVAL_CYCLES {
        if machine.step().is_err() {
            break;
        }
    }
    let fb_calm = grab_fb(&machine);

    // Latch a hard tilt into the sensor — the same `set_sample` path the
    // playground sliders use through `set_i2c_sensor_sample`.
    let mut found = false;
    for p in machine.bus.peripherals.iter_mut() {
        let Some(any) = p.dev.as_any_mut() else {
            continue;
        };
        let Some(i2c) = any.downcast_mut::<I2c>() else {
            continue;
        };
        for device in i2c.attached_devices() {
            let mut device = device.borrow_mut();
            if let Some(sensor) = device
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Fxos8700>())
            {
                sensor.set_sample(0x2000, -0x2000, 0x1000); // 2 g X, -2 g Y
                found = true;
            }
        }
    }
    assert!(found, "no FXOS8700 attached to an I2C bus");

    for _ in 0..SURVIVAL_CYCLES {
        if machine.step().is_err() {
            break;
        }
    }
    let fb_active = grab_fb(&machine);

    // ASCII snapshot of the ACTIVE pose (the calm one is dumped by
    // test_kw41z_lcd_renders_screen) so both moods can be eyeballed.
    eprintln!("┌{}┐", "─".repeat(84));
    for bank in 0..6 {
        for sub in 0..8 {
            let mut row = String::with_capacity(84);
            for x in 0..84 {
                let byte = fb_active[bank * 84 + x];
                row.push(if (byte >> sub) & 1 != 0 { '#' } else { ' ' });
            }
            eprintln!("│{row}│");
        }
    }
    eprintln!("└{}┘", "─".repeat(84));

    let diff: u32 = fb_calm
        .iter()
        .zip(fb_active.iter())
        .map(|(a, b)| (*a ^ *b).count_ones())
        .sum();
    assert!(
        diff >= 300,
        "cow reaction too subtle: only {diff} pixels changed between calm and hard-tilt \
         — the demo would look dead in the playground"
    );
}

/// Regression (browser "cow" blank via the wasm `new_from_config` path):
///
/// The FRDM-KW41Z LCD blanked in the deployed browser while the native test
/// rendered the cow. Root cause was NOT the display model but the GPIO factory:
/// a `type: gpio` port with no `config.profile` silently defaulted to STM32F1
/// (ODR @0x0C), so the PCD8544 D/C line — which the bus latches from the
/// driving GPIO's output register — resolved to an address the Kinetis firmware
/// (PDOR @0x00) never drives. D/C stayed low, every pixel byte decoded as a
/// command, and the panel rendered blank. Critically the display attach did
/// NOT error, because `resolve_pin_odr` DID find a `GpioPort` (just the wrong
/// family) and returned a valid-but-wrong offset — bypassing the pcd8544
/// fail-loud guard.
///
/// This test locks in BOTH halves of the fix, on the same `SystemBus::from_config`
/// path the wasm `WasmSimulator::new_from_config` browser entry uses:
///   1. The real KW41Z config builds gpioc as a *behavioural* Kinetis `GpioPort`
///      (ODR/PDOR @0x00), and D/C ("PC0") resolves to `gpioc_base + 0x00`.
///   2. Stripping gpioc's `profile` makes `from_config` FAIL LOUD (no silent
///      STM32F1 fallback), instead of building a bus that renders a blank panel.
#[test]
fn kw41z_gpioc_is_behavioural_kinetis_and_profileless_gpio_errors() {
    use labwired_core::peripherals::gpio::GpioPort;

    let (chip, manifest) = load_system("mkw41z4", "frdm-kw41z-lcd");

    // --- Half 1: the shipped config yields a behavioural Kinetis GPIOC. ---
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus builds from real config");
    let idx = bus
        .find_peripheral_index_by_name("gpioc")
        .expect("gpioc peripheral present");
    let base = bus.peripherals[idx].base;
    let gpioc = bus.peripherals[idx]
        .dev
        .as_any()
        .and_then(|a| a.downcast_ref::<GpioPort>())
        .expect("gpioc must be a behavioural GpioPort, not a passive declarative bank");
    assert_eq!(
        gpioc.odr_offset(),
        0x00,
        "gpioc must use the Kinetis layout (PDOR/output @0x00), not STM32F1 (@0x0C)"
    );
    // D/C pin (PC0) must resolve to the Kinetis output register @ base + 0x00.
    let dc = SystemBus::resolve_pin_odr_pub(&bus, "PC0")
        .expect("PC0 must resolve to a driveable GPIO output");
    assert_eq!(
        dc,
        (base, 0),
        "PCD8544 D/C (PC0) must latch from the Kinetis output register at gpioc base + 0x00"
    );

    // --- Half 2: a profileless bare `type: gpio` must fail loudly. ---
    let mut broken = chip.clone();
    let gpioc_cfg = broken
        .peripherals
        .iter_mut()
        .find(|p| p.id == "gpioc")
        .expect("gpioc in chip descriptor");
    assert_eq!(gpioc_cfg.r#type, "gpio");
    gpioc_cfg.config.remove("profile");
    gpioc_cfg.config.remove("register_layout");
    let err = match SystemBus::from_config(&broken, &manifest) {
        Ok(_) => panic!("profileless bare `type: gpio` must NOT silently default to STM32F1"),
        Err(e) => e,
    };
    let msg = format!("{err:#}");
    assert!(
        msg.contains("gpioc") && msg.contains("profile"),
        "error must name the peripheral and demand an explicit profile, got: {msg}"
    );
    // The error must surface at bus-construction time (from_config), not after
    // the machine has run and quietly rendered a blank panel.
    assert!(
        msg.contains("STM32F1"),
        "error should explain it refuses the silent STM32F1 fallback, got: {msg}"
    );
}

/// End-to-end proof that the universal bus-trace logic analyzer (Tasks 1-2)
/// captures REAL transactions from the unmodified KW41Z-LCD demo firmware:
/// an I2C address frame for the FXOS8700 accelerometer and at least one SPI
/// frame for the PCD8544 LCD (on spi0). This is the same machine setup as
/// `test_kw41z_lcd_renders_screen` (the `SystemBus::from_config` path that
/// wires `set_bus_trace` before devices are attached), just reading the
/// trace log instead of the framebuffer.
///
/// NOTE: the FXOS8700 7-bit address on THIS board's config
/// (`configs/systems/frdm-kw41z-lcd.yaml`, mirrored in
/// `peripherals/components/fxos8700.rs::default()`) is `0x1f`, not the
/// `0x1E` used in some external references/datasheets for other SA0/SA1
/// strap combos. Asserting against the actual wired address (0x1f).
#[test]
fn kw41z_lcd_bus_trace_captures_i2c_and_spi() {
    use labwired_core::bus::bus_trace::{BusPayload, I2cSym};

    let (chip, manifest) = load_system("mkw41z4", "frdm-kw41z-lcd");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&fixtures().join("kw41z-lcd-activity.elf"))
        .expect("load lcd elf");
    machine.load_firmware(&image).expect("load fw");
    for _ in 0..SURVIVAL_CYCLES {
        if machine.step().is_err() {
            break;
        }
    }

    let events = machine.bus.bus_trace_snapshot();
    eprintln!("bus trace captured {} events", events.len());
    // The trace ring is bounded (BUS_TRACE_LIMIT), so by the end of a long run it
    // can be dominated by the LCD's continuous SPI redraws. Print a few samples
    // from each bus (not just the head) so both protocols are visible in evidence.
    for ev in events.iter().filter(|e| e.bus == "i2c1").take(6) {
        eprintln!("  [i2c1] seq={} payload={:?}", ev.seq, ev.payload);
    }
    for ev in events.iter().filter(|e| e.bus == "spi0").take(6) {
        eprintln!("  [spi0] seq={} payload={:?}", ev.seq, ev.payload);
    }
    assert!(
        !events.is_empty(),
        "bus trace is empty — tracing wrappers were not engaged for the kw41z-lcd firmware path"
    );

    let fxos8700_addr_event = events.iter().any(|ev| {
        ev.bus == "i2c1"
            && matches!(
                &ev.payload,
                BusPayload::I2c { kind: I2cSym::AddrWrite | I2cSym::AddrRead, byte, .. }
                    if (byte >> 1) == 0x1f
            )
    });
    assert!(
        fxos8700_addr_event,
        "no I2C address frame for the FXOS8700 (0x1f) seen on i2c1; events: {events:?}"
    );

    let i2c_data_event = events.iter().any(|ev| {
        ev.bus == "i2c1"
            && matches!(
                &ev.payload,
                BusPayload::I2c {
                    kind: I2cSym::Data,
                    ..
                }
            )
    });
    assert!(
        i2c_data_event,
        "no I2C data event seen on i2c1; events: {events:?}"
    );

    let spi_event = events
        .iter()
        .any(|ev| ev.bus == "spi0" && matches!(&ev.payload, BusPayload::Spi { .. }));
    assert!(
        spi_event,
        "no SPI frame seen on spi0 (PCD8544 LCD); events: {events:?}"
    );
}

#[test]
fn test_kw41z_zephyr_fxos8700_survival() {
    // The stock fxos8700 sample sleeps k_sleep(K_MSEC(160)) before its first
    // fetch+print; at the KW41Z's 40 MHz that is ~6.4M cycles, so this fixture
    // needs a larger budget than the default to reach the first "AX=" line.
    let case = case_by_name("kw41z_zephyr_fxos8700");
    let (pc, uart_bytes) = run_cortex_m_firmware(
        case.chip,
        case.system,
        fixtures().join(case.fixture),
        8_000_000,
    );
    assert_pc_in_range(pc, 8_000_000, case.valid_pc_ranges);
    assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
}

#[test]
fn test_stm32h563_demo_survival() {
    run_survival_case(case_by_name("stm32h563_demo"));
}

#[test]
fn test_riscv_ci_fixture_survival() {
    run_survival_case(case_by_name("riscv_ci_fixture"));
}

#[test]
fn test_esp32c3_demo_survival() {
    run_survival_case(case_by_name("esp32c3_demo"));
}

#[test]
fn test_nucleo_l476rg_smoke_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_smoke"));
}

#[test]
fn test_nucleo_l476rg_spi_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_spi"));
}

#[test]
fn test_nucleo_l476rg_pll_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_pll"));
}

#[test]
fn test_nucleo_l476rg_misc_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_misc"));
}

#[test]
fn test_nucleo_l476rg_l4periphs_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_l4periphs"));
}

#[test]
fn test_nucleo_l476rg_demo_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_demo"));
}

#[test]
fn test_nucleo_l476rg_dma_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_dma"));
}

#[test]
fn test_nucleo_l476rg_adc_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_adc"));
}

#[test]
fn test_nucleo_l476rg_i2c_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_i2c"));
}

#[test]
fn test_nucleo_l476rg_l4periphs2_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_l4periphs2"));
}

#[test]
fn test_nucleo_l476rg_tim1_advanced_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_tim1_advanced"));
}

#[test]
fn test_nucleo_l476rg_r11_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_r11"));
}

#[test]
fn test_nucleo_l476rg_r12_survival() {
    run_survival_case(case_by_name("nucleo_l476rg_r12"));
}

#[test]
fn test_nucleo_f407_smoke_survival() {
    run_survival_case(case_by_name("nucleo_f407_smoke"));
}

#[test]
fn test_nucleo_f407_i2c_survival() {
    // Capture and print whatever sim produces — the literal in
    // SURVIVAL_CASES gets updated to match after the first run, then
    // diffed against silicon.
    let case = case_by_name("nucleo_f407_i2c");
    let firmware = fixtures().join(case.fixture);
    let (pc, uart) = run_cortex_m_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES * 2);
    assert_pc_in_range(pc, SURVIVAL_CYCLES * 2, case.valid_pc_ranges);
    eprintln!("--- F407 I2C SIM UART ---");
    eprintln!("{}", String::from_utf8_lossy(&uart));
    eprintln!("--- END ---");
    eprintln!("escaped: {:?}", String::from_utf8_lossy(&uart));
    assert_uart_contains(&uart, case.expected_uart_output, case.name);
}

#[test]
fn test_nucleo_l073rz_smoke_survival() {
    run_survival_case(case_by_name("nucleo_l073rz_smoke"));
}

#[test]
fn test_nucleo_l476rg_cubemx_hal_survival() {
    // HAL flow needs more cycles than other tests because it spends most
    // of its time in HAL_Delay() polling SysTick (RVR=80_000-1).
    let case = case_by_name("nucleo_l476rg_cubemx_hal");
    let firmware = fixtures().join(case.fixture);
    let (pc, uart_bytes) =
        run_cortex_m_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES * 4);
    assert_pc_in_range(pc, SURVIVAL_CYCLES * 4, case.valid_pc_ranges);
    assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
}

#[test]
fn test_nucleo_l476rg_arduino_serial_survival() {
    // Regression for the STM32L4 PLLSAI1RDY boot hang: a plain Arduino sketch
    // hangs in SystemClock_Config polling RCC_CR.PLLSAI1RDY (bit 27) unless the
    // RCC model sets that flag when PLLSAI1ON (bit 26) is enabled.
    let case = case_by_name("nucleo_l476rg_arduino_serial");
    let firmware = fixtures().join(case.fixture);
    let (pc, uart_bytes) = run_cortex_m_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES);
    assert_pc_in_range(pc, SURVIVAL_CYCLES, case.valid_pc_ranges);
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
fn capture_r12_sim_output() {
    let firmware = fixtures().join("nucleo-l476rg-r12.elf");
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
