// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 UART controller (UART0/UART1).
//!
//! The C3 carries the same Espressif UART IP as the ESP32-S3 — identical
//! register map (ESP32-C3 TRM §UART: FIFO@0x00, INT_RAW@0x04, INT_ST@0x08,
//! INT_ENA@0x0C, INT_CLR@0x10, CLKDIV@0x14, STATUS@0x1C with TXFIFO_CNT[25:16],
//! CONF0@0x20, CONF1@0x24 with TXFIFO_EMPTY_THRHD[19:10]), the same
//! `uart_ll.h` interrupt bit positions (RXFIFO_FULL b0, TXFIFO_EMPTY b1,
//! RXFIFO_OVF b4, RXFIFO_TOUT b8, TX_DONE b14), and the same 128-entry
//! (`SOC_UART_FIFO_LEN`) TX/RX FIFOs. So this family reuses
//! [`EspUart`](crate::peripherals::esp_uart::EspUart) rather than
//! duplicating the twin; only the interrupt-matrix source ids and the core
//! clock differ, and both are passed in here.
//!
//! ## Why this matters (the bug it fixes)
//!
//! `configs/chips/esp32c3.yaml` previously declared `uart0`/`uart1` as the
//! vendor-neutral `type: "uart"`, which resolves to the *STM32* register map
//! (SR@0x00, DR@0x04). On that map the C3's FIFO writes land on a status
//! register, `STATUS` (0x1C) reads back 0 — so `uart_ll_get_txfifo_len()`
//! always reported all 128 entries free — and nothing ever asserted the UART
//! interrupt. ESP-IDF's blocking `uart_tx_all()` fills the TX FIFO, and for any
//! write larger than the 128-byte FIFO sets `tx_waiting_fifo`, enables
//! `UART_INTR_TXFIFO_EMPTY` and blocks on `tx_fifo_sem` until the ISR signals
//! room. With no TX interrupt that semaphore was never given: **any
//! `Serial.print` longer than 128 bytes wedged the sketch permanently**, with
//! every task blocked and the idle task parked in `wfi`. Short lines fit in one
//! FIFO fill and masked it; so did BROM `ets_printf`, which writes the FIFO
//! directly and never waits on the driver.
//!
//! Reset values for the config/version registers (`DATE`, `ID`, `CLK_CONF`, …)
//! come from the S3 twin and are NOT silicon-pinned for the C3 — ESP-IDF does
//! not gate on them. The behavioral surface that firmware does depend on
//! (FIFO depth, STATUS occupancy, the interrupt set) is shared IP and applies
//! to both parts.

use crate::peripherals::esp_uart::EspUart;

/// Interrupt-matrix source for UART0 (`ETS_UART0_INTR_SOURCE`). Pinned by the
/// same enum that gives SPI2 = 19, LEDC = 23, I2C_EXT0 = 29 and APB_ADC = 43,
/// all already asserted by the C3 models in this directory.
pub const UART0_INTR_SOURCE_ID: u32 = 21;
/// Interrupt-matrix source for UART1 (`ETS_UART1_INTR_SOURCE`).
pub const UART1_INTR_SOURCE_ID: u32 = 22;

/// ESP32-C3 core clock. The UART twin scales its baud pacing (`10 * clkdiv`
/// UART-clock cycles) into CPU ticks, so this must be the C3's rate, not the
/// S3's 240 MHz.
pub const CPU_CLOCK_HZ: u64 = 160_000_000;

/// Peripheral base addresses, used to pick the default source id when the chip
/// descriptor does not name one explicitly.
pub const UART0_BASE: u64 = 0x6000_0000;
pub const UART1_BASE: u64 = 0x6001_0000;

/// The interrupt-matrix source for a UART at `base`. UART1's base selects 22;
/// anything else (i.e. UART0) selects 21.
pub fn default_source_id(base: u64) -> u32 {
    if base == UART1_BASE {
        UART1_INTR_SOURCE_ID
    } else {
        UART0_INTR_SOURCE_ID
    }
}

/// Build a C3 UART instance: the shared Espressif twin, paced at 160 MHz.
/// `echo_stdout` routes shifted-out TX to the host console (UART0's default,
/// the Arduino `Serial` console); UART1 stays capture-only.
pub fn new(echo_stdout: bool, source_id: u32) -> EspUart {
    EspUart::new_with_cpu_clock(echo_stdout, source_id, CPU_CLOCK_HZ)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    /// The source ids must match the C3 interrupt-matrix enum its sibling
    /// models are already pinned to (SPI2 = 19, LEDC = 23, I2C_EXT0 = 29).
    #[test]
    fn source_ids_match_the_c3_interrupt_matrix() {
        assert_eq!(UART0_INTR_SOURCE_ID, 21);
        assert_eq!(UART1_INTR_SOURCE_ID, 22);
        assert_eq!(default_source_id(UART0_BASE), 21);
        assert_eq!(default_source_id(UART1_BASE), 22);
    }

    /// The regression this module exists for: with the ESP-IDF register map,
    /// `STATUS` reports true TX occupancy, so `uart_ll_get_txfifo_len()`
    /// (128 - TXFIFO_CNT) sees a FULL FIFO after 128 bytes instead of the
    /// STM32-layout's constant 0 → "128 free forever".
    #[test]
    fn status_reports_txfifo_occupancy_so_the_driver_sees_a_full_fifo() {
        let mut u = new(false, UART0_INTR_SOURCE_ID);
        for _ in 0..128 {
            u.write_u32(0x00, b'x' as u32).unwrap();
        }
        let txfifo_cnt = (u.read_u32(0x1C).unwrap() >> 16) & 0x3FF;
        assert_eq!(txfifo_cnt, 128, "STATUS.TXFIFO_CNT must track occupancy");
        assert_eq!(128 - txfifo_cnt, 0, "uart_ll_get_txfifo_len() → no room");
    }

    /// …and the TXFIFO_EMPTY interrupt that `uart_tx_all()` blocks on is
    /// actually asserted once the FIFO drains below the threshold, so the
    /// driver's `tx_fifo_sem` gets given and a >128-byte write completes.
    #[test]
    fn txfifo_empty_interrupt_is_asserted_to_the_matrix_when_the_fifo_drains() {
        let mut u = new(false, UART0_INTR_SOURCE_ID);
        // CONF1.TXFIFO_EMPTY_THRHD[19:10] = 10, as UART_EMPTY_THRESH_DEFAULT.
        u.write_u32(0x24, 10 << 10).unwrap();
        u.write_u32(0x0C, 1 << 1).unwrap(); // INT_ENA: TXFIFO_EMPTY
        for _ in 0..128 {
            u.write_u32(0x00, b'x' as u32).unwrap();
        }
        let mut sources = Vec::new();
        u.matrix_irq_sources_into(&mut sources);
        assert!(
            sources.is_empty(),
            "a full TX FIFO must not claim TXFIFO_EMPTY"
        );

        // Shift the FIFO out; the level bit rises as occupancy drops below 10.
        for _ in 0..2_000_000 {
            u.tick();
            sources.clear();
            u.matrix_irq_sources_into(&mut sources);
            if sources.contains(&UART0_INTR_SOURCE_ID) {
                return;
            }
        }
        panic!("TXFIFO_EMPTY never reached the interrupt matrix — driver would block forever");
    }

    /// Externally-injected bytes (`uart_injections:` / interactive serial)
    /// reach the RX FIFO, keeping parity with the generic `Uart` this replaces.
    #[test]
    fn injected_rx_bytes_reach_the_fifo() {
        let u = new(false, UART0_INTR_SOURCE_ID);
        u.rx_buffer().lock().unwrap().extend(*b"AB");
        assert_eq!(u.read_u32(0x00).unwrap(), b'A' as u32);
        assert_eq!(u.read_u32(0x00).unwrap(), b'B' as u32);
    }
}
