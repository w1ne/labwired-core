// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Cross-core yield latency on the dual-core ESP32-S3 — the contract that keeps
//! ESP-IDF SMP FreeRTOS alive on a BATCHING engine (the browser's
//! `Sim::step_batch` → `Machine::advance(AdvanceRequest::run(..))`).
//!
//! ## The bug these gate
//!
//! ESP-IDF implements `portYIELD_WITHIN_API()` as
//! `esp_crosscore_int_send_yield(xPortGetCoreID())`: a core rings its **own**
//! `SYSTEM_CPU_INTR_FROM_CPU_n` doorbell from *inside* a critical section and
//! relies on the interrupt landing the instant `portEXIT_CRITICAL` re-enables
//! interrupts. Deliver it one instruction late and nothing happens; deliver it a
//! few hundred instructions late and `xQueueReceive` falls out of its `for(;;)`
//! still runnable, calls `vTaskPlaceOnEventList` a SECOND time for a task that
//! is already on that list, and `vListInsert` links the item's `pxNext` to
//! itself. The next insert walks that self-loop forever — with the queue
//! spinlock held — while the other core spins in `spinlock_acquire`. A hard SMP
//! deadlock at ~2 MIPS of forward motion and zero forward progress.
//!
//! Two engine defects produced exactly that on the ESP32-S3 Doom lab. Native
//! `labwired run` never saw either, because its Xtensa loop drives
//! `AdvanceRequest::single()` — one instruction, one peripheral boundary. The
//! browser drives `AdvanceRequest::run(..)`, and only the browser deadlocked.
//!
//! 1. **No S3 write choke.** The routed per-core IRQ bitmap (`pending_cpu_irqs`,
//!    what the Xtensa cores poll every instruction) was rebuilt *only* by the
//!    per-tick aggregation. The ESP32-C3 has re-derived it at the MMIO write
//!    choke since its walk-free migration (`sync_esp32c3_irq_cache_write`); the
//!    S3 never grew the twin. Gated by
//!    [`from_cpu_ipi_is_routed_at_the_write_not_at_the_next_tick`].
//!
//! 2. **A batch window wider than the tick interval.** `plan_cpu_window`'s
//!    `SECONDARY_PARKED` clause ran the primary up to 1024 instructions per
//!    machine boundary while the APP core was WAITI-parked — *even at
//!    `peripheral_tick_interval == 1`*, which its own comment advertised. Gated
//!    by `parked_secondary_batch_stays_inside_the_peripheral_tick_interval` in
//!    `crates/core/src/tests/machine_advance.rs` (feature-independent, so it
//!    runs in the plain `cargo test -p labwired-core` too).
//!
//! 3. **A dual-core machine claiming a relaxable tick interval.**
//!    `SystemBus::max_safe_tick_interval` answers "is every peripheral
//!    scheduler-driven", which bounds *peripheral observation* error to one
//!    interval. On an SMP machine an interrupt-delivery skew of one interval is
//!    not an observation error, it is a semantic one — see above. Gated by
//!    [`a_dual_core_machine_never_relaxes_the_peripheral_tick_interval`].
//!
//! ## Negative control
//!
//! Each test names the exact line to delete to see it fail. Deleting the
//! `sync_esp32s3_irq_write` call from `bus/accessors.rs` turns test 1 red;
//! restoring `clamp!(count, .., SECONDARY_PARKED, 1024)` without the tick clamp
//! turns the `machine_advance` twin red; deleting the `cpu_secondary.is_some()`
//! early return in `Machine::max_safe_tick_interval` turns test 3 red.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::XtensaLx7;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Bus, Machine};

/// `SYSTEM_CPU_INTR_FROM_CPU_0_REG` — core 0's own yield doorbell.
const FROM_CPU_0_REG: u64 = 0x600C_0030;
/// `FROM_CPU_INTR0` interrupt-matrix source ID (TRM §9.2 table).
const FROM_CPU_INTR0_SOURCE: u32 = 79;
const INTMATRIX_BASE: u64 = 0x600C_2000;
/// Any free CPU interrupt slot; the ISR side is irrelevant here.
const CPU_IRQ_SLOT: u8 = 13;

/// A bare ESP32-S3 bus with `FROM_CPU_INTR0` bound to `CPU_IRQ_SLOT` on core 0,
/// running at a relaxed tick interval so that NOTHING will tick between the
/// doorbell write and the assertion.
fn s3_bus_with_ipi_routed() -> SystemBus {
    let mut bus = SystemBus::new();
    let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    bus.write_u32(
        INTMATRIX_BASE + u64::from(FROM_CPU_INTR0_SOURCE) * 4,
        u32::from(CPU_IRQ_SLOT),
    )
    .unwrap();
    // The failing configuration: a wide tick grid, so "it will be picked up at
    // the next tick" is up to 512 cycles of nothing.
    bus.config.peripheral_tick_interval = 512;
    bus
}

/// Ringing core 0's own FROM_CPU doorbell must show up in the routed IRQ mask
/// **at the store**, with no peripheral tick in between.
///
/// Negative control: delete the `self.sync_esp32s3_irq_write(idx);` call from
/// `SystemBus::write_u32`'s peripheral branch in `crates/core/src/bus/accessors.rs`
/// and this test fails with `routed after doorbell = 0x0`.
#[test]
fn from_cpu_ipi_is_routed_at_the_write_not_at_the_next_tick() {
    let mut bus = s3_bus_with_ipi_routed();
    let slot_mask = 1u32 << CPU_IRQ_SLOT;

    assert_eq!(
        bus.pending_cpu_irqs(0) & slot_mask,
        0,
        "core 0 must start with the FROM_CPU slot clear"
    );

    // `crosscore_int_ll_trigger_interrupt(0)` — the whole of `portYIELD_WITHIN_API`.
    bus.write_u32(FROM_CPU_0_REG, 1).unwrap();

    // NO tick_peripherals here. This is the point: on silicon the level is
    // asserted by the store itself, and the core polls it on the next
    // instruction. An engine that only re-derives levels on a tick boundary
    // hands ESP-IDF a yield that arrives up to one tick interval late.
    assert_ne!(
        bus.pending_cpu_irqs(0) & slot_mask,
        0,
        "FROM_CPU_INTR0 must be routed to core 0 at the doorbell write, not at \
         the next peripheral tick (routed = {:#x})",
        bus.pending_cpu_irqs(0)
    );

    // The other half of the same contract: `esp_crosscore_isr` acknowledges by
    // writing 0, and the level must drop at THAT store too — otherwise the ISR
    // re-enters itself until the next tick.
    bus.write_u32(FROM_CPU_0_REG, 0).unwrap();
    assert_eq!(
        bus.pending_cpu_irqs(0) & slot_mask,
        0,
        "the ISR's acknowledge must de-assert the level at the write"
    );
}

/// The doorbell for the OTHER core must not leak onto core 0's line — the
/// choke re-derives the whole routed mask, so a bug there would show up as a
/// core-0 assertion from a core-1 doorbell.
#[test]
fn from_cpu_1_doorbell_does_not_assert_core_0() {
    let mut bus = s3_bus_with_ipi_routed();
    bus.write_u32(FROM_CPU_0_REG + 4, 1).unwrap(); // FROM_CPU_1
    assert_eq!(
        bus.pending_cpu_irqs(0) & (1u32 << CPU_IRQ_SLOT),
        0,
        "FROM_CPU_INTR1 is not bound to a core-0 slot and must not route there"
    );
}

/// A machine with a second CPU may not batch peripherals, however relaxable the
/// bus's own peripherals are.
///
/// Negative control: delete the `if self.cpu_secondary.is_some() { return 1; }`
/// early return from `Machine::max_safe_tick_interval` and this fails with
/// `dual-core machine reported 512`.
#[test]
fn a_dual_core_machine_never_relaxes_the_peripheral_tick_interval() {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let bus_says = bus.max_safe_tick_interval();

    // Non-vacuity: this bus must be one that WOULD relax. If the S3 bus ever
    // stops being walk-deletable this assert fires and the test below stops
    // proving anything, which is the point of asserting it.
    assert!(
        bus_says > 1,
        "the S3 bus is expected to be walk-deletable (bus says {bus_says}); \
         without that this gate cannot distinguish the clamp from the default"
    );

    let cpu: XtensaLx7 = wiring.cpu;
    let app_cpu = XtensaLx7::new_app_cpu();
    let single = Machine::new(cpu, bus);
    assert_eq!(
        single.max_safe_tick_interval(),
        bus_says,
        "a single-core machine must pass the bus's answer through unchanged"
    );

    let dual = single.with_secondary_cpu(app_cpu);
    assert_eq!(
        dual.max_safe_tick_interval(),
        1,
        "a dual-core machine must stay at interval 1: cross-core IPI delivery \
         skew is a semantic error, not an observation error"
    );
}
