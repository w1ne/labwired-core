// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The classic-ESP32 boot ROM's console routines — as REAL CODE.
//!
//! This module exists to *remove* thunks, not to add any. It replaces five
//! nop'd ROM stubs with Espressif's actual instructions, loaded from the boot
//! ROM and executed by the CPU.
//!
//! ## What was broken
//!
//! `uart_tx_one_char` (ROM 0x4000_9200) is the character sink for every
//! firmware that prints without Arduino's `HardwareSerial`: bare-metal Rust via
//! `esp-println`, which transmutes exactly that address and calls it once per
//! byte, plus IDF early-boot logging and panic handlers. It was registered as
//! `nop_return_zero`, so the byte was **discarded** — such a firmware printed to
//! nothing at all and no serial assertion could ever see its output.
//!
//! Worse, four sibling entries carried real ROM symbol names at addresses the
//! ROM does not use, so they were dead registrations that merely looked like
//! coverage. The mistake was undetectable from outside: `RomThunkBank`
//! pre-fills its whole range with `BREAK 1, 14` and falls back to
//! `nop_return_zero`, so a name at a dead address and a correctly-addressed nop
//! behave identically — both silently eat the output.
//!
//! `ets_printf` (0x4000_7d54) was a third variant of the same problem: a Rust
//! reimplementation that formatted the string and wrote it to `tracing::info!`.
//! Output appeared on the host's stderr but never entered the UART, so it could
//! not be captured, asserted on, or timed.
//!
//! ## What runs now
//!
//! The real ROM code, fetched and executed by the CPU:
//!
//! ```text
//! ets_printf → ets_write_char → (installed putc1) → ets_write_char_uart
//!                                                 → uart_tx_one_char → UART0 FIFO
//! ```
//!
//! Real windowed ABI (`entry` / `retw.n`), real `l32r` literal loads, real
//! `callx8` dispatch through the installed putc pointer, real base computation
//! from the ROM's own UART descriptor, real spin on `STATUS.TXFIFO_CNT`, real
//! store to the FIFO — into the same `Esp32Uart` model the Arduino path uses.
//! Nothing between the firmware and the peripheral.
//!
//! Writing these by hand would also have got details wrong. The IDF's C
//! reference for `uart_tx_one_char` spins on `TXFIFO_CNT >= 126`; the silicon
//! ROM tests `STATUS & 0x0080_0000` (count >= 128). And `ets_write_char_uart`
//! expands `\n` to `\r\n`, which a reimplementation would likely miss. Real
//! code cannot drift from real behaviour.
//!
//! ## Why only these routines
//!
//! The whole BROM is not loaded. Most of it drives hardware this simulator does
//! not model — flash cache, PLL/efuse bring-up, the SPI download loader — and
//! executing that for real would hang rather than help; that is what the
//! remaining thunks in `esp32.rs` are for. The routines here are different:
//! they touch only UART STATUS/FIFO plus their own literals, all faithfully
//! modelled, so the real code runs correctly and the thunks can go.
//!
//! `_cvt` (integer formatting) calls `__udivdi3` / `__umoddi3`, which keep
//! their existing thunks. Those compute real quotients rather than faking a
//! result, so `%d` and `%x` format correctly through them.

use crate::bus::SystemBus;
use crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkBank;
use crate::Bus;

/// Espressif ESP32 boot-ROM slices, concatenated in `ROM_SPANS` order.
///
/// Extracted from the boot ROM rather than written here, and
/// `esp32_rom_console_slices_match_the_real_brom` re-derives every byte from
/// `tests/fixtures/esp32_brom.elf`, so this cannot quietly decay into "roughly
/// what the ROM does".
const BROM_CONSOLE_BLOB: &[u8] = include_bytes!("../../../roms/esp32/brom_console.bin");

/// Address and length of each slice inside [`BROM_CONSOLE_BLOB`].
///
/// | span | contents |
/// |------|----------|
/// | `0x4000_7c50` | `ets_write_char`, `_cvt`, `ets_write_char_uart`, `ets_install_putc1` / `ets_install_uart_printf` / `ets_install_putc2`, `ets_printf`, and their literal pools |
/// | `0x4000_9200` | `uart_tx_one_char`, `uart_tx_one_char2`, `uart_tx_flush` |
/// | 4-byte spans | `l32r` literals the UART routines load, which live in *neighbouring* functions' pools rather than beside the code |
/// | `0x3ff9_c305`, `0x3ff9_ee27` | ROM rodata `ets_printf` reads: the `0123456789abcdef` / `...ABCDEF` digit tables and `<null>`. Without these every formatted number would come out as NUL bytes |
const ROM_SPANS: [(u32, usize); 8] = [
    (0x4000_7c50, 0x0510),
    (0x4000_9200, 0x0078),
    (0x4000_8720, 0x0004),
    (0x4000_8f44, 0x0004),
    (0x4000_3514, 0x0004),
    (0x4000_0638, 0x0004),
    (0x3ff9_c305, 0x0022),
    (0x3ff9_ee27, 0x0012),
];

/// Entry point of `uart_tx_one_char`, the per-character ROM console sink.
pub const ROM_UART_TX_ONE_CHAR: u32 = 0x4000_9200;
/// Entry point of `ets_write_char_uart`, the ROM's default `putc1`.
pub const ROM_ETS_WRITE_CHAR_UART: u32 = 0x4000_7cf8;
/// `putc1`: the function pointer `ets_write_char` dispatches through.
/// `ets_printf` returns early when both putc slots are null.
pub const ROM_PUTC1_GLOBAL: u64 = 0x3ffa_e014;

/// Spans that belong in the ROM thunk bank (everything below 0x4000_0000 is
/// data, seeded into the mapped `brom_data` window instead).
fn is_rom_bank_span(addr: u32) -> bool {
    addr >= 0x4000_0000
}

/// Iterate `(addr, bytes)` for each slice in the blob.
fn rom_slices() -> impl Iterator<Item = (u32, &'static [u8])> {
    let mut offset = 0usize;
    ROM_SPANS.into_iter().map(move |(addr, len)| {
        let slice = &BROM_CONSOLE_BLOB[offset..offset + len];
        offset += len;
        (addr, slice)
    })
}

/// Load the real ROM console code into `bank`.
///
/// Must run AFTER the thunk registrations in `configure_xtensa_esp32`:
/// `RomThunkBank::register` writes `BREAK 1, 14` over its address, and the CPU
/// only dispatches a thunk when it *executes* that BREAK. Overwriting those
/// bytes with real instructions is exactly what takes the console off the thunk
/// mechanism.
pub fn install_rom_console(bank: &mut RomThunkBank) {
    for (addr, bytes) in rom_slices() {
        if is_rom_bank_span(addr) {
            bank.preload_bytes(addr, bytes);
        }
    }
}

/// Seed the ROM's console state into mapped memory, the way the boot ROM does
/// for itself before handing control to an application.
///
/// Two things, both of which are hardware state rather than behaviour:
///
///  * the rodata slices (digit tables, `<null>`) into the `brom_data` window,
///    which is otherwise an empty RAM region reading as zeros;
///  * `putc1` = `ets_write_char_uart`, which real silicon installs during boot
///    via `ets_install_uart_printf`. We skip the ROM's boot path, so without
///    this `ets_printf` finds both putc slots null and returns having printed
///    nothing — the routine would be present and correct and still mute.
///
/// Call after the peripherals are registered, since it writes through the bus.
pub fn seed_rom_console_state(bus: &mut SystemBus) {
    for (addr, bytes) in rom_slices() {
        if is_rom_bank_span(addr) {
            continue;
        }
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(addr as u64 + i as u64, *b);
        }
    }
    let _ = bus.write_u32(ROM_PUTC1_GLOBAL, ROM_ETS_WRITE_CHAR_UART);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Read `len` bytes at virtual address `addr` out of an Xtensa ELF.
    fn read_elf_vaddr(elf: &[u8], addr: u32, len: usize) -> Option<Vec<u8>> {
        let phoff = u32::from_le_bytes(elf[0x1c..0x20].try_into().ok()?) as usize;
        let phentsize = u16::from_le_bytes(elf[0x2a..0x2c].try_into().ok()?) as usize;
        let phnum = u16::from_le_bytes(elf[0x2c..0x2e].try_into().ok()?) as usize;
        for i in 0..phnum {
            let o = phoff + i * phentsize;
            let typ = u32::from_le_bytes(elf[o..o + 4].try_into().ok()?);
            let off = u32::from_le_bytes(elf[o + 4..o + 8].try_into().ok()?) as usize;
            let vaddr = u32::from_le_bytes(elf[o + 8..o + 12].try_into().ok()?);
            let filesz = u32::from_le_bytes(elf[o + 16..o + 20].try_into().ok()?) as usize;
            if typ != 1 || filesz == 0 || addr < vaddr {
                continue;
            }
            let delta = (addr - vaddr) as usize;
            if delta + len <= filesz {
                return Some(elf[off + delta..off + delta + len].to_vec());
            }
        }
        None
    }

    // Every embedded byte must be Espressif's, verbatim.
    //
    // Without this the blob is 1484 opaque bytes that nobody can review, and a
    // well-meaning edit could turn the console back into a hand-rolled
    // approximation — the thunk-shaped thing this module replaced. Re-deriving
    // from the checked-in boot ROM keeps "it is the real ROM" a checked claim.
    #[test]
    fn esp32_rom_console_slices_match_the_real_brom() {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/esp32_brom.elf");
        let elf = std::fs::read(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));

        let total: usize = ROM_SPANS.iter().map(|(_, len)| len).sum();
        assert_eq!(
            total,
            BROM_CONSOLE_BLOB.len(),
            "ROM_SPANS lengths must account for the whole blob"
        );
        for (addr, bytes) in rom_slices() {
            let real = read_elf_vaddr(&elf, addr, bytes.len())
                .unwrap_or_else(|| panic!("BROM does not cover 0x{addr:08x}+{}", bytes.len()));
            assert_eq!(
                real,
                bytes.to_vec(),
                "slice at 0x{addr:08x} has drifted from the real ESP32 boot ROM"
            );
        }
    }
}
