// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Classic-ESP32 UART0 has TWO windows onto ONE TX FIFO. Both must arm the drain.
//!
//! `configure_xtensa_esp32` registers the UART twice: `uart0` at the APB base
//! (`0x3FF4_0000`) and `uart0_ahb_fifo` at `0x6000_0000`, a four-byte alias
//! sharing the same `Arc<Mutex<UartCore>>`. A write to EITHER pushes into the
//! SAME `tx_fifo` — and `crates/core/src/tests/esp32.rs` already records which
//! one production firmware uses: "Classic ESP32 IDF writes TX via
//! UART_FIFO_AHB_REG(0)=0x6000_0000, not APB."
//!
//! The two windows stopped being equivalent the moment `Esp32Uart` left the
//! legacy walk. The walk ticked the model every cycle regardless of which
//! address was written, so arming the drain was nobody's job. The scheduler's
//! drain is armed by `SystemBus::collect_scheduled_events(idx)` on the index
//! that was WRITTEN, and the alias is its own bus entry whose `uses_scheduler()`
//! is the default `false`. So an alias write harvests the alias, the alias
//! schedules nothing, `Esp32Uart` is never woken, and the bytes sit in the FIFO
//! for the rest of the run. That took `esp32/L0_serial_boot` and six more
//! matrix cells from pass to `boot_fail` — an empty console — at 1c75eb0a.
//!
//! # Why this asserts the WAKE and not the bytes
//!
//! The two tests that already cover these windows
//! (`esp32_uart0_emits_to_sink`, `esp32_uart0_ahb_fifo_emits_to_sink`) drain by
//! calling `bus.peripherals[uart0_idx].dev.tick()` in a loop, by their own
//! comment "independent of the bus's scheduler cadence". Hand-driving `tick()`
//! proves the model shifts bytes when ticked; it cannot see whether anything
//! ever ticks it, which is precisely what broke. So this asserts the link those
//! cannot: that the write ARMS a scheduler wake for the peripheral that owns
//! the FIFO.
//!
//! Pairing the windows is the point. Either window alone passes on the APB side
//! and proves nothing about the alias; same byte, same FIFO, same machine, only
//! the address differs, so a divergence can only be the wake path.

#![cfg(feature = "event-scheduler")]

use std::sync::{Arc, Mutex};

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::Bus;

/// UART0 TX FIFO, APB window.
const UART0_APB_FIFO: u64 = 0x3FF4_0000;
/// UART0 TX FIFO, AHB alias window (`uart0_ahb_fifo`) — the one IDF/Arduino use.
const UART0_AHB_FIFO: u64 = 0x6000_0000;

/// Write one byte to `fifo_addr` on a fresh classic-ESP32 bus and report
/// `(wakes armed for the uart0 index, TXFIFO_CNT seen through APB STATUS)`.
fn wake_and_fifo_depth_after_write(fifo_addr: u64) -> (usize, u32) {
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink, false);

    let uart0 = bus
        .find_peripheral_index_by_name("uart0")
        .expect("uart0 mapped");

    // Anything queued by construction is not what this write did.
    bus.pending_schedule.clear();
    bus.write_u32(fifo_addr, u32::from(b'Z')).expect("tx write");

    let wakes = bus
        .pending_schedule
        .iter()
        .filter(|(idx, _, _)| *idx == uart0)
        .count();
    let depth = (bus.read_u32(0x3FF4_001C).expect("status") >> 16) & 0xFF;
    (wakes, depth)
}

/// Both windows must reach the same FIFO. If this ever fails, the rest of the
/// file is testing something other than one shared buffer.
#[test]
fn both_windows_push_into_the_same_tx_fifo() {
    assert_eq!(
        wake_and_fifo_depth_after_write(UART0_APB_FIFO).1,
        1,
        "APB write did not land in uart0's TX FIFO"
    );
    assert_eq!(
        wake_and_fifo_depth_after_write(UART0_AHB_FIFO).1,
        1,
        "AHB alias write did not land in uart0's TX FIFO"
    );
}

/// The control: the path the existing gates covered, and the one that worked.
#[test]
fn an_apb_write_arms_the_uart_drain() {
    let (wakes, _) = wake_and_fifo_depth_after_write(UART0_APB_FIFO);
    assert_eq!(wakes, 1, "APB FIFO write armed no scheduler wake for uart0");
}

/// The regression. Same byte, same FIFO, same bus — only the address moved.
#[test]
fn an_ahb_alias_write_arms_the_uart_drain_too() {
    let (wakes, _) = wake_and_fifo_depth_after_write(UART0_AHB_FIFO);
    assert_eq!(
        wakes, 1,
        "AHB alias write armed NO scheduler wake for uart0, while the same \
         byte through the APB window arms one. The alias shares the owner's \
         tx_fifo but is its own bus entry, so `collect_scheduled_events` \
         harvested the alias — which schedules nothing — and `Esp32Uart` is \
         never woken to drain. This is what empties the arduino-esp32 console."
    );
}
