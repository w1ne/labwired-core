// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 (Xtensa LX7) system glue, split out of `system::xtensa`.

use super::RamPeripheral;
use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::peripherals::esp32s3::flash_xip::{new_mmu_table, Esp32s3MmuTable, FlashXipPeripheral};
use crate::peripherals::esp32s3::gpio::{Esp32s3Gpio, GpioObserver};
use crate::peripherals::esp32s3::i2c::{Esp32s3I2c, I2C0_BASE, I2C0_SIZE};
use crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix;
use crate::peripherals::esp_xtensa_common::rom_thunks::{self, RomThunkBank};
use crate::peripherals::esp_xtensa_common::system_stub::{EfuseStub, RtcCntlStub, SystemStub};
use crate::{Bus, Cpu};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Esp32s3Opts {
    pub iram_size: u32,
    pub dram_size: u32,
    pub flash_size: u32,
    pub cpu_clock_hz: u32,
    /// Select the flash-XIP model. `true` = real-reset boot (`--rom-boot`): the
    /// ROM + 2nd-stage bootloader program the hardware MMU, so both cache
    /// windows alias one physical flash backing and translate through that
    /// table — exactly as silicon. `false` (default) = fast-boot: the caller
    /// jumps straight into the app (the bootloader's MMU programming is
    /// skipped), so each window gets its own identity-mapped backing that
    /// `fast_boot` populates from the ELF's flash segments. Independent of the
    /// ROM-image choice: a real ROM is still loaded for its function calls in
    /// both modes.
    pub real_reset_boot: bool,
    /// Caller-injected boot ROM images. When `Some`, used directly instead of
    /// `provision_rom_images()`. This is how wasm gets the faithful ROM: the
    /// 512 KiB is NOT baked into the bundle (see
    /// `esp32s3_rom::vendored_rom_images`, wasm → None) — the playground
    /// fetches it as an on-demand asset and injects it here. `None` (default)
    /// → the native provision chain (env pins / toolchain / vendored blob).
    pub rom_images: Option<crate::boot::esp32s3_rom::RomImages>,
}

impl Default for Esp32s3Opts {
    fn default() -> Self {
        Self {
            iram_size: 512 * 1024,
            // Match chip yaml / TRM internal SRAM data view (512 KiB from
            // 0x3FC8_8000). The old 480 KiB cut off the top of the heap region
            // firmware uses for deep FreeRTOS/RMT stacks.
            dram_size: 512 * 1024,
            flash_size: 4 * 1024 * 1024,
            cpu_clock_hz: 80_000_000,
            real_reset_boot: false,
            rom_images: None,
        }
    }
}

/// Which ROM path the ESP32-S3 model booted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Esp32s3BootMode {
    /// Real Espressif boot ROM loaded (faithful path, zero thunks).
    Faithful,
    /// No ROM blob found — running on the thunk harness (degraded).
    Harness,
}

/// Result of `configure_xtensa_esp32s3` — exposes the flash backings so the
/// boot path can write to them (Task 8).
///
/// On real ESP32-S3 silicon, the I-cache (0x4200_0000) and D-cache
/// (0x3C00_0000) windows alias the same physical SPI flash, but each window
/// has its own MMU page table. The bootloader programs them so the same
/// physical page can appear at different virtual offsets in each window.
///
/// In our fast-boot model, the firmware ELF carries `.rodata` at vaddr
/// 0x3c000020 and `.text` at vaddr 0x42000020 — both with vaddr-offset 0x20
/// in their respective windows. If we shared a single backing buffer, the
/// later-loaded segment would overwrite the earlier one. So each window
/// gets its own backing buffer; fast_boot picks the correct one based on
/// which window the segment's vaddr falls into.
pub struct Esp32s3Wiring {
    pub cpu: XtensaLx7,
    pub icache_backing: Arc<Mutex<Vec<u8>>>,
    pub dcache_backing: Arc<Mutex<Vec<u8>>>,
    pub boot_mode: Esp32s3BootMode,
}

impl Esp32s3Wiring {
    /// Install a GPIO observer on the wired bus's `gpio` peripheral.
    ///
    /// Walks `bus.peripherals` to find the entry named `"gpio"`, downcasts
    /// to `Esp32s3Gpio`, and pushes the observer onto its list. Panics if
    /// no GPIO peripheral was registered (configure_xtensa_esp32s3 always
    /// registers one, so this only fires if the caller used a different
    /// configure path).
    pub fn add_gpio_observer(&self, bus: &mut SystemBus, observer: Arc<dyn GpioObserver>) {
        for p in bus.peripherals.iter_mut() {
            if p.name == "gpio" {
                if let Some(any) = p.dev.as_any_mut() {
                    if let Some(gpio) = any.downcast_mut::<Esp32s3Gpio>() {
                        gpio.add_observer(observer);
                        return;
                    }
                }
            }
        }
        panic!("add_gpio_observer: no gpio peripheral registered on the bus");
    }
}

/// Outputs of [`configure_esp32s3_memmap`]: the boot mode plus the flash
/// backings the caller threads into the SPIMEM1 controller and `Esp32s3Wiring`.
struct Esp32s3MemMap {
    boot_mode: Esp32s3BootMode,
    icache_backing: Arc<Mutex<Vec<u8>>>,
    dcache_backing: Arc<Mutex<Vec<u8>>>,
    shared_flash_backing: Arc<Mutex<Vec<u8>>>,
}

/// Install the ESP32-S3 memory map: IRAM/DRAM/RTC SRAM banks, the flash-XIP
/// cache windows (real-MMU or fast-boot identity), and the boot ROM (faithful
/// image or thunk harness). Core wiring, independent of the peripheral models.
fn configure_esp32s3_memmap(bus: &mut SystemBus, opts: &Esp32s3Opts) -> Esp32s3MemMap {
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

    // ── RTC SLOW / RTC FAST memory ────────────────────────────────────────
    // The ESP32-S3 has two small always-on SRAM banks the toolchain places
    // linker sections in even for a trivial sketch:
    //   * RTC SLOW @ 0x5000_0000, 8 KiB — RTC_SLOW_MEM / `RTC_DATA_ATTR`,
    //     deep-sleep retention. Arduino-ESP32 emits a 32-byte segment here.
    //   * RTC FAST @ 0x600F_E000, 8 KiB — RTC_FAST_MEM / `RTC_NOINIT_ATTR`
    //     and the deep-sleep wake stub; a 40-byte segment lands near the top.
    // Without these the ELF loader refuses to place those segments. They are
    // plain RAM to the CPU (read/write, zero-on-reset via RamPeripheral).
    bus.add_peripheral(
        "rtc_slow",
        0x5000_0000,
        0x2000,
        None,
        Box::new(RamPeripheral::new(0x2000)),
    );
    bus.add_peripheral(
        "rtc_fast",
        0x600F_E000,
        0x2000,
        None,
        Box::new(RamPeripheral::new(0x2000)),
    );

    // ── Flash-XIP windows ─────────────────────────────────────────────────
    // Proper model (real ROM): both cache windows alias one physical flash
    // backing and translate through the real hardware MMU table the firmware
    // programs at DR_REG_MMU_TABLE (0x600C_5000) — exactly as silicon. The
    // window spans the full 32 MiB linear range so the ROM's bootloader-load
    // reads (e.g. virtual 0x3C80_0000) resolve. Fast-boot keeps the legacy
    // per-window static identity mapping over separate 4 MiB backings.
    //
    // The MMU model is selected for the real-reset (`--rom-boot`) path, where
    // the ROM + 2nd-stage bootloader actually program DR_REG_MMU_TABLE. For
    // fast-boot the bootloader's MMU programming is skipped, so an unprogrammed
    // (all-invalid) table would make every app XIP read return 0 — instead we
    // use per-window identity backings that `fast_boot` fills from the ELF's
    // flash segments. This is INDEPENDENT of whether a real ROM is loaded: a
    // fast-booted app still calls the real ROM (see the ROM block below), it
    // just reaches its own `.rodata`/`.text` through identity XIP rather than a
    // table the skipped bootloader never wrote.
    // Caller-injected ROM (wasm's on-demand asset) wins; else the native
    // provision chain (env pins / toolchain / vendored blob — None on wasm).
    let rom_images = opts
        .rom_images
        .clone()
        .or_else(crate::boot::esp32s3_rom::provision_rom_images);
    // Fast-boot also uses MMU XIP so `spi_flash_mmap` / partition-table load
    // and `cache2phys` share one translation (seeded after `fast_boot`).
    // Identity-only XIP made mmap of flash 0x8000 read the wrong dcache page.
    let mmu_model = opts.real_reset_boot
        || std::env::var_os("LABWIRED_ESP32S3_FASTBOOT").is_some()
        || std::env::var_os("LABWIRED_ESP32S3_MMU_XIP").is_some();
    // Shared flash backing for the proper-model path, loaded from the real
    // flash image so XIP reads (and the SPI-flash controller below) return real
    // bytes. In fast-boot this is unused; the legacy per-window backings apply.
    let shared_flash_backing = {
        let mut buf = vec![0xFFu8; opts.flash_size as usize];
        if let Ok(p) = std::env::var("LABWIRED_ESP32S3_FLASH") {
            if let Ok(bytes) = std::fs::read(&p) {
                let n = bytes.len().min(buf.len());
                buf[..n].copy_from_slice(&bytes[..n]);
                eprintln!("configure_xtensa_esp32s3: loaded flash image ({n} bytes) from {p}");
            }
        }
        Arc::new(Mutex::new(buf))
    };
    // Backings exposed on Esp32s3Wiring. In the MMU model both windows alias
    // one physical flash backing; in fast-boot they stay independent.
    let (icache_backing, dcache_backing) = if mmu_model {
        let mmu_table = new_mmu_table();
        const XIP_WINDOW: u64 = 0x0200_0000; // 32 MiB linear MMU window
        let icache = FlashXipPeripheral::new_mmu(
            shared_flash_backing.clone(),
            0x4200_0000,
            mmu_table.clone(),
        );
        let dcache = FlashXipPeripheral::new_mmu(
            shared_flash_backing.clone(),
            0x3C00_0000,
            mmu_table.clone(),
        );
        bus.add_peripheral(
            "flash_icache",
            0x4200_0000,
            XIP_WINDOW,
            None,
            Box::new(icache),
        );
        bus.add_peripheral(
            "flash_dcache",
            0x3C00_0000,
            XIP_WINDOW,
            None,
            Box::new(dcache),
        );
        // The MMU table register block the firmware programs (512 × u32).
        bus.add_peripheral(
            "mmu_table",
            0x600C_5000,
            0x800,
            None,
            Box::new(Esp32s3MmuTable::new(mmu_table)),
        );
        (shared_flash_backing.clone(), shared_flash_backing.clone())
    } else {
        // Fast-boot: legacy per-window static identity mapping over separate
        // 4 MiB backings (see Esp32s3Wiring docs).
        let icache_backing = Arc::new(Mutex::new(vec![0u8; opts.flash_size as usize]));
        let dcache_backing = Arc::new(Mutex::new(vec![0u8; opts.flash_size as usize]));
        let mut icache = FlashXipPeripheral::new_shared(icache_backing.clone(), 0x4200_0000);
        let mut dcache = FlashXipPeripheral::new_shared(dcache_backing.clone(), 0x3C00_0000);
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
        (icache_backing, dcache_backing)
    };

    // ── ROM: faithful real-silicon image (default) or thunk harness (fallback) ─
    // provision_rom_images() resolves the ROM either from explicit pre-extracted
    // bins (LABWIRED_ESP32S3_ROM/_DROM) or by discovering + extracting the ROM
    // ELF from the installed toolchain. None → no blob available → thunk harness.
    let boot_mode = match rom_images {
        Some(images) => {
            let rom = RamPeripheral::with_image(0x6_0000, &images.irom);
            bus.add_peripheral("rom", 0x4000_0000, 0x6_0000, None, Box::new(rom));
            let drom = RamPeripheral::with_image(0x2_0000, &images.drom);
            bus.add_peripheral("drom", 0x3FF0_0000, 0x2_0000, None, Box::new(drom));
            // Replicate the ROM reset's `.data` copy into DRAM. fast-boot skips
            // the ROM's own startup, so its DRAM data structures (e.g.
            // rom_cache_internal_table_ptr @0x3FCEFFC4) would otherwise stay
            // null and the real ROM cache helpers esp-hal calls dispatch
            // through a null table → callx8 0. This lands the genuine values
            // exactly as silicon does — zero thunks.
            let init_writes = crate::boot::esp32s3_rom::s3_rom_data_init_writes(&images.irom);
            let mut init_bytes = 0usize;
            for (dst, bytes) in &init_writes {
                for (i, b) in bytes.iter().enumerate() {
                    let _ = bus.write_u8(*dst as u64 + i as u64, *b);
                }
                init_bytes += bytes.len();
            }
            eprintln!(
                "configure_xtensa_esp32s3: faithful ROM loaded ({} B IROM, {} B DROM) — real boot ROM, zero thunks; ROM .data init: {} records, {} B",
                images.irom.len(),
                images.drom.len(),
                init_writes.len(),
                init_bytes
            );
            Esp32s3BootMode::Faithful
        }
        None => {
            // Code ROM is thunk-backed. Still map DROM (0x3FF0_0000, 128 KiB)
            // as zero RAM: Arduino/IDF reads ROM data tables through that
            // window; leaving it unmapped faults at e.g. 0x3FF1_FFFC.
            // Prefer vendored/env DROM image when present even in harness.
            let drom_bytes = std::env::var("LABWIRED_ESP32S3_DROM")
                .ok()
                .and_then(|p| std::fs::read(p).ok())
                .or_else(|| {
                    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("roms/esp32s3/esp32s3_drom.bin");
                    std::fs::read(p).ok()
                })
                .unwrap_or_else(|| vec![0u8; 0x2_0000]);
            let drom = RamPeripheral::with_image(0x2_0000, &drom_bytes);
            bus.add_peripheral("drom", 0x3FF0_0000, 0x2_0000, None, Box::new(drom));
            let mut rom_bank = RomThunkBank::new(0x4000_0000, 0x6_0000);
            register_default_thunks(&mut rom_bank);
            bus.add_peripheral(
                "rom_thunks",
                0x4000_0000,
                0x6_0000,
                None,
                Box::new(rom_bank),
            );
            eprintln!(
                "configure_xtensa_esp32s3: ESP32-S3 ROM harness (thunk code + {} B DROM)",
                drom_bytes.len()
            );
            Esp32s3BootMode::Harness
        }
    };

    Esp32s3MemMap {
        boot_mode,
        icache_backing,
        dcache_backing,
        shared_flash_backing,
    }
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
    //
    // Disable Cortex-M bit-band aliasing — its 0x4200_0000–0x4400_0000 range
    // collides with the ESP32-S3 flash-XIP I-cache window. With bit-band
    // enabled, instruction fetches from 0x4200_xxxx get translated as
    // single-bit reads of a synthetic peripheral byte instead of going
    // through our FlashXipPeripheral. ESP32-S3 has no bit-band hardware.
    bus.bit_band_enabled = false;

    let Esp32s3MemMap {
        boot_mode,
        icache_backing,
        dcache_backing,
        shared_flash_backing,
    } = configure_esp32s3_memmap(bus, opts);

    // ── Interrupt Matrix (Plan 3) ────────────────────────────────────────
    // Core SMP wiring; GPIO and IO_MUX are peripheral models and now live in
    // register_esp32s3_peripherals. Routing is address-pure (greatest-start-
    // wins, see bus::find_peripheral_index), so intmatrix at 0x600C_2000 wins
    // its window over the broad 0x600C_0000 "system" stub regardless of order.
    // CORE0 map table at +0x000, CORE1 map table at +0x800 (both on the
    // shared interrupt base) — widened to 0x1000 so the APP_CPU's interrupt
    // routing (e.g. FROM_CPU_1, source 80, at +0x940) lands in the matrix
    // rather than the round-tripping "system" catch-all below.
    bus.add_peripheral(
        "intmatrix",
        0x600C_2000,
        0x1000,
        None,
        Box::new(Esp32s3IntMatrix::new()),
    );
    // ── Cross-core IPI doorbell (SYSTEM_CPU_INTR_FROM_CPU_0..3, 0x600C_0030) ─
    // The SMP cross-core software interrupt. Writing FROM_CPU_{core} asserts
    // the level source FROM_CPU_INTR{core} (79 for core 0, 80 for core 1),
    // which the interrupt matrix routes to that core. Without it the FreeRTOS
    // SMP scheduler can never yield/IPC a remote core, so a pinned task like
    // Arduino's loopTask is never dispatched. MUST register BEFORE the
    // 0x600C_0000 "system" catch-all so these four words get real semantics.
    bus.add_peripheral(
        "crosscore_ipi",
        crate::peripherals::esp32s3::crosscore_ipi::BASE,
        crate::peripherals::esp32s3::crosscore_ipi::SIZE,
        None,
        Box::new(crate::peripherals::esp32s3::crosscore_ipi::Esp32s3CrossCoreIpi::new()),
    );
    // ── SYSTEM_CORE_1_CONTROL (0x600C_0000) ──────────────────────────────
    // APP_CPU reset/clock-gate control. Registered BEFORE the 0x600C_0000
    // "system" catch-all so the RESETING 1→0 edge (APP_CPU out of reset) is
    // observed and the run loop can boot core 1 from the real ROM.
    bus.add_peripheral(
        "core1_control",
        0x600C_0000,
        0x8,
        None,
        Box::new(crate::peripherals::esp32s3::core1_control::Esp32s3Core1Control::new()),
    );
    // ── EXTMEM cache controller (0x600C_4000) ────────────────────────────
    // The boot ROM drives cache invalidate/writeback/sync through this block
    // using a launch-bit/done-bit handshake (CACHE_SYNC_CTRL @+0x28: write an
    // enable bit, poll SYNC_DONE bit 3). Must register BEFORE the 0x600C_0000
    // catch-all "system" stub (read-as-zero would never set the done bit, so
    // the ROM's `bnone a9, 8` poll spins forever). Verified resting value 0x8
    // off silicon via JTAG.
    bus.add_peripheral(
        "extmem",
        0x600C_4000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::extmem::Esp32s3Extmem::new()),
    );

    // ── SPIMEM1 flash-command controller (0x6000_2000) ───────────────────
    // Real command executor (READ/RDSR/RDID) over a flash backing — replaces
    // the auto-clear stub. Registered BEFORE the 0x6000_0000 catch-all. The
    // backing is the raw flash image (LABWIRED_ESP32S3_FLASH = the flashed
    // firmware.bin/.factory.bin) so READ returns real bytes; 0xFF otherwise.
    bus.add_peripheral(
        "spimem1",
        0x6000_2000,
        0x100,
        None,
        Box::new(
            crate::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
                shared_flash_backing.clone(),
            ),
        ),
    );

    // ── SYSTEM / RTC_CNTL / EFUSE stubs ──────────────────────────────────
    // SYSTEM peripheral on ESP32-S3 is followed by INTERRUPT_CORE0/1 at
    // 0x600C_2000 / 0x600C_2800 (CPU intr-from-CPU mapping) and the AES /
    // SHA / RSA accelerator block around 0x600C_4000-0x600C_FFFF. esp-hal's
    // init pokes the interrupt-mapping registers and a few accelerator
    // resets. Cover [0x600C_0000, 0x600D_0000) with one round-tripping
    // stub — SystemStub::new() (read-as-zero on unwritten) so that the
    // interrupt mapping reads back exactly what was written. The intmatrix
    // peripheral registered above takes precedence at 0x600C_2000..0x600C_2800.
    bus.add_peripheral(
        "system",
        0x600C_0000,
        0x1_0000,
        None,
        Box::new(SystemStub::new()),
    );
    // ── SYSTEM clock/reset register file (0x600C_0000..0x600C_1000) ───────
    // Faithful fixed-register model for the 42 architected SYSTEM registers
    // (silicon reset values + per-register write masks; unmapped offsets read
    // as zero and ignore writes — NOT round-trip). Registered AFTER the broad
    // `system` SystemStub above so that, by the bus router's last-start-wins
    // tie-break on the shared 0x600C_0000 base, this narrower model serves the
    // register-block window while the SystemStub keeps serving the
    // accelerator/interrupt-map region above 0x600C_1000.
    //
    // CRITICAL — the model is registered as TWO windows straddling a HOLE at
    // 0x600C_0030..0x600C_0040 so it never overlaps `crosscore_ipi` (the
    // stateful SMP doorbell whose tick() re-asserts level sources 79/80). The
    // bus's `peripheral_hint` cache short-circuits to the last peripheral that
    // *contains* an address, so a single 0x1000-wide window covering the
    // doorbell would swallow a doorbell write at runtime once any neighbouring
    // SYSTEM register had been accessed first — deadlocking SMP boot. Leaving
    // the hole means `crosscore_ipi` is the ONLY narrow peripheral covering
    // 0x030..0x03F and always wins it. `core1_control` shares the 0x600C_0000
    // base; its sole behavior (the CORE_1_CONTROL_0 RESETING 1→0 edge that
    // releases the APP_CPU) is replicated by this model, so APP_CPU boot is
    // preserved even though window A shadows 0x000/0x004. `resolve_window(
    // 0x600C_0000)` returns window A (NON-round-tripping), giving the coverage
    // probe a clean baseline. The empirical routing test
    // (`system_register_block_routing`) pins all of this down.
    //
    // Window A: 0x600C_0000..0x600C_0030 (CORE_1_CONTROL_0/1 .. BT_LPCK_DIV_*).
    bus.add_peripheral(
        "system_regs",
        0x600C_0000,
        crate::peripherals::esp32s3::crosscore_ipi::BASE - 0x600C_0000, // 0x30
        None,
        Box::new(crate::peripherals::esp32s3::system::Esp32s3System::new()),
    );
    // Window B: 0x600C_0040..0x600C_1000 (RSA_PD_CTRL .. PVT .. DATE@0xFFC).
    // The second instance carries window_base=0x40 so its base-relative offsets
    // map back onto the architected (absolute) register offsets.
    let win_b_base = crate::peripherals::esp32s3::crosscore_ipi::BASE
        + crate::peripherals::esp32s3::crosscore_ipi::SIZE; // 0x600C_0040
    bus.add_peripheral(
        "system_regs_hi",
        win_b_base,
        0x600C_0000 + crate::peripherals::esp32s3::system::SIZE - win_b_base, // up to 0x1000
        None,
        Box::new(
            crate::peripherals::esp32s3::system::Esp32s3System::with_window_base(
                win_b_base - 0x600C_0000, // 0x40
            ),
        ),
    );
    // RTC_CNTL through APB_CTRL/SYSCON are a contiguous block of register
    // banks ESP-HAL pokes during init (clock muxing, voltage rails, GPIO
    // mux, sensor ADC, PMS). Cover [0x6000_8000, 0x6001_0000) with a single
    // round-tripping stub — 32 KiB of tracked-word storage matches the
    // SystemStub semantics and is enough for any benign register hammer.
    // The io_mux peripheral registered above takes precedence at
    // 0x6000_9000..0x6000_9100.
    bus.add_peripheral(
        "rtc_cntl",
        0x6000_8000,
        0x8000,
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

    // UART0..2, SPI0/1, GPIO, GPIO_SD, SDIO host live in the
    // [0x6000_0000, 0x6000_7000) window. esp-hal touches GPIO config and
    // (later) UART for the panic path; we provide a round-tripping stub so
    // those reads/writes settle. Read-as-zero on unwritten matches the
    // power-on register values for these blocks (no busy-waits live here).
    bus.add_peripheral(
        "low_mmio",
        0x6000_0000,
        0x7000,
        None,
        Box::new(SystemStub::new()),
    );

    // Catch-all for the rest of the high-MMIO range that esp-hal / the boot
    // ROM / 2nd-stage bootloader poke during init (LEDC, RMT, GPIO matrix,
    // GDMA, APB_SARADC (bootloader RNG/entropy enable @0x6004_0000), LCD_CAM,
    // RTC calibration timer, …). Real silicon has dozens of distinct
    // peripherals in this window with bit-precise behaviour; for hello-world
    // we only need round-trip register storage and an "everything's ready"
    // default for status polls. Use the unwritten-ones variant so that
    // calibration-RDY / FIFO-empty / link-up bits trip on the first iteration.
    // The block covers [0x6001_0000, 0x6005_0000) — 256 KiB.
    bus.add_peripheral(
        "mmio_rest",
        0x6001_0000,
        0x4_0000,
        None,
        Box::new(SystemStub::with_unwritten_ones()),
    );

    // ESP32-S3 peripheral models. Factored into a separate unit so Stage 3 can
    // build them from a chip YAML via `SystemBus::from_config` instead. Called
    // after the catch-all stubs so each twin wins its own (higher-base) window.
    register_esp32s3_peripherals(bus, opts);

    // Power-on register state the real boot ROM checks before booting from
    // flash. Values captured from silicon over JTAG: without them the ROM reads
    // reset-reason = 0 ("invalid reset") and boot-strap = 0 (DOWNLOAD mode) and
    // never boots from flash (it traps at ets_main.c:691). Needed for --rom-boot;
    // harmless for the fast-boot/thunk path (those don't read these regs).
    let _ = bus.write_u32(0x6000_4038, 0x0000_0008); // GPIO_STRAP = SPI_FAST_FLASH_BOOT
    let _ = bus.write_u32(0x6000_8038, 0x0000_f30c); // RTC_CNTL_RESET_STATE = valid reset cause

    let mut cpu = XtensaLx7::new();
    cpu.reset(bus).expect("xtensa reset");

    Esp32s3Wiring {
        cpu,
        icache_backing,
        dcache_backing,
        boot_mode,
    }
}

/// Register the ESP32-S3 peripheral models on `bus`.
///
/// Split out of [`configure_xtensa_esp32s3`] so the migration's Stage 3 can
/// build these from a chip YAML through `SystemBus::from_config` (the same
/// data-driven path the Cortex-M and RISC-V chips use). Each model is also
/// reachable by type string via `peripherals::esp32s3::factory::try_build`.
pub(crate) fn register_esp32s3_peripherals(bus: &mut SystemBus, opts: &Esp32s3Opts) {
    use crate::peripherals::esp32s3::factory;
    use labwired_config::PeripheralConfig;
    use std::collections::HashMap;

    // Data-driven: build every ESP32-S3 peripheral from the canonical
    // ESP32S3_PERIPHERALS table through the esp32s3 factory. Behaviour matches
    // the former hand-wired registrations (pinned by
    // factory_descriptors_match_hardwired_peripherals). Per-model rationale
    // lives in the model modules under peripherals/esp32s3/. Routing is
    // address-pure (greatest-start-wins), and this runs after the core catch-all
    // stubs, so the same-base UART windows still win.
    for &(id, ty, base, size, irq) in ESP32S3_PERIPHERALS {
        if id == "i2c0" {
            // Built below with its board I2C slaves attached.
            continue;
        }
        let mut config: HashMap<String, serde_yaml::Value> = HashMap::new();
        match id {
            // systimer ticks at the configured CPU clock (not the timer-group's
            // fixed 240 MHz), so thread opts.cpu_clock_hz through as config.
            "systimer" => {
                config.insert(
                    "cpu_clock_hz".to_string(),
                    serde_yaml::Value::Number((opts.cpu_clock_hz as u64).into()),
                );
            }
            // uart0 echoes TX to the host console; uart1/2 are capture-only.
            "uart1_s3" | "uart2_s3" => {
                config.insert("echo_stdout".to_string(), serde_yaml::Value::Bool(false));
            }
            _ => {}
        }
        let cfg = PeripheralConfig {
            id: id.to_string(),
            r#type: ty.to_string(),
            base_address: base,
            size: None,
            irq,
            clock: None,
            config,
        };
        let dev = factory::try_build(ty, &cfg)
            .unwrap_or_else(|| panic!("esp32s3 factory missing type {ty} for {id}"));
        // Bus-entry irq stays None; the source id is baked into the model by the
        // factory via cfg.irq.
        bus.add_peripheral(id, base, size, None, dev);
    }

    // Register the on-chip I2C0 controller. Off-chip I2C slaves (TMP102, SSD1306,
    // SH1107, PCA9685, …) are NOT attached here — they are wired from the board
    // manifest's `external_devices` by `attach_esp32_external_devices`, the same
    // way real hardware is: the board says what's on the bus, not the SoC config.
    let i2c0 = Esp32s3I2c::new();
    bus.add_peripheral("i2c0", I2C0_BASE as u64, I2C0_SIZE, None, Box::new(i2c0));
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
    // intr_matrix_set / esp_rom_route_intr_matrix — binds peripheral source
    // IDs to CPU IRQ slots (FROM_CPU yield, systimer tick, UART, …).
    bank.register(0x4000_1b54, rom_thunks::esp32s3_rom_route_intr_matrix);
    // ROM MD5 — `CONFIG_PARTITION_TABLE_MD5` hashes every 32-byte partition
    // entry then compares the 0xEBEB trailer. Without real MD5, load_partitions
    // fails MD5 verify → empty list → OTA `it != NULL` assert in initArduino.
    bank.register(0x4000_1c5c, rom_thunks::rom_md5_init); // MD5Init / esp_rom_md5_init
    bank.register(0x4000_1c68, rom_thunks::rom_md5_update); // MD5Update
    bank.register(0x4000_1c74, rom_thunks::rom_md5_final); // MD5Final
                                                           // esp_rom_spiflash_unlock — flash write helper. Boot path doesn't write,
                                                           // but the symbol may be linked in.
    bank.register(0x4000_0a2c, rom_thunks::esp_rom_spiflash_unlock);
    // rtc_get_reset_reason(cpu_idx) — esp-hal queries this during init to
    // distinguish power-on from soft reset; we always report POWERON_RESET.
    bank.register(0x4000_057c, rom_thunks::rtc_get_reset_reason);
    // rom_config_data_cache_mode — analogous to instruction cache config; NOP.
    bank.register(0x4000_1a28, rom_thunks::nop_return_zero);
    // ets_get_cpu_frequency() → MHz; Arduino log timestamps divide CCOUNT
    // by (mhz*40)-ish — zero ⇒ IntegerDivideByZeroCause.
    bank.register(0x4000_1a40, rom_thunks::rom_cpu_freq_240mhz);
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
    // ...and the read/write-with-mask variants (rom_i2c_*Reg_Mask), hit by the
    // BBPLL/clock tuning path on the full ESP-IDF image.
    bank.register(0x4000_5d54, rom_thunks::nop_return_zero); // esp_rom_regi2c_read_mask
    bank.register(0x4000_5d6c, rom_thunks::nop_return_zero); // esp_rom_regi2c_write_mask
                                                             // memcpy and __udivdi3 do real work — emulate them so the firmware
                                                             // doesn't get garbage from the boot-init copy paths.
    bank.register(0x4000_11f4, rom_thunks::rom_memcpy);
    // strlen — Print::write / Serial.println length; see rom_strlen docs.
    bank.register(0x4000_1248, rom_thunks::rom_strlen);
    bank.register(0x4000_2544, rom_thunks::rom_udivdi3);
    // libgcc 64-bit arithmetic + bit/byte helpers, in ROM. The full ESP-IDF
    // image pulls these from the C runtime (printf, timers, hashing). Each has
    // a real emulation thunk.
    bank.register(0x4000_21b4, rom_thunks::rom_ashldi3); // __ashldi3
    bank.register(0x4000_21c0, rom_thunks::rom_ashrdi3); // __ashrdi3
    bank.register(0x4000_21cc, rom_thunks::rom_bswapdi2); // __bswapdi2
    bank.register(0x4000_21d8, rom_thunks::rom_bswapsi2); // __bswapsi2
    bank.register(0x4000_2214, rom_thunks::rom_clzsi2); // __clzsi2
    bank.register(0x4000_2238, rom_thunks::rom_ctzsi2); // __ctzsi2
    bank.register(0x4000_225c, rom_thunks::rom_divdi3); // __divdi3
    bank.register(0x4000_23d0, rom_thunks::rom_lshrdi3); // __lshrdi3
    bank.register(0x4000_23f4, rom_thunks::rom_moddi3); // __moddi3
    bank.register(0x4000_2574, rom_thunks::rom_umoddi3); // __umoddi3
                                                         // eFuse config getters (flash pin/WP routing, UART/USB print-disable,
                                                         // boot-mode/security flags). The sim models none of these straps; return 0
                                                         // everywhere — the default SPI pin mapping and the permissive "feature not
                                                         // disabled" answer, which is what an unburned dev part reports.
    for addr in [
        0x4000_1f74, // ets_efuse_get_spiconfig / get_flash_gpio_info
        0x4000_1f80, // ets_efuse_usb_print_is_disabled
        0x4000_1f8c, // ets_efuse_usb_serial_jtag_print_is_disabled
        0x4000_1f98, // ets_efuse_get_uart_print_control
        0x4000_1fa4, // ets_efuse_get_wp_pad / get_flash_wp_gpio
        0x4000_1fb0, // ets_efuse_legacy_spi_boot_mode_disabled
        0x4000_1fbc, // ets_efuse_security_download_modes_enabled
    ] {
        bank.register(addr, rom_thunks::nop_return_zero);
    }
    // ROM libc siblings of memcpy. A full ESP-IDF/Arduino image (unlike the
    // minimal esp-hal hello-world) calls these from the C runtime startup and
    // FreeRTOS init. Addresses are the ESP32-S3 ROM symbol table values.
    bank.register(0x4000_11e8, rom_thunks::rom_memset);
    bank.register(0x4000_1200, rom_thunks::rom_memmove);
    bank.register(0x4000_120c, rom_thunks::rom_memcmp);
    // qsort — heap reserved-region sort in soc_get_available_memory_regions.
    bank.register(0x4000_1488, rom_thunks::rom_qsort);
    // ROM cache-management API. A full ESP-IDF/Arduino image drives the whole
    // family during flash/MMU bring-up; the esp-hal path only touched
    // suspend/resume-DCache (0x4000_18b4 / 0x4000_18c0, registered above).
    //
    // IRAM wrappers for Suspend/Freeze_* poll EXTMEM CACHE_STATE (0x600C_4130)
    // after the ROM call — those must drive the matching field. Enable/Disable
    // update the same idle bits so a Disable→Suspend sequence (flash ops)
    // observes state=1 after Suspend rather than spinning forever.
    // MMU size/info / occupy / page-count remain nops (XIP is identity-mapped).
    bank.register(0x4000_186c, rom_thunks::cache_disable_icache);
    bank.register(0x4000_1878, rom_thunks::cache_enable_icache);
    bank.register(0x4000_1884, rom_thunks::cache_disable_dcache);
    bank.register(0x4000_1890, rom_thunks::cache_enable_dcache);
    bank.register(0x4000_189c, rom_thunks::cache_suspend_icache);
    bank.register(0x4000_18a8, rom_thunks::cache_resume_icache);
    for addr in [
        0x4000_1914, // Cache_Set_IDROM_MMU_Size
        0x4000_1950, // Cache_Set_IDROM_MMU_Info
        0x4000_1980, // Cache_Occupy_ICache_MEMORY
        0x4000_198c, // Cache_Occupy_DCache_MEMORY
        0x4000_19bc, // Cache_Count_Flash_Pages
    ] {
        bank.register(addr, rom_thunks::nop_return_zero);
    }
    // Cache freeze enable/disable: the IRAM wrappers busy-wait on the cache
    // state register (0x600C_4130) after calling these, so they must drive the
    // matching field rather than nop. (Suspend/Resume_DCache registered above.)
    bank.register(0x4000_18e4, rom_thunks::cache_freeze_icache_enable);
    bank.register(0x4000_18f0, rom_thunks::cache_freeze_icache_disable);
    bank.register(0x4000_18fc, rom_thunks::cache_freeze_dcache_enable);
    bank.register(0x4000_1908, rom_thunks::cache_freeze_dcache_disable);
    // ROM interrupt primitives used by FreeRTOS critical sections. The ESP-IDF
    // image calls these from interrupt context during scheduler bring-up; they
    // live in ROM (not the loaded image) so they must be thunked.
    bank.register(0x4000_1c38, rom_thunks::xtos_set_intlevel); // _xtos_set_intlevel
    bank.register(0x4000_1c08, rom_thunks::xtos_restore_intlevel); // _xtos_restore_intlevel
                                                                   // WDT + SYSTIMER ROM HALs — Arduino `system_early_init` / FreeRTOS tick
                                                                   // setup call these. Without them the harness ROM bank returns 0 / faults
                                                                   // on undecoded BREAK. NOP is enough: real TIMG/SYSTIMER MMIO models drive
                                                                   // the observable side effects the app needs later.
    for addr in [
        0x4000_0dbc, // wdt_hal_init
        0x4000_0dc8, // wdt_hal_deinit
        0x4000_0dd4, // wdt_hal_config_stage
        0x4000_0de0, // wdt_hal_write_protect_disable
        0x4000_0dec, // wdt_hal_write_protect_enable
        0x4000_0df8, // wdt_hal_enable
        0x4000_0e04, // wdt_hal_disable
        0x4000_0e10, // wdt_hal_handle_intr
        0x4000_0e1c, // wdt_hal_feed
        0x4000_0e28, // wdt_hal_set_flashboot_en
        0x4000_0e34, // wdt_hal_is_enabled
        0x4000_0e40, // systimer_hal_get_counter_value
        0x4000_0e4c, // systimer_hal_get_time
        0x4000_0e58, // systimer_hal_set_alarm_target
        0x4000_0e64, // systimer_hal_set_alarm_period
        0x4000_0e70, // systimer_hal_get_alarm_value
        0x4000_0e7c, // systimer_hal_enable_alarm_int
        0x4000_0e88, // systimer_hal_on_apb_freq_update
        0x4000_0e94, // systimer_hal_counter_value_advance
        0x4000_0ea0, // systimer_hal_enable_counter
        0x4000_0eac, // systimer_hal_init
        0x4000_0eb8, // systimer_hal_select_alarm_mode
        0x4000_0ec4, // systimer_hal_connect_alarm_counter
    ] {
        bank.register(addr, rom_thunks::nop_return_zero);
    }
}

// ── RamPeripheral helper ────────────────────────────────────────────────

/// Canonical `(id, factory type, window base, window size, irq source)` for
/// every ESP32-S3 peripheral that [`register_esp32s3_peripherals`] installs.
///
/// This is the Stage-3 source of truth — the data destined for `esp32s3.yaml`
/// so the peripheral set is built through the esp32s3 factory / `from_config`
/// instead of hand-wired `add_peripheral` calls. The irq column is the model's
/// ETS interrupt source id (a constructor argument; the bus-entry irq stays
/// `None`). Proven equivalent to the hand-wired path by the test
/// `factory_descriptors_match_hardwired_peripherals`.
#[rustfmt::skip]
pub(crate) const ESP32S3_PERIPHERALS: &[(&str, &str, u64, u64, Option<u32>)] = &[
    ("usb_serial_jtag", "esp32s3_usb_serial_jtag", 0x6003_8000, 0x1000, None),
    ("systimer",        "esp32s3_systimer",        0x6002_3000, 0x1000, None),
    ("gpio",            "esp32s3_gpio",            0x6000_4000, 0x0800, None),
    ("io_mux",          "esp32s3_io_mux",          0x6000_9000, 0x0100, None),
    ("sens_s3",         "esp32s3_sens",            0x6000_8800, 0x0400, None),
    ("rng",             "esp32s3_rng",             0x6003_5000, 0x0100, None),
    ("sha",             "esp32s3_sha",             0x6003_B000, 0x0100, None),
    ("pcnt",            "esp32s3_pcnt",            0x6001_7000, 0x1000, Some(41)),
    ("ledc",            "esp32s3_ledc",            0x6001_9000, 0x1000, None),
    ("timg0_s3",        "esp32s3_timer_group",     0x6001_F000, 0x1000, Some(50)),
    ("timg1_s3",        "esp32s3_timer_group",     0x6002_0000, 0x1000, Some(53)),
    ("rmt_s3",          "esp32s3_rmt",             0x6001_6000, 0x1000, Some(40)),
    ("spi2_s3",         "esp32s3_spi",             0x6002_4000, 0x1000, Some(21)),
    ("spi3_s3",         "esp32s3_spi",             0x6002_5000, 0x1000, Some(22)),
    ("sar_adc_s3",      "esp32s3_sar_adc",         0x6004_0000, 0x1000, Some(64)),
    ("gdma",            "esp32s3_gdma",            0x6003_F000, 0x1000, Some(66)),
    ("i2s0_s3",         "esp32s3_i2s",             0x6000_F000, 0x1000, Some(25)),
    ("i2s1_s3",         "esp32s3_i2s",             0x6002_D000, 0x1000, Some(26)),
    ("twai",            "esp32s3_twai",            0x6002_B000, 0x1000, Some(37)),
    ("aes",             "esp32s3_aes",             0x6003_A000, 0x1000, Some(77)),
    ("rsa",             "esp32s3_rsa",             0x6003_C000, 0x1000, Some(76)),
    ("hmac",            "esp32s3_hmac",            0x6003_E000, 0x1000, Some(0)),
    ("ds",              "esp32s3_ds",              0x6003_D000, 0x1000, Some(0)),
    ("mcpwm0",          "esp32s3_mcpwm",           0x6001_E000, 0x1000, Some(38)),
    ("mcpwm1",          "esp32s3_mcpwm",           0x6002_C000, 0x1000, Some(39)),
    ("sdmmc",           "esp32s3_sdmmc",           0x6002_8000, 0x1000, Some(36)),
    ("lcd_cam",         "esp32s3_lcd_cam",         0x6004_1000, 0x1000, Some(24)),
    ("usb_otg",         "esp32s3_usb_otg",         0x6008_0000, 0x1000, Some(23)),
    ("i2c0",            "esp32s3_i2c",             0x6001_3000, 0x1000, Some(42)),
    ("i2c1",            "esp32s3_i2c",             0x6002_7000, 0x1000, Some(43)),
    ("uart0_s3",        "esp32s3_uart",            0x6000_0000, 0x0100, Some(27)),
    ("uart1_s3",        "esp32s3_uart",            0x6001_0000, 0x0100, Some(28)),
    ("uart2_s3",        "esp32s3_uart",            0x6002_E000, 0x0100, Some(29)),
];
