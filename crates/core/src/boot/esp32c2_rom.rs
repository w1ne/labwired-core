// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C2 mask-ROM `.data` unpack for the bare-ELF fast boot.
//!
//! On silicon the C2 boot ROM copies its initialised `.data` from mask ROM to
//! the top of DRAM before it runs anything else: `_rom_store_table` lists one
//! 16-byte record `{dst_start, dst_end, src, 0}` per output section. Fast boot
//! never runs the ROM, so those words stay zero. esp-hal 1.x calls ROM
//! routines that dispatch through them: `rom_i2c_readReg` (clock setup in
//! `esp_hal::init`) loads `rom_phyFuns` (0x3FCD_F978, from `.data_phyrom`)
//! and jumps through it, so with the table unrun the first call lands at
//! 0xFFFF_FFFE.
//!
//! This module replays that loop from the ROM bytes already mapped on the
//! bus (the `rom` region of `configs/chips/esp32c2.yaml`). It copies nothing
//! from a table of its own; when the ROM window is zero (no image) every
//! record reads as empty and nothing is written.

use labwired_config::ChipDescriptor;

use crate::bus::SystemBus;

/// `_rom_store_table` / `_data_start` in `esp32c2_rev200_rom.elf`.
pub const ROM_STORE_TABLE_START: u64 = 0x4008_1EEC;
/// `_data_end` in the same ELF (exclusive): 43 records.
pub const ROM_STORE_TABLE_END: u64 = 0x4008_219C;
/// Bytes per `_rom_store_table` record.
const RECORD_LEN: u64 = 16;
/// The ROM `.data` destinations all sit in DRAM below `SOC_DRAM_HIGH`.
const DRAM: std::ops::Range<u32> = 0x3FCA_0000..0x3FCE_0000;
/// The sources all sit in the IROM mask window.
const IROM: std::ops::Range<u32> = 0x4000_0000..0x4009_0000;

/// True for the `esp32c2` descriptor.
pub fn is_esp32c2(chip: &ChipDescriptor) -> bool {
    chip.name.eq_ignore_ascii_case("esp32c2")
}

/// Replay the C2 ROM `.data` unpack. Returns the number of bytes copied.
///
/// A record whose range is empty, reversed or outside DRAM, or whose source
/// is outside the IROM window, is skipped: a zero ROM window, or a different
/// ROM revision supplied through `LABWIRED_ESP32C2_ROM`, then copies nothing
/// rather than writing through a stray pointer.
pub fn seed_rom_data(bus: &mut SystemBus) -> usize {
    use crate::Bus;
    let mut copied = 0;
    let mut record = ROM_STORE_TABLE_START;
    while record + RECORD_LEN <= ROM_STORE_TABLE_END {
        let word = |off: u64| bus.read_u32(record + off).unwrap_or(0);
        let (dst, end, src) = (word(0), word(4), word(8));
        record += RECORD_LEN;
        let len = end.wrapping_sub(dst);
        if end <= dst
            || !DRAM.contains(&dst)
            || end > DRAM.end
            || !IROM.contains(&src)
            || src + len > IROM.end
        {
            continue;
        }
        for i in 0..len {
            let byte = bus.read_u8((src + i) as u64).unwrap_or(0);
            if bus.write_u8((dst + i) as u64, byte).is_ok() {
                copied += 1;
            }
        }
    }
    copied
}

/// Route interrupts through the C2 `INTERRUPT_CORE0` matrix: asserted
/// peripheral sources and the SYSTEM `FROM_CPU_INTR_n` doorbells reach the
/// CPU lines, and `INTR_STATUS_REG_0/1` mirror the source levels. The C3
/// arms the same fabric on its ROM-boot path; the C2 has only this fast boot,
/// so it arms it here. The control offsets come from the chip's
/// `interrupt_core0.yaml` (see `bus::MatrixCtrlOffsets`).
pub fn arm_interrupt_matrix(bus: &mut SystemBus) {
    bus.irq_fabric.esp32c3.routing = true;
    bus.refresh_peripheral_index();
}

/// UART0 CLKDIV (`DR_REG_UART_BASE` 0x6000_0000 + 0x14).
const UART0_CLKDIV: u64 = 0x6000_0014;
/// The ROM console rate and the C2 UART source clock (PLL_F40M).
const ROM_CONSOLE_BAUD: u64 = 115_200;
const UART_SCLK_HZ: u64 = 40_000_000;

/// Set the UART0 console to 115200 baud, as the C2 ROM does before the app
/// runs. The divider is `(sclk << 4) / baud` = 5555: CLKDIV[11:0] = 347,
/// FRAG[23:20] = 3. Without it UART0 keeps the reset CLKDIV 0x2B6 (694),
/// which on the 40 MHz source is 57600 baud, and every console line takes
/// twice as long as on silicon.
pub fn seed_console_uart(bus: &mut SystemBus) {
    use crate::Bus;
    let div = (UART_SCLK_HZ << 4) / ROM_CONSOLE_BAUD;
    let value = ((div >> 4) & 0xFFF) | ((div & 0xF) << 20);
    let _ = bus.write_u32(UART0_CLKDIV, value as u32);
}

/// `mstatus.MIE`.
const MSTATUS_MIE: u32 = 1 << 3;

/// Give the app the CPU state the C2 ROM and 2nd-stage bootloader hand over.
///
/// * `mstatus.MIE` set. The ROM enables interrupts through `ets_intr_unlock`
///   (`csrrsi mstatus, 8` at 0x4005_B0AE in `esp32c2_rev200_rom.elf`), and
///   esp-hal 1.2 never sets MIE from zero: its critical sections only restore
///   the previous state. With MIE clear no `#[handler]` ever runs.
/// * `mtimecmp` parked. The C2, like the C3, has no CLINT timer; line 7 is a
///   matrix line, so the model's default `mtimecmp = 0` must not pend MTIP.
pub fn prepare_fast_boot_cpu(cpu: &mut crate::cpu::RiscV) {
    cpu.mstatus |= MSTATUS_MIE;
    cpu.mtimecmp = u64::MAX;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use labwired_config::SystemManifest;

    fn c2_bus() -> SystemBus {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip = ChipDescriptor::from_file(root.join("configs/chips/esp32c2.yaml"))
            .expect("load esp32c2.yaml");
        SystemBus::from_config(&chip, &SystemManifest::default()).expect("C2 bus")
    }

    #[test]
    fn unpack_fills_rom_phy_funcs_from_the_vendored_rom() {
        let mut bus = c2_bus();
        assert_eq!(bus.read_u32(0x3FCD_F978).unwrap(), 0, "DRAM starts zero");
        // 19 non-empty records in the rev 2 ROM table, 0x4C5 bytes in all
        // (summed from `_rom_store_table` in the vendored image).
        assert_eq!(seed_rom_data(&mut bus), 0x4C5);
        // rom_phyFuns: the last word of `.data_phyrom` (LMA 0x4008_2628,
        // VMA 0x3FCD_F788, 0x1F4 bytes) — the ROM's own pointer, read back
        // from the source so the test does not restate a magic value.
        let src = bus.read_u32(0x4008_2628 + 0x1F0).unwrap();
        assert_ne!(src, 0);
        assert_eq!(bus.read_u32(0x3FCD_F978).unwrap(), src);
    }

    #[test]
    fn console_uart_is_seeded_to_115200_on_the_40mhz_source() {
        let mut bus = c2_bus();
        assert_eq!(bus.read_u32(UART0_CLKDIV).unwrap() & 0xFFF, 0x2B6, "reset");
        seed_console_uart(&mut bus);
        // 40 MHz / (347 + 3/16) = 115_207 baud.
        assert_eq!(bus.read_u32(UART0_CLKDIV).unwrap(), 347 | (3 << 20));
    }

    #[test]
    fn drom_window_mirrors_irom() {
        // `.rodata` VMA 0x3FF4_2C00 is LMA 0x4008_2C00 in the ROM ELF.
        let bus = c2_bus();
        let irom = bus.read_u32(0x4008_2C00).unwrap();
        assert_ne!(irom, 0);
        assert_eq!(bus.read_u32(0x3FF4_2C00).unwrap(), irom);
    }
}
