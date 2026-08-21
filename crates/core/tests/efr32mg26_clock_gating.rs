// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! EFR32MG26 clock gating — a peripheral firmware never clocked must not
//! answer.
//!
//! On Series 2 a peripheral's bus interface is dead until its `CMU_CLKEN` bit
//! is set: reads return zero, writes are dropped. That is the single most
//! common bring-up mistake on this family, and it is a quiet one — the symptom
//! on silicon is a block that reads back all zeroes, not a fault. A simulator
//! that answers regardless is *permissive*: the firmware passes in the twin and
//! fails on the bench, which is the exact failure mode a twin exists to catch.
//!
//! # How these tests avoid proving nothing
//!
//! Each probe reads a **read-only status register with a non-zero reset value**
//! and checks it two-sided: zero while gated, the silicon reset value once
//! clocked.
//!
//! Probing by writing a writable register and reading it back does NOT work and
//! has produced false passes in this codebase before: a gated model still holds
//! the written bytes in its own state, so the read-back succeeds even with the
//! `clock:` key deleted. `USART1_STATUS` (offset 0x18, reset `0x2040` =
//! TXBL|TXIDLE — `efr32mg26_usart.h`) has neither problem.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

const CMU: u64 = 0x4000_8000;
/// `CMU_CLKEN0`, absolute — walked from `CMU_TypeDef`.
const CMU_CLKEN0: u64 = CMU + 0x64;
/// `CMU_CLKEN2`, absolute.
const CMU_CLKEN2: u64 = CMU + 0x6C;
/// `CMU_CLKEN0.GPIO`.
const CLKEN0_GPIO: u32 = 1 << 26;
/// `CMU_CLKEN2.USART1` — USART1 is on CLKEN2, USART0 on CLKEN0 bit 9.
const CLKEN2_USART1: u32 = 1 << 7;

/// `USART1_S_BASE`, the BRD2709A VCOM console.
const USART1: u64 = 0x400A_4000;
/// `USART_TypeDef.STATUS` on Series 2, reset TXBL|TXIDLE.
const USART_STATUS: u64 = 0x18;
const USART_STATUS_RESET: u32 = 0x2040;

/// GPIOC port struct; DIN (+0x14) is read-only.
const GPIOC: u64 = 0x4003_C090;
/// `MODEH` (+0x0C) holds the 4-bit mode nibble for pins 8..15; `0x4` is
/// PUSHPULL. A pad only drives DIN from DOUT once its mode says it is an
/// output, so every DIN probe below configures PC08/PC09 first — the same two
/// nibbles the demo firmware writes for LED0/LED1.
const GPIO_MODEH: u64 = 0x0C;
const GPIO_MODEH_PC08_PC09_PUSHPULL: u32 = 0x0000_0044;
const GPIO_DOUT: u64 = 0x10;
const GPIO_DIN: u64 = 0x14;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn bus() -> SystemBus {
    let abs = root("configs/chips/efr32mg26.yaml");
    let chip = ChipDescriptor::from_file(&abs).expect("load chip descriptor");
    let manifest = SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "efr32mg26-clock-gating".to_string(),
        chip: abs.to_string_lossy().to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
        cpu_hz: None,
    };
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

#[test]
fn usart1_is_dead_until_clken2_bit_7_is_set() {
    let mut bus = bus();

    assert_eq!(
        bus.read_u32(USART1 + USART_STATUS).unwrap(),
        0,
        "an unclocked USART1 must read zero, not its reset value"
    );

    bus.write_u32(CMU_CLKEN2, CLKEN2_USART1).unwrap();
    assert_eq!(
        bus.read_u32(USART1 + USART_STATUS).unwrap(),
        USART_STATUS_RESET,
        "once CLKEN2.USART1 is set the model must answer with the silicon \
         reset value (TXBL|TXIDLE)"
    );

    // And clock gating is not one-way: `CMU_ClockEnable(..., false)` puts the
    // block back to sleep.
    bus.write_u32(CMU_CLKEN2, 0).unwrap();
    assert_eq!(bus.read_u32(USART1 + USART_STATUS).unwrap(), 0);
}

/// The bit matters, not merely "something in CLKEN2". USART2 is bit 8; setting
/// it must not wake USART1.
#[test]
fn a_neighbours_clock_bit_does_not_wake_usart1() {
    let mut bus = bus();
    bus.write_u32(CMU_CLKEN2, 1 << 8).unwrap(); // USART2
    assert_eq!(bus.read_u32(USART1 + USART_STATUS).unwrap(), 0);
}

/// The register matters too. GPIO is CLKEN0 bit 26; bit 26 of CLKEN2 is
/// reserved and must not gate anything open.
#[test]
fn the_gate_names_a_register_not_just_a_bit() {
    let mut bus = bus();
    bus.write_u32(CMU_CLKEN2, 1 << 26).unwrap();
    bus.write_u32(GPIOC + GPIO_DOUT, 1 << 8).unwrap();
    assert_eq!(
        bus.read_u32(GPIOC + GPIO_DIN).unwrap(),
        0,
        "GPIO lives on CLKEN0; the same bit number in CLKEN2 must not clock it"
    );

    bus.write_u32(CMU_CLKEN0, CLKEN0_GPIO).unwrap();
    bus.write_u32(GPIOC + GPIO_MODEH, GPIO_MODEH_PC08_PC09_PUSHPULL)
        .unwrap();
    bus.write_u32(GPIOC + GPIO_DOUT, 1 << 8).unwrap();
    assert_eq!(
        bus.read_u32(GPIOC + GPIO_DIN).unwrap(),
        1 << 8,
        "a clocked, push-pull port drives DIN from DOUT"
    );
}

/// Writes to a gated peripheral are dropped, not buffered until the clock
/// arrives. Silicon does not replay them, and a model that did would let
/// firmware "work" with its clock enable in the wrong order.
#[test]
fn writes_made_while_gated_are_lost_not_replayed() {
    let mut bus = bus();

    bus.write_u32(GPIOC + GPIO_DOUT, 1 << 8).unwrap();
    bus.write_u32(CMU_CLKEN0, CLKEN0_GPIO).unwrap();
    assert_eq!(
        bus.read_u32(GPIOC + GPIO_DOUT).unwrap(),
        0,
        "the pre-clock DOUT write must not appear once the clock arrives"
    );
}

/// The CMU itself is never gated — it cannot be, since it is what holds the
/// gates. A chip whose CMU needed clocking could never boot.
#[test]
fn the_cmu_answers_before_any_clock_is_enabled() {
    let bus = bus();
    assert_eq!(
        bus.read_u32(CMU).unwrap(),
        7,
        "CMU_IPVERSION must read its reset value on a cold bus"
    );
}

/// The clock enable is reachable through the Series-2 SET alias, which is how
/// the Gecko SDK actually writes it (`CMU->CLKEN0_SET = CMU_CLKEN0_GPIO`).
/// Gating and the alias decode have to work *together*: either alone leaves
/// stock vendor firmware dead.
#[test]
fn the_gecko_sdk_spelling_of_a_clock_enable_works() {
    let mut bus = bus();
    bus.write_u32(CMU_CLKEN0 + 0x1000, CLKEN0_GPIO).unwrap();
    assert_eq!(bus.read_u32(CMU_CLKEN0).unwrap(), CLKEN0_GPIO);

    bus.write_u32(GPIOC + GPIO_MODEH, GPIO_MODEH_PC08_PC09_PUSHPULL)
        .unwrap();
    bus.write_u32(GPIOC + GPIO_DOUT, 1 << 9).unwrap();
    assert_eq!(bus.read_u32(GPIOC + GPIO_DIN).unwrap(), 1 << 9);

    // ...and `CMU_ClockEnable(cmuClock_GPIO, false)` through the CLR alias.
    bus.write_u32(CMU_CLKEN0 + 0x2000, CLKEN0_GPIO).unwrap();
    assert_eq!(bus.read_u32(CMU_CLKEN0).unwrap(), 0);
    assert_eq!(bus.read_u32(GPIOC + GPIO_DIN).unwrap(), 0);
}
