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
    /// A committed fixture run through the `Session` builder, waiting on the
    /// case's `expected_uart_output` marker instead of stepping a fixed cycle
    /// budget, so the run costs the cycles to the marker, not the CLI's step
    /// cap. Tier-1 self-test fixtures (addressed as `tier1/<name>.elf`)
    /// additionally gate on the class transcript via `requires_tier1_classes`
    /// and `assert_tier1_classes_pass`; demo smoke fixtures at the fixtures
    /// root gate on their marker alone. The Session path exposes no register
    /// read-back, so these cases carry `valid_pc_ranges: &[]`.
    Session,
}

/// Which toolchain produced the fixture. Lets coverage-per-HAL be read off the
/// table instead of inferred from case-name substrings.
// Read by the HAL-coverage assertions added in Task 8.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hal {
    /// Hand-written `no_std` / vendor-HAL firmware.
    Bare,
    /// Unmodified upstream Zephyr build.
    Zephyr,
    /// Arduino core sketch.
    Arduino,
    /// Unmodified upstream Contiki-NG build.
    Contiki,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SurvivalCase {
    name: &'static str,
    core: &'static str,
    family: CpuFamily,
    hal: Hal,
    chip: &'static str,
    system: &'static str,
    fixture: &'static str,
    valid_pc_ranges: &'static [(u32, u32)],
    /// For `CortexM`/`RiscV` cases, bytes that must appear somewhere in the UART
    /// output after the case's cycle budget. For `Session` cases, the marker
    /// `expect` waits for; Tier-1 (`tier1/`) Session cases additionally gate on
    /// the class transcript via `requires_tier1_classes`, while the demo cases
    /// gate on the marker alone. Proves the firmware executed real application
    /// logic, not just a reset loop.
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
        hal: Hal::Arduino,
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
        hal: Hal::Arduino,
        chip: "stm32f401",
        system: "nucleo-f401re",
        fixture: "stm32f401-blinky.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        // Same `blink_sketch.ino.cpp` as the F103 case above, built by the STM32
        // Arduino core for nucleo_f401re — so the banner below is the *firmware's*
        // string, cross-checked against the F103 sibling, not a transcript of
        // whatever the simulator happened to print.
        //
        // This case used to assert `b""`, which asserted nothing: the empty-expected
        // early return in `assert_uart_contains` skipped the check entirely, leaving
        // only the PC-range test, so a wedged USART2 would still have passed. The
        // comment justifying it ("does not yet produce deterministic UART bytes")
        // was stale — the F401 model emits these 45 bytes deterministically, byte
        // for byte, at every budget from 200k to 3.2M cycles.
        //
        // `LED ON` (from `loop()`, not `setup()`) is included deliberately: it
        // proves the sketch reached its main loop, not just the banner in setup().
        // The next line only arrives after the sketch's `delay()` elapses — at
        // 100M cycles this prints ON/OFF/ON, while the 72 MHz F103 gets one more
        // toggle in the same cycle count, which is exactly what a fixed
        // millisecond delay predicts for the faster 84 MHz part.
        expected_uart_output: b"LabWired Playground - Arduino Blink\r\nLED ON\r\n",
    },
    SurvivalCase {
        // Unmodified Zephyr 3.7 hello_world for nucleo_f401re. Drives the kernel
        // from Cortex SysTick (not an SoC timer), so it exercises the SysTick
        // count-down/reload path and the USART2 console end-to-end.
        name: "stm32f401_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
        chip: "stm32wb55",
        system: "mb1355c",
        fixture: "stm32wb55-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"Hello World! nucleo_wb55rg",
    },
    SurvivalCase {
        // Cortex-M33: exercises the WBA-specific RCC (CFGR1@0x1C, BDCR1@0xF0,
        // the PLL1CFGR PLL1RCLKPRE → PLL1RCLKPRERDY handshake at 0x28) and the
        // PWR VOSR voltage-ready handshake.
        name: "stm32wba52_zephyr",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Zephyr,
        chip: "stm32wba52",
        system: "nucleo_wba52cg",
        fixture: "stm32wba52-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"Hello World! nucleo_wba52cg",
    },
    SurvivalCase {
        // Stock Zephyr 3.7.2 hello_world for `nucleo_u575zi_q` (Zephyr commit
        // c66235fb7346). First U5 part: exercises the U5 RCC (PLL1CFGR/DIVR/
        // FRACR @0x28/0x34/0x38 plus the MSISRDY/HSI48RDY ready pairs) and the
        // USART1 VCP console end to end. The committed ELF is the stock sample
        // build — NOT the Zephyr matrix's in-tree `LW_Z0_OK` app.
        name: "stm32u575_zephyr",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Zephyr,
        chip: "stm32u575",
        system: "nucleo-u575zi",
        fixture: "stm32u575-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x200B_FFFF)],
        expected_uart_output: b"Hello World! nucleo_u575zi_q",
    },
    SurvivalCase {
        name: "rp2040_demo",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Arduino,
        chip: "rp2040",
        system: "rp2040-pico",
        fixture: "rp2040-arduino-serial.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x101F_FFFF), (0x2000_0000, 0x2004_1FFF)],
        expected_uart_output: b"verdict=GOOD rp2040",
    },
    SurvivalCase {
        // NUCLEO-L552ZE-Q: STM32L552ZET6Q (Cortex-M33 / Armv8-M), 512 KiB flash
        // at 0x08000000, 192 KiB SRAM1 at 0x20000000 (+64 KiB SRAM2 at
        // 0x20030000). LPUART1 (PG7/PG8, AF8) is the VCP console; the smoke
        // walks the PC7/PB7/PA9 LEDs with ODR readback and reads PC13.
        // SIM-DERIVED (no STM32L552 bench part); TrustZone/MPU not modelled.
        name: "nucleo_l552ze_q_smoke",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "stm32l552",
        system: "nucleo-l552ze-q",
        fixture: "nucleo-l552ze-q-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"NUCLEO-L552ZE-Q SMOKE\n",
    },
    SurvivalCase {
        // CC2538DK: CC2538SF53 (Cortex-M3), 512 KiB flash at 0x00200000 (the
        // ROM, not flash, sits at 0x0), 32 KiB SRAM at 0x20000000. UART0
        // (PA0/PA1 via the IOC) is the console; the smoke reads CLOCK_STA,
        // derives IBRD/FBRD and walks LED2-LED4 (PC1-PC3) with address-masked
        // GPIO_DATA readback. SIM-DERIVED.
        name: "cc2538dk_smoke",
        core: "cortex-m3",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "cc2538sf53",
        system: "cc2538dk",
        fixture: "cc2538dk-smoke.elf",
        valid_pc_ranges: &[(0x0020_0000, 0x0027_FFFF), (0x2000_0000, 0x2000_7FFF)],
        expected_uart_output: b"CC2538DK SMOKE\n",
    },
    SurvivalCase {
        // CC2538DK running REAL, unmodified upstream Contiki-NG v4.9 —
        // examples/hello-world for TARGET=cc2538dk with the full default
        // netstack (RPL Lite, 6LoWPAN, CSMA on the cc2538 RF driver), the
        // binary Renode's cc2538_single-node.robot runs. It boots through the
        // in-tree clean-room ROM (CCA -> VTOR 0x0020_2000, the 0x48 utility
        // API for .data/.bss), waits out SYNC_32K, seeds its PRNG from the RF
        // core (ISRXON, RSSI_VALID, RFRND), brings up the radio, reads its
        // IEEE address from the info page and prints from its etimer process.
        // `fade()` in platform_init busy-loops ~0.5 s of 16-MHz cycles first,
        // hence the wider budget (see `case_cycles`). Rebuild:
        // crates/firmware-cc2538dk-contiki/build.sh.
        name: "cc2538dk_contiki_hello",
        core: "cortex-m3",
        family: CpuFamily::CortexM,
        hal: Hal::Contiki,
        chip: "cc2538sf53",
        system: "cc2538dk",
        fixture: "cc2538dk-contiki-hello.elf",
        valid_pc_ranges: &[
            (0x0000_0000, 0x0001_FFFF),
            (0x0020_0000, 0x0027_FFFF),
            (0x2000_0000, 0x2000_7FFF),
        ],
        expected_uart_output: b"Hello, world",
    },
    SurvivalCase {
        // EK-RA6M5: R7FA6M5BH (Cortex-M33), 2 MiB code flash at 0x0, 512 KiB
        // SRAM at 0x20000000. SCI0 (P411/P410) is the J-Link OB console; the
        // smoke runs on the reset MOCO, walks LED1-LED3 (P006-P008) with
        // PCNTR1 PODR readback and reads S1/S2 through the PFS pull-ups.
        // SIM-DERIVED.
        name: "ek_ra6m5_smoke",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "r7fa6m5bh",
        system: "ek-ra6m5",
        fixture: "ek-ra6m5-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x001F_FFFF), (0x2000_0000, 0x2007_FFFF)],
        expected_uart_output: b"EK-RA6M5 SMOKE\n",
    },
    SurvivalCase {
        // Apollo4 Plus EVB: AMAP42KP-KBR (Cortex-M4F), 2 MiB MRAM at 0x0 with
        // the application vector table at 0x18000 (the secure bootloader owns
        // 0x0-0x17FFF and is not modelled), RAM from 0x10000000. UART0
        // (GPIO60/47) is the console after the PWRCTRL power handshake; the
        // smoke walks LED0-LED2 (GPIO30/90/97) with RD readback. SIM-DERIVED.
        name: "apollo4p_evb_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "amap42kp",
        system: "apollo4p-evb",
        fixture: "apollo4p-evb-smoke.elf",
        valid_pc_ranges: &[(0x0001_8000, 0x001F_FFFF), (0x1000_0000, 0x1005_FFFF)],
        expected_uart_output: b"APOLLO4P EVB SMOKE\n",
    },
    SurvivalCase {
        // Stock Zephyr v3.7.0 hello_world for `apollo4p_evb` (Zephyr commit
        // 36940db938a8, hal_ambiq e25327f026df,
        // crates/firmware-apollo4p-zephyr/build.sh). The AmbiqSuite HAL runs
        // unpatched: am_hal_pwrctrl_low_power_init powers domains through
        // PWRCTRL, flushes the DAXI (CPU block, DAXIREADY) via SYNC_READ, and
        // am_hal_delay_us calls the boot ROM's br_util_delay_cycles at
        // 0x0800_009D (the in-tree clean-room table, roms/apollo4).
        name: "apollo4p_evb_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Zephyr,
        chip: "amap42kp",
        system: "apollo4p-evb",
        fixture: "amap42kp-zephyr-hello.elf",
        valid_pc_ranges: &[
            (0x0001_8000, 0x001F_FFFF),
            (0x0800_0000, 0x0800_0FFF),
            (0x1000_0000, 0x102B_FFFF),
        ],
        expected_uart_output: b"Hello World! apollo4p_evb",
    },
    SurvivalCase {
        // FRDM-K64F: MK64FN1M0VLL12 (Cortex-M4F), 1 MiB flash at 0x00000000,
        // 64 KiB SRAM_L at 0x1FFF0000 + 192 KiB SRAM_U at 0x20000000. UART0
        // (PTB16/PTB17, ALT3) is the OpenSDA VCP console; the smoke walks the
        // active-low RGB LED with PDOR readback. UART-only L1: ENET/TCP and the
        // MCG PLL are deferred. SIM-DERIVED.
        name: "frdm_k64f_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "mk64f12",
        system: "frdm-k64f",
        fixture: "frdm-k64f-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_0000, 0x2002_FFFF)],
        expected_uart_output: b"FRDM-K64F SMOKE\n",
    },
    SurvivalCase {
        // Stock Zephyr v3.7.0 hello_world for `frdm_k64f` (Zephyr commit
        // 36940db938a8, crates/firmware-frdm-k64f-zephyr/build.sh). Zephyr's
        // own K6x bring-up runs unpatched: nxp_mpu programs the SYSMPU,
        // k6x_init writes PMC_REGSC, CLOCK_InitOsc0 writes OSC_CR and
        // CLOCK_BootToPeeMode walks the MCG to PEE (PLL0), then UART0 prints.
        name: "frdm_k64f_zephyr",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Zephyr,
        chip: "mk64f12",
        system: "frdm-k64f",
        fixture: "mk64f12-zephyr-hello.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x1FFF_0000, 0x2002_FFFF)],
        expected_uart_output: b"Hello World! frdm_k64f",
    },
    SurvivalCase {
        // SAM4S Xplained: ATSAM4S16C (Cortex-M4), 1 MiB flash at 0x00400000,
        // 128 KiB SRAM at 0x20000000. UART0 is the board's virtual COM port;
        // the smoke walks the PIOC LED with ODSR readback and reads the
        // pulled-up PA5 button. SIM-DERIVED.
        name: "sam4s_xplained_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "sam4s16c",
        system: "sam4s-xplained",
        fixture: "sam4s-xplained-smoke.elf",
        valid_pc_ranges: &[(0x0040_0000, 0x004F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"SAM4S XPLAINED SMOKE\n",
    },
    SurvivalCase {
        // SAM E70 Xplained: ATSAME70Q21B (Cortex-M7), 2 MiB flash at
        // 0x00400000, 384 KiB SRAM at 0x20400000. USART1 (PA21/PB4) is the
        // EDBG virtual COM port; the smoke disables the WDT, releases PB4
        // through CCFG_SYSIO, walks LED0 (PC8, active low) with ODSR readback
        // and reads the pulled-up SW0 (PA11). SIM-DERIVED.
        name: "sam_e70_xplained_smoke",
        core: "cortex-m7",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "same70q21b",
        system: "sam-e70-xplained",
        fixture: "sam-e70-xplained-smoke.elf",
        valid_pc_ranges: &[(0x0040_0000, 0x005F_FFFF), (0x2040_0000, 0x2045_FFFF)],
        expected_uart_output: b"SAM E70 XPLAINED SMOKE\n",
    },
    SurvivalCase {
        // S32K388EVB-Q289: S32K388 (CM7_0, Cortex-M7), 8 MiB program flash
        // at 0x00400000 (boot header at the base, vector table at
        // 0x00401000), SRAM_0-2 at 0x20400000. The smoke disables SWT_0,
        // gates SIUL2/LPUART2 in MC_ME, prints on LPUART2 and walks the six
        // RGB LED pads through GPDO/PGPDO/MPGPDO with PGPDI readback.
        // SIM-DERIVED.
        name: "s32k388_evb_smoke",
        core: "cortex-m7",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "s32k388",
        system: "s32k388-evb",
        fixture: "s32k388-evb-smoke.elf",
        valid_pc_ranges: &[(0x0040_1000, 0x00BF_FFFF), (0x2040_0000, 0x204B_FFFF)],
        expected_uart_output: b"S32K388 EVB SMOKE\n",
    },
    SurvivalCase {
        // MIMXRT595-EVK: MIMXRT595S (Cortex-M33), XIP from the FlexSPI0 NOR
        // window at 0x08000000 (FCB at 0x08000400, vector table at
        // 0x08001000), data in SRAM at 0x20280000. The smoke sets the clock
        // gates/resets and pads, prints on FLEXCOMM0 (USART) and walks the RGB
        // LED through the GPIO SET/B/W/NOT/MPIN/CLR views with PIN readback.
        // SIM-DERIVED.
        name: "mimxrt595_evk_smoke",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "mimxrt595s",
        system: "mimxrt595-evk",
        fixture: "mimxrt595-evk-smoke.elf",
        valid_pc_ranges: &[(0x0800_1000, 0x087F_FFFF), (0x2000_0000, 0x204F_FFFF)],
        expected_uart_output: b"MIMXRT595 EVK SMOKE\n",
    },
    SurvivalCase {
        // STM32F072B-DISCO: STM32F072RBT6 (Cortex-M0), 128 KiB flash at
        // 0x08000000, 16 KiB SRAM at 0x20000000. USART1 (PA9/PA10) is the
        // ST-LINK VCP; the smoke walks the four user LEDs with GPIOC ODR
        // readback and reads the pulled-up user button. SIM-DERIVED.
        name: "stm32f072b_disco_smoke",
        core: "cortex-m0",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "stm32f072",
        system: "stm32f072b-disco",
        fixture: "stm32f072b-disco-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0801_FFFF), (0x2000_0000, 0x2000_3FFF)],
        expected_uart_output: b"STM32F072B-DISCO SMOKE\n",
    },
    SurvivalCase {
        name: "nrf52840_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "nrf52840",
        system: "nrf52840-dk",
        fixture: "nrf52840-demo.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"NRF52840_SMOKE_OK\n",
    },
    SurvivalCase {
        // Arduino Nano 33 BLE (ABX00030): nRF52840, 1 MiB flash at 0x0, 256 KiB
        // RAM at 0x20000000. UARTE0 (EasyDMA) is the board's D0/D1 `Serial1`;
        // the RGB LED is active low, so the smoke prints the GPIO0 OUT readback
        // to pin the red/green/blue channel mapping. SIM-DERIVED.
        name: "arduino_nano_33_ble_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "nrf52840",
        system: "arduino-nano-33-ble",
        fixture: "arduino-nano-33-ble-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"ARDUINO NANO 33 BLE SMOKE\n",
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
        hal: Hal::Arduino,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Bare,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Bare,
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
        hal: Hal::Zephyr,
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
        hal: Hal::Bare,
        chip: "nrf52832",
        system: "nrf52-dk",
        fixture: "nrf52832-demo.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2000_FFFF)],
        // This fixture used to be a build of `crates/firmware-nrf52840-demo`
        // (mangled symbols `_ZN22firmware_nrf52840_demo…`, banner
        // `NRF52840_SMOKE_OK`), so its vector table carried the nRF52840
        // initial SP of 0x20040000 — the top of 256 KB. Real nRF52832 silicon
        // has 64 KB of RAM ending at 0x20010000, which is what
        // configs/chips/nrf52832.yaml correctly models, so every push landed
        // in unmapped space and was silently discarded. The case could not
        // assert any UART output as a result.
        //
        // It is now built from `crates/firmware-nrf52832-demo` against a 64 KB
        // memory.x, so the stack is real and the EasyDMA console banner comes
        // out. The fixture was the wrong side, not the chip config: growing
        // the model's RAM to fit a mis-targeted binary would have made the
        // twin lie about the part.
        expected_uart_output: b"NRF52832_SMOKE_OK\n",
    },
    SurvivalCase {
        // BBC micro:bit v2, target nRF52833. Bare-metal UARTE0 EasyDMA smoke:
        // PSEL.TXD = P0.06 / PSEL.RXD = P1.08 (the interface-MCU bridge; the
        // micro:bit schematic labels these UART_INT_RX/TX from the interface
        // side — codal's MICROBIT_PIN_UART_TX is P0.06), BAUDRATE 115200,
        // ENABLE = 8, then TXD.PTR/MAXCNT/STARTTX and an EVENTS_ENDTX poll.
        // The banner buffer is a `static mut` in .data (RAM), because EasyDMA
        // reads RAM: a stack local's initialization was dead-code-eliminated
        // before the TXD.PTR write in an earlier build and the DMA read zeros.
        name: "nrf52833_microbit_v2_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "nrf52833",
        system: "microbit-v2",
        fixture: "microbit-v2-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        // BBC micro:bit V1, target nRF51822, with the descriptor's S110
        // application-region map: flash from 0x18000, nothing below it. Bare
        // legacy-UART smoke (the nRF51 has no EasyDMA): PSELTXD = P0.24 /
        // PSELRXD = P0.25 (the interface-MCU bridge), BAUDRATE 115200, ENABLE
        // = 4, STARTTX, then TXD + an EVENTS_TXDRDY poll per byte. No
        // SoftDevice: the image's vector table sits at the application base.
        name: "nrf51822_microbit_v1_smoke",
        core: "cortex-m0",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "nrf51822",
        system: "microbit-v1",
        fixture: "microbit-v1-smoke.elf",
        valid_pc_ranges: &[(0x0001_8000, 0x0003_FFFF), (0x2000_0000, 0x2000_3FFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        name: "stm32h563_demo",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
        chip: "esp32c3",
        system: "esp32c3-devkit",
        fixture: "esp32c3-demo.elf",
        valid_pc_ranges: &[(0x4200_0000, 0x423F_FFFF), (0x3FC8_0000, 0x3FEF_FFFF)],
        expected_uart_output: b"ESP OK\n",
    },
    SurvivalCase {
        // ESP32-C6-DevKitC-1 / ESP32-C6 HP core (RV32IMAC on silicon; this
        // fixture links for riscv32imc — the smoke needs no A extension).
        // PCR ungates UART0 (no C3-style SYSTEM/APB_CTRL), IO_MUX routes
        // GPIO16/U0TXD, then the ESP UART twin's FIFO shifts `OK\n` to the
        // capture sink. The C6's unified flash window (0x4200_0000) and HP
        // SRAM (0x4080_0000) are its own, so the PC ranges pinned here do not
        // transfer from the C3. SIM-DERIVED — no silicon diff.
        name: "esp32c6_demo",
        core: "rv32imac",
        family: CpuFamily::RiscV,
        hal: Hal::Bare,
        chip: "esp32c6",
        system: "esp32c6-devkitc",
        fixture: "esp32c6-demo.elf",
        valid_pc_ranges: &[(0x4200_0000, 0x42FF_FFFF), (0x4080_0000, 0x4087_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        // LiteX VexRiscv SoC (RV32IM — no compressed, no atomics): the smoke
        // links into the SoC's integrated SRAM (0x10000000) and prints over
        // the LiteX UART at 0x60001800. The chip descriptor declares a generic
        // `rv32` core, so the run selects the standard RV32 profile rather
        // than the ESP32-C3's vendor CSR set. SIM-DERIVED — no silicon diff.
        name: "litex_vexriscv_smoke",
        core: "rv32im",
        family: CpuFamily::RiscV,
        hal: Hal::Bare,
        chip: "litex-vexriscv",
        system: "litex-vexriscv",
        fixture: "litex-vexriscv-smoke.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x1003_FFFF)],
        expected_uart_output: b"LITEX VEXRISCV SMOKE\n",
    },
    SurvivalCase {
        // Digilent Arty A7-35T (LiteX VexRiscv SoC, RV32IM): the smoke links
        // into the SoC's integrated SRAM, walks the control-and-status LEDs,
        // and prints over the LiteX UART. The chip declares `core: rv32im`, so
        // runs select the standard RV32 profile. SIM-DERIVED — no bitstream
        // diff exists.
        name: "arty_litex_smoke",
        core: "rv32im",
        family: CpuFamily::RiscV,
        hal: Hal::Bare,
        chip: "arty-litex-vexriscv",
        system: "arty-litex-vexriscv",
        fixture: "arty-litex-smoke.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x1003_FFFF)],
        expected_uart_output: b"ARTY LITEX SMOKE\n",
    },
    SurvivalCase {
        // Vega board (RI5CY, RV32IMC): 1 MiB flash at 0x0, 192 KiB SRAM at
        // 0x20000000, LPUART0 at 0x40042000. The smoke boots from flash and
        // prints over the LPUART; SCG/PCC/INTMUX/LPTMR remain declared stubs.
        // SIM-DERIVED.
        name: "vegaboard_ri5cy_smoke",
        core: "rv32imc",
        family: CpuFamily::RiscV,
        hal: Hal::Bare,
        chip: "vegaboard-ri5cy",
        system: "vegaboard-ri5cy",
        fixture: "vegaboard-ri5cy-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2002_FFFF)],
        expected_uart_output: b"VEGABOARD RI5CY SMOKE\n",
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        //
        // `BTN=` is the one line that was NOT a silicon capture. The demo
        // prints `(GPIOC_IDR >> 13) & 1 == 0` — active-low, so `BTN=1` means
        // *pressed*. This board's B1 is untouched, and real silicon holds PC13
        // at VDD through R34, so silicon prints `BTN=0`. It read `BTN=1` here
        // only because the `board_io` button binding materialised as nothing:
        // PC13 sat at its reset-value 0 and the sim reported a phantom press.
        // `firmware-l476-demo` documented that gap in its own source. Now that
        // a button is a real bus-resident device driving its pin, this line
        // agrees with silicon like every other line above.
        // `board_io_button_stimulus.rs` runs this same ELF with B1 pressed and
        // asserts it prints `BTN=1`, so both states are pinned, not just this
        // one.
        name: "nucleo_l476rg_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
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
BTN=0\r\n\
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Arduino,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
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
        hal: Hal::Bare,
        chip: "stm32l073",
        system: "nucleo-l073rz",
        fixture: "nucleo-l073rz-demo.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0802_FFFF), (0x2000_0000, 0x2000_4FFF)],
        expected_uart_output: b"DEV=20086447\nCLK=00000004\nCRC=B874177A\nDMA=OK\n",
    },
    // ------------------------------------------------------------------
    // Arduino-startup corpus (`L0_serial_boot`, verbatim from Core Arduino
    // Matrix). Every ELF below is the exact binary the matrix builds and
    // runs, and the assertion is the matrix's own L0 oracle: the sketch must
    // reach `Serial.println("LW_L0_OK")` — i.e. the whole vendor startup
    // (reset handler -> SystemInit -> HAL_Init -> SystemClock_Config ->
    // HardwareSerial::begin) must complete, not merely "not crash".
    //
    // Why these live here and not only in the matrix: Core Arduino Matrix is
    // a **main-only** workflow. `firmware_survival` runs on every PR. Twice
    // an MMIO change (memory-violation propagation) passed the PR gate and
    // then broke the matrix on main, because the only fixtures for these
    // chips were Zephyr builds and Zephyr's minimal init never touches the
    // peripherals STM32Cube's HAL touches. These cases put the Arduino
    // startup path on the PR gate so that class of regression is caught
    // before merge.
    SurvivalCase {
        // Adafruit/Nordic nRF52 Arduino core on nRF52832 (64 KB RAM part).
        // `Serial` drives the legacy UART personality of UART0 (ENABLE=4):
        // byte to TXD (0x51C), spin on EVENTS_TXDRDY (0x11C). Same console
        // path as the nRF52840 case, but on the 64 KB/512 KB die, so it also
        // pins that the nrf52832 chip config's smaller RAM/flash windows do
        // not clip the Arduino core's stack and heap during startup.
        name: "nrf52832_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "nrf52832",
        system: "nrf52-dk",
        fixture: "nrf52832-arduino-serial.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2000_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // STM32 Arduino core on F1: legacy USART (SR/DR, not ISR/TDR) plus the
        // Cube HAL bring-up — HAL_Init's SysTick at 1 kHz and
        // SystemClock_Config spinning on RCC_CR HSERDY/PLLRDY and
        // RCC_CFGR.SWS — before HardwareSerial can emit a byte.
        name: "stm32f103_arduino_serial",
        core: "cortex-m3",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32f103",
        system: "stm32f103-bare",
        fixture: "stm32f103-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // STM32 Arduino core on F4 (NUCLEO-F401RE). The existing
        // `stm32f401_blinky` case is also an Arduino build; it used to assert
        // *no* UART bytes and so could not distinguish a working F401 from a
        // wedged one, but it now pins the blink sketch's own banner. This case
        // stays because it pins a different path end-to-end: F4 RCC (HSI ->
        // PLL, PLLRDY, SWS) then USART2 TXE/TC, against the L0 matrix oracle.
        name: "stm32f401_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32f401",
        system: "nucleo-f401re",
        fixture: "stm32f401-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // STM32 Arduino core on F4 (NUCLEO-F407). F407 previously had only
        // bare-metal fixtures (`nucleo-f407-smoke`, `nucleo-f407-i2c`), so no
        // vendor-HAL startup ran against it on a PR at all. Exercises the F4
        // RCC PLL bring-up plus FLASH ACR latency/prefetch programming that
        // HAL_RCC_ClockConfig performs before switching to 84 MHz.
        name: "stm32f407_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32f407",
        system: "nucleo-f407",
        fixture: "stm32f407-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // *** This case exists because of the CRC peripheral. ***
        // Zephyr startup never calls `HAL_CRC_Init`; STM32Cube's Arduino core
        // startup does, and it writes CRC->POL at 0x4002_3014. While
        // stm32g474re.yaml had no `crc` peripheral that write landed in
        // unmapped space, which was harmless until memory violations began to
        // propagate — at which point it became a hard stop. That combination
        // reverted the propagation work twice, because the only G474 fixture
        // on the PR gate was `stm32g474-zephyr-hello.elf`, which cannot reach
        // the CRC path. Deleting `crc` from configs/chips/stm32g474re.yaml
        // must fail this test.
        name: "stm32g474re_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32g474re",
        system: "nucleo_g474re",
        fixture: "stm32g474re-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // STM32 Arduino core on H5 (Cortex-M33). Runs the H5 PWR VOS /
        // FLASH latency / RCC bring-up under the Cube HAL rather than under
        // Zephyr, and drives the modern USART (ISR/TDR + TEACK) console.
        name: "stm32h563_arduino_serial",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32h563",
        system: "nucleo-h563zi-demo",
        fixture: "stm32h563-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x200A_0000)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // STM32 Arduino core on L0 (Cortex-M0+, ARMv6-M). The Cube HAL
        // startup on L0 goes through PWR CR VOS + RCC ICSCR/CFGR on the
        // dedicated `stm32l0` RCC layout, all decoded by the ARMv6-M subset.
        name: "stm32l073_arduino_serial",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32l073",
        system: "nucleo-l073rz",
        fixture: "stm32l073-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0802_FFFF), (0x2000_0000, 0x2000_4FFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // STM32 Arduino core on L4. Complements
        // `nucleo_l476rg_arduino_serial` (the PLLSAI1RDY hang regression)
        // with the matrix's own L0 binary and oracle, so a matrix-visible
        // break on this board reproduces from the PR gate too.
        name: "stm32l476_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "stm32l476-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // *** This case exists because of the CRC peripheral. ***
        // Same defect class as the G474 case above: Zephyr startup does not
        // touch CRC, Arduino/STM32Cube startup calls `HAL_CRC_Init`, which
        // writes CRC->POL at 0x4002_3014. The only WB55 fixture on the PR
        // gate was `stm32wb55-zephyr-hello.elf`, so the unmapped write was
        // invisible until Core Arduino Matrix ran on main. Deleting `crc`
        // from configs/chips/stm32wb55.yaml must fail this test.
        // Also carries the dual-core (M4 + M0+) HSEM lock path on the way in.
        name: "stm32wb55_arduino_serial",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32wb55",
        system: "mb1355c",
        fixture: "stm32wb55-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // *** This case exists because of the CRC peripheral. ***
        // Third chip in the same revert: Zephyr startup does not touch CRC,
        // Arduino/STM32Cube startup calls `HAL_CRC_Init` and writes CRC->POL
        // at 0x4002_3014. `stm32wba52-zephyr-hello.elf` on the PR gate could
        // not see it. Deleting `crc` from configs/chips/stm32wba52.yaml must
        // fail this test. Also exercises the WBA-specific RCC (CFGR1@0x1C,
        // BDCR1@0xF0, the PLL1CFGR PLL1RCLKPRE → PLL1RCLKPRERDY poll at 0x28)
        // and the PWR VOSR ready handshake under the Cube HAL.
        name: "stm32wba52_arduino_serial",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32wba52",
        system: "nucleo_wba52cg",
        fixture: "stm32wba52-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // *** This case exists because of the CRC peripheral. ***
        // Same STM32Cube-startup class as the WBA52 Arduino case: the Arduino
        // core's `HAL_CRC_Init` writes CRC->POL at 0x4002_3014 during startup,
        // before the sketch can print `LW_L0_OK`. It also drives the U5-only
        // CRS register surface (`HAL_RCCEx_CRSConfig` at the end of the
        // STM32duino `SystemClock_Config`) and the PLL1 bring-up. Built by the
        // Arduino matrix (PlatformIO, `nucleo_u575zi_q`), copied from
        // `validation/arduino-matrix/out/stm32u575/L0_serial_boot/firmware.elf`.
        name: "stm32u575_arduino_serial",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Arduino,
        chip: "stm32u575",
        system: "nucleo-u575zi",
        fixture: "stm32u575-arduino-serial.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x200B_FFFF)],
        expected_uart_output: b"LW_L0_OK",
    },
    SurvivalCase {
        // NUCLEO-U545RE-Q bare-metal blinky (examples/nucleo-u545re/firmware,
        // arm-none-eabi-gcc, no HAL): RCC AHB2ENR1/APB2ENR gates, PA9/PA10 AF7,
        // USART1 VCP banner, PA5 LD2 toggle loop. Machine-run pin proofs live in
        // `nucleo_u545re.rs`; this case is the PR-run chip behavior gate.
        name: "stm32u545_blinky",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "stm32u545",
        system: "nucleo-u545re",
        fixture: "nucleo-u545re-blinky.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        // SAMD21G18A Nano 33 IoT bare-metal UART smoke: PM APBCMASK + GCLK
        // SERCOM5_CORE, then three DATA writes of "OK\n" on Serial1.
        name: "atsamd21_nano33_smoke",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "atsamd21",
        system: "nano-33-iot",
        fixture: "atsamd21-nano33-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0003_FFFF), (0x2000_0000, 0x2000_7FFF)],
        expected_uart_output: b"OK",
    },
    SurvivalCase {
        // SAMD51J19A Metro M4 bare-metal UART smoke: MCLK APBBMASK + GCLK
        // PCHCTRL[24] SERCOM3_CORE, then three DATA writes of "OK\n" on Serial1.
        name: "atsamd51_metro_m4_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "atsamd51",
        system: "metro-m4",
        fixture: "atsamd51-metro-m4-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2002_FFFF)],
        expected_uart_output: b"OK",
    },
    SurvivalCase {
        // R7FA4M1AB Uno R4 Minima bare-metal UART smoke: HOCO/OSCSF, then
        // three SCI2 TDR writes of "OK\n" and P111 toggle via POSR.
        name: "ra4m1_uno_r4_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "ra4m1",
        system: "arduino-uno-r4-minima",
        fixture: "ra4m1-uno-r4-smoke.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0003_FFFF), (0x2000_0000, 0x2000_7FFF)],
        expected_uart_output: b"OK",
    },
    SurvivalCase {
        // Teensy 4.1 / i.MX RT106x (chip yaml imxrt1064, RT1064-class cousin of
        // MIMXRT1062): CCM CCGR ungating, LPUART6 DATA "OK\n", GPIO2_IO03
        // DR_TOGGLE. Soft-float image linked in DTCM (XIP skipped).
        name: "imxrt1064_teensy41_smoke",
        core: "cortex-m7",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "imxrt1064",
        system: "teensy-41",
        fixture: "imxrt1064-teensy41-smoke.elf",
        valid_pc_ranges: &[(0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"OK",
    },
    SurvivalCase {
        // STM32F7 Discovery / STM32F746NG bare-metal UART smoke: RCC AHB1/APB2
        // ungating, USART1 TDR "OK\n" (stm32v2), PI1 BSRR toggle. Soft-float
        // flash @ 0x08000000 / DTCM @ 0x20000000. SIM-DERIVED.
        name: "stm32f746_discovery_smoke",
        core: "cortex-m7",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "stm32f746",
        system: "stm32f7-discovery",
        fixture: "stm32f746-discovery-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2000_FFFF)],
        expected_uart_output: b"OK",
    },
    SurvivalCase {
        // NUCLEO-G071RB (STM32G071RB) bare-metal UART/LED smoke: RCC IOPENR
        // GPIOA gate (0x34) + APBENR1 USART2 gate (0x3C) at the G0 offsets,
        // PA2/PA3 AF1, USART2 TDR "OK\n", LD4 PA5 BSRR toggle. SIM-DERIVED —
        // no silicon diff. Pins the dedicated `stm32g0` RCC layout: an L0/L4
        // offset here leaves the UART clock-gated and silent, so a green run
        // is real evidence for the G0 register map.
        name: "nucleo_g071rb_smoke",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        hal: Hal::Bare,
        chip: "stm32g071",
        system: "nucleo-g071rb",
        fixture: "nucleo-g071rb-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0801_FFFF), (0x2000_0000, 0x2000_8FFF)],
        expected_uart_output: b"OK",
    },
    // ── Tier-1 self-test fixtures promoted to PR-run gates ──────────────────
    //
    // Each fixture is the committed Tier-1 image for the chip. It drives the
    // rubric classes, prints `TIER1 <class> PASS` lines, then `TIER1 done`.
    // These run through the Session builder (`CpuFamily::Session`), which
    // stops at the marker instead of burning a fixed cycle budget, so the gate
    // costs the cycles to completion rather than the 8M-step CLI cap. The
    // marker is the assertion: a fixture that dies mid-sequence never prints
    // it. `valid_pc_ranges` is empty by design — the Session path exposes no
    // register read-back, and the transcript is the gate.
    SurvivalCase {
        name: "esp32_tier1",
        core: "xtensa-lx6",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "esp32",
        system: "esp32-wroom-32",
        fixture: "tier1/esp32.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    // The S3 pair shares one fixture and one transcript: the fast-boot builder
    // consumes only `cpu_hz` from the chip descriptor, so both descriptors run
    // the same machine and these two cases are behaviourally identical. The
    // zero case is kept so the zero descriptor + system YAMLs are loaded,
    // parsed and dispatched by a PR-run gate (a zero-specific regression in
    // those files fails here); a zero-specific memory-map check would need the
    // rom-boot path, which does consume the descriptor's flash size.
    SurvivalCase {
        name: "esp32s3_tier1",
        core: "xtensa-lx7",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "esp32s3",
        system: "esp32s3",
        fixture: "tier1/esp32s3.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    SurvivalCase {
        name: "esp32s3_zero_tier1",
        core: "xtensa-lx7",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "esp32s3-zero",
        system: "esp32s3-zero",
        fixture: "tier1/esp32s3.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    SurvivalCase {
        name: "stm32f411_tier1",
        core: "cortex-m4",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "stm32f411ceu6",
        system: "stm32f411ceu6-blackpill",
        fixture: "tier1/stm32f411.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    SurvivalCase {
        name: "stm32f405_tier1",
        core: "cortex-m4",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "stm32f405",
        system: "feather-f405",
        fixture: "tier1/stm32f405.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    SurvivalCase {
        name: "stm32f767_tier1",
        core: "cortex-m7",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "stm32f767",
        system: "nucleo-f767zi",
        fixture: "tier1/stm32f767.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    SurvivalCase {
        name: "stm32h735_tier1",
        core: "cortex-m7",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "stm32h735",
        system: "stm32h735-smoke",
        fixture: "tier1/stm32h735.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"TIER1 done",
    },
    // Fixtures built from workspace crates and copied in as committed blobs.
    // Rebuild (hashes are of the committed blobs; the workspace release
    // profile embeds absolute paths, so a rebuild on another machine is
    // functionally identical but not byte-identical):
    //   stm32f401cdu6-blackpill-demo.elf  sha256 8c85af57395f73c2ea8091ba4fddc0cfc57b888a77d66ee556625b798225928e
    //     cargo build -p firmware-f401cdu6-blackpill-demo --release \
    //       --target thumbv7em-none-eabi
    //     cp target/thumbv7em-none-eabi/release/firmware-f401cdu6-blackpill-demo \
    //       tests/fixtures/stm32f401cdu6-blackpill-demo.elf
    //   rp2350-demo.elf  sha256 6b76fb286a2447aed3b2f6ea261703f8eeb8e52de0997895fe4f4cfe72fa5818
    //     (cd crates/firmware-rp2350-demo && \
    //       cargo build --release --target thumbv8m.main-none-eabi)
    //     cp target/thumbv8m.main-none-eabi/release/firmware-rp2350-demo \
    //       tests/fixtures/rp2350-demo.elf
    SurvivalCase {
        name: "stm32f401cdu6_demo",
        core: "cortex-m4",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "stm32f401cdu6",
        system: "stm32f401cdu6-blackpill",
        fixture: "stm32f401cdu6-blackpill-demo.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"OK",
    },
    SurvivalCase {
        name: "rp2350_demo",
        core: "cortex-m33",
        family: CpuFamily::Session,
        hal: Hal::Bare,
        chip: "rp2350",
        system: "rp2350-zero",
        fixture: "rp2350-demo.elf",
        valid_pc_ranges: &[],
        expected_uart_output: b"RP2350_SMOKE_OK",
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
    // An empty `expected` used to mean "no assertion" and silently returned here,
    // which turned the whole case into a survival-only check: it could not tell a
    // working chip from a wedged one, and passed as long as the sim did not crash.
    // Every case now carries a real oracle, so an empty expected is a bug in the
    // table, not an opt-out. Fail loudly rather than skipping the check.
    assert!(
        !expected.is_empty(),
        "Board '{name}': expected_uart_output is empty, which asserts nothing. Give this \
         case a real oracle (run the fixture and pin what it genuinely emits), or assert \
         something else meaningful about it — do not leave it vacuous.",
    );
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
    match case.family {
        CpuFamily::Session => {
            let uart_bytes = run_session_firmware(case, firmware);
            assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
        }
        CpuFamily::CortexM => {
            let cycles = case_cycles(case);
            let (pc, uart_bytes) = run_cortex_m_firmware(case.chip, case.system, firmware, cycles);
            assert_pc_in_range(pc, cycles, case.valid_pc_ranges);
            assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
        }
        CpuFamily::RiscV => {
            let cycles = case_cycles(case);
            let (pc, uart_bytes) = run_riscv_firmware(case.chip, case.system, firmware, cycles);
            assert_pc_in_range(pc, cycles, case.valid_pc_ranges);
            assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
        }
    }
}

/// Per-case cycle budget. Most firmwares emit their banner within the default
/// window. The RP2040 Arduino ELF first calls `busy_wait_us(100_000)` from
/// `_GLOBAL__sub_I.00101_dt`, before `main`: at its declared 125 MHz clock that
/// constructor alone needs 12.5M cycles. Allow 500 ms for that boot delay,
/// peripheral/USB setup and the sketch's 200 ms loop delay. The old 2M-cycle
/// window covered only 16 ms once TIMER runs at its correct 1 MHz rate.
fn case_cycles(case: &SurvivalCase) -> u32 {
    match case.name {
        "rp2040_arduino_serial" => 125_000_000 / 2,
        // Contiki-NG's cc2538dk platform_init runs a busy-loop LED `fade()`
        // (~8.0M cycles at 16 MHz) before the first "Hello, world".
        "cc2538dk_contiki_hello" => 10_000_000,
        _ => SURVIVAL_CYCLES,
    }
}

/// `TIER1 … FAIL` lines a Session case tolerates: honest, documented gaps the
/// fixture itself reports (e.g. no general-purpose mem-to-mem DMA on the
/// classic ESP32). Every other FAIL line fails the gate.
fn allowed_tier1_failures(case: &SurvivalCase) -> &'static [&'static str] {
    match case.name {
        "esp32_tier1" => &["TIER1 dma FAIL code=esp32-no-mem2mem-dma"],
        _ => &[],
    }
}

/// Failure bound for Session-backed gates, in simulated steps. Success stops at
/// the case's marker, so this only bounds a run that never reaches it: 20M is
/// 2.5x the CLI's 8M-step fast-boot cap (its ROM-boot path has a separate 30M
/// cap this runner never uses), and the classic ESP32 Tier-1 marker lands at
/// ~3.98M steps. `expect` takes virtual time, so the step count is divided by
/// the session's `cpu_hz` at the call site.
const SESSION_STEP_BUDGET: u64 = 20_000_000;

/// Classify the `TIER1 <class> <status>` lines of a Session transcript: return
/// the ` FAIL` lines not in `allowed`, and the number of ` PASS` lines. The
/// `TIER1 done` terminator is not a class line.
fn tier1_class_scan(allowed: &[&str], transcript: &str) -> (Vec<String>, usize) {
    let mut failures = Vec::new();
    let mut passes = 0usize;
    for line in transcript.lines().map(str::trim) {
        if line == "TIER1 done" || !line.starts_with("TIER1 ") {
            continue;
        }
        if line.contains(" FAIL") {
            if !allowed.contains(&line) {
                failures.push(line.to_string());
            }
        } else if line.ends_with(" PASS") {
            passes += 1;
        }
    }
    (failures, passes)
}

/// Enforce class-level correctness on a Session transcript: no ` FAIL` line
/// outside the case's allowlist, and at least one ` PASS` line so a transcript
/// carrying only the terminator cannot satisfy the gate.
fn assert_tier1_classes_pass(case_name: &str, allowed: &[&str], transcript: &str) {
    let (failures, passes) = tier1_class_scan(allowed, transcript);
    assert!(
        failures.is_empty(),
        "{}: Tier-1 class failed: {}\n--- console ---\n{transcript}",
        case_name,
        failures.join(", ")
    );
    assert!(
        passes > 0,
        "{}: transcript has no `TIER1 … PASS` line, so the marker alone would assert \
         nothing\n--- console ---\n{transcript}",
        case_name
    );
}

/// Whether a Session case gates on the Tier-1 rubric transcript. The Tier-1
/// self-test images live under `tests/fixtures/tier1/` and terminate with
/// `TIER1 done`; their class lines are the oracle. Other Session cases (the
/// demo smoke fixtures at the fixtures root) gate on their UART marker alone,
/// exactly like the CortexM/RiscV paths.
fn requires_tier1_classes(case: &SurvivalCase) -> bool {
    case.family == CpuFamily::Session && case.fixture.starts_with("tier1/")
}

/// Run a committed fixture through the builder/session path (the one
/// `session_builder_xtensa` uses) and return the console transcript once the
/// case's marker appears; Tier-1 (`tier1/`) cases also check their class lines
/// via `requires_tier1_classes`. Panics with the console tail on timeout or on
/// a class failure.
fn run_session_firmware(case: &SurvivalCase, firmware_path: PathBuf) -> Vec<u8> {
    use labwired_core::session::{OpenOptions, Session};
    use labwired_core::system::builder::{
        BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
    };
    use std::time::Duration;

    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );
    let (chip, manifest) = load_system(case.chip, case.system);
    let fw = std::fs::read(&firmware_path)
        .unwrap_or_else(|e| panic!("read fixture {:?}: {e}", firmware_path));
    let mut session = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{}: build machine: {e:#}", case.name));

    let pattern = regex::escape(
        std::str::from_utf8(case.expected_uart_output).expect("case marker is ASCII"),
    );
    let timeout = Duration::from_secs_f64(SESSION_STEP_BUDGET as f64 / session.cpu_hz() as f64);
    session.expect(&pattern, timeout).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\n--- console ---\n{}",
            case.name,
            session.uart_transcript()
        )
    });
    let transcript = session.uart_transcript();
    if requires_tier1_classes(case) {
        assert_tier1_classes_pass(case.name, allowed_tier1_failures(case), &transcript);
    }
    transcript.into_bytes()
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
    machine.add_observer(trace.clone());

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
    // The chip descriptor selects the RISC-V core profile (name + `core:`),
    // through the same one resolution the CLI, browser and debugger paths
    // use — a generic `rv32*` chip must not silently run the ESP32-C3's
    // vendor CSR set.
    let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.add_observer(trace.clone());

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

/// One survival gate per `SURVIVAL_CASES` row.
///
/// Each invocation keeps the CI-visible test name (filters, failure
/// attribution) while the four-line
/// `#[test] fn …() { run_survival_case(case_by_name(…)); }` boilerplate lives
/// once. `case_by_name` still panics naming the row if the case was renamed.
macro_rules! survival_gate {
    ($(#[$doc:meta])* $name:ident, $case:literal) => {
        $(#[$doc])*
        #[test]
        fn $name() {
            run_survival_case(case_by_name($case));
        }
    };
}

survival_gate!(test_stm32f103_blinky_survival, "stm32f103_blinky");

survival_gate!(test_stm32f401_blinky_survival, "stm32f401_blinky");

survival_gate!(test_stm32f401_zephyr_survival, "stm32f401_zephyr");

survival_gate!(test_stm32f103_zephyr_survival, "stm32f103_zephyr");

survival_gate!(test_stm32l073_zephyr_survival, "stm32l073_zephyr");

survival_gate!(test_stm32l476_zephyr_survival, "stm32l476_zephyr");

survival_gate!(test_stm32g474_zephyr_survival, "stm32g474_zephyr");

survival_gate!(test_stm32h563_zephyr_survival, "stm32h563_zephyr");

survival_gate!(test_stm32wb55_zephyr_survival, "stm32wb55_zephyr");

survival_gate!(test_stm32wba52_zephyr_survival, "stm32wba52_zephyr");

survival_gate!(test_stm32u575_zephyr_survival, "stm32u575_zephyr");

survival_gate!(test_rp2040_demo_survival, "rp2040_demo");

survival_gate!(test_rp2040_zephyr_hello_survival, "rp2040_zephyr_hello");

survival_gate!(
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
    /// Uses a wider cycle budget (see `case_cycles`) for the ELF's 100 ms global
    /// constructor delay before `main`, peripheral/USB setup and 200 ms loop delay.
    test_rp2040_arduino_serial_survival,
    "rp2040_arduino_serial"
);

survival_gate!(test_nrf52840_demo_survival, "nrf52840_demo");

survival_gate!(test_sam4s_xplained_smoke_survival, "sam4s_xplained_smoke");

survival_gate!(
    test_sam_e70_xplained_smoke_survival,
    "sam_e70_xplained_smoke"
);

survival_gate!(test_s32k388_evb_smoke_survival, "s32k388_evb_smoke");

survival_gate!(test_mimxrt595_evk_smoke_survival, "mimxrt595_evk_smoke");

survival_gate!(test_cc2538dk_smoke_survival, "cc2538dk_smoke");
survival_gate!(
    test_cc2538dk_contiki_hello_survival,
    "cc2538dk_contiki_hello"
);
survival_gate!(test_apollo4p_evb_smoke_survival, "apollo4p_evb_smoke");
survival_gate!(test_apollo4p_evb_zephyr_survival, "apollo4p_evb_zephyr");

/// The Apollo4 secure bootloader owns MRAM 0x0-0x17FFF and hands over to the
/// application vector table at 0x0001_8000. The committed smoke ELF must load
/// nothing below that, and its vector table must give an SP inside the TCM
/// and a Thumb reset handler inside the application MRAM.
#[test]
fn test_apollo4p_evb_smoke_is_linked_after_the_bootloader() {
    const APP: u64 = 0x0001_8000;
    let bytes = std::fs::read(fixtures().join("apollo4p-evb-smoke.elf")).expect("read fixture");
    let elf = goblin::elf::Elf::parse(&bytes).expect("parse fixture ELF");
    let loads: Vec<_> = elf
        .program_headers
        .iter()
        .filter(|ph| ph.p_type == goblin::elf::program_header::PT_LOAD && ph.p_filesz > 0)
        .collect();
    assert!(
        loads.iter().all(|ph| ph.p_paddr >= APP),
        "a segment loads into the bootloader region"
    );
    let vt = loads
        .iter()
        .find(|ph| ph.p_paddr == APP)
        .map(|ph| &bytes[ph.p_offset as usize..ph.p_offset as usize + 8])
        .expect("no segment starts at the 0x18000 vector table");
    let sp = u32::from_le_bytes(vt[0..4].try_into().unwrap());
    let reset = u32::from_le_bytes(vt[4..8].try_into().unwrap());
    assert!((0x1000_0000..=0x1006_0000).contains(&sp), "SP {sp:#x}");
    assert_eq!(reset & 1, 1, "reset handler must be Thumb");
    assert!(
        (0x0001_8000..0x0020_0000).contains(&(reset & !1)),
        "reset {reset:#x}"
    );
}

survival_gate!(test_ek_ra6m5_smoke_survival, "ek_ra6m5_smoke");

/// The CC2538 boot ROM reads the Customer Configuration Area (SWRU319C §8.6)
/// in the top 44 bytes of the last flash page and only enters a valid image.
/// The committed smoke ELF must carry: backdoor byte 0xF3 (enabled, active
/// low, PA3), image valid = 0, vector table at 0x0020_0000, all pages and
/// debug unlocked (0xFF).
#[test]
fn test_cc2538dk_smoke_customer_configuration_area_boots() {
    const CCA: u64 = 0x0027_FFD4;
    let bytes = std::fs::read(fixtures().join("cc2538dk-smoke.elf")).expect("read fixture");
    let elf = goblin::elf::Elf::parse(&bytes).expect("parse fixture ELF");
    let cca = elf
        .program_headers
        .iter()
        .filter(|ph| ph.p_type == goblin::elf::program_header::PT_LOAD)
        .find(|ph| ph.p_paddr <= CCA && CCA + 44 <= ph.p_paddr + ph.p_filesz)
        .map(|ph| {
            let start = (ph.p_offset + CCA - ph.p_paddr) as usize;
            &bytes[start..start + 44]
        })
        .expect("no loadable segment covers the CCA at 0x0027FFD4");
    let mut expected = [0xFFu8; 44];
    expected[3] = 0xF3; // backdoor config (byte 2007)
    expected[4..8].copy_from_slice(&0u32.to_le_bytes()); // image valid
    expected[8..12].copy_from_slice(&0x0020_0000u32.to_le_bytes()); // vector table
    assert_eq!(cca, expected, "CC2538DK smoke CCA");
}

survival_gate!(test_nucleo_l552ze_q_smoke_survival, "nucleo_l552ze_q_smoke");
survival_gate!(test_frdm_k64f_smoke_survival, "frdm_k64f_smoke");
survival_gate!(test_frdm_k64f_zephyr_survival, "frdm_k64f_zephyr");

/// The K64F loads FSEC/FOPT/FPROT from the Flash Configuration Field at
/// 0x400..0x40F on reset. Code bytes there can secure (or, with MEEN=0b10,
/// permanently lock) a real FRDM-K64F, so the committed smoke ELF must carry
/// the unsecured FCF: key/FPROT all 0xFF, FSEC=0xFE, FOPT/FEPROT/FDPROT=0xFF.
#[test]
fn test_frdm_k64f_smoke_flash_config_field_is_unsecured() {
    let bytes = std::fs::read(fixtures().join("frdm-k64f-smoke.elf")).expect("read fixture");
    let elf = goblin::elf::Elf::parse(&bytes).expect("parse fixture ELF");
    let fcf = elf
        .program_headers
        .iter()
        .filter(|ph| ph.p_type == goblin::elf::program_header::PT_LOAD)
        .find(|ph| ph.p_paddr <= 0x400 && 0x410 <= ph.p_paddr + ph.p_filesz)
        .map(|ph| {
            let start = (ph.p_offset + 0x400 - ph.p_paddr) as usize;
            &bytes[start..start + 16]
        })
        .expect("no loadable segment covers the FCF at 0x400..0x410");
    let mut expected = [0xFFu8; 16];
    expected[12] = 0xFE; // FSEC: unsecured, mass erase enabled
    assert_eq!(fcf, expected, "FRDM-K64F smoke FCF at 0x400");
}

survival_gate!(
    test_stm32f072b_disco_smoke_survival,
    "stm32f072b_disco_smoke"
);

survival_gate!(
    test_arduino_nano_33_ble_smoke_survival,
    "arduino_nano_33_ble_smoke"
);

survival_gate!(
    test_nrf52840_arduino_serial_survival,
    "nrf52840_arduino_serial"
);

survival_gate!(test_nrf52832_demo_survival, "nrf52832_demo");

survival_gate!(
    test_nrf52833_microbit_v2_smoke_survival,
    "nrf52833_microbit_v2_smoke"
);

survival_gate!(
    test_nrf51822_microbit_v1_smoke_survival,
    "nrf51822_microbit_v1_smoke"
);

survival_gate!(test_nrf5340_zephyr_survival, "nrf5340_zephyr");

survival_gate!(test_nrf54l15_smoke_survival, "nrf54l15_smoke");

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
    // P2 is mapped at the MDK/SVD NRF_P2_S_BASE (0x5005_0400) with the nRF54L
    // register profile, so OUT lands at +0x000 and DIR at +0x010 — the offsets
    // the MDK nrf54l15_types.h gives this family's NRF_GPIO_Type.
    const GPIO_P2: u64 = 0x5005_0400;
    const GPIO_OUT: u64 = 0x000;
    const GPIO_DIR: u64 = 0x010;
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

survival_gate!(test_nrf54l15_zephyr_survival, "nrf54l15_zephyr");

survival_gate!(test_kw41z_smoke_survival, "kw41z_smoke");

survival_gate!(test_kw41z_nxp_survival, "kw41z_nxp");

survival_gate!(test_kw41z_zephyr_survival, "kw41z_zephyr");

survival_gate!(test_kw41z_lcd_activity_survival, "kw41z_lcd_activity");

/// End-to-end proof that the activity bar-graph reaches the screen: boot the
/// firmware, then read back the PCD8544 model's framebuffer and confirm the
/// display was turned on and real pixels were drawn — i.e. the FXOS8700 read
/// (Kinetis I2C), the DSPI master, the GPIO D/C latch and the display model all
/// cooperated, not just that the CPU survived.
#[test]
fn test_kw41z_lcd_renders_screen() {
    use labwired_core::peripherals::components::GenericDisplay;
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
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<GenericDisplay>()))
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
    use labwired_core::peripherals::components::GenericDisplay;
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
            .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<GenericDisplay>()))
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

    // Drive a hard tilt into the sensor — the same `set_input` channel path the
    // playground sliders use.
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
            if let Some(sensor) = device.as_sim_input_mut() {
                if sensor.input_channels().iter().any(|c| c.key == "x") {
                    sensor.set_input("x", 2.0).expect("x");
                    sensor.set_input("y", -2.0).expect("y");
                    sensor.set_input("z", 1.0).expect("z");
                    found = true;
                }
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

survival_gate!(test_stm32h563_demo_survival, "stm32h563_demo");

survival_gate!(test_riscv_ci_fixture_survival, "riscv_ci_fixture");

survival_gate!(test_esp32c3_demo_survival, "esp32c3_demo");

survival_gate!(test_esp32c6_demo_survival, "esp32c6_demo");

survival_gate!(test_litex_vexriscv_smoke_survival, "litex_vexriscv_smoke");

survival_gate!(test_arty_litex_smoke_survival, "arty_litex_smoke");
survival_gate!(test_vegaboard_ri5cy_smoke_survival, "vegaboard_ri5cy_smoke");

survival_gate!(test_nucleo_l476rg_smoke_survival, "nucleo_l476rg_smoke");

survival_gate!(test_nucleo_l476rg_spi_survival, "nucleo_l476rg_spi");

survival_gate!(test_nucleo_l476rg_pll_survival, "nucleo_l476rg_pll");

survival_gate!(test_nucleo_l476rg_misc_survival, "nucleo_l476rg_misc");

survival_gate!(
    test_nucleo_l476rg_l4periphs_survival,
    "nucleo_l476rg_l4periphs"
);

survival_gate!(test_nucleo_l476rg_demo_survival, "nucleo_l476rg_demo");

survival_gate!(test_nucleo_l476rg_dma_survival, "nucleo_l476rg_dma");

survival_gate!(test_nucleo_l476rg_adc_survival, "nucleo_l476rg_adc");

survival_gate!(test_nucleo_l476rg_i2c_survival, "nucleo_l476rg_i2c");

survival_gate!(
    test_nucleo_l476rg_l4periphs2_survival,
    "nucleo_l476rg_l4periphs2"
);

survival_gate!(
    test_nucleo_l476rg_tim1_advanced_survival,
    "nucleo_l476rg_tim1_advanced"
);

survival_gate!(test_nucleo_l476rg_r11_survival, "nucleo_l476rg_r11");

survival_gate!(test_nucleo_l476rg_r12_survival, "nucleo_l476rg_r12");

survival_gate!(test_nucleo_f407_smoke_survival, "nucleo_f407_smoke");

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

survival_gate!(test_nucleo_l073rz_smoke_survival, "nucleo_l073rz_smoke");

// --- Arduino-startup corpus (Core Arduino Matrix `L0_serial_boot`) ---------
// Each asserts the sketch reaches `LW_L0_OK`, which is the matrix's own L0
// oracle: the full vendor startup must complete. See the block comment on
// these cases in SURVIVAL_CASES for why they are on the PR gate.

survival_gate!(
    test_nrf52832_arduino_serial_survival,
    "nrf52832_arduino_serial"
);

survival_gate!(
    test_stm32f103_arduino_serial_survival,
    "stm32f103_arduino_serial"
);

survival_gate!(
    test_stm32f401_arduino_serial_survival,
    "stm32f401_arduino_serial"
);

survival_gate!(
    test_stm32f407_arduino_serial_survival,
    "stm32f407_arduino_serial"
);

survival_gate!(
    /// Regression for the unmapped `CRC->POL` write (0x4002_3014) that reverted the
    /// Cortex-M memory-violation propagation work twice. Zephyr never calls
    /// `HAL_CRC_Init`; this Arduino/STM32Cube startup does.
    test_stm32g474re_arduino_serial_survival,
    "stm32g474re_arduino_serial"
);

survival_gate!(
    test_stm32h563_arduino_serial_survival,
    "stm32h563_arduino_serial"
);

survival_gate!(
    test_stm32l073_arduino_serial_survival,
    "stm32l073_arduino_serial"
);

survival_gate!(
    test_stm32l476_arduino_serial_survival,
    "stm32l476_arduino_serial"
);

survival_gate!(
    /// Regression for the unmapped `CRC->POL` write (0x4002_3014) on WB55 — see
    /// `test_stm32g474re_arduino_serial_survival`.
    test_stm32wb55_arduino_serial_survival,
    "stm32wb55_arduino_serial"
);

survival_gate!(
    /// Regression for the unmapped `CRC->POL` write (0x4002_3014) on WBA52 — see
    /// `test_stm32g474re_arduino_serial_survival`.
    test_stm32wba52_arduino_serial_survival,
    "stm32wba52_arduino_serial"
);

survival_gate!(test_stm32u545_blinky_survival, "stm32u545_blinky");

survival_gate!(
    test_stm32u575_arduino_serial_survival,
    "stm32u575_arduino_serial"
);

survival_gate!(test_atsamd21_nano33_smoke_survival, "atsamd21_nano33_smoke");

survival_gate!(
    test_atsamd51_metro_m4_smoke_survival,
    "atsamd51_metro_m4_smoke"
);

survival_gate!(test_ra4m1_uno_r4_smoke_survival, "ra4m1_uno_r4_smoke");

survival_gate!(
    test_imxrt1064_teensy41_smoke_survival,
    "imxrt1064_teensy41_smoke"
);

survival_gate!(
    test_stm32f746_discovery_smoke_survival,
    "stm32f746_discovery_smoke"
);

survival_gate!(test_nucleo_g071rb_smoke_survival, "nucleo_g071rb_smoke");

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

survival_gate!(test_stm32f401cdu6_demo_survival, "stm32f401cdu6_demo");

survival_gate!(test_rp2350_demo_survival, "rp2350_demo");

survival_gate!(test_esp32_tier1_survival, "esp32_tier1");

survival_gate!(test_esp32s3_tier1_survival, "esp32s3_tier1");

survival_gate!(test_esp32s3_zero_tier1_survival, "esp32s3_zero_tier1");

survival_gate!(test_stm32f411_tier1_survival, "stm32f411_tier1");

survival_gate!(test_stm32f405_tier1_survival, "stm32f405_tier1");

survival_gate!(test_stm32f767_tier1_survival, "stm32f767_tier1");

survival_gate!(test_stm32h735_tier1_survival, "stm32h735_tier1");

#[test]
fn session_cases_carry_no_pc_ranges() {
    for case in SURVIVAL_CASES {
        match case.family {
            CpuFamily::Session => assert!(
                case.valid_pc_ranges.is_empty(),
                "{}: Session cases expose no register read-back, so they must carry no \
                 valid_pc_ranges",
                case.name
            ),
            CpuFamily::CortexM | CpuFamily::RiscV => assert!(
                !case.valid_pc_ranges.is_empty(),
                "{}: non-Session cases must pin the PC range they end in",
                case.name
            ),
        }
    }
}

/// The class-transcript oracle is wired by convention (`tier1/` fixtures), so
/// pin the convention to the marker the fixtures actually emit: a Session case
/// that waits on `TIER1 done` must enforce class lines, and one that does not
/// must not. A fixture moved between the root and `tier1/`, or a marker
/// changed, fails here instead of silently weakening the oracle.
#[test]
fn session_tier1_class_gating_matches_the_marker() {
    for case in SURVIVAL_CASES {
        if case.family != CpuFamily::Session {
            continue;
        }
        let waits_on_terminator = case.expected_uart_output == b"TIER1 done";
        assert_eq!(
            requires_tier1_classes(case),
            waits_on_terminator,
            "{}: fixture {:?} — the `tier1/` path and the `TIER1 done` marker disagree",
            case.name,
            case.fixture
        );
    }
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

// The esp32 transcript contains an allowlisted `dma FAIL`, so the passing gate
// alone cannot show the scan would reject a regression. These feed the scan
// synthetic transcripts instead.
#[test]
fn tier1_transcript_scan_rejects_an_unallowlisted_failure() {
    let (failures, passes) = tier1_class_scan(
        allowed_tier1_failures(case_by_name("esp32_tier1")),
        "TIER1 clock PASS\r\nTIER1 adc FAIL code=boom\r\nTIER1 done\r\n",
    );
    assert_eq!(failures, ["TIER1 adc FAIL code=boom"]);
    assert_eq!(passes, 1);
}

#[test]
fn tier1_transcript_scan_tolerates_only_the_documented_gap() {
    let (failures, passes) = tier1_class_scan(
        allowed_tier1_failures(case_by_name("esp32_tier1")),
        "TIER1 clock PASS\r\nTIER1 dma FAIL code=esp32-no-mem2mem-dma\r\nTIER1 done\r\n",
    );
    assert!(failures.is_empty());
    assert_eq!(passes, 1);
}

#[test]
fn tier1_transcript_scan_flags_a_terminator_only_transcript() {
    let (failures, passes) = tier1_class_scan(&[], "TIER1 done\r\n");
    assert!(failures.is_empty());
    assert_eq!(passes, 0);
}
