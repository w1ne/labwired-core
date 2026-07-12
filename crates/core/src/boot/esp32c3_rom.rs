// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 boot-ROM provisioning for the faithful `--rom-boot` path.
//!
//! The two flat images the model loads (see `configs/chips/esp32c3.yaml`
//! memory regions `rom` / `rom_data`):
//!   * IROM (code + `.data` copy sources) 0x4000_0000..0x4006_0000 (384 KiB)
//!   * DROM (ROM constant data)           0x3FF0_0000..0x3FF2_0000 (128 KiB)
//!
//! Resolution order mirrors [`super::esp32s3_rom`]: explicit env pins
//! (`LABWIRED_ESP32C3_ROM` / `LABWIRED_ESP32C3_ROM_DATA`), then the ROM ELF
//! shipped with the user's installed toolchain (PlatformIO/ESP-IDF,
//! `esp32c3_rev3_rom.elf`), then the vendored images under
//! `crates/core/roms/esp32c3/` (embedded at build time, non-wasm only) so
//! `--rom-boot` works out of the box with no toolchain installed.
//!
//! Extraction reuses the S3 module's window builder: PT_LOAD placement, the
//! boot ROM's `.data` copy-source reconstruction against the C3 DRAM window
//! (the startup `unpackloop` at 0x40001ef8 walks 16-byte
//! {dst_start, dst_end, src, 0} records at 0x40059200), and a PROGBITS
//! overlay for sections in no PT_LOAD segment.

use super::esp32s3_rom::{build_window, DramWindow, RomImages};
use goblin::elf::Elf;
use std::path::PathBuf;

pub const IROM_BASE: u32 = 0x4000_0000;
pub const IROM_SIZE: usize = 0x6_0000; // 384 KiB
pub const DROM_BASE: u32 = 0x3FF0_0000;
pub const DROM_SIZE: usize = 0x2_0000; // 128 KiB

/// C3 DRAM: the ROM's `.data` runtime window. `.data` ends exactly at the
/// DRAM top (`ets_ops_table_ptr` @ 0x3FCD_FFFC..0x3FCE_0000).
const DRAM: DramWindow = DramWindow {
    lo: 0x3FC8_0000,
    hi: 0x3FCE_0000,
};

/// Replicate the ESP32-C3 boot ROM's reset-time `.data` initialization for the
/// fast-boot path, the RISC-V analogue of [`super::esp32s3_rom::s3_rom_data_init_writes`].
///
/// Fast-boot jumps straight into the app and skips the ROM's own startup
/// `unpackloop` (0x40001ef8), so the ROM's DRAM globals — e.g. the ROM function
/// tables esp-hal's init dispatches through (`ets_ops_table_ptr` @0x3FCDFFFC) —
/// would stay zero and a ROM call jumps through a null/garbage pointer. Walking
/// the copy table here lands the genuine values exactly as the real ROM reset
/// does (zero thunks); idempotent with `--rom-boot`, which runs the copy itself.
/// The copy-table format and IROM base match the S3, so this reuses the S3
/// walker with the C3 DRAM window.
pub fn c3_rom_data_init_writes(irom: &[u8]) -> Vec<(u32, Vec<u8>)> {
    super::esp32s3_rom::rom_data_init_writes(irom, IROM_BASE, DRAM)
}

/// Extract the IROM and DROM flat images from the genuine C3 ROM ELF bytes.
pub fn extract_rom_images(elf_bytes: &[u8]) -> Result<RomImages, String> {
    let elf = Elf::parse(elf_bytes).map_err(|e| format!("parse ROM ELF: {e}"))?;
    let irom = build_window(&elf, elf_bytes, IROM_BASE, IROM_SIZE, true, DRAM);
    let drom = build_window(&elf, elf_bytes, DROM_BASE, DROM_SIZE, false, DRAM);
    Ok(RomImages { irom, drom })
}

/// Resolve the C3 ROM images for the faithful path, or `None` when nothing
/// resolves (native builds always resolve via the vendored images).
///
/// Order:
///   1. Explicit pre-extracted `LABWIRED_ESP32C3_ROM`/`_ROM_DATA` bins —
///      the same env pins `from_config` honors for the chip's rom regions.
///   2. Discover the toolchain ROM ELF, extract (cached by content hash).
///   3. Vendored images embedded at build time (non-wasm only).
pub fn provision_rom_images() -> Option<RomImages> {
    if let (Ok(rp), Ok(dp)) = (
        std::env::var("LABWIRED_ESP32C3_ROM"),
        std::env::var("LABWIRED_ESP32C3_ROM_DATA"),
    ) {
        if let (Ok(irom), Ok(drom)) = (std::fs::read(&rp), std::fs::read(&dp)) {
            return Some(RomImages { irom, drom });
        }
    }

    if let Some(elf_path) = discover_rom_elf() {
        if let Ok(elf_bytes) = std::fs::read(&elf_path) {
            let key = fnv1a_64(&elf_bytes);
            let dir = cache_dir();
            let irom_path = dir.join(format!("esp32c3_irom_{key:016x}.bin"));
            let drom_path = dir.join(format!("esp32c3_drom_{key:016x}.bin"));

            if let (Ok(irom), Ok(drom)) = (std::fs::read(&irom_path), std::fs::read(&drom_path)) {
                if irom.len() == IROM_SIZE && drom.len() == DROM_SIZE {
                    return Some(RomImages { irom, drom });
                }
            }

            if let Ok(images) = extract_rom_images(&elf_bytes) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&irom_path, &images.irom);
                let _ = std::fs::write(&drom_path, &images.drom);
                return Some(images);
            }
        }
    }

    vendored_rom_images()
}

#[cfg(not(target_arch = "wasm32"))]
fn vendored_rom_images() -> Option<RomImages> {
    static VENDORED_IROM: &[u8] = include_bytes!("../../roms/esp32c3/esp32c3_rom.bin");
    static VENDORED_DROM: &[u8] = include_bytes!("../../roms/esp32c3/esp32c3_drom.bin");
    debug_assert_eq!(VENDORED_IROM.len(), IROM_SIZE);
    debug_assert_eq!(VENDORED_DROM.len(), DROM_SIZE);
    Some(RomImages {
        irom: VENDORED_IROM.to_vec(),
        drom: VENDORED_DROM.to_vec(),
    })
}

#[cfg(target_arch = "wasm32")]
fn vendored_rom_images() -> Option<RomImages> {
    None
}

/// Cache directory for extracted ROM images (`$XDG_CACHE_HOME` or `~/.cache`).
fn cache_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(x).join("labwired/esp32c3-rom");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache/labwired/esp32c3-rom");
    }
    std::env::temp_dir().join("labwired-esp32c3-rom")
}

/// FNV-1a 64-bit hash (no extra deps; stable cache key across runs).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Locate the genuine ESP32-C3 ROM ELF in the user's installed toolchain.
///
/// Preference order:
///   1. `LABWIRED_ESP32C3_ROM_ELF` env var (explicit path).
///   2. PlatformIO (`packages/` and `tools/` layouts): `esp32c3_rev3_rom.elf`.
///   3. ESP-IDF: `~/.espressif/tools/esp-rom-elfs/<ver>/esp32c3_rev3_rom.elf`.
pub(crate) fn discover_rom_elf() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LABWIRED_ESP32C3_ROM_ELF") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;

    for layout in ["packages", "tools"] {
        let pio = PathBuf::from(format!(
            "{home}/.platformio/{layout}/tool-esp-rom-elfs/esp32c3_rev3_rom.elf"
        ));
        if pio.is_file() {
            return Some(pio);
        }
    }

    // ESP-IDF nests the elfs under a version directory; scan one level.
    let idf_root = PathBuf::from(format!("{home}/.espressif/tools/esp-rom-elfs"));
    if let Ok(entries) = std::fs::read_dir(&idf_root) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("esp32c3_rev3_rom.elf");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Options for [`build_rom_boot_machine`].
#[derive(Default)]
pub struct RomBootOpts {
    /// Program a distinct factory MAC into the eFuse MAC words so multiple
    /// instances are distinguishable on the shared VirtualWifi air. `None`
    /// leaves the seeded defaults.
    pub efuse_mac: Option<[u8; 6]>,
    /// If set, USB-Serial-JTAG console bytes are mirrored into this sink (the
    /// browser widget's Serial tab). Native leaves it `None` — the same
    /// console bytes already reach stdout via UART0, and a second echo here
    /// would double every character.
    pub usb_serial_sink: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>,
}

/// Assemble the faithful ESP32-C3 ROM-boot machine from a prepared bus (whose
/// `rom`/`rom_data` regions already hold the genuine boot ROM images) and a
/// flash-image byte vector (`bootloader@0x0 + partition-table@0x8000 +
/// app@0x10000`).
///
/// This is the shared core of the native CLI `--rom-boot` path and the wasm
/// browser rom-boot path: it wires SPIMEM0/1 flash controllers, the flash-cache
/// MMU + two FlashXip windows, the EXTMEM cache, the USB-Serial-JTAG console,
/// and the SHA/RNG/SARADC/SYSTIMER/RTC/ana-I²C models the real mask ROM and
/// 2nd-stage bootloader drive — then resets to the BROM vector `0x4000_0000`
/// so the genuine ROM runs from reset through `app_main()`. Zero thunks.
///
/// Callers own image provisioning: the native CLI resolves the ROM (env pins /
/// toolchain ELF / vendored) and reads the flash file; the browser fetches the
/// two ROM bins and the merged flash image and passes them in. This function is
/// pure (no env / filesystem access), so it compiles and runs on wasm.
///
/// `wrap` bridges the CPU type the two callers need from the same wiring: the
/// native CLI keeps the concrete `RiscV` (`|c| c`), while the wasm
/// `WasmSimulator` needs a `Machine<Box<dyn Cpu>>` (`|c| Box::new(c)`). The
/// `RiscV`-specific setup (reset vector, `mtimecmp`) runs before `wrap`, so the
/// boxed and unboxed machines are byte-for-byte identical.
pub fn build_rom_boot_machine<C: crate::Cpu, F: FnOnce(crate::cpu::RiscV) -> C>(
    mut bus: crate::bus::SystemBus,
    flash_bytes: Vec<u8>,
    opts: RomBootOpts,
    wrap: F,
) -> crate::Machine<C> {
    use crate::Cpu as _;
    use std::sync::{Arc, Mutex};

    let backing = Arc::new(Mutex::new(flash_bytes));
    // Route reads through peripherals first: the fast path checks the
    // chip's `flash`/`drom` memory-regions (zero-filled in rom-boot) before
    // peripherals, which would shadow the FlashXip windows we install at the
    // same XIP addresses. Disabling it lets the MMU-translating FlashXip
    // serve 0x4200_0000 / 0x3C00_0000 from the real flash image.
    bus.config.optimized_bus_access = false;
    // SPIMEM1 flash-command controller (0x6000_2000) backed by the real
    // image, overriding the declarative stub — a narrower, later-registered
    // window wins, so the BROM's READ/RDID/RDSR commands return real bytes.
    // The C3's SPI1 shares the S3's SPIMEM register layout, so the S3 model
    // drops in unchanged.
    bus.add_peripheral(
        "spimem1_flash",
        0x6000_2000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(backing.clone())),
    );
    // SPIMEM0 (0x6000_3000) — the cache's auto-fetch MSPI controller. Back
    // it with the same flash image too, in case the BROM's bootloader load
    // path issues commands here rather than on SPIMEM1.
    bus.add_peripheral(
        "spimem0_flash",
        0x6000_3000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(backing.clone())),
    );
    // Flash cache MMU: the 2nd-stage bootloader programs the virtual→flash
    // page table at 0x600C_5000, then runs the app from the XIP windows
    // (IROM 0x4200_0000, DROM 0x3C00_0000). Model the real MMU table shared
    // with two FlashXip windows that translate through it (C3 entry format:
    // invalid=BIT(8), 0xFF page field, 8 MiB span) over the flash image —
    // so the app executes from flash exactly like silicon.
    use crate::peripherals::esp32s3::flash_xip::{
        new_mmu_table, Esp32s3MmuTable, FlashXipPeripheral, MMU_FMT_C3,
    };
    let mmu_table = new_mmu_table();
    bus.add_peripheral(
        "mmu_table",
        0x600C_5000,
        0x800,
        None,
        Box::new(Esp32s3MmuTable::new(mmu_table.clone())),
    );
    // EXTMEM cache controller (0x600C_4000): auto-completes the cache
    // invalidate/sync launch→done handshake the ROM busy-polls (offset 0x28,
    // launch bit0 / done bit1). Overrides the declarative stub, which never
    // asserts done and spins Cache_Invalidate_ICache_Items forever.
    bus.add_peripheral(
        "extmem_cache",
        0x600C_4000,
        0x400,
        None,
        Box::new(crate::peripherals::esp32c3::cache::Esp32c3Cache::new()),
    );
    // Analog I²C master / ANA_CONFIG block (0x6000_E000, DR_REG_I2C_ANA_MST_BASE):
    // rom_i2c_writeReg drives it (read-modify-write of ANA_CONFIG regs) during
    // PHY/clock bring-up; the libphy full RF calibration also touches regs up
    // past 0x6000_E130, so the window spans 0x400. The model reports the
    // master FSM status (0x50 bits[26:24]=7, idle/done) so the ROM's
    // transaction busy-poll exits; all other regs are register-backed.
    bus.add_peripheral(
        "rtc_i2c_ana",
        0x6000_E000,
        0x400,
        None,
        Box::new(crate::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
    );
    // USB-Serial-JTAG (0x6004_3000): the BROM's console prints to BOTH UART0
    // and the USB CDC port — `usb_uart_tx_one_char` busy-polls EP1_CONF
    // (offset 0x04) for SERIAL_IN_EP_DATA_FREE. The declarative usb_device
    // stub reads 0 there, wedging boot_prepare's very first ets_printf line
    // (observed: banner, then a uart/usb tx ping-pong, then ret-to-0). The C3
    // block is the same IP as the S3's, so the S3 behavioral model (EP1_CONF
    // always WR_DONE|DATA_FREE, bytes appended/echoed) drops in unchanged.
    // Native leaves the sink `None` (the same console bytes reach stdout via
    // UART0; a second echo doubles every character); the browser widget passes
    // a sink so its Serial tab shows esp-hal / jtag-serial output.
    let mut usb_serial = crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag::new();
    usb_serial.set_sink(opts.usb_serial_sink.clone(), false);
    bus.add_peripheral(
        "usb_serial_jtag",
        0x6004_3000,
        0x100,
        None,
        Box::new(usb_serial),
    );
    // FE/PHY register block (0x6001_1000): libphy's set_rx_gain_table also
    // writes gain/FE config into the gap between uart1 (0x6001_0000) and
    // i2c0 (0x6001_3000). Register-backed storage for those RF tables.
    bus.add_peripheral(
        "wifi_fe",
        0x6001_1000,
        0x2000,
        None,
        Box::new(crate::peripherals::esp32c3::reg_block::Esp32c3RegBlock::new(0x2000)),
    );
    // Baseband/RF register block (0x6001_C000): libphy writes the RX gain
    // table and other BB/RF config here (set_rx_gain_table). Unmapped, the
    // gain-table store faults. Register-backed window up to the declarative
    // peripheral at 0x6001_CC00. (RF air-gap: storage is enough — there's
    // no real RF that would act on these values.)
    bus.add_peripheral(
        "wifi_bb",
        0x6001_C000,
        0xC00,
        None,
        Box::new(crate::peripherals::esp32c3::reg_block::Esp32c3RegBlock::new(0xC00)),
    );
    // Radio front-end PLL-lock status (RADIO_FE 0x6000_6000 + 0x174, bit16):
    // the libphy pll_cal launches the BBPLL/RF PLL then busy-polls this bit
    // for lock; without real RF it never sets and pll_cal spins/retries
    // ("pll_cal exceeds 2ms!!!"). Force-assert it (RF air-gap cut) over just
    // that one word, leaving the declarative radio_fe descriptors intact.
    bus.add_peripheral(
        "radio_fe_pll_lock",
        0x6000_6174,
        0x4,
        None,
        Box::new(
            crate::peripherals::esp32c3::forced_status::Esp32c3ForcedStatus::new(
                0x4,
                vec![(0x0, 1 << 16)],
            ),
        ),
    );
    // WiFi MAC (WIFI_MAC 0x6003_3000, 12 KiB) — behavioral model for the
    // MAC <-> SimNet bridge: register-backed bring-up, MAC-ready bit (0xD14
    // b0, polled by hal_init), RX descriptor-ring DMA + RX-frame injection,
    // and MAC interrupt (matrix source 0) on RX-done. Overrides the
    // declarative wifi_mac window. See docs/esp32c3_wifi_mac_bridge.md.
    bus.add_peripheral(
        "wifi_mac",
        0x6003_3000,
        0x3000,
        None,
        Box::new(crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac::new()),
    );
    // Hardware RNG data register (WDEV_RND_REG, 0x6002_60B0): yields a fresh
    // word per read. bootloader_fill_random XORs successive reads and
    // process_segments refills ram_obfs_value until non-zero — a constant
    // RNG gives 0 and spins forever. Override the SYSCON stub at this word.
    bus.add_peripheral(
        "wdev_rnd",
        0x6002_60B0,
        0x4,
        None,
        Box::new(crate::peripherals::esp32c3::rng::Esp32c3Rng::new()),
    );
    // SHA accelerator (0x6003_B000): the 2nd-stage bootloader verifies the
    // app image's appended SHA-256 with it; an unmodelled (zero) digest
    // makes it reject the image. Real SHA-256 block compression here.
    bus.add_peripheral(
        "sha",
        0x6003_B000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32c3::sha::Esp32c3Sha::new()),
    );
    bus.add_peripheral(
        "flash_irom_xip",
        0x4200_0000,
        0x80_0000, // 8 MiB I-cache window
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            backing.clone(),
            0x4200_0000,
            mmu_table.clone(),
            MMU_FMT_C3,
        )),
    );
    bus.add_peripheral(
        "flash_drom_xip",
        0x3C00_0000,
        0x80_0000, // 8 MiB D-cache window
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            backing.clone(),
            0x3C00_0000,
            mmu_table.clone(),
            MMU_FMT_C3,
        )),
    );
    // SAR ADC (APB_SARADC, 0x6004_0000): the IDF's adc_hal_self_calibration
    // triggers single conversions and polls a data-valid flag (0x44 bit31/
    // bit30) before reading the result; the declarative stub never asserts
    // it, so read_cal_channel spins forever after spi_flash init. Model
    // conversions as instant (valid flags set, mid-scale sample) so the
    // bounded cal search converges and boot continues.
    bus.add_peripheral(
        "apb_saradc",
        0x6004_0000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32c3::sar_adc::Esp32c3SarAdc::new()),
    );
    // SYSTIMER (0x6002_3000): the 16 MHz free-running counter behind
    // esp_timer and the FreeRTOS tick. systimer_hal_get_counter_value sets
    // UNITx_OP bit30 (UPDATE) then polls bit29 (VALUE_VALID) before reading
    // the snapshot; the declarative stub never asserts VALUE_VALID, so the
    // counter read spins forever right after heap_init. The C3 SYSTIMER is
    // the same IP as the S3 (identical register layout), so the S3 model
    // drops in: it asserts VALUE_VALID, advances the counter, and supports
    // the alarm/IRQ path FreeRTOS needs. Clocked relative to the 160 MHz
    // CPU (10 CPU cycles per 16 MHz tick).
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        // C3 SYSTIMER_TARGET0 routes through the interrupt matrix on source
        // 37 (TARGET1/2 at 38/39), unlike the S3's 57; the FreeRTOS tick
        // alarm fires on that source.
        //
        // Walk-free (C3 SYSTIMER batch): scheduler mode. The free-running
        // counter advances lazily (write-path `sync_to` + the OP-update snapshot
        // pulling the bus-published `CycleClock`, both to the batch-start
        // cycle), and alarms fire as scheduled events at their exact expiry
        // cycle, delivered through the C3 interrupt matrix by
        // `apply_event_result`'s C3 routing arm. This un-pins SYSTIMER from the
        // per-cycle walk; delivery stays cycle-identical to the legacy walk at
        // a given tick interval (differential-gated).
        Box::new(crate::peripherals::esp32s3::systimer::Systimer::new_with_source(160_000_000, 37)),
    );
    // RTC_CNTL main timer (0x6000_8000): the free-running slow-clock counter
    // the IDF reads via rtc_time_get (set TIME_UPDATE @0x0C bit31 to latch,
    // read TIME0 @0x10 / TIME1 @0x14). A frozen counter makes every
    // RTC-deadline wait spin forever — most notably calibrate_ocode, which
    // polls a regi2c comparator that never settles without real RF and
    // relies on a ~10 ms RTC timeout to give up and continue. A real
    // advancing timer lets that loop (and other RTC delays) reach the
    // timeout exactly as silicon does. Overrides the declarative RTC_CNTL
    // stub for this window; non-timer regs stay register-backed so the
    // reset-cause seed at 0x38 below still reads back.
    bus.add_peripheral(
        "rtc_cntl_timer",
        0x6000_8000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32c3::rtc_timer::Esp32c3RtcTimer::new()),
    );
    // Seed the power-on hardware reset state the BROM reads to decide it's a
    // normal flash boot (silicon has this at reset; the sim starts zeroed):
    //   * RTC_CNTL reset-cause (0x6000_8038, bits[5:0]) = 1 (POWERON_RESET).
    //     rtc_get_reset_reason returns this; BROM main treats reset_reason 0
    //     as an error and bails (ret to 0) — 1 lets it continue to flash.
    //   * GPIO_STRAP (0x6000_4038) bit3 = SPI fast-flash-boot (matches the
    //     Xtensa rom-boot strap).
    let _ = bus.write_u32(0x6000_8038, 0x0000_0001);
    let _ = bus.write_u32(0x6000_4038, 0x0000_0008);
    //   * eFuse wafer version (EFUSE_RD_MAC_SPI_SYS_3 @ 0x6000_8850,
    //     WAFER_VERSION_MINOR_LO bits[20:18]) = 4 → chip rev v0.4. The real
    //     C3 is v0.4; without it eFuse reads v0.0 and the 2nd-stage
    //     bootloader rejects the app ("requires chip rev >= v0.3").
    let _ = bus.write_u32(0x6000_8850, 0x0010_0000);
    // Enable C3 RISC-V interrupt routing: the bus routes asserted peripheral
    // sources + the SYSTEM FROM_CPU IPI registers through the INTERRUPT_CORE0
    // matrix into the CPU's external interrupt lines. FreeRTOS's first
    // context switch (vPortYield → FROM_CPU SW interrupt) depends on this.
    bus.esp32c3_irq_routing = true;
    // Re-derive walk-deletion over the COMPLETE rom-boot bus. `from_config`
    // computed `legacy_walk_disabled` from the chip-yaml peripheral set alone,
    // BEFORE the rom-boot path appended its real walk workers above (notably the
    // walk-pinning `wifi_mac`, plus the behavioral rtc_cntl_timer/systimer). Once
    // every chip-yaml peripheral is scheduler-migrated (the LEDC timer port
    // emptied the last chip-yaml pinner), that early derivation would read
    // walk-DELETED and leave the per-cycle walk globally skipped — starving
    // wifi_mac's tick(). Recomputing here over the full set restores the correct
    // value (wifi_mac pins → walk enabled → interval 1), keeping the rom-boot
    // behavior byte-identical to before the migration.
    #[cfg(feature = "event-scheduler")]
    {
        bus.legacy_walk_disabled = bus.derive_walk_deletable();
    }
    if let Some(mac) = opts.efuse_mac {
        let lo =
            mac[5] as u32 | (mac[4] as u32) << 8 | (mac[3] as u32) << 16 | (mac[2] as u32) << 24;
        let hi = mac[1] as u32 | (mac[0] as u32) << 8;
        let _ = bus.write_u32(0x6000_8844, lo);
        let _ = bus.write_u32(0x6000_8848, hi);
    }
    let mut cpu = crate::system::riscv::configure_riscv(&mut bus);
    cpu.set_pc(0x4000_0000);
    // Disable the internal CLINT timer: the C3 has no standard MTIP — its
    // 31 interrupt lines (incl. line 7) are ESP matrix lines, so a
    // self-pending MTIP would collide. mtimecmp=MAX keeps mip bit7 clear.
    cpu.mtimecmp = u64::MAX;
    let mut machine = crate::Machine::new(wrap(cpu), bus);
    // ROM-boot firmware is interrupt-driven (FreeRTOS tick via the interrupt
    // matrix). Instruction batching freezes peripherals — and interrupt
    // delivery — across each 10k-step batch, so the scheduler never runs and
    // the app spins in vPortEnterCritical forever. Cycle-accurate stepping is
    // required for correctness here, exactly like the cycle-tight GPIO-timing
    // devices the test loop already exempts.
    machine.config.batch_mode_enabled = false;
    machine
}

/// Inject the C3 boot ROM images into the bus's zero-filled `rom` / `rom_data`
/// memory regions (IROM `0x4000_0000`, DROM `0x3FF0_0000`), matching how the
/// native `--rom-boot` path fills them from env pins or the vendored images.
/// Returns `true` if the IROM region was populated (i.e. the real ROM is
/// present and rom-boot can proceed). Shared by the native CLI and the wasm
/// browser path so both provision the ROM identically.
pub fn inject_rom_regions(bus: &mut crate::bus::SystemBus, images: &RomImages) -> bool {
    let mut irom_present = false;
    for mem in bus.extra_mem.iter_mut() {
        let src = if mem.base_addr == IROM_BASE as u64 {
            irom_present = true;
            &images.irom
        } else if mem.base_addr == DROM_BASE as u64 {
            &images.drom
        } else {
            continue;
        };
        let n = src.len().min(mem.data.len());
        mem.data[..n].copy_from_slice(&src[..n]);
    }
    irom_present
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vendored images must be exactly what the extractor produces from
    /// the toolchain's ROM ELF — including the `.data` copy sources whose
    /// last record ends exactly at the DRAM top (`ets_ops_table_ptr`). A
    /// zeroed ops table is the failure mode this pins down: boot dies in
    /// `ets_run_flash_bootloader` with a jalr to 0.
    #[test]
    fn vendored_images_match_extraction_and_ops_table_is_populated() {
        let Some(elf_path) = discover_rom_elf() else {
            eprintln!("no C3 ROM ELF installed; skipping consistency check");
            return;
        };
        let elf_bytes = std::fs::read(&elf_path).expect("read ROM ELF");
        let images = extract_rom_images(&elf_bytes).expect("extract");
        assert_eq!(images.irom.len(), IROM_SIZE);
        assert_eq!(images.drom.len(), DROM_SIZE);

        // ets_ops_table_ptr copy source (IROM 0x40059660, per the startup
        // unpack table) must hold a non-zero DRAM pointer.
        let rel = (0x4005_9660 - IROM_BASE) as usize;
        let ops = u32::from_le_bytes(images.irom[rel..rel + 4].try_into().unwrap());
        assert_ne!(ops, 0, "ets_ops_table_ptr copy source is zero");
        // The ops table itself lives in ROM constant data (DROM); accept a
        // DRAM pointer too in case a future ROM revision moves it.
        assert!(
            (0x3FC8_0000..0x3FCE_0000).contains(&ops)
                || (DROM_BASE..DROM_BASE + DROM_SIZE as u32).contains(&ops),
            "ets_ops_table_ptr copy source {ops:#010x} points at neither DRAM nor DROM"
        );

        let vendored = vendored_rom_images().expect("vendored images");
        assert_eq!(vendored.irom, images.irom, "vendored IROM drifted");
        assert_eq!(vendored.drom, images.drom, "vendored DROM drifted");
    }
}
