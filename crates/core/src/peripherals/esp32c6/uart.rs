// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C6 UART controller (UART0/UART1).
//!
//! The C6 carries a newer revision of the Espressif UART IP than the C3/S3,
//! but the head register map a console path actually drives is offset-identical
//! (verified by diffing `tests/fixtures/real_world/esp32c6.svd` against the C3
//! SVD): FIFO@0x00, INT_RAW@0x04, INT_ST@0x08, INT_ENA@0x0C, INT_CLR@0x10,
//! CLKDIV@0x14, RX_FILT@0x18, STATUS@0x1C, CONF0@0x20, CONF1@0x24. The C6 also
//! keeps the 128-entry `SOC_UART_FIFO_LEN` TX/RX FIFOs and the `uart_ll.h`
//! interrupt bit positions, so the model itself stays chip-neutral in
//! [`crate::peripherals::esp_uart::EspUart`] and this module only supplies the
//! C6's per-family values.
//!
//! ## What differs on the C6 (NOT modelled yet)
//!
//! The tail after `CONF1` moved: C6 adds `SLEEP_CONF0..2` at 0x30..0x38, which
//! pushes `CLK_CONF` to 0x88, `DATE` to 0x8C and `ID` to 0x9C (C3: 0x78 / 0x7C
//! / 0x80). The shared model's tail registers are still the C3/S3 offsets, so
//! a C6 driver that reads `UART_ID` at 0x9C gets 0, and the model's 0x78/0x7C
//! answers do not correspond to any C6 register. The head map the smoke path
//! uses is correct; the tail is listed as a limitation on the board page.

use crate::peripherals::esp_uart::EspUart;

/// Interrupt-matrix source for UART0 (`ETS_UART0_INTR_SOURCE`), from the
/// ESP32-C6 SVD (`esp32c6.svd`, INTERRUPT_CORE0 interrupt `UART0` = 43).
/// This is NOT the C3's 21.
pub const UART0_INTR_SOURCE_ID: u32 = 43;
/// Interrupt-matrix source for UART1 (`ETS_UART1_INTR_SOURCE`); C6 SVD `UART1` = 44.
pub const UART1_INTR_SOURCE_ID: u32 = 44;

/// ESP32-C6 HP-core clock (ESP32-C6 Datasheet v1.5 §4.1.1.1: up to 160 MHz).
/// The UART twin scales its baud pacing (`10 * clkdiv` UART-clock cycles) into
/// CPU ticks, so this must be the C6's rate.
pub const CPU_CLOCK_HZ: u64 = 160_000_000;

/// The default interrupt-matrix source when a chip descriptor does not name one
/// with `irq:`. UART1's page base (`0x6000_1000`, soc/reg_base.h
/// `DR_REG_UART1_BASE`) selects 44; anything else (i.e. UART0) selects 43.
///
/// Deliberately takes the base as a parameter instead of comparing a local
/// `UART1_BASE` constant: the base is the descriptor's fact, and the
/// `yaml_owned_base_contract` gate exists because a model re-stating a chip's
/// addresses is how the nRF52 GPIOTE defect happened.
pub fn default_source_id(base: u64) -> u32 {
    if base == 0x6000_1000 {
        UART1_INTR_SOURCE_ID
    } else {
        UART0_INTR_SOURCE_ID
    }
}

/// Build a C6 UART instance: the shared Espressif twin, paced at 160 MHz.
/// `echo_stdout` routes shifted-out TX to the host console (UART0's default,
/// the DevKitC-1's USB-to-UART bridge); UART1 stays capture-only.
pub fn new(echo_stdout: bool, source_id: u32) -> EspUart {
    EspUart::new_with_cpu_clock(echo_stdout, source_id, CPU_CLOCK_HZ).with_c6_reg_map()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    /// The C6 source ids are its own, not the C3's (21/22). Pinned to the SVD
    /// so a copy-paste from the C3 family fails here instead of misrouting the
    /// console UART's interrupt to a random matrix slot.
    #[test]
    fn source_ids_match_the_c6_interrupt_matrix() {
        assert_eq!(UART0_INTR_SOURCE_ID, 43);
        assert_eq!(UART1_INTR_SOURCE_ID, 44);
        assert_ne!(
            UART0_INTR_SOURCE_ID,
            crate::peripherals::esp32c3::uart::UART0_INTR_SOURCE_ID
        );
        assert_eq!(default_source_id(0x6000_0000), 43);
        assert_eq!(default_source_id(0x6000_1000), 44);
    }

    /// The twin must be paced against the C6's 160 MHz, not the S3's 240 MHz:
    /// at reset CLKDIV (0x2B6 = 694) one 10-bit frame is 10*694*160/80 = 13_880
    /// CPU cycles — so a byte written to the FIFO is still queued at 13_879
    /// cycles and gone by 13_880. At 240 MHz it would leave at 20_820.
    #[test]
    fn uart_is_paced_at_the_c6_core_clock() {
        let mut u = new(false, UART0_INTR_SOURCE_ID);
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()));
        u.write_u32(0x00, b'O' as u32).unwrap();

        u.tick_elapsed(13_879);
        assert!(
            sink.lock().unwrap().is_empty(),
            "a 160 MHz frame must not complete before 13_880 cycles"
        );
        u.tick_elapsed(1);
        assert_eq!(sink.lock().unwrap().as_slice(), b"O");
    }

    /// The head register map the console path uses must be offset-identical to
    /// the C3/S3 twin's, which is the entire reason this family reuses it.
    /// UART0's CLKDIV is a plain round-trip; STATUS exposes live TX occupancy.
    #[test]
    fn head_register_map_matches_the_shared_twin() {
        let mut u = new(false, UART0_INTR_SOURCE_ID);
        u.write_u32(0x14, 694).unwrap();
        assert_eq!(u.read_u32(0x14).unwrap() & 0xFFF, 694, "CLKDIV round-trips");
        for _ in 0..3 {
            u.write_u32(0x00, b'x' as u32).unwrap();
        }
        assert_eq!(
            (u.read_u32(0x1C).unwrap() >> 16) & 0x3FF,
            3,
            "STATUS.TXFIFO_CNT tracks occupancy"
        );
    }


    #[test]
    fn fsm_status_idle_at_0x70_for_rom_tx_wait() {
        // C6 UART_FSM_STATUS_REG @ 0x70; ST_UTX_OUT[7:4] must read 0 or the
        // mask-ROM uart_tx_wait_idle spins forever (C3 POSPULSE was 0xFFF here).
        let u = new(false, UART0_INTR_SOURCE_ID);
        assert_eq!(u.read_u32(0x70).unwrap() & 0xF0, 0);
    }
}
