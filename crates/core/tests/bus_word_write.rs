// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Integration tests for the word-granular bus write path (Task A2).
//!
//! Verifies that `Bus::write_u32` dispatches a single word-level trigger
//! through `Peripheral::write_word_32`, and that byte writes do NOT activate
//! `WriteWord` timing hooks.

use labwired_config::{
    Access, PeripheralDescriptor, RegisterDescriptor, TimingAction, TimingDescriptor,
    TimingTrigger, TriggerMatch,
};
use labwired_core::bus::SystemBus;
use labwired_core::memory::LinearMemory;
use labwired_core::peripherals::declarative::GenericPeripheral;
use labwired_core::Bus;

/// Build a minimal `SystemBus` with no default peripherals to avoid address
/// conflicts with the STM32 default peripheral map embedded in `SystemBus::new()`.
fn minimal_bus() -> SystemBus {
    SystemBus {
        flash: LinearMemory::new(0, 0x0),
        ram: LinearMemory::new(0, 0x0),
        peripherals: Vec::new(),
        nvic: None,
        pending_cpu_irqs: 0,
    }
}

/// Build a declarative peripheral descriptor with:
/// - `CTRL` register at offset 0x00 (32-bit, RW, reset=0)
/// - `STATUS` register at offset 0x04 (32-bit, RW, reset=0)
/// - A `WriteWord` timing hook on CTRL matching 0xDEAD_BEEF:
///   fires with delay=0, sets bit 0 of STATUS.
fn make_word_trigger_descriptor() -> PeripheralDescriptor {
    PeripheralDescriptor {
        peripheral: "test_word".to_string(),
        version: "1.0".to_string(),
        registers: vec![
            RegisterDescriptor {
                id: "CTRL".to_string(),
                address_offset: 0x00,
                size: 32,
                access: Access::ReadWrite,
                reset_value: 0,
                fields: vec![],
                side_effects: None,
            },
            RegisterDescriptor {
                id: "STATUS".to_string(),
                address_offset: 0x04,
                size: 32,
                access: Access::ReadWrite,
                reset_value: 0,
                fields: vec![],
                side_effects: None,
            },
        ],
        interrupts: None,
        timing: Some(vec![TimingDescriptor {
            id: "word_trigger".to_string(),
            trigger: TimingTrigger::WriteWord {
                register: "CTRL".to_string(),
                match_value: Some(TriggerMatch::Word(0xDEAD_BEEF)),
            },
            delay_cycles: 0,
            action: TimingAction::SetBits {
                register: "STATUS".to_string(),
                bits: 0x01,
            },
            interrupt: None,
        }]),
    }
}

/// A single `write_u32` must trigger exactly once, not four times.
///
/// The `WriteWord` hook fires when `write_word_32` is called by the bus.
/// After `tick()` (delay=0 fires immediately), STATUS bit-0 must be 1.
#[test]
fn word_write_triggers_once_not_four_times() {
    let desc = make_word_trigger_descriptor();
    let peripheral = GenericPeripheral::new(desc);

    let mut bus = minimal_bus();
    bus.add_peripheral("test", 0x4000_0000, 0x1000, None, Box::new(peripheral));

    // Single coherent 32-bit write of 0xDEAD_BEEF to CTRL (offset 0x00).
    bus.write_u32(0x4000_0000, 0xDEAD_BEEF).unwrap();

    // Tick to advance the delay=0 event into fired state.
    bus.tick_peripherals();

    // STATUS should have bit 0 set exactly once (not 4 times from 4 byte writes).
    // Since SetBits is idempotent (OR), we verify by checking the value is 1,
    // and then verify the byte-write test separately shows 0.
    let status_lo = bus.read_u8(0x4000_0004).unwrap();
    assert_eq!(
        status_lo, 0x01,
        "one 32-bit write should fire WriteWord trigger exactly once; STATUS[0] must be 1"
    );
}

/// Four individual byte writes that together form 0xDEAD_BEEF must NOT fire
/// the `WriteWord` trigger.
///
/// `WriteWord` hooks are only fired from `write_word_32`, which is only called
/// by `write_u32`. Direct byte writes go through the byte-level path which
/// explicitly skips `WriteWord` hooks.
#[test]
fn byte_writes_do_not_activate_word_match_triggers() {
    let desc = make_word_trigger_descriptor();
    let peripheral = GenericPeripheral::new(desc);

    let mut bus = minimal_bus();
    bus.add_peripheral("test", 0x4000_0000, 0x1000, None, Box::new(peripheral));

    // Write the bytes of 0xDEAD_BEEF in little-endian order individually.
    bus.write_u8(0x4000_0000, 0xEF).unwrap(); // byte 0
    bus.write_u8(0x4000_0001, 0xBE).unwrap(); // byte 1
    bus.write_u8(0x4000_0002, 0xAD).unwrap(); // byte 2
    bus.write_u8(0x4000_0003, 0xDE).unwrap(); // byte 3

    bus.tick_peripherals();

    // STATUS must remain 0 — byte writes must not activate WriteWord triggers.
    let status_lo = bus.read_u8(0x4000_0004).unwrap();
    assert_eq!(
        status_lo, 0x00,
        "byte writes must not activate WriteWord triggers; STATUS[0] must remain 0"
    );
}
