// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Does a probe on an STM32F103 or RP2040 lab's serial pad show the waveform?
//!
//! Same question, and the same construction, as
//! `crates/core/tests/logic_analyzer_lab_pad_visibility.rs` asks of the STM32
//! V2 family: build the lab the way the PLAYGROUND builds it —
//! `SystemManifest::from_file` on the lab's own `system.yaml`, its `chip:`
//! resolved relatively, `SystemBus::from_config` — and replay ONLY the register
//! writes the firmware really performs. No `wire_*_pads` call by hand.
//!
//! The replays below are not invented and are not tuned. Each one mirrors a
//! named function in a named source file, listed on the replay itself. If that
//! firmware changes, the replay is wrong and must change with it.
//!
//! # History
//!
//! Every gate here was written as a REPRODUCTION and every one of them failed.
//! Three families of firmware had learned that setup is optional, because
//! LabWired's UART models transmit on any data-register write:
//!
//! * The STM32F103 Rust labs enabled the USART clock and then wrote `DR` raw.
//!   No `CRL`/`CRH` mux, no `BRR`, and mostly no `CR1`.
//! * The STM32F103 bare-metal C firmwares set `CR1 = UE|TE` and stopped there.
//! * `crates/firmware-rp2040-demo` was `write_volatile(UART0_DR, b)` and
//!   nothing else at all — no `RESETS` deassert, no `clk_peri`, no `IO_BANK0`
//!   `FUNCSEL`, no divisor, and no `UARTCR.UARTEN`, which resets to 0.
//!
//! An unmuxed pad SHOULD be dark and a zero divisor SHOULD have no bit period;
//! the engine was right both times. Correctly configured firmware works on the
//! current permissive models with no engine change, which is why this is a
//! firmware-only fix. The `diagnostic_*` tests pin each half of the old
//! behaviour so the reason the pads were dark stays documented.
//!
//! # Payloads
//!
//! Every byte transmitted below is bit-asymmetric. `0xA5`, `0x5A`, `0x00` and
//! `0xFF` are palindromic under the LSB-first bit order serial uses, so a
//! narrator that reversed the bits would still produce a plausible trace.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::logic_capture::LogicSource;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

type Cm = Machine<CortexM>;

const SRAM_BASE: u64 = 0x2000_0000;

/// Bit-asymmetric under LSB-first: 0x4C→0x32, 0x77→0xEE, 0x13→0xC8, 0x2E→0x74.
const PAYLOAD: &[u8] = &[0x4C, 0x77, 0x13, 0x2E];

// ── STM32F103 (configs/chips/stm32f103.yaml) ────────────────────────────────

const RCC_BASE: u64 = 0x4002_1000;
const RCC_APB2ENR: u64 = 0x18;
const RCC_APB1ENR: u64 = 0x1C;

/// `AFIOEN | IOPAEN | USART1EN` — RM0008's APB2 bits 0, 2 and 14. `stm32f103.yaml`
/// gates AFIO, GPIOA and USART1 on exactly these, so an unset bit means the
/// peripheral's writes are dropped on the bus, as on silicon.
const APB2_AFIO_GPIOA_USART1: u32 = (1 << 0) | (1 << 2) | (1 << 14);
const APB1_I2C1: u32 = 1 << 21;
const APB1_USART2: u32 = 1 << 17;
const APB1_CAN1: u32 = 1 << 25;

const GPIOA_BASE: u64 = 0x4001_0800;
/// The F1 pad mux: four bits per pin, MODE[1:0] then CNF[1:0]. `CRL` covers
/// PA0..PA7, `CRH` covers PA8..PA15. There is no `MODER` and no `AFR` here.
const GPIOA_CRL: u64 = 0x00;
const GPIOA_CRH: u64 = 0x04;
/// MODE `0b11` (output, 50 MHz) + CNF `0b10` (alternate function, push-pull).
const CRX_AF_PUSH_PULL_50MHZ: u32 = 0xB;
/// PA9 is `USART1_TX` and PA2 is `USART2_TX`, both in the **Default**
/// alternate-function column (DS5319 Rev 20, Table 5, pp.31 and 29), so no AFIO
/// remap is involved and both are in `wire_stm32_uart_pads`'s F1 table.
const PA9: u8 = 9;
const PA2: u8 = 2;

const USART1_BASE: u64 = 0x4001_3800;
const USART2_BASE: u64 = 0x4000_4400;
/// F1 USART layout: SR @ 0x00, DR @ 0x04, BRR @ 0x08, CR1 @ 0x0C.
const F1_DR: u64 = 0x04;
const F1_BRR: u64 = 0x08;
const F1_CR1: u64 = 0x0C;
const F1_CR1_UE_TE: u32 = (1 << 13) | (1 << 3);

/// `BRR` = f_PCLK / baud at 16× oversampling. None of these labs touches the
/// PLL, so the part runs on the 8 MHz HSI it selects at reset (DS5319 Rev 20
/// §2.3.7, p.15): 8_000_000 / 115_200 = 69.44 → 69 = 0x45.
const F1_BRR_115200: u32 = 0x45;

// ── RP2040 (configs/chips/rp2040.yaml) ──────────────────────────────────────

const RESETS_BASE: u64 = 0x4000_C000;
const RESETS_RESET: u64 = 0x00;
const RESETS_RESET_DONE: u64 = 0x08;
const RESETS_UART0: u32 = 1 << 22;
const RESETS_IO_BANK0: u32 = 1 << 5;
const RESETS_PADS_BANK0: u32 = 1 << 8;

const CLOCKS_BASE: u64 = 0x4000_8000;
const CLK_PERI_CTRL: u64 = 0x48;
const CLK_PERI_CTRL_ENABLE: u32 = 1 << 11;

const IO_BANK0_BASE: u64 = 0x4001_4000;
/// `GPIOn_CTRL` = 0x04 + 8n (RP2040 datasheet Table 283, p.245).
const GPIO0_CTRL: u64 = 0x04;
/// GP0 function F2 is `UART0 TX` (RP2040 datasheet Table 279, p.238).
const GPIO_FUNC_UART: u32 = 2;
const GP0: u8 = 0;

const RP_UART0_BASE: u64 = 0x4003_4000;
const PL011_DR: u64 = 0x00;
const PL011_IBRD: u64 = 0x24;
const PL011_FBRD: u64 = 0x28;
const PL011_LCR_H: u64 = 0x2C;
const PL011_CR: u64 = 0x30;
const PL011_LCR_H_8N1_FIFO: u32 = (0b11 << 5) | (1 << 4);
/// `UARTEN` (bit 0) + `TXE` (bit 8). UARTEN resets to 0 (Table 433, p.435).
const PL011_CR_UARTEN_TXE: u32 = (1 << 0) | (1 << 8);

/// 115 200 from the ~6.5 MHz ring oscillator the chip boots on (RP2040
/// datasheet §2.13.2, p.129), which is what `clk_peri` carries when firmware
/// brings up neither XOSC nor a PLL: 6_500_000 / (16 × 115_200) = 3.5264, so
/// IBRD = 3 and FBRD = round(0.5264 × 64) = 34.
const RP_IBRD: u32 = 3;
const RP_FBRD: u32 = 34;
/// The engine's PL011 bit period, (64 × IBRD + FBRD) / 4 clocks.
const RP_BIT_TIME: u64 = (64 * RP_IBRD as u64 + RP_FBRD as u64) / 4;

/// Build a lab exactly as the playground does, from a repo-relative path.
fn lab_machine(relative_system_yaml: &str) -> Cm {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative_system_yaml);
    let manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {relative_system_yaml}: {e}"));
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build lab bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // A NOP slab in SRAM ending in a Thumb branch back to its start, so
    // `step()` advances cycles deterministically without needing a release ELF
    // (those are sha-pinned GitHub Release assets, not committed files).
    for i in 0..1022u64 {
        let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
        machine.bus.write_u8(SRAM_BASE + i, byte).unwrap();
    }
    machine.bus.write_u8(SRAM_BASE + 1022, 0xFF).unwrap();
    machine.bus.write_u8(SRAM_BASE + 1023, 0xE5).unwrap();
    machine.cpu.pc = SRAM_BASE as u32;
    machine
}

/// Arm the analyzer on one pad of a named peripheral, push bytes through a data
/// register, let the wire have its time, and count the edges captured.
///
/// The waiting is not scaffolding. The models report their transmitter empty
/// immediately, so firmware hands over a whole string in a few cycles, but the
/// narrator only publishes once the line has physically had the bit periods to
/// carry it.
fn tx_edges(
    machine: &mut Cm,
    pad_peripheral: &str,
    pin: u8,
    dr_addr: u64,
    dr_is_u8: bool,
    bytes: &[u8],
    bit_time: u64,
) -> usize {
    let pad_idx = machine
        .bus
        .find_peripheral_index_by_name(pad_peripheral)
        .unwrap_or_else(|| panic!("{pad_peripheral} on the lab bus"));
    machine.logic_watch(&[Some(LogicSource::Pad {
        peripheral: pad_idx,
        pin,
    })]);

    for &byte in bytes {
        if dr_is_u8 {
            machine.bus.write_u8(dr_addr, byte).unwrap();
        } else {
            machine.bus.write_u32(dr_addr, u32::from(byte)).unwrap();
        }
        // Ten bit periods per character, plus slack.
        for _ in 0..(bit_time * 12) {
            machine.step().expect("step");
        }
    }
    for _ in 0..(bit_time * 40) {
        machine.step().expect("step");
    }

    machine.logic_read_edges(0).edges.len()
}

/// Read-modify-write, the way every one of these firmwares touches an enable
/// register. A blind `write_u32` would mask a missing bit somewhere else.
fn set_bits(machine: &mut Cm, addr: u64, bits: u32) {
    let old = machine.bus.read_u32(addr).unwrap();
    machine.bus.write_u32(addr, old | bits).unwrap();
}

/// Write the four-bit F1 mux nibble for `pin` into `CRL` or `CRH`.
fn f1_pad_af(machine: &mut Cm, gpio_base: u64, pin: u8) {
    let (reg, shift) = if pin < 8 {
        (GPIOA_CRL, u32::from(pin) * 4)
    } else {
        (GPIOA_CRH, u32::from(pin - 8) * 4)
    };
    let crx = machine.bus.read_u32(gpio_base + reg).unwrap();
    machine
        .bus
        .write_u32(
            gpio_base + reg,
            (crx & !(0xF << shift)) | (CRX_AF_PUSH_PULL_50MHZ << shift),
        )
        .unwrap();
}

// ════════════════════════════════════════════════════════════════════════════
// Family A — the STM32F103 Rust labs
// ════════════════════════════════════════════════════════════════════════════

/// `enable_peripheral_clocks()` + `usart1_init()` from
/// `examples/bme280-weather-lab/src/main.rs`, register for register. The other
/// six USART1 I²C labs (ads1115, adxl345, ds3231, ina219, mpu6050, vl53l1x)
/// carry a byte-identical pair.
fn f1_rust_bme280_init(machine: &mut Cm) {
    set_bits(machine, RCC_BASE + RCC_APB2ENR, APB2_AFIO_GPIOA_USART1);
    set_bits(machine, RCC_BASE + RCC_APB1ENR, APB1_I2C1);
    f1_pad_af(machine, GPIOA_BASE, PA9);
    machine
        .bus
        .write_u32(USART1_BASE + F1_BRR, F1_BRR_115200)
        .unwrap();
    machine
        .bus
        .write_u32(USART1_BASE + F1_CR1, F1_CR1_UE_TE)
        .unwrap();
}

/// What `enable_peripheral_clocks()` was before the repair, and the whole of
/// what those labs did to USART1: the clock bit, and then a raw `DR` write.
/// Kept ONLY to drive the `diagnostic_*` tests.
fn f1_rust_legacy_clock_only(machine: &mut Cm) {
    set_bits(machine, RCC_BASE + RCC_APB2ENR, 1 << 14);
    set_bits(machine, RCC_BASE + RCC_APB1ENR, APB1_I2C1);
}

/// `uart2_init()` from `examples/ntc-thermistor-lab/src/main.rs`. A second pad
/// row: PA2 on USART2, not PA9 on USART1.
fn f1_rust_ntc_init(machine: &mut Cm) {
    set_bits(machine, RCC_BASE + RCC_APB2ENR, (1 << 0) | (1 << 2));
    set_bits(machine, RCC_BASE + RCC_APB1ENR, APB1_USART2);
    f1_pad_af(machine, GPIOA_BASE, PA2);
    machine
        .bus
        .write_u32(USART2_BASE + F1_BRR, F1_BRR_115200)
        .unwrap();
    machine
        .bus
        .write_u32(USART2_BASE + F1_CR1, F1_CR1_UE_TE)
        .unwrap();
}

/// THE GATE for the F103 Rust labs. A probe on PA9 must see USART1 talk.
#[test]
fn f103_rust_lab_shows_uart_edges_on_pa9() {
    let mut machine = lab_machine("examples/bme280-weather-lab/system.yaml");
    f1_rust_bme280_init(&mut machine);

    let edges = tx_edges(
        &mut machine,
        "gpioa",
        PA9,
        USART1_BASE + F1_DR,
        true,
        PAYLOAD,
        u64::from(F1_BRR_115200),
    );

    assert!(
        edges > 0,
        "a probe on PA9 (USART1_TX) captured {edges} edges while USART1 \
         transmitted — the lab's logic analyzer shows a flat line for traffic \
         the bus monitor decodes fine"
    );
}

/// THE GATE for the second F103 pad row: PA2 / USART2.
#[test]
fn f103_rust_lab_shows_uart_edges_on_pa2() {
    let mut machine = lab_machine("examples/ntc-thermistor-lab/system.yaml");
    f1_rust_ntc_init(&mut machine);

    let edges = tx_edges(
        &mut machine,
        "gpioa",
        PA2,
        USART2_BASE + F1_DR,
        false,
        PAYLOAD,
        u64::from(F1_BRR_115200),
    );

    assert!(
        edges > 0,
        "a probe on PA2 (USART2_TX) captured {edges} edges while USART2 \
         transmitted"
    );
}

/// Arm 1 of the old behaviour: the pad IS muxed to an alternate-function
/// output, but `BRR` is still 0. `Uart::bit_time_cycles` returns `None` for a
/// zero divisor, so `wire_push` drops every character before it reaches the
/// wire — there is nothing to narrate.
///
/// HISTORY, pinned: it is why muxing the pad alone would not have fixed these
/// labs. Nothing in the shipping firmware reaches this state any more.
#[test]
fn diagnostic_f103_pad_mux_alone_is_not_enough_without_brr() {
    let mut machine = lab_machine("examples/bme280-weather-lab/system.yaml");
    set_bits(&mut machine, RCC_BASE + RCC_APB2ENR, APB2_AFIO_GPIOA_USART1);
    f1_pad_af(&mut machine, GPIOA_BASE, PA9);
    machine
        .bus
        .write_u32(USART1_BASE + F1_CR1, F1_CR1_UE_TE)
        .unwrap();

    let edges = tx_edges(
        &mut machine,
        "gpioa",
        PA9,
        USART1_BASE + F1_DR,
        true,
        PAYLOAD,
        u64::from(F1_BRR_115200),
    );
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: no BRR ⇒ no narration at all"
    );
}

/// Arm 2: `BRR` IS programmed, but `CRH` is left in its reset state — which is
/// the state every one of these labs left PA9 in. The wire carries the
/// waveform; no route reaches the pad, so `read_gpio_pad` answers with the GPIO
/// latch instead.
///
/// Also HISTORY: it is why programming the divisor alone would not have fixed
/// them either. Both halves were missing and both are now written.
#[test]
fn diagnostic_f103_brr_alone_is_not_enough_without_the_pad_mux() {
    let mut machine = lab_machine("examples/bme280-weather-lab/system.yaml");
    f1_rust_legacy_clock_only(&mut machine);
    machine
        .bus
        .write_u32(USART1_BASE + F1_BRR, F1_BRR_115200)
        .unwrap();
    machine
        .bus
        .write_u32(USART1_BASE + F1_CR1, F1_CR1_UE_TE)
        .unwrap();

    let edges = tx_edges(
        &mut machine,
        "gpioa",
        PA9,
        USART1_BASE + F1_DR,
        true,
        PAYLOAD,
        u64::from(F1_BRR_115200),
    );
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: unrouted pad ⇒ the latch, not the wire"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Family B — the STM32F103 bare-metal C firmwares
// ════════════════════════════════════════════════════════════════════════════

/// `rcc_init()` + `uart_init()` from `examples/f103-uds-ecu/firmware/main.c`,
/// register for register. `f103-uds-ecu/firmware/diff/main.c`,
/// `f103-j1939-monitor/firmware/main.c`, `canmod-gps-sim/firmware/main.c` and
/// `f103-fidelity-bench/firmware/main.c` carry the same pair.
fn f1_c_uds_ecu_init(machine: &mut Cm) {
    set_bits(machine, RCC_BASE + RCC_APB2ENR, APB2_AFIO_GPIOA_USART1);
    set_bits(machine, RCC_BASE + RCC_APB1ENR, APB1_CAN1);
    f1_pad_af(machine, GPIOA_BASE, PA9);
    // `U1_BRR_115200_AT_8MHZ` is 69 decimal, spelled 0x45 above.
    machine
        .bus
        .write_u32(USART1_BASE + F1_BRR, F1_BRR_115200)
        .unwrap();
    machine
        .bus
        .write_u32(USART1_BASE + F1_CR1, F1_CR1_UE_TE)
        .unwrap();
}

/// What `uart_init()` was: `U1_CR1 = CR1_UE | CR1_TE`, and nothing else.
fn f1_c_legacy_cr1_only(machine: &mut Cm) {
    set_bits(machine, RCC_BASE + RCC_APB2ENR, APB2_AFIO_GPIOA_USART1);
    set_bits(machine, RCC_BASE + RCC_APB1ENR, APB1_CAN1);
    machine
        .bus
        .write_u32(USART1_BASE + F1_CR1, F1_CR1_UE_TE)
        .unwrap();
}

/// THE GATE for the F103 C firmwares.
#[test]
fn f103_c_lab_shows_uart_edges_on_pa9() {
    let mut machine = lab_machine("examples/f103-uds-ecu/system.yaml");
    f1_c_uds_ecu_init(&mut machine);

    let edges = tx_edges(
        &mut machine,
        "gpioa",
        PA9,
        USART1_BASE + F1_DR,
        false,
        PAYLOAD,
        u64::from(F1_BRR_115200),
    );

    assert!(
        edges > 0,
        "a probe on PA9 (USART1_TX) captured {edges} edges while the UDS ECU's \
         USART1 transmitted"
    );
}

/// The pre-fix replay, kept as a pinned negative: `CR1 = UE|TE` on its own
/// leaves the pad dark AND the wire without a timebase.
#[test]
fn diagnostic_f103_c_cr1_alone_leaves_the_pad_dark() {
    let mut machine = lab_machine("examples/f103-uds-ecu/system.yaml");
    f1_c_legacy_cr1_only(&mut machine);

    let edges = tx_edges(
        &mut machine,
        "gpioa",
        PA9,
        USART1_BASE + F1_DR,
        false,
        PAYLOAD,
        u64::from(F1_BRR_115200),
    );
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: CR1 alone ⇒ no mux and no divisor"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Family C — the RP2040 Pico demo
// ════════════════════════════════════════════════════════════════════════════

/// `uart0_init()` from `crates/firmware-rp2040-demo/src/main.rs`, register for
/// register. This is the firmware behind the `rp2040-pico` lab, whose own
/// description is "Boot real Pico firmware and read what it prints on serial".
fn rp2040_uart0_init(machine: &mut Cm) {
    let bits = RESETS_UART0 | RESETS_IO_BANK0 | RESETS_PADS_BANK0;
    let reset = machine.bus.read_u32(RESETS_BASE + RESETS_RESET).unwrap();
    machine
        .bus
        .write_u32(RESETS_BASE + RESETS_RESET, reset & !bits)
        .unwrap();
    // The firmware spins on this. If RESET_DONE never answered, the real thing
    // would hang here rather than print, so the poll is asserted, not skipped.
    let done = machine
        .bus
        .read_u32(RESETS_BASE + RESETS_RESET_DONE)
        .unwrap();
    assert_eq!(
        done & bits,
        bits,
        "RESETS.RESET_DONE must acknowledge the deassert the firmware waits on"
    );

    set_bits(machine, CLOCKS_BASE + CLK_PERI_CTRL, CLK_PERI_CTRL_ENABLE);
    machine
        .bus
        .write_u32(IO_BANK0_BASE + GPIO0_CTRL, GPIO_FUNC_UART)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_IBRD, RP_IBRD)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_FBRD, RP_FBRD)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_LCR_H, PL011_LCR_H_8N1_FIFO)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_CR, PL011_CR_UARTEN_TXE)
        .unwrap();
}

/// THE GATE for the Pico demo. A probe on GP0 must see UART0 talk.
#[test]
fn rp2040_pico_lab_shows_uart_edges_on_gp0() {
    let mut machine = lab_machine("configs/systems/rp2040-pico.yaml");
    rp2040_uart0_init(&mut machine);

    let edges = tx_edges(
        &mut machine,
        "sio",
        GP0,
        RP_UART0_BASE + PL011_DR,
        false,
        PAYLOAD,
        RP_BIT_TIME,
    );

    assert!(
        edges > 0,
        "a probe on GP0 (UART0 TX) captured {edges} edges while UART0 \
         transmitted — on real silicon this firmware used to transmit nothing \
         at all"
    );
}

/// The strongest form of the RP2040 gate: no replay at all. Load the COMMITTED
/// `tests/fixtures/rp2040-demo.elf` — the same artifact
/// `crates/core/tests/firmware_survival.rs` boots — let the CPU execute
/// `uart0_init()` and the print loop itself, and watch GP0.
///
/// This is what makes the replay above accountable. A replay can drift from the
/// firmware it claims to mirror; a fixture cannot. If someone edits
/// `crates/firmware-rp2040-demo/src/main.rs` and rebuilds this fixture without
/// the pad and divisor setup, THIS test goes red on the artifact itself.
///
/// ⚠️ The fixture is a committed binary, so it only tracks the source when it is
/// rebuilt: `RUSTFLAGS="-C link-arg=-Tlink.x" cargo build --release
/// -p firmware-rp2040-demo --target thumbv6m-none-eabi`.
///
/// ⚠️ ORDER MATTERS, and not for a reason this test invented. The probe is armed
/// AFTER the firmware has muxed GP0, which is what the playground does —
/// `watch_logic_signals` arms an analyzer on a sim that is already running.
/// Arming it BEFORE binds the channel to the pad latch and it never rebinds:
/// `Rp2040Sio::sync_pad_routes` runs only from `install_logic_tap` and from an
/// SIO `GPIO_OUT`/`GPIO_OE` write, and this firmware touches neither after
/// `uart0_init()`. Written the other way round this test failed with "0 edges"
/// against firmware that was demonstrably transmitting (the console showed
/// forty `RP2040_SMOKE_OK` lines). That is a real observability gap in the
/// engine, recorded here rather than worked around silently; closing it is an
/// engine change and out of this fix's scope.
#[test]
fn rp2040_demo_elf_puts_real_edges_on_gp0() {
    let mut machine = lab_machine("configs/systems/rp2040-pico.yaml");
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/rp2040-demo.elf");
    let image = labwired_loader::load_elf(&fixture)
        .unwrap_or_else(|e| panic!("load {}: {e}", fixture.display()));
    machine
        .load_firmware(&image)
        .expect("load rp2040-demo.elf into the lab machine");

    // Let the CPU run `uart0_init()` and reach the print loop.
    for _ in 0..50_000 {
        machine.step().expect("step");
    }

    let sio = machine
        .bus
        .find_peripheral_index_by_name("sio")
        .expect("sio on the lab bus");
    let initial = machine.logic_watch(&[Some(LogicSource::Pad {
        peripheral: sio,
        pin: GP0,
    })]);
    assert_eq!(
        initial,
        vec![Some(true)],
        "a configured, idle serial line rests at mark, so a start bit is a \
         falling edge; the pad latch would not be reported as the wire"
    );

    // Several `RP2040_SMOKE_OK\n` messages at RP_BIT_TIME clocks per bit.
    for _ in 0..200_000 {
        machine.step().expect("step");
    }

    let edges = machine.logic_read_edges(0).edges.len();
    assert!(
        edges > 0,
        "the real Pico demo ELF ran and a probe on GP0 saw {edges} edges — the \
         lab's own description is \"read what it prints on serial\""
    );
}

/// Arm 1 of the old behaviour, RP2040 side: GP0 IS muxed to the UART function,
/// but both divisor registers are still at their 0 reset value.
#[test]
fn diagnostic_rp2040_funcsel_alone_is_not_enough_without_a_divisor() {
    let mut machine = lab_machine("configs/systems/rp2040-pico.yaml");
    machine
        .bus
        .write_u32(IO_BANK0_BASE + GPIO0_CTRL, GPIO_FUNC_UART)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_CR, PL011_CR_UARTEN_TXE)
        .unwrap();

    let edges = tx_edges(
        &mut machine,
        "sio",
        GP0,
        RP_UART0_BASE + PL011_DR,
        false,
        PAYLOAD,
        RP_BIT_TIME,
    );
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: IBRD = FBRD = 0 ⇒ no bit period"
    );
}

/// Arm 2: the divisors ARE programmed, but GP0's `FUNCSEL` is left at its reset
/// value of 0x1F (NULL) — which is exactly what the demo firmware left it at.
#[test]
fn diagnostic_rp2040_divisor_alone_is_not_enough_without_funcsel() {
    let mut machine = lab_machine("configs/systems/rp2040-pico.yaml");
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_IBRD, RP_IBRD)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_FBRD, RP_FBRD)
        .unwrap();
    machine
        .bus
        .write_u32(RP_UART0_BASE + PL011_CR, PL011_CR_UARTEN_TXE)
        .unwrap();

    let edges = tx_edges(
        &mut machine,
        "sio",
        GP0,
        RP_UART0_BASE + PL011_DR,
        false,
        PAYLOAD,
        RP_BIT_TIME,
    );
    assert_eq!(
        edges, 0,
        "documenting the old behaviour: FUNCSEL = NULL ⇒ the pad latch, not the wire"
    );
}
