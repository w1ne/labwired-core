// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-P4 UART controller (UART0/UART1/UART2).
//!
//! The P4 carries the same Espressif UART IP revision family as the C6/S3: the
//! head register map a console path drives is offset-identical
//! (`esp32p4.svd` UART0: FIFO@0x00, INT_RAW@0x04, INT_ST@0x08, INT_ENA@0x0C,
//! INT_CLR@0x10, CLKDIV@0x14, RX_FILT@0x18, STATUS@0x1C, CONF0@0x20,
//! CONF1@0x24). The SVD spells some of those `CLKDIV_SYNC` / `CONF0_SYNC`; the
//! `_SYNC` suffix is a naming detail and does not move any offset. The P4 keeps
//! the 128-entry `SOC_UART_FIFO_LEN` FIFOs and the `uart_ll.h` interrupt bit
//! positions, so the model itself stays chip-neutral in
//! [`crate::peripherals::esp_uart::EspUart`] and this module only supplies the
//! P4's per-family values.
//!
//! The P4 also has `FSM_STATUS` at 0x70 (esp32p4.svd `UART0.FSM_STATUS`), so it
//! must be built with the C6 register map: the C3 `POSPULSE` reset value 0xFFF
//! at the same offset wedges a `uart_tx_wait_idle` poll on `ST_UTX_OUT == 0`.
//!
//! ## What differs on the P4 (NOT modelled)
//!
//! There is no P4 `clock:` gate on the UARTs: the P4 clock/reset block
//! (`HP_SYS_CLKRST`) has a different register layout from the C6 `PCR` and its
//! tree is not modelled. The UART tail registers beyond `CONF1` are the shared
//! model's C3/S3 offsets, so a P4 driver that reads a tail register past 0x24
//! gets the shared model's answer, not a P4-specific one — listed as a
//! limitation on the board page.

use crate::peripherals::esp_uart::EspUart;

/// Default interrupt-matrix source when a descriptor does not name one with
/// `irq:`. The P4 interrupt matrix assigns UART0/1/2/3/4 sources 32..=36
/// (`esp32p4.svd` INTERRUPT_CORE0 source ids); UART0 is the console. The value
/// is documentation at L1 because the P4 matrix is not routed through the bus,
/// but it is kept honest for the debugger.
pub const UART0_INTR_SOURCE_ID: u32 = 32;
/// See [`UART0_INTR_SOURCE_ID`]; UART1 = 33 on the P4.
pub const UART1_INTR_SOURCE_ID: u32 = 33;
/// See [`UART0_INTR_SOURCE_ID`]; UART2 = 34 on the P4.
pub const UART2_INTR_SOURCE_ID: u32 = 34;

/// ESP32-P4 HP-core clock (ESP32-P4 datasheet v0.5 §4.1: CPU_CLK up to
/// 400 MHz; the default is 360 MHz). The UART twin scales its baud pacing
/// (`10 * clkdiv` UART-clock cycles) into CPU ticks, so this must be the P4's
/// configured rate.
pub const CPU_CLOCK_HZ: u64 = 360_000_000;

/// The default interrupt-matrix source when a chip descriptor does not name one
/// with `irq:`. Selected by the descriptor's base address (esp32p4.svd):
/// UART1 @ 0x500C_B000 -> 33, UART2 @ 0x500C_C000 -> 34, anything else
/// (i.e. UART0 @ 0x500C_A000) -> 32.
///
/// Deliberately takes the base as a parameter instead of comparing a local
/// `UART1_BASE` constant: the base is the descriptor's fact.
pub fn default_source_id(base: u64) -> u32 {
    match base {
        0x500C_B000 => UART1_INTR_SOURCE_ID,
        0x500C_C000 => UART2_INTR_SOURCE_ID,
        _ => UART0_INTR_SOURCE_ID,
    }
}

/// Build a P4 UART instance: the shared Espressif twin, paced at 360 MHz and
/// built with the C6 register map (FSM_STATUS at 0x70, not the C3 POSPULSE).
/// `echo_stdout` routes shifted-out TX to the host console (UART0's default,
/// the module's console path); UART1/UART2 stay capture-only.
pub fn new(echo_stdout: bool, source_id: u32) -> EspUart {
    EspUart::new_with_cpu_clock(echo_stdout, source_id, CPU_CLOCK_HZ).with_c6_reg_map()
}
