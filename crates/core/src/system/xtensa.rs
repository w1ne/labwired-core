// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 / ESP32-S3 system glue.
//!
//! `configure_xtensa_esp32s3` registers all peripherals defined for the
//! ESP32-S3-Zero and returns a fresh `XtensaLx7` CPU.  After calling this,
//! the caller invokes `boot::esp32s3::fast_boot` to load an ELF and
//! synthesise CPU state, then enters the simulation loop.

use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::peripherals::esp32s3::flash_xip::FlashXipPeripheral;
use crate::peripherals::esp32s3::rom_thunks::{self, RomThunkBank};
use crate::peripherals::esp32s3::system_stub::{EfuseStub, RtcCntlStub, SystemStub};
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use crate::Cpu;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Esp32s3Opts {
    pub iram_size: u32,
    pub dram_size: u32,
    pub flash_size: u32,
    pub cpu_clock_hz: u32,
}

impl Default for Esp32s3Opts {
    fn default() -> Self {
        Self {
            iram_size: 512 * 1024,
            dram_size: 480 * 1024,
            flash_size: 4 * 1024 * 1024,
            cpu_clock_hz: 80_000_000,
        }
    }
}

/// Result of `configure_xtensa_esp32s3` — exposes the shared flash backing
/// so the boot path can write to it (Task 8).
pub struct Esp32s3Wiring {
    pub cpu: XtensaLx7,
    pub flash_backing: Arc<Mutex<Vec<u8>>>,
}

/// Register all ESP32-S3 peripherals on `bus` and return the CPU + the
/// shared flash backing buffer.
pub fn configure_xtensa_esp32s3(bus: &mut SystemBus, opts: &Esp32s3Opts) -> Esp32s3Wiring {
    // SystemBus::new() seeds the bus with STM32 default peripherals
    // (tim2 at 0x4000_0000, tim3 at 0x4000_0400, …). On ESP32-S3 the
    // 0x4000_0000–0x4006_0000 window is the BROM, and on STM32 it's the
    // peripheral aliased region — completely different memory maps. Drop
    // the seeded peripherals before installing the ESP32-S3 bank, otherwise
    // a tim3 read at 0x4000_057c shadows our `rtc_get_reset_reason` thunk
    // and the BREAK 1,14 dispatch never fires.
    bus.peripherals.clear();
    // The seeded `flash` and `ram` LinearMemory slabs use STM32 base
    // addresses (0x0 and 0x2000_0000) so they don't overlap, but they're
    // dead weight on Xtensa — leave them allocated; the bus accessors check
    // `addr >= base_addr` first and fall through to peripherals on miss.

    // ── IRAM (instruction fetch view) ─────────────────────────────────────
    bus.add_peripheral(
        "iram",
        0x4037_0000,
        opts.iram_size as u64,
        None,
        Box::new(RamPeripheral::new(opts.iram_size as usize)),
    );

    // ── DRAM (data view of the same physical SRAM0) ───────────────────────
    bus.add_peripheral(
        "dram",
        0x3FC8_8000,
        opts.dram_size as u64,
        None,
        Box::new(RamPeripheral::new(opts.dram_size as usize)),
    );

    // ── Flash-XIP backing, shared between I-cache and D-cache aliases ─────
    let flash_backing = Arc::new(Mutex::new(vec![0u8; opts.flash_size as usize]));
    let mut icache = FlashXipPeripheral::new_shared(flash_backing.clone(), 0x4200_0000);
    let mut dcache = FlashXipPeripheral::new_shared(flash_backing.clone(), 0x3C00_0000);
    icache.map_identity();
    dcache.map_identity();
    bus.add_peripheral(
        "flash_icache",
        0x4200_0000,
        opts.flash_size as u64,
        None,
        Box::new(icache),
    );
    bus.add_peripheral(
        "flash_dcache",
        0x3C00_0000,
        opts.flash_size as u64,
        None,
        Box::new(dcache),
    );

    // ── ROM thunk bank ────────────────────────────────────────────────────
    let mut rom_bank = RomThunkBank::new(0x4000_0000, 0x6_0000);
    register_default_thunks(&mut rom_bank);
    bus.add_peripheral(
        "rom_thunks",
        0x4000_0000,
        0x6_0000,
        None,
        Box::new(rom_bank),
    );

    // ── USB_SERIAL_JTAG ───────────────────────────────────────────────────
    bus.add_peripheral(
        "usb_serial_jtag",
        0x6003_8000,
        0x1000,
        None,
        Box::new(UsbSerialJtag::new()),
    );

    // ── SYSTIMER ──────────────────────────────────────────────────────────
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x1000,
        None,
        Box::new(Systimer::new(opts.cpu_clock_hz)),
    );

    // ── SYSTEM / RTC_CNTL / EFUSE stubs ──────────────────────────────────
    bus.add_peripheral(
        "system",
        0x600C_0000,
        0x1000,
        None,
        Box::new(SystemStub::new()),
    );
    bus.add_peripheral(
        "rtc_cntl",
        0x6000_8000,
        0x1000,
        None,
        Box::new(RtcCntlStub::new()),
    );
    bus.add_peripheral(
        "efuse",
        0x6000_7000,
        0x1000,
        None,
        Box::new(EfuseStub::new()),
    );

    let mut cpu = XtensaLx7::new();
    cpu.reset(bus).expect("xtensa reset");

    Esp32s3Wiring { cpu, flash_backing }
}

/// Register the default thunk set for esp-hal hello-world boot.
///
/// Addresses are taken from `esp-rom-sys-0.1.4/ld/esp32s3/rom/esp32s3.rom.ld`
/// (PROVIDE statements) and verified against the disassembled firmware.
fn register_default_thunks(bank: &mut RomThunkBank) {
    // Cache maintenance — esp-hal pre_init disables instruction cache before
    // touching XIP-mapped flash and re-enables it after.
    bank.register(0x4000_18b4, rom_thunks::cache_suspend_dcache);
    bank.register(0x4000_18c0, rom_thunks::cache_resume_dcache);
    // rom_config_instruction_cache_mode(cache_size, ways, line_size) — esp-hal
    // calls this in pre_init to set up the I-cache to the bootloader's chosen
    // geometry. NOP is fine because we don't model the cache.
    bank.register(0x4000_1a1c, rom_thunks::rom_config_instruction_cache_mode);
    // ets_printf — esp-hal panic / boot diagnostics call this.
    bank.register(0x4000_05d0, rom_thunks::ets_printf);
    // ets_set_appcpu_boot_addr — single-core build skips this, but multicore
    // hal calls it to point cpu1 at park-loop. NOP is safe.
    bank.register(0x4000_0720, rom_thunks::ets_set_appcpu_boot_addr);
    // esp_rom_spiflash_unlock — flash write helper. Boot path doesn't write,
    // but the symbol may be linked in.
    bank.register(0x4000_0a2c, rom_thunks::esp_rom_spiflash_unlock);
    // rtc_get_reset_reason(cpu_idx) — esp-hal queries this during init to
    // distinguish power-on from soft reset; we always report POWERON_RESET.
    bank.register(0x4000_057c, rom_thunks::rtc_get_reset_reason);
    // rom_config_data_cache_mode — analogous to instruction cache config; NOP.
    bank.register(0x4000_1a28, rom_thunks::nop_return_zero);
    // ets_update_cpu_frequency(freq_mhz) — informs the ROM of the new clock
    // so subsequent ets_delay_us calls calibrate correctly. We don't model
    // ROM timing, so accepting and discarding the value is fine.
    bank.register(0x4000_1a4c, rom_thunks::nop_return_zero);
    // ets_delay_us(us) — busy-wait the requested microseconds. The simulator
    // doesn't model wall-clock so we return immediately; real silicon would
    // spin. Side-effect-free callers (boot timing) accept this.
    bank.register(0x4000_0600, rom_thunks::nop_return_zero);
    // esp_rom_regi2c_read / rom_i2c_writeReg — analog regulator I²C bus;
    // ESP-IDF init touches this to tweak BBPLL. NOP-return-0 is acceptable
    // here because we don't model the analog domain.
    bank.register(0x4000_5d48, rom_thunks::nop_return_zero);
    bank.register(0x4000_5d60, rom_thunks::nop_return_zero);
    // memcpy and __udivdi3 do real work — emulate them so the firmware
    // doesn't get garbage from the boot-init copy paths.
    bank.register(0x4000_11f4, rom_thunks::rom_memcpy);
    bank.register(0x4000_2544, rom_thunks::rom_udivdi3);
}

// ── RamPeripheral helper (private) ───────────────────────────────────────

/// Flat-array `Peripheral` used for IRAM + DRAM mappings.
struct RamPeripheral {
    data: std::cell::RefCell<Vec<u8>>,
}

impl RamPeripheral {
    fn new(size: usize) -> Self {
        Self {
            data: std::cell::RefCell::new(vec![0u8; size]),
        }
    }
}

impl std::fmt::Debug for RamPeripheral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RamPeripheral({}B)", self.data.borrow().len())
    }
}

impl crate::Peripheral for RamPeripheral {
    fn read(&self, offset: u64) -> crate::SimResult<u8> {
        Ok(*self.data.borrow().get(offset as usize).unwrap_or(&0))
    }
    fn write(&mut self, offset: u64, value: u8) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        if let Some(slot) = d.get_mut(offset as usize) {
            *slot = value;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;

    #[test]
    fn configure_registers_all_peripherals() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        // Confirm core regions are reachable.
        assert!(bus.read_u8(0x4037_0000).is_ok(), "IRAM");
        assert!(bus.read_u8(0x3FC8_8000).is_ok(), "DRAM");
        assert!(bus.read_u8(0x4200_0000).is_ok(), "flash I-cache");
        assert!(bus.read_u8(0x3C00_0000).is_ok(), "flash D-cache");
        assert!(bus.read_u8(0x6003_8000).is_ok(), "USB_SERIAL_JTAG");
        assert!(bus.read_u8(0x6002_3000).is_ok(), "SYSTIMER");
        assert!(bus.read_u8(0x600C_0000).is_ok(), "SYSTEM");
        assert!(bus.read_u8(0x6000_8000).is_ok(), "RTC_CNTL");
        assert!(bus.read_u8(0x6000_7000).is_ok(), "EFUSE");
    }

    #[test]
    fn iram_writeable_and_readable() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        bus.write_u8(0x4037_0010, 0xAB).unwrap();
        assert_eq!(bus.read_u8(0x4037_0010).unwrap(), 0xAB);
    }

    #[test]
    fn flash_xip_aliases_share_backing() {
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        // Write directly into the flash backing (mimics fast-boot doing so).
        wiring.flash_backing.lock().unwrap()[0] = 0xCA;
        wiring.flash_backing.lock().unwrap()[1] = 0xFE;
        // Both aliases must reflect it.
        assert_eq!(bus.read_u8(0x4200_0000).unwrap(), 0xCA);
        assert_eq!(bus.read_u8(0x3C00_0000).unwrap(), 0xCA);
        assert_eq!(bus.read_u8(0x4200_0001).unwrap(), 0xFE);
    }
}
