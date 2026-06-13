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
use crate::peripherals::esp32s3::flash_xip::{new_mmu_table, Esp32s3MmuTable, FlashXipPeripheral};
use crate::peripherals::esp32s3::gpio::{Esp32s3Gpio, GpioObserver};
use crate::peripherals::esp32s3::i2c::{Esp32s3I2c, I2C0_BASE, I2C0_INTR_SOURCE_ID, I2C0_SIZE};
use crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix;
use crate::peripherals::esp32s3::io_mux::Esp32s3IoMux;
use crate::peripherals::esp32s3::rom_thunks::{self, RomThunkBank};
use crate::peripherals::esp32s3::system_stub::{EfuseStub, RtcCntlStub, SystemStub};
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::peripherals::esp32s3::tmp102::Tmp102;
use crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use crate::Bus;
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

/// Phase B compatibility shim. Delegates to `configure_xtensa_esp32s3`
/// with default options and discards the wiring (icache/dcache backings)
/// that callers from Phase B's CLI/Python paths don't yet consume.
pub fn configure_xtensa(bus: &mut SystemBus) -> XtensaLx7 {
    configure_xtensa_esp32s3(bus, &Esp32s3Opts::default()).cpu
}

/// Attach external devices declared in `manifest.external_devices` to an
/// ESP32-classic bus that was already set up by `configure_xtensa_esp32`.
///
/// Currently supports `ssd1680_tricolor_290` / `epd-2in9-tricolor` (the
/// Waveshare 2.9" tri-color e-paper panel on SPI3/VSPI).  Other device
/// types emit a `tracing::warn` and are skipped so that future labs with
/// additional devices don't break existing runs.
///
/// This is the canonical implementation; `crates/wasm/src/lib.rs` delegates
/// to it (the wasm crate no longer carries its own copy).
pub fn attach_esp32_external_devices(
    bus: &mut SystemBus,
    manifest: &labwired_config::SystemManifest,
) -> anyhow::Result<()> {
    use crate::peripherals::spi::SpiDevice;

    for ext in &manifest.external_devices {
        let cs_pin = ext
            .config
            .get("cs_pin")
            .and_then(|v| v.as_str())
            .unwrap_or("GPIO5")
            .to_string();
        let dc_pin = ext
            .config
            .get("dc_pin")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Build the panel for this block type. Both tri-color e-paper models
        // are SpiDevices driven over the real SPI3 peripheral; the only block-
        // specific bit is which controller's command set the model decodes.
        let mut panel: Box<dyn SpiDevice> = match ext.r#type.as_str() {
            "uc8151d_tricolor_290" | "epd-2in9-uc8151d" => {
                let mut p = crate::peripherals::components::Uc8151dTricolor290::new(cs_pin.clone());
                if let Some(dc) = &dc_pin {
                    p = p.with_dc_pin(dc.clone());
                }
                Box::new(p)
            }
            // GxEPD2_290_C90c (GDEY029Z90c / Waveshare 2.9" 3-color) is an
            // SSD1680-controller panel — see the GxEPD2 driver header
            // "Controller: SSD1680". It drives SSD1680 opcodes, so it maps to the
            // SSD1680 model, NOT UC8151D.
            "ssd1680_tricolor_290" | "epd-2in9-tricolor" | "gxepd2_290_c90c" => {
                let mut p = crate::peripherals::components::Ssd1680Tricolor290::new(cs_pin.clone());
                if let Some(dc) = &dc_pin {
                    p = p.with_dc_pin(dc.clone());
                }
                Box::new(p)
            }
            other => {
                tracing::warn!(
                    "ESP32 external_devices: unsupported type '{}' on '{}'; skipping",
                    other,
                    ext.id
                );
                continue;
            }
        };

        // Resolve the D/C GPIO to its (output-register address, bit) so the bus
        // can latch the real pin level before each transfer — silicon-accurate
        // command/data framing, no GxEPD2 thunk. Immutable bus borrow first.
        if let Some(dc) = &dc_pin {
            if let Some((odr_addr, bit)) = crate::bus::SystemBus::resolve_pin_odr_pub(bus, dc) {
                panel.set_dc_source(odr_addr, bit);
            } else {
                tracing::warn!(
                    "ESP32 external_devices: dc_pin '{}' on '{}' did not resolve to a GPIO; \
                     framing falls back to protocol-state inference",
                    dc,
                    ext.id
                );
            }
        }

        let idx = bus
            .find_peripheral_index_by_name(&ext.connection)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "External device '{}' references missing connection '{}'",
                    ext.id,
                    ext.connection
                )
            })?;
        let any = bus.peripherals[idx].dev.as_any_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "External device '{}' connection '{}' cannot be downcast",
                ext.id,
                ext.connection
            )
        })?;
        // ESP32-classic buses register `Esp32Spi` controllers (spi2/spi3);
        // ESP32-S3 buses register the GP-SPI model `Esp32s3Spi`
        // (spi2_s3/spi3_s3). Both expose the same `attach` surface, so a
        // manifest can wire an external device to either family's SPI.
        if let Some(spi) = any.downcast_mut::<crate::peripherals::esp32::spi::Esp32Spi>() {
            spi.attach(panel);
        } else if let Some(spi) =
            any.downcast_mut::<crate::peripherals::esp32s3::gpspi::Esp32s3Spi>()
        {
            spi.attach(panel);
        } else {
            anyhow::bail!(
                "External device '{}' connection '{}' is not an ESP32 SPI peripheral",
                ext.id,
                ext.connection
            );
        }
    }
    Ok(())
}

/// Register a minimum-viable ESP32 (classic, Xtensa LX6) memory map on
/// `bus` and return the CPU.  Reuses `XtensaLx7` for the CPU — LX6 is a
/// near-subset of LX7 for the instructions a demo firmware uses (base
/// ALU, windowed registers, branches, loads/stores).  Real LX6-only
/// firmware that hits LX7-extension opcodes would need a proper LX6
/// CPU type; the on-the-line demo doesn't.
///
/// What's wired:
///   * IRAM (SRAM0, instruction view) at 0x4008_0000
///   * DRAM (SRAM2, data view)        at 0x3FFB_0000
///   * Flash XIP (I-cache)            at 0x400D_0000
///   * Flash XIP (D-cache alias)      at 0x3F40_0000
///   * ROM0 (Espressif boot ROM)      at 0x4000_0000
///   * UART0 (STM32F1-style layout)   at 0x3FF4_0000
///
/// What's NOT wired (silicon has these but they're out of scope for
/// the hello-world / survival-test slice):
///   * Wi-Fi MAC, Bluetooth controller, RTC, eFuse, GPIO matrix,
///     SPI0/SPI1/SPI2/SPI3, I²C0/I²C1, TIMG0/TIMG1, second LX6 core,
///     ULP coprocessor, hardware crypto.
///
/// UART0 caveat: the existing `peripherals::uart::Uart` defaults to
/// the STM32F1 register layout (SR @ 0x00, DR @ 0x04).  Real ESP32
/// UART places its TX/RX FIFO at offset 0x00, not 0x04.  The demo
/// firmware in `firmware-esp32-demo` writes to the STM32F1 DR offset
/// so the simulator's UART model emits cleanly; a dedicated
/// `UartRegisterLayout::Esp32` variant is the follow-up that would
/// let unmodified Espressif firmware run.
pub fn configure_xtensa_esp32(bus: &mut SystemBus) -> XtensaLx7 {
    use crate::peripherals::uart::Uart;

    // Same rationale as configure_xtensa_esp32s3: drop the seeded STM32
    // peripherals and disable Cortex-M bit-band — neither applies to Xtensa.
    bus.peripherals.clear();
    bus.bit_band_enabled = false;

    // IRAM (SRAM0, 128 KiB).
    bus.add_peripheral(
        "iram",
        0x4008_0000,
        0x20000,
        None,
        Box::new(RamPeripheral::new(0x20000)),
    );
    // BROM `.data` region (SRAM2 lower window). The Espressif BROM ELF
    // places ~1.3 KiB of `.data` at 0x3FFADAFC, just below the firmware
    // DRAM base. Mapping this keeps BROM init from bus-faulting on its
    // own globals before it touches firmware DRAM. (The 0x3FF9_xxxx data
    // alias of BROM rodata is mapped further down as `brom_data`.)
    bus.add_peripheral(
        "brom_low_data",
        0x3FFA_0000,
        0xE000,
        None,
        Box::new(RamPeripheral::new(0xE000)),
    );

    // SDIO slave block — SLC + HOST_SLC + SDMMC host. The ESP32 BROM's
    // `slc_init_attach` / `slc_set_host_io_max_window` touch these regs
    // during early init regardless of whether SDIO is actually used. We
    // don't model SDIO; plain RAM stubs catch the writes, and HOST_SLC
    // uses a smart stub that auto-sets its FSM-done bit so the BROM's
    // poll loop on offset 0x40 exits on the first read.
    // CHEAT(STUB): SDIO SLC peripheral faked as plain RAM — real: model the SLC
    // registers/DMA. (host_slc below is a smarter FSM stub.) See FIDELITY.md §D.
    bus.add_peripheral(
        "slc",
        0x3FF4_B000,
        0x1000,
        None,
        Box::new(RamPeripheral::new(0x1000)),
    );
    bus.add_peripheral(
        "host_slc",
        0x3FF5_8000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::sdio_stub::HostSlc::new()),
    );
    // CHEAT(STUB): SDMMC host peripheral faked as plain RAM — real: model the
    // SDMMC controller registers. See FIDELITY.md §D.
    bus.add_peripheral(
        "sdmmc_host",
        0x3FF5_5000,
        0x1000,
        None,
        Box::new(RamPeripheral::new(0x1000)),
    );
    // DRAM (SRAM2, 200 KiB) — full SRAM2 range 0x3FFAE000–0x3FFE0000.
    // Arduino-ESP32's startup zeroes .bss starting at 0x3FFAE291 (within
    // SRAM2 but below the 0x3FFB0000 region our hand-rolled Rust
    // firmware uses), so we map the wider region to keep both happy.
    bus.add_peripheral(
        "dram",
        0x3FFA_E000,
        0x32000,
        None,
        Box::new(RamPeripheral::new(0x32000)),
    );

    // SRAM1 (128 KiB, data-view) — Arduino-ESP32 places its initial stack
    // near 0x3FFE_0000 and overflows back into SRAM2 from there. Maps
    // 0x3FFE_0000–0x4000_0000 (the whole SRAM1 data-view window).
    bus.add_peripheral(
        "sram1",
        0x3FFE_0000,
        0x20000,
        None,
        Box::new(RamPeripheral::new(0x20000)),
    );
    // Flash XIP, instruction window (4 MiB).
    bus.add_peripheral(
        "flash_icache",
        0x400D_0000,
        0x400000,
        None,
        Box::new(RamPeripheral::new(0x400000)),
    );
    // Flash XIP, data-cache alias (4 MiB at a different virtual base).
    bus.add_peripheral(
        "flash_dcache",
        0x3F40_0000,
        0x400000,
        None,
        Box::new(RamPeripheral::new(0x400000)),
    );

    // Synthesize the `esp_image_header` at the start of the data XIP
    // window. On silicon the flash MMU maps the app partition's first
    // page (flash 0x10000, beginning with the 24-byte image header) to
    // 0x3F40_0000, so the header's 0xE9 magic is visible there. The sim
    // loads ELF *segments* (DROM data starts at 0x3F40_0020), leaving the
    // 32-byte header slot empty — which reads as 0. ESP-IDF >= 5.x's
    // `system_early_init` self-checks `*(uint8_t*)0x3F40_0000 == 0xE9`
    // and `abort()`s with "Invalid app image header" otherwise (older
    // cores lacked this check, so the gap surfaced only on newer
    // arduino-esp32). We reconstruct a minimal valid ESP32 header; only
    // the magic is validated at runtime (the BROM/bootloader that would
    // consume the rest is modeled as already-done), but the remaining
    // fields are filled with sane values for faithfulness. The ELF load
    // that follows never touches 0x3F40_0000..0x3F40_001F, so this
    // persists.
    const ESP32_IMAGE_HEADER: [u8; 24] = [
        0xE9, // magic
        0x03, // segment_count
        0x02, // spi_mode = DIO
        0x10, // spi_speed (40 MHz) | spi_size (2 MB)
        0x00, 0x00, 0x00, 0x00, // entry_addr (unused post-BROM)
        0xEE, // wp_pin (disabled)
        0x00, 0x00, 0x00, // spi_pin_drv[3]
        0x00, 0x00, // chip_id = ESP32 (0)
        0x00, // min_chip_rev (deprecated)
        0x00, 0x00, // min_chip_rev_full
        0x00, 0x00, // max_chip_rev_full
        0x00, 0x00, 0x00, 0x00, // reserved[4]
        0x00, // hash_appended
    ];
    for (i, &b) in ESP32_IMAGE_HEADER.iter().enumerate() {
        let _ = bus.write_u8(0x3F40_0000 + i as u64, b);
    }

    // External SRAM (PSRAM) data view at 0x3F800000-0x3FC00000.
    // Arduino-ESP32's startup probes this region during PSRAM
    // detection — accesses should be tolerable even on chips without
    // PSRAM (reads back 0). 4 MiB stub.
    bus.add_peripheral(
        "psram",
        0x3F80_0000,
        0x400000,
        None,
        Box::new(RamPeripheral::new(0x400000)),
    );
    // ROM0 (Espressif boot ROM, 448 KiB). RomThunkBank — same backing
    // store as a RamPeripheral but lets us pre-fill specific addresses
    // with BREAK 1,14 so the CPU's BREAK exec arm dispatches a Rust thunk
    // when esp-hal calls a BROM function (rtc_get_reset_reason, etc).
    let mut rom_bank = rom_thunks::RomThunkBank::new(0x4000_0000, 0x70000);
    // ESP32-classic BROM function addresses (per ESP-IDF rom/esp32.rom.ld).
    // Returning 0 means "POWERON_RESET" — adequate for first-boot init.
    // ESP32-classic BROM thunks (addresses per ESP-IDF rom/esp32.rom.ld).
    rom_bank.register(0x4000_81d4, rom_thunks::rtc_get_reset_reason);
    rom_bank.register(0x4000_2a40, rom_thunks::nop_return_zero); // Cache_Read_Disable
    rom_bank.register(0x4000_29ac, rom_thunks::nop_return_zero); // Cache_Read_Enable
                                                                 // libc-equivalents the firmware links against ROM copies of:
    rom_bank.register(0x4000_c260, rom_thunks::rom_memcmp);
    rom_bank.register(0x4000_c2c8, rom_thunks::rom_memcpy);
    rom_bank.register(0x4000_c3c0, rom_thunks::rom_memmove);
    rom_bank.register(0x4000_c44c, rom_thunks::rom_memset);
    // BROM helpers esp-hal's `esp32_init` calls via a jump table. We don't
    // model the per-pin defaults they apply — returning 0 is safe because
    // our sim doesn't enforce IO_MUX pre-state.
    rom_bank.register(0x4000_8534, rom_thunks::nop_return_zero); // ets_delay_us
    rom_bank.register(0x4000_8550, rom_thunks::nop_return_zero); // ets_update_cpu_frequency
                                                                 // ets_get_cpu_frequency() — returns CPU freq in MHz. We don't model
                                                                 // clock-tree changes so return the post-init default of 240 MHz.
    rom_bank.register(0x4000_855c, rom_thunks::rom_cpu_freq_240mhz);
    // ets_get_detected_xtal_freq() — returns XTAL freq in MHz. Return
    // 40 (matches the RTC_APB_FREQ_REG 0x0050_0050 encoding the RtcCntl
    // peripheral seeds at construction).
    rom_bank.register(0x4000_8588, rom_thunks::rom_xtal_freq_40mhz);
    // ets_printf — formats and writes to UART. Reuse the S3 thunk.
    rom_bank.register(0x4000_7d54, rom_thunks::ets_printf);
    // esp_rom_spiflash_config_clk — configures flash SPI clock divider.
    // No-op in sim; returns 0 (success).
    rom_bank.register(0x4006_2bc8, rom_thunks::nop_return_zero);
    rom_bank.register(0x4000_9200, rom_thunks::nop_return_zero); // (unnamed esp32_init helper)
    rom_bank.register(0x4000_4348, rom_thunks::nop_return_zero); // rom_i2c_writeReg vicinity
    rom_bank.register(0x4000_41a4, rom_thunks::nop_return_zero); // rom_i2c_writeReg
                                                                 // Cache control — esp-hal pokes these during boot. We don't model
                                                                 // flash cache state so all four are no-ops.
    rom_bank.register(0x4000_9a14, rom_thunks::nop_return_zero); // Cache_Flush_rom
    rom_bank.register(0x4000_9a84, rom_thunks::nop_return_zero); // Cache_Read_Enable_rom
    rom_bank.register(0x4000_9ab8, rom_thunks::nop_return_zero); // Cache_Read_Disable_rom
    rom_bank.register(0x4000_95e0, rom_thunks::nop_return_zero); // cache_flash_mmu_set_rom
    rom_bank.register(0x4000_97f4, rom_thunks::nop_return_zero); // cache_sram_mmu_set_rom
                                                                 // GPIO ROM helpers — Arduino-ESP32 uses these to set up VSPI pins.
                                                                 // No-op in sim (our Esp32Gpio/Esp32Spi peripherals accept signals
                                                                 // directly without IO_MUX-state enforcement).
    rom_bank.register(0x4000_9edc, rom_thunks::nop_return_zero); // esp_rom_gpio_connect_in_signal
    rom_bank.register(0x4000_9fdc, rom_thunks::nop_return_zero); // esp_rom_gpio_pad_select_gpio
                                                                 // MMU / cache setup helpers — discovered iteratively while booting
                                                                 // the the reference firmware Arduino-ESP32 binary in sim. All no-ops because the
                                                                 // sim's flash XIP peripheral is a flat RamPeripheral, no MMU model.
    rom_bank.register(0x4000_95a4, rom_thunks::nop_return_zero); // mmu_init
                                                                 // libgcc helpers — Arduino-ESP32 links against ROM copies for
                                                                 // hot paths (flash header parsing reads big-endian values).
    rom_bank.register(0x4006_4ae0, rom_thunks::rom_bswapsi2); // __bswapsi2
    rom_bank.register(0x4006_4b08, rom_thunks::rom_bswapdi2); // __bswapdi2
                                                              // libgcc 64-bit math helpers (in BROM at 0x4000c8xx).
    rom_bank.register(0x4000_c818, rom_thunks::rom_ashldi3); // __ashldi3
    rom_bank.register(0x4000_c830, rom_thunks::rom_ashrdi3); // __ashrdi3
    rom_bank.register(0x4000_c84c, rom_thunks::rom_lshrdi3); // __lshrdi3
    rom_bank.register(0x4000_ca84, rom_thunks::rom_divdi3); // __divdi3
    rom_bank.register(0x4000_cd4c, rom_thunks::rom_moddi3); // __moddi3
    rom_bank.register(0x4000_cff8, rom_thunks::rom_udivdi3); // __udivdi3
    rom_bank.register(0x4000_d280, rom_thunks::rom_umoddi3); // __umoddi3
    rom_bank.register(0x4000_c7e8, rom_thunks::rom_clzsi2); // __clzsi2
    rom_bank.register(0x4000_c7f0, rom_thunks::rom_ctzsi2); // __ctzsi2
                                                            // esp_crc8 — used by get_efuse_factory_mac to validate the MAC blob
                                                            // against the stored CRC byte. Dallas/Maxim 1-Wire CRC-8 algorithm.
    rom_bank.register(0x4005_d144, rom_thunks::rom_esp_crc8);
    // SPI flash / eFuse helpers — used by Arduino-ESP32's flash init.
    rom_bank.register(0x4000_8658, rom_thunks::nop_return_zero);
    // _xtos_set_intlevel(level) -> prev. Sets PS.INTLEVEL to `level`,
    // returns the previous value. FreeRTOS critical-section exit relies
    // on this to drop INTLEVEL back so pending IRQs (timer tick, FROM_CPU
    // crosscore IPI) can be delivered.
    rom_bank.register(0x4000_bfdc, rom_thunks::xtos_set_intlevel);
    // Interrupt-matrix + APP_CPU setup helpers (ESP32-classic BROM).
    // We don't model the second core or the interrupt matrix in this sim,
    // so noop-return is safe.
    rom_bank.register(0x4000_681c, rom_thunks::esp_rom_route_intr_matrix); // intr_matrix_set / esp_rom_route_intr_matrix
    rom_bank.register(0x4000_689c, rom_thunks::ets_set_appcpu_boot_addr); // releases APP_CPU
                                                                          // UART putc / printf install hooks — called by call_start_cpu1 to
                                                                          // wire CPU 1's stdout. We don't model UART output, so no-op.
                                                                          // BROM newlib syscalls — ESP-IDF >= 5.x console/stdio VFS init
                                                                          // (console_open) calls these; unmodeled ROM pages would fault. They
                                                                          // trampoline through the firmware's syscall table to esp_vfs_*.
    rom_bank.register(0x4000_178c, rom_thunks::rom_open); // newlib open
    rom_bank.register(0x4000_1778, rom_thunks::rom_close); // newlib close
    rom_bank.register(0x4000_17dc, rom_thunks::rom_read); // newlib read
    rom_bank.register(0x4000_181c, rom_thunks::rom_write); // newlib write
    rom_bank.register(0x4000_7d18, rom_thunks::nop_return_zero); // ets_install_putc1
    rom_bank.register(0x4000_7d28, rom_thunks::nop_return_zero); // ets_install_uart_printf
    rom_bank.register(0x4000_7d38, rom_thunks::nop_return_zero); // ets_install_putc2
    rom_bank.register(0x4000_9028, rom_thunks::nop_return_zero); // uart_tx_switch
    rom_bank.register(0x4000_9024, rom_thunks::nop_return_zero); // uart_tx_wait_idle
    rom_bank.register(0x4000_8fcc, rom_thunks::nop_return_zero); // uart_tx_flush
    rom_bank.register(0x4000_8fa8, rom_thunks::nop_return_zero); // uart_tx_one_char
    rom_bank.register(0x4000_9018, rom_thunks::nop_return_zero); // uart_tx_one_char2
    rom_bank.register(0x4000_05a4, rom_thunks::nop_return_zero); // cache_flush_rom
    rom_bank.register(0x4005_a980, rom_thunks::nop_return_zero); // Cache_Read_Disable
    rom_bank.register(0x4005_a917, rom_thunks::nop_return_zero); // Cache_Flush
    rom_bank.register(0x4005_aa10, rom_thunks::nop_return_zero); // Cache_Read_Enable
    rom_bank.register(0x4005_a888, rom_thunks::nop_return_zero); // esp_rom_spiflash_attach
                                                                 // intr_matrix_set is at 0x4000_681c (above); cpu1 calls intr_matrix_set
                                                                 // for its own intr table — same thunk works since we don't model the
                                                                 // interrupt matrix per-CPU.
                                                                 // GPIO matrix routing helpers — used by Arduino's spiAttach{SCK,MOSI,MISO}
                                                                 // and HardwareSerial pin attach. We don't model the GPIO matrix; signals
                                                                 // routed via SPI3 controller flow directly to attached SPI devices.
                                                                 // gpio_matrix_in (0x4000_9edc) is the same BROM entry already registered
                                                                 // above as esp_rom_gpio_connect_in_signal — just two ABI-compatible names
                                                                 // for the same function. Only register the new alias (gpio_matrix_out).
    rom_bank.register(0x4000_9f0c, rom_thunks::nop_return_zero); // gpio_matrix_out

    // ESP-IDF partition-table verification uses ROM MD5. Stubbing all three
    // entry points as no-ops makes verify_data_checksum() compute a zero
    // hash; partitions with `verify_checksum=false` (default for the standard
    // factory partition table) sail through. Real silicon: BROM at these
    // addresses per esp32.rom.ld.
    rom_bank.register(0x4005_da7c, rom_thunks::nop_return_zero); // esp_rom_md5_init
    rom_bank.register(0x4005_da9c, rom_thunks::nop_return_zero); // esp_rom_md5_update
    rom_bank.register(0x4005_db1c, rom_thunks::nop_return_zero); // esp_rom_md5_final
    bus.add_peripheral("rom", 0x4000_0000, 0x70000, None, Box::new(rom_bank));
    // UART0 — STM32F1 layout for now (see caveat above).
    bus.add_peripheral("uart0", 0x3FF4_0000, 0x100, None, Box::new(Uart::new()));

    // SPI0 / SPI1 — flash SPI controllers used by the BROM during boot.
    // Sim doesn't model the flash MMU, but Arduino-ESP32's
    // `bootloader_flash_execute_command_common` polls SPI_CMD_REG until
    // it clears (real hardware does this asynchronously). Reusing our
    // Esp32Spi controller gives us the same auto-clear CMD.USR
    // semantics, even with no attached devices — bytes streamed into
    // the FIFO just go nowhere, which is fine since we don't model
    // flash content reads.
    bus.add_peripheral(
        "spi0",
        0x3FF4_3000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::spi::Esp32Spi::new()),
    );
    bus.add_peripheral(
        "spi1",
        0x3FF4_2000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::spi::Esp32Spi::new()),
    );

    // GPIO controller (TRM §4.10). The e-paper lab routes CS/RST/DC/BUSY
    // through this peripheral; SCK/MOSI flow through SPI3 below.
    bus.add_peripheral(
        "gpio",
        0x3FF4_4000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::gpio::Esp32Gpio::new()),
    );

    // SPI3 / VSPI (TRM §7). Default pinmux puts SCK on GPIO18, MOSI on
    // GPIO23, CS on GPIO5 — matches the Waveshare e-paper module wiring.
    // We don't model the IO_MUX/GPIO matrix routing; bytes flowing through
    // the SPI3 controller go straight to its attached devices.
    bus.add_peripheral(
        "spi3",
        0x3FF6_5000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::spi::Esp32Spi::new()),
    );

    // DPORT (TRM v5.0 §6 + §7). Real ESP32-classic peripheral — seeds
    // PERIP_CLK_EN with all bits set (we treat every peripheral as live;
    // simpler than tracking gating), PERIP_RST_EN with 0 (nothing in
    // reset), and CPU_PER_CONF with 0 (undivided CPU clock — matches
    // silicon reset value). Every other offset reads as zero until
    // written, including DPORT_APPCPU_CTRL_B at 0x3FF0_0030 — Arduino-ESP32's
    // `system_early_init` checks that register to decide whether to bring
    // up the second core, and zero means "skip the bringup path" (which
    // is what we want, since APP_CPU isn't modeled and the bringup loop
    // would spin forever waiting on `s_cpu_up`).
    //
    // Writes to the cross-core IPI region (CPU_INTR_FROM_CPU_0..3 at
    // 0xDC..0xE8 and PRO/APP_INTR_FROM_CPU_0..3 at 0x164..0x174) are
    // observable on subsequent reads — the WASM IPI bridge in
    // `crates/wasm/src/lib.rs::step_with_esp32_aids` depends on that
    // contract.
    //
    // MUST register BEFORE the analog-AHB catch-all stub below: SystemBus
    // dispatches by first-registered-wins on overlapping ranges, and we
    // want the 4 KiB DPORT window to win over any wider stub.
    bus.add_peripheral(
        "dport",
        crate::peripherals::esp32::dport::Dport::BASE as u64,
        crate::peripherals::esp32::dport::Dport::SIZE as u64,
        None,
        Box::new(crate::peripherals::esp32::dport::Dport::new()),
    );

    // SHA hardware accelerator (TRM §24) at 0x3FF0_3000. Real FIPS-180-4
    // SHA-1/SHA-256 block compression so firmware digests match silicon
    // instead of round-tripping zeros through the analog-AHB catch-all.
    // MUST register BEFORE dport_analog_ahb (first-registered-wins; the
    // 0x3FF0_1000..0x3FF1_FFFF stub would otherwise shadow this window).
    bus.add_peripheral(
        "sha",
        crate::peripherals::esp32::sha::Sha::BASE as u64,
        crate::peripherals::esp32::sha::Sha::SIZE as u64,
        None,
        Box::new(crate::peripherals::esp32::sha::Sha::new()),
    );

    // Analog AHB / reserved region immediately above DPORT
    // (0x3FF0_1000..0x3FF1_FFFF, 60 KiB). Arduino-ESP32's startup touches
    // a handful of analog calibration registers in this window; nothing
    // here has documented semantics in scope for the model, so a plain
    // read-as-zero round-trip stub satisfies the access pattern.
    bus.add_peripheral(
        "dport_analog_ahb",
        0x3FF0_1000,
        0x1_F000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::SystemStub::new()),
    );

    // IO_MUX (TRM §4.11). Firmware configures pin function + drive strength
    // here before VSPI/GPIO signals reach the package pins. Sim doesn't
    // route through IO_MUX — SPI bytes go straight to attached devices —
    // so we just round-trip writes.
    bus.add_peripheral(
        "io_mux",
        0x3FF4_9000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::SystemStub::new()),
    );

    // RTC_CNTL (TRM §13). Real ESP32-classic peripheral — seeds POWERON_RESET
    // for both cores at construction, pre-loads RTC_APB_FREQ_REG with the
    // 40 MHz encoding (0x0050_0050) so Arduino-ESP32's XTAL probe finds a
    // sane value without needing a wasm-layer fake-write, and exposes the
    // monotonic slow-counter via TIME0/TIME1 reads. STORE0..3 round-trip
    // as retention scratch words; ANA_CONF / DIG_PWC / BIAS_CONF accept
    // any value (no analog domain modeled).
    //
    // Size 0x200 covers the documented register window 0x3FF4_8000..0x3FF4_80FC
    // plus the OPTIONS alias range up to 0x3FF4_8200. RTC_IO at 0x3FF4_8400
    // is registered separately by the catch-all stub block below.
    bus.add_peripheral(
        "rtc_cntl",
        0x3FF4_8000,
        0x200,
        None,
        Box::new(crate::peripherals::esp32::rtc_cntl::RtcCntl::new()),
    );

    // TIMG0 / TIMG1 — ESP32-classic Timer Group (TRM §16). Per-group
    // 64-bit T0/T1 general-purpose counters, watchdog, RTC calibration.
    // Preserves the auto-RDY-on-START behavior of the older `TimgStub`
    // so `rtc_clk_wait_for_slow_cycle` still completes in one iteration,
    // and adds monotonic counter reads so ESP-IDF's timer-state probes
    // see forward progress. Interrupt firing is intentionally deferred.
    bus.add_peripheral(
        "timg0",
        0x3FF5_F000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::timg::Timg::new(0x3FF5_F000)),
    );
    bus.add_peripheral(
        "timg1",
        0x3FF6_0000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::timg::Timg::new(0x3FF6_0000)),
    );

    // EFUSE — ESP32 BROM and esp-hal read BLK0 (MAC + chip_revision)
    // during reset-handler init. Returning a coherent rev3 + non-zero
    // MAC unblocks the ILL.N stall at PC 0x4000fdd3 on cold boot.
    //
    // Documented register range ends at DEC_STATUS (0x11C in
    // ESP-IDF's efuse_reg.h), but the address-decode window is the
    // standard 4 KiB peripheral page — BROM probes beyond 0x100 and
    // a smaller size triggers a "memory access violation". Keep the
    // full 4 KiB so unmapped offsets read as 0 (== unblown fuse).
    bus.add_peripheral(
        "efuse",
        0x3FF5_A000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::efuse::Efuse::new()),
    );

    // SYSCON (TRM §13.2) — system controller. Owns SYSCLK_CONF, TICK_CONF,
    // SARADC_CTRL, FRONT_END_MEM_PD, and the RND_DATA TRNG output the BROM
    // samples. Seeds TICK_CONF with XTAL_TICK_NUM=39 (40 MHz / 1 MHz - 1)
    // and SYSCLK_CONF with the XTAL-selected reset value (0). Sits at the
    // first 0x100 bytes of the 0x1000-byte APB-CTRL window; remaining
    // 0x100..0xFFF offsets fall through to the apb_ctrl stub registered
    // below (registration order wins on overlap — see bus.rs).
    bus.add_peripheral(
        "syscon",
        0x3FF6_6000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32::syscon::Syscon::new()),
    );

    // APB_CTRL — clock source select etc. Read/write stub. Covers the
    // 0x3FF6_6100..0x3FF6_6FFF tail of the APB-CTRL window; the 0x100
    // header is handled by the SYSCON peripheral above.
    bus.add_peripheral(
        "apb_ctrl",
        0x3FF6_6000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::SystemStub::with_unwritten_ones()),
    );

    // LEDC — LED PWM controller (TRM §14) at 0x3FF5_9000. Real model: 8 HS +
    // 8 LS channels over 4 HS / 4 LS timers, CONF1.DUTY_START latch so
    // ledc_get_duty()/ledcRead() read back the committed duty, derived duty
    // fraction + frequency. (PWM-edge emission to GPIO deferred.) Registered
    // before the catch-all loop below so its window wins (first-registered).
    bus.add_peripheral(
        "ledc",
        crate::peripherals::esp32::ledc::Ledc::BASE as u64,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::ledc::Ledc::new(
            crate::peripherals::esp32::ledc::Ledc::BASE,
        )),
    );

    // TWAI / CAN controller (TRM §27) at 0x3FF6_B000. SJA1000-derived:
    // reset-mode handshake, single-shot TX completion + IRQ, SELF_RX, and
    // the read-and-clear interrupt register so twai_driver_install()/
    // twai_start() make forward progress instead of faulting.
    bus.add_peripheral(
        "twai",
        0x3FF6_B000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::twai::Esp32Twai::new()),
    );

    // MCPWM0 — Motor Control PWM (TRM §16) at 0x3FF5_E000. Real model of the
    // PWM-generation path: per-timer period/prescale → frequency, per-operator
    // compare-A → duty, so mcpwm_get_duty()/mcpwm_get_frequency() read back
    // what was set and a bound actuator (servo/ESC) tracks the live duty.
    // Registered before the catch-all so its window wins over the pwm0 stub.
    bus.add_peripheral(
        "mcpwm0",
        crate::peripherals::esp32::mcpwm::Mcpwm::BASE as u64,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32::mcpwm::Mcpwm::new(
            crate::peripherals::esp32::mcpwm::Mcpwm::BASE,
        )),
    );

    // Catch-all stubs for the rest of the APB peripheral block
    // (0x3FF4A000–0x3FF6FFFF). ESP32 packs ~30 peripherals here
    // (RTC_IO, SAR ADC, I2S0/1, BB, UART1/2, I2C0/1, MCPWM, PCNT, RMT,
    // LEDC, etc). Most are touched briefly during esp-idf init; round-
    // trip stubs satisfy the access pattern even without modeling
    // any specific peripheral semantics.
    for (name, base) in [
        ("sdio_host", 0x3FF4_A000u64),
        ("rtcio", 0x3FF4_8400), // sub-range of RTC_CNTL window, leave 4 KiB span
        ("sar_adc", 0x3FF4_C000),
        ("i2s0", 0x3FF4_F000),
        ("uart1", 0x3FF5_0000),
        ("i2c0", 0x3FF5_3000),
        ("uhci0", 0x3FF5_4000),
        ("i2s1", 0x3FF6_D000),
        ("uart2", 0x3FF6_E000),
        // pwm0 (0x3FF5_E000) is now the real MCPWM0 model registered above.
        ("ledc2", 0x3FF6_8000),
        ("rmt", 0x3FF5_6000),
        ("pcnt", 0x3FF5_7000),
    ] {
        bus.add_peripheral(
            name,
            base,
            0x1000,
            None,
            Box::new(crate::peripherals::esp32s3::system_stub::SystemStub::new()),
        );
    }

    // RTC slow memory (8 KiB at 0x5000_0000). Arduino-ESP32 stores
    // sleep-mode reference counts and bootloader state here.
    bus.add_peripheral(
        "rtc_slow",
        0x5000_0000,
        0x2000,
        None,
        Box::new(RamPeripheral::new(0x2000)),
    );

    // WiFi MAC / PHY / RNG block (0x6000_0000..0x6004_3000 on real silicon).
    // The only register esp_random() touches at boot is RNG_DATA_REG at
    // 0x6003_5144 — a read returns 32 random bits. A round-tripping stub
    // satisfies the access; reads return zero (deterministic, but legal
    // — RNG semantics permit any value including all-zero).
    bus.add_peripheral(
        "wifi_mac_phy",
        0x6000_0000,
        0x4_3000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::SystemStub::new()),
    );

    // RTC fast memory (8 KiB at 0x3FF8_0000) — alias for instruction view.
    bus.add_peripheral(
        "rtc_fast",
        0x3FF8_0000,
        0x2000,
        None,
        Box::new(RamPeripheral::new(0x2000)),
    );

    // BROM data view (0x3FF9_0000-0x3FF9_FFFF on real silicon). Holds
    // newlib's `_ctype_` table at 0x3FF9_6354, used by isalnum / isspace /
    // tolower / toupper / etc. Without the table mapped, the firmware
    // faults when GxEPD2's logging or Arduino-ESP32's parsing calls into
    // ctype.h functions. Empty RAM region — uninitialized reads give 0,
    // so all characters classify as "not alnum / not space" which is wrong
    // but doesn't fault. Real silicon's BROM has the canonical table here.
    bus.add_peripheral(
        "brom_data",
        0x3FF9_0000,
        0x10000,
        None,
        Box::new(RamPeripheral::new(0x10000)),
    );

    // Phase 2B.3c (issue #192): every peripheral registered above is either
    // migrated to the event scheduler (uart0, gpio, rtc_cntl, timg0/1) or
    // inert (esp32 spi, efuse, syscon, and the SystemStub batch). So under the
    // `event-scheduler` feature the per-cycle peripheral walk is skipped
    // entirely — the ~2.4x throughput win. Verified: the full ESP32-classic
    // test suite passes with the walk disabled (e2e renders byte-perfect).
    // No effect with the feature off (the flag is only read there).
    bus.legacy_walk_disabled = true;

    XtensaLx7::new()
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
    // The proper model is selected whenever a real ROM is resolved — either
    // auto-provisioned from the installed toolchain or pinned via
    // LABWIRED_ESP32S3_ROM/_DROM. Without a real ROM blob we fall back to
    // fast-boot XIP with the thunk harness.
    let rom_images = crate::boot::esp32s3_rom::provision_rom_images();
    let proper_model = rom_images.is_some();
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
    // Backings exposed on Esp32s3Wiring. In the proper model both windows alias
    // one physical flash backing; in fast-boot they stay independent.
    let (icache_backing, dcache_backing) = if proper_model {
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
            eprintln!(
                "configure_xtensa_esp32s3: faithful ROM loaded ({} B IROM, {} B DROM) — real boot ROM, zero thunks",
                images.irom.len(),
                images.drom.len()
            );
            Esp32s3BootMode::Faithful
        }
        None => {
            // DROM (0x3FF0_0000) is intentionally not mapped in harness mode;
            // only the faithful path loads the real DROM image.
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
                "configure_xtensa_esp32s3: ESP32-S3 ROM not found; running in degraded harness mode \
                 — install the ESP toolchain (or set LABWIRED_ESP32S3_ROM_ELF) for faithful simulation"
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

    // ── GPIO / IO_MUX / Interrupt Matrix (Plan 3) ────────────────────────
    // These three specific peripherals MUST register BEFORE the catch-all
    // stubs below. SystemBus does first-match-wins iteration in
    // read_u8/write_u8, so a catch-all covering 0x600C_0000..0x600D_0000
    // (system) registered first would shadow intmatrix at 0x600C_2000.
    bus.add_peripheral(
        "gpio",
        0x6000_4000,
        0x800,
        None,
        Box::new(Esp32s3Gpio::new()),
    );
    bus.add_peripheral(
        "io_mux",
        0x6000_9000,
        0x100,
        None,
        Box::new(Esp32s3IoMux::new()),
    );
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

    // ── I²C0 + attached slaves ───────────────────────────────────────────
    // TMP102 @ 0x48 (Plan 4 demo). Opt-in PCA9685 @ 0x40 for the SpiceDispenser
    // board (LABWIRED_ESP32S3_PCA9685=1): the two servos hang off its PWM
    // channels, so attaching it lets the unmodified firmware's I²C dispense
    // path ACK and drive the servos instead of erroring on an empty bus.
    let mut i2c0 = Esp32s3I2c::new();
    i2c0.attach_slave(Box::new(Tmp102::new()));
    if std::env::var("LABWIRED_ESP32S3_PCA9685").is_ok() {
        i2c0.attach_slave(Box::new(
            crate::peripherals::components::pca9685::Pca9685::new(),
        ));
        eprintln!("configure_xtensa_esp32s3: attached PCA9685 @ 0x40 on I²C0");
    }
    bus.add_peripheral("i2c0", I2C0_BASE as u64, I2C0_SIZE, None, Box::new(i2c0));
    // Bind the I²C0 source ID through the intmatrix helper so esp-hal's
    // poll-then-read driver path doesn't depend on routing existing yet —
    // routing is firmware-controlled, this just leaves the source visible.
    let _ = I2C0_INTR_SOURCE_ID;

    // ── I²C1 ─────────────────────────────────────────────────────────────
    // Second controller, same faithful model, no slaves attached (an empty
    // bus NACKs every address, exactly like real hardware with nothing
    // wired to the I2C1 pins). Asserts ETS_I2C_EXT1_INTR_SOURCE (43).
    bus.add_peripheral(
        "i2c1",
        crate::peripherals::esp32s3::i2c::I2C1_BASE as u64,
        crate::peripherals::esp32s3::i2c::I2C1_SIZE,
        None,
        Box::new(Esp32s3I2c::with_intr_source(
            crate::peripherals::esp32s3::i2c::I2C1_INTR_SOURCE_ID,
        )),
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
    // ── SENS (DR_REG_SENS_BASE, 0x6000_8800) ─────────────────────────────
    // Faithful register file for the RTC-domain SAR-ADC / touch / TSENS
    // controller + immediate SAR1/SAR2 oneshot completion (the IDF oneshot
    // ADC driver busy-polls MEAS*_DONE_SAR). Registered AFTER the broad
    // round-tripping `rtc_cntl` stub above: by the bus router's
    // greatest-start-wins containment lookup this narrower window takes the
    // SENS block while the stub keeps serving the rest of the RTC range.
    // The window is exactly 0x400 — the gap up to RTC_I2C @ 0x6000_8C00 —
    // so the neighbouring RTC_IO / RTC_I2C blocks stay on the stub.
    bus.add_peripheral(
        "sens_s3",
        0x6000_8800,
        0x400,
        None,
        Box::new(crate::peripherals::esp32s3::sens::Esp32s3Sens::new()),
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

    // ── RNG data register (WDEV_RND_REG, 0x6003_507C) ────────────────────
    // Must register BEFORE the mmio_rest catch-all (which would return a
    // constant). The 2nd-stage bootloader reads this for entropy and retries
    // until it gets a non-zero random value; a constant makes that loop spin
    // forever. Modeled with a deterministic PRNG (reproducible per LabWired).
    bus.add_peripheral(
        "rng",
        0x6003_5000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::rng::Esp32s3Rng::new()),
    );

    // ── SHA accelerator (DR_REG_SHA_BASE, 0x6003_B000) ───────────────────
    // Real SHA-256 (sha2::compress256) so the boot ROM / bootloader can verify
    // the app image's appended hash. Without it the digest reads 0xFF and every
    // image is rejected. Registered before the mmio_rest catch-all.
    bus.add_peripheral(
        "sha",
        0x6003_B000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::sha::Esp32s3Sha::new()),
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

    // ── UART0/1/2 — real ESP32-S3 layout (soc/uart_reg.h) ────────────────
    // FIFO @0x0, STATUS @0x1C (TXFIFO_CNT[25:16]=0 → always room), CONF0 @0x20,
    // interrupt regs @0x04..0x10 (TXFIFO_EMPTY/TX_DONE level-source). Bases
    // DR_REG_UART{,1,2}_BASE = 0x6000_0000 / 0x6001_0000 / 0x6002_E000.
    // MUST register AFTER the wide low_mmio (0x6000_0000+0x7000) and mmio_rest
    // (0x6001_0000+0x40000) stubs: peripheral lookup resolves an overlap to the
    // LAST range whose start ≤ addr, so these narrower, same-start UART windows
    // only win when registered last. A separate, self-contained type from the
    // STM32 `Uart`, so the S3 layout never perturbs the ARM UART model. UART0
    // echoes TX to the host console (ESP-IDF / Arduino `Serial`); UART1/2 are
    // capture-only.
    bus.add_peripheral(
        "uart0_s3",
        0x6000_0000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::uart::Esp32s3Uart::new(
            true, 27,
        )),
    );
    bus.add_peripheral(
        "uart1_s3",
        0x6001_0000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::uart::Esp32s3Uart::new(
            false, 28,
        )),
    );
    bus.add_peripheral(
        "uart2_s3",
        0x6002_E000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::uart::Esp32s3Uart::new(
            false, 29,
        )),
    );

    // ESP32-S3 peripheral models. Factored into a separate unit so Stage 3 can
    // build them from a chip YAML via `SystemBus::from_config` instead. Called
    // after the catch-all stubs so each twin wins its own (higher-base) window.
    register_esp32s3_peripherals(bus);

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
fn register_esp32s3_peripherals(bus: &mut SystemBus) {
    // ── Peripheral digital twins (PCNT / LEDC / TIMG0 / TIMG1) ───────────
    // Registered AFTER the low_mmio/mmio_rest catch-alls so they win their
    // own (higher-base) windows via the bus's last-start lookup — same
    // discipline as the UARTs. Uniquely named (*_s3) so they never collide
    // with the classic-ESP32 timg0/timg1 in the name-keyed snapshot. The TIMG
    // twins model RTCCALICFG auto-RDY, which the bootloader's RTC-clock
    // calibration busy-polls (a plain stub leaves RDY clear and hangs boot).
    bus.add_peripheral(
        "pcnt",
        0x6001_7000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::pcnt::Esp32s3Pcnt::new(41)),
    );
    bus.add_peripheral(
        "ledc",
        0x6001_9000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::ledc::Esp32s3Ledc::new()),
    );
    bus.add_peripheral(
        "timg0_s3",
        0x6001_F000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::timer_group::Esp32s3TimerGroup::new(50, 240_000_000)),
    );
    bus.add_peripheral(
        "timg1_s3",
        0x6002_0000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::timer_group::Esp32s3TimerGroup::new(53, 240_000_000)),
    );

    // ── More twins (SAR-ADC / RMT / GP-SPI2 / GP-SPI3) ───────────────────
    // Same registration discipline (after the catch-alls, *_s3 names so they
    // never collide with the classic-ESP32 rmt/sar_adc/spi3 stubs in the
    // name-keyed snapshot). SAR-ADC is touched by the 2nd-stage bootloader's
    // RNG entropy enable, so its twin round-trips those writes + auto-completes
    // any conversion-done bit (never blocks boot).
    bus.add_peripheral(
        "rmt_s3",
        0x6001_6000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::rmt::Esp32s3Rmt::new(40)),
    );
    bus.add_peripheral(
        "spi2_s3",
        0x6002_4000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::gpspi::Esp32s3Spi::new(21)),
    );
    bus.add_peripheral(
        "spi3_s3",
        0x6002_5000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::gpspi::Esp32s3Spi::new(22)),
    );
    bus.add_peripheral(
        "sar_adc_s3",
        0x6004_0000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::sar_adc::Esp32s3SarAdc::new(64)),
    );

    // ── More twins (GDMA / I2S0 / I2S1 / TWAI) ───────────────────────────
    // After the catch-alls; *_s3 names for i2s avoid the classic-ESP32 i2s0/i2s1
    // stub name collision in the snapshot. GDMA moves real bytes: M2M
    // descriptor walks plus peripheral-coupled transfers (UART/UHCI0, SPI2/3,
    // I2S0/1 — routed by PERI_SEL); unmodeled peripheral ids keep the
    // auto-complete fallback (see gdma.rs module doc for the full contract).
    bus.add_peripheral(
        "gdma",
        0x6003_F000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::gdma::Esp32s3Gdma::new(66)),
    );
    bus.add_peripheral(
        "i2s0_s3",
        0x6000_F000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::i2s::Esp32s3I2s::new(25)),
    );
    bus.add_peripheral(
        "i2s1_s3",
        0x6002_D000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::i2s::Esp32s3I2s::new(26)),
    );
    bus.add_peripheral(
        "twai",
        0x6002_B000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::twai::Esp32s3Twai::new(37)),
    );
    // AES accelerator (DR_REG_AES_BASE, 0x6003_A000) — functionally-exact
    // FIPS-197 Rijndael (ECB block), ETS_AES_INTR_SOURCE = 77.
    bus.add_peripheral(
        "aes",
        0x6003_A000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::aes::Esp32s3Aes::new(77)),
    );
    // RSA accelerator (DR_REG_RSA_BASE, 0x6003_C000) — functionally-exact
    // bignum modular exponentiation, ETS_RSA_INTR_SOURCE = 76.
    bus.add_peripheral(
        "rsa",
        0x6003_C000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::rsa::Esp32s3Rsa::new(76)),
    );
    // HMAC accelerator (DR_REG_HMAC_BASE, 0x6003_E000) — HMAC-SHA256 over a
    // modeled efuse key; firmware polls QUERY_BUSY, so no interrupt source.
    bus.add_peripheral(
        "hmac",
        0x6003_E000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::hmac::Esp32s3Hmac::new(0)),
    );
    // Digital Signature (DR_REG_DIGITAL_SIGNATURE_BASE, 0x6003_D000) — polled
    // RSA-signature engine over modeled key params; no interrupt source.
    bus.add_peripheral(
        "ds",
        0x6003_D000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::ds::Esp32s3Ds::new(0)),
    );
    // MCPWM0 / MCPWM1 (DR_REG_PWM0/1_BASE) — motor-control PWM, two units.
    // ETS_PWM0_INTR_SOURCE = 38, ETS_PWM1_INTR_SOURCE = 39.
    bus.add_peripheral(
        "mcpwm0",
        0x6001_E000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::mcpwm::Esp32s3Mcpwm::new(38)),
    );
    bus.add_peripheral(
        "mcpwm1",
        0x6002_C000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::mcpwm::Esp32s3Mcpwm::new(39)),
    );
    // SD/MMC host (DR_REG_SDMMC_BASE, 0x6002_8000). ETS_SDIO_HOST_INTR_SOURCE
    // = 36. Liveness model of the register handshake (no physical card).
    bus.add_peripheral(
        "sdmmc",
        0x6002_8000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::sdmmc::Esp32s3Sdmmc::new(36)),
    );
    // LCD_CAM (DR_REG_LCD_CAM_BASE, 0x6004_1000). ETS_LCD_CAM_INTR_SOURCE = 24.
    bus.add_peripheral(
        "lcd_cam",
        0x6004_1000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::lcd_cam::Esp32s3LcdCam::new(24)),
    );
    // USB-OTG (DWC_otg core, DR_REG_USB_BASE, 0x6008_0000). ETS_USB_INTR_SOURCE
    // = 23. Liveness model of the DWC2 register interface (no host/PHY attached).
    bus.add_peripheral(
        "usb_otg",
        0x6008_0000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::usb_otg::Esp32s3UsbOtg::new(23)),
    );
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
    // ...and the read/write-with-mask variants (rom_i2c_*Reg_Mask), hit by the
    // BBPLL/clock tuning path on the full ESP-IDF image.
    bank.register(0x4000_5d54, rom_thunks::nop_return_zero); // esp_rom_regi2c_read_mask
    bank.register(0x4000_5d6c, rom_thunks::nop_return_zero); // esp_rom_regi2c_write_mask
                                                             // memcpy and __udivdi3 do real work — emulate them so the firmware
                                                             // doesn't get garbage from the boot-init copy paths.
    bank.register(0x4000_11f4, rom_thunks::rom_memcpy);
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
    // ROM cache-management API. A full ESP-IDF/Arduino image drives the whole
    // family during flash/MMU bring-up; the esp-hal path only touched
    // suspend/resume-DCache (0x4000_18b4 / 0x4000_18c0, registered above).
    // We model flash-XIP as identity-mapped, so every cache op — enable,
    // disable, freeze, occupy, MMU size/info — is a no-op for the simulator.
    // Addresses are ESP32-S3 ROM symbol-table values.
    for addr in [
        0x4000_186c, // Cache_Disable_ICache
        0x4000_1878, // Cache_Enable_ICache
        0x4000_1884, // Cache_Disable_DCache
        0x4000_1890, // Cache_Enable_DCache
        0x4000_189c, // Cache_Suspend_ICache
        0x4000_18a8, // Cache_Resume_ICache
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
}

// ── RamPeripheral helper ────────────────────────────────────────────────

/// Flat-array `Peripheral` used for IRAM + DRAM + flash XIP mappings.
///
/// Pub so `SystemBus::fetch_slice` can downcast and hand the CPU a raw
/// pointer into the backing buffer for the IRAM/flash fetch-cache fast
/// path (#119 Phase 1.2). The `data` field stays private; access is via
/// [`backing_ptr_len`] which returns a raw `(*const u8, usize)` pair.
///
/// INVARIANT: `data` is allocated once in [`new`] and never re-sized
/// (no `push`, `extend`, `resize`, `clear`). All read/write paths index
/// in-place via slice access, and `restore_runtime_snapshot` requires
/// the new bytes match the existing length. This stability is what makes
/// it safe to hand a raw `*const u8` to the CPU and re-use it across
/// many `step()` calls.
pub struct RamPeripheral {
    data: std::cell::RefCell<Vec<u8>>,
}

impl RamPeripheral {
    pub fn new(size: usize) -> Self {
        Self {
            data: std::cell::RefCell::new(vec![0u8; size]),
        }
    }

    /// Allocate `size` bytes and preload the low bytes from `image` (used to
    /// load a real ROM dump). The buffer stays exactly `size` (image is
    /// truncated/zero-padded), preserving the fixed-length INVARIANT.
    pub fn with_image(size: usize, image: &[u8]) -> Self {
        let mut buf = vec![0u8; size];
        let n = image.len().min(size);
        buf[..n].copy_from_slice(&image[..n]);
        Self {
            data: std::cell::RefCell::new(buf),
        }
    }

    /// Return a raw pointer + length to the backing buffer for the
    /// fetch-cache fast path. The pointer is stable for the lifetime
    /// of `self` because [`Self::new`] is the only allocation site and
    /// no path resizes the `Vec` (see struct-level INVARIANT).
    ///
    /// Reading through this pointer is safe iff no concurrent
    /// `borrow_mut()` is live AND `self` is not moved/dropped while
    /// the pointer is in use. The fetch-cache holds the pointer only
    /// across read-only `step()` calls; any bus write that lands in
    /// the cached range MUST invalidate the cache first so we never
    /// race a fetch against a `borrow_mut()`.
    pub fn backing_ptr_len(&self) -> (*const u8, usize) {
        let d = self.data.borrow();
        (d.as_ptr(), d.len())
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

    // Word/halfword bulk paths. The default trait impl decomposes each
    // multi-byte access into N byte calls — for Xtensa every instruction
    // fetch (read_u32 of IRAM) hits this 4 times per instruction, each
    // taking a fresh RefCell::borrow() + Vec bounds check. The single-shot
    // slice path here cuts each fetch from 4 borrows to 1.
    // See labwired-core#119 (JIT roadmap Phase 1.1).
    fn read_u16(&self, offset: u64) -> crate::SimResult<u16> {
        let d = self.data.borrow();
        let off = offset as usize;
        let bytes = d.get(off..off + 2);
        Ok(match bytes {
            Some(b) => u16::from_le_bytes([b[0], b[1]]),
            None => 0, // out-of-range reads return 0 to match the byte path
        })
    }
    fn read_u32(&self, offset: u64) -> crate::SimResult<u32> {
        let d = self.data.borrow();
        let off = offset as usize;
        let bytes = d.get(off..off + 4);
        Ok(match bytes {
            Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        })
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        let off = offset as usize;
        if let Some(slot) = d.get_mut(off..off + 2) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        let off = offset as usize;
        if let Some(slot) = d.get_mut(off..off + 4) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }

    /// Dump the backing buffer verbatim. Snapshot stays compact (a 200 KiB
    /// DRAM round-trips as 200 KiB on disk — bincode adds an 8-byte length
    /// prefix and that's it).
    fn runtime_snapshot(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        let mut d = self.data.borrow_mut();
        if bytes.len() != d.len() {
            return Err(crate::SimulationError::NotImplemented(format!(
                "RamPeripheral runtime snapshot size mismatch: expected {} bytes, got {}",
                d.len(),
                bytes.len()
            )));
        }
        d.copy_from_slice(bytes);
        Ok(())
    }

    /// Expose `&dyn Any` so `SystemBus::fetch_slice` can downcast and
    /// reach `backing_ptr_len` without a virtual-call detour.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use crate::Peripheral;

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

    /// Empirical routing proof for the layered 0x600C_0000 SYSTEM region.
    /// Verifies WHICH peripheral the bus router dispatches each probe offset to
    /// (by the distinct window each owns) and that behavior is correct:
    ///   * crosscore_ipi (size 0x10) still serves 0x030..0x03C,
    ///   * the faithful SYSTEM model (windows A/B) serves the register block
    ///     and `resolve_window(0x600C_0000)` (window A, NON-round-tripping),
    ///   * the big SystemStub (size 0x1_0000) still serves ≥ 0x600C_1000.
    #[test]
    fn system_register_block_routing() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        let ipi_base = crate::peripherals::esp32s3::crosscore_ipi::BASE; // 0x600C_0030
        let ipi_size = crate::peripherals::esp32s3::crosscore_ipi::SIZE; // 0x10

        // 1. resolve_window(0x600C_0000) must return the faithful model's
        //    window A (base 0x600C_0000, size 0x30) — NON-round-tripping, so
        //    the probe builds a clean baseline and credits the modeled
        //    registers. (The model is split into two windows straddling the
        //    crosscore_ipi hole; window A is what the probe resolves.)
        let (base, size) = bus.resolve_window(0x600C_0000).expect("SYSTEM window");
        assert_eq!(base, 0x600C_0000);
        assert_eq!(size, ipi_base - 0x600C_0000, "window A size = 0x30");

        // 2. crosscore_ipi (size 0x10) still serves 0x030..0x03C — the boot
        //    doorbell must NOT be shadowed by the SYSTEM model. The hole in the
        //    SYSTEM windows guarantees this regardless of the hint cache, so
        //    re-query after touching a neighbouring SYSTEM register to prove
        //    the cache does not pull it back into a SYSTEM window.
        let _ = bus.read_u32(0x600C_0008); // pollute hint with window A
        for off in [0x030u64, 0x034, 0x038, 0x03C] {
            let (b, s) = bus.resolve_window(0x600C_0000 + off).expect("ipi window");
            assert_eq!(
                (b, s),
                (ipi_base, ipi_size),
                "offset {off:#x} must route to crosscore_ipi (hole preserved)"
            );
        }

        // 3a. Window A serves an architected register (PERIP_CLK_EN0 @ 0x018):
        //     reads its HW reset value and round-trips a masked write.
        assert_eq!(
            bus.read_u32(0x600C_0018).unwrap(),
            0xF9C1_E06F,
            "PERIP_CLK_EN0 reads its HW-validated reset value"
        );
        bus.write_u32(0x600C_0018, 0x1234_5678).unwrap();
        assert_eq!(
            bus.read_u32(0x600C_0018).unwrap(),
            0x1234_5678,
            "PERIP_CLK_EN0 round-trips a write under its mask"
        );

        // 3b. Window B serves the high registers (RTC_FASTMEM_CONFIG @ 0x050,
        //     DATE @ 0xFFC) with correct absolute-offset translation.
        assert_eq!(
            bus.read_u32(0x600C_0050).unwrap(),
            0x7FF0_0000,
            "RTC_FASTMEM_CONFIG reset value (window B, base-offset translated)"
        );
        assert_eq!(
            bus.read_u32(0x600C_0FFC).unwrap(),
            0x0210_1220,
            "DATE constant (window B tail)"
        );

        // 3c. An unmapped offset inside window B reads as zero and does NOT
        //     round-trip (the anti-gaming property the coverage probe relies on).
        assert_eq!(bus.read_u32(0x600C_0100).unwrap(), 0, "unmapped reads 0");
        bus.write_u32(0x600C_0100, 0xFFFF_FFFF).unwrap();
        assert_eq!(
            bus.read_u32(0x600C_0100).unwrap(),
            0,
            "unmapped offset must NOT round-trip"
        );

        // 4. The big SystemStub (size 0x1_0000) still serves the region above
        //    the register block, e.g. 0x600C_1800 (interrupt-map / accelerator).
        let (b, s) = bus.resolve_window(0x600C_1800).expect("stub window");
        assert_eq!(
            (b, s),
            (0x600C_0000, 0x1_0000),
            "0x600C_1800 → big SystemStub"
        );
        bus.write_u32(0x600C_1800, 0xDEAD_BEEF).unwrap();
        assert_eq!(bus.read_u32(0x600C_1800).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn iram_writeable_and_readable() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        bus.write_u8(0x4037_0010, 0xAB).unwrap();
        assert_eq!(bus.read_u8(0x4037_0010).unwrap(), 0xAB);
    }

    /// Phase 1.2: `fetch_slice` MUST hand back a `&[u8]` that aliases
    /// the RAM peripheral's backing store, observe writes that go
    /// through the bus, and only cover RAM-backed regions.
    #[test]
    fn fetch_slice_aliases_iram_and_skips_non_ram() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        // IRAM range 0x4037_0000 + 0x40000. fetch_slice should cover
        // the whole region (~256 KiB) and observe a fresh byte write.
        let pc = 0x4037_0010u64;
        bus.write_u8(pc, 0x37).unwrap();
        let (start, end, slice) = bus.fetch_slice(pc).expect("IRAM fetch_slice");
        assert!(start <= pc && pc < end, "pc must lie in returned range");
        let off = (pc - start) as usize;
        assert_eq!(slice[off], 0x37, "slice must mirror current RAM byte");

        // Writes propagate without invalidating the slice itself —
        // the consumer is responsible for invalidating its cached
        // pointer when a write lands in-range. We just need to see
        // the new value through the same slice (vec is in place).
        bus.write_u8(pc, 0xA5).unwrap();
        let (start, _, slice) = bus.fetch_slice(pc).unwrap();
        assert_eq!(slice[(pc - start) as usize], 0xA5);

        // GPIO at 0x6000_4000 is not a RamPeripheral — slow path
        // must stay active there.
        assert!(
            bus.fetch_slice(0x6000_4000).is_none(),
            "GPIO must not serve fetch_slice"
        );
    }

    #[test]
    fn configure_registers_gpio_io_mux_intmatrix() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        let names: Vec<&str> = bus.peripherals.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"gpio"), "gpio missing; have: {names:?}");
        assert!(names.contains(&"io_mux"), "io_mux missing; have: {names:?}");
        assert!(
            names.contains(&"intmatrix"),
            "intmatrix missing; have: {names:?}"
        );
    }

    #[test]
    fn add_gpio_observer_installs_on_gpio_peripheral() {
        use crate::peripherals::esp32s3::gpio::GpioObserver;
        use std::sync::{Arc, Mutex};

        #[derive(Debug, Default)]
        struct CountObserver(Mutex<u32>);
        impl GpioObserver for CountObserver {
            fn on_pin_change(&self, _pin: u8, _from: bool, _to: bool, _sim_cycle: u64) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        let obs = Arc::new(CountObserver::default());
        wiring.add_gpio_observer(&mut bus, obs.clone());

        // Trigger a GPIO transition by writing OUT_W1TS bit 5 via the bus.
        // GPIO base 0x6000_4000, OUT_W1TS at offset 0x08.
        bus.write_u8(0x6000_4008, 0x20).unwrap(); // bit 5 = 0x20
        bus.write_u8(0x6000_4009, 0).unwrap();
        bus.write_u8(0x6000_400A, 0).unwrap();
        bus.write_u8(0x6000_400B, 0).unwrap();

        assert!(*obs.0.lock().unwrap() >= 1, "observer should have fired");
    }

    #[test]
    fn configure_registers_i2c0_with_tmp102_attached() {
        let mut bus = SystemBus::new();
        let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        // I2C0 should be present at 0x6001_3000.
        let names: Vec<_> = bus.peripherals.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"i2c0"), "i2c0 missing; have: {names:?}");

        // The attached TMP102 should respond at address 0x48 by setting
        // INT_NACK to 0 after a one-byte write probe.
        let i2c_idx = bus
            .peripherals
            .iter()
            .position(|p| p.name == "i2c0")
            .unwrap();
        let i2c_any = bus.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .expect("i2c0 should expose as_any_mut");
        let i2c = i2c_any
            .downcast_mut::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>()
            .expect("downcast to Esp32s3I2c");

        // Build a probe: RSTART; WRITE 1 (addr+W=0x90); STOP.
        // Opcodes per ESP32-S3 TRM § 29.5: 1=WRITE, 2=STOP, 6=RSTART.
        i2c.write_u32(0x58, 6u32 << 11).unwrap(); // RSTART (opcode 6)
        i2c.write_u32(0x5C, (1u32 << 11) | 1).unwrap(); // WRITE 1 byte
        i2c.write_u32(0x60, 2u32 << 11).unwrap(); // STOP (opcode 2)
        i2c.write_u32(0x1C, 0x90).unwrap(); // addr+W (DATA at 0x1c)
        i2c.write_u32(0x04, 1 << 5).unwrap(); // TRANS_START
        let int_raw = i2c.read_u32(0x20).unwrap();
        assert_eq!(
            int_raw & (1 << 11),
            0,
            "TMP102 attached at 0x48 must ACK; got INT_RAW=0x{int_raw:08x}"
        );
    }

    #[test]
    fn flash_xip_windows_have_independent_backings() {
        // Real silicon shares the SPI flash between both windows but each has
        // its own MMU page table; for fast-boot we model this as two distinct
        // backing buffers so that ELFs with .rodata at 0x3c000020 and .text at
        // 0x42000020 don't collide on the same physical offset. Force fast-boot
        // so the assertion is deterministic regardless of whether the host has
        // the ESP toolchain ROM installed (which would auto-select faithful mode).
        std::env::set_var("LABWIRED_ESP32S3_FASTBOOT", "1");
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        std::env::remove_var("LABWIRED_ESP32S3_FASTBOOT");
        wiring.icache_backing.lock().unwrap()[0] = 0xCA;
        wiring.dcache_backing.lock().unwrap()[0] = 0xFE;
        assert_eq!(bus.read_u8(0x4200_0000).unwrap(), 0xCA, "I-cache alias");
        assert_eq!(bus.read_u8(0x3C00_0000).unwrap(), 0xFE, "D-cache alias");
    }

    /// `configure_xtensa_esp32` + `attach_esp32_external_devices` must register
    /// `spi3` on the bus and attach an `Ssd1680Tricolor290` panel to it when the
    /// manifest declares an `ssd1680_tricolor_290` external device on `spi3`.
    ///
    /// This is the unit-level guard that the manifest/CLI path (which was
    /// previously broken — `config_error: external device 'epaper' references
    /// missing connection 'spi3'`) now wires up correctly.
    #[test]
    fn attach_esp32_external_devices_registers_spi3_and_epaper() {
        use labwired_config::{ExternalDevice, SystemManifest};
        use std::collections::HashMap;

        // Build a minimal manifest that declares the SSD1680 e-paper panel
        // on spi3 — matching the real `configs/systems/esp32-wroom-epaper.yaml`.
        let mut config = HashMap::new();
        config.insert(
            "cs_pin".to_string(),
            serde_yaml::Value::String("GPIO5".to_string()),
        );
        let manifest = SystemManifest {
            walk_deleted: false,
            schema_version: "1.0".to_string(),
            name: "test-esp32-epaper".to_string(),
            chip: "esp32.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            peripherals: vec![],
            external_devices: vec![ExternalDevice {
                id: "epaper".to_string(),
                r#type: "ssd1680_tricolor_290".to_string(),
                connection: "spi3".to_string(),
                config,
            }],
            board_io: vec![],
        };

        let mut bus = SystemBus::new();
        let _cpu = configure_xtensa_esp32(&mut bus);

        // spi3 must exist after configure_xtensa_esp32.
        assert!(
            bus.find_peripheral_index_by_name("spi3").is_some(),
            "spi3 must be registered by configure_xtensa_esp32"
        );

        // Attaching external devices must succeed (no 'missing connection' error).
        attach_esp32_external_devices(&mut bus, &manifest)
            .expect("attach_esp32_external_devices must not error for spi3 + epaper");

        // The SSD1680 panel must now be attached to spi3.
        let idx = bus
            .find_peripheral_index_by_name("spi3")
            .expect("spi3 still present after attach");
        let any = bus.peripherals[idx]
            .dev
            .as_any()
            .expect("spi3 supports as_any");
        let spi = any
            .downcast_ref::<crate::peripherals::esp32::spi::Esp32Spi>()
            .expect("spi3 is Esp32Spi");
        let panel_count = spi
            .attached_devices
            .iter()
            .filter(|d| {
                d.as_any()
                    .and_then(|a| {
                        a.downcast_ref::<crate::peripherals::components::Ssd1680Tricolor290>()
                    })
                    .is_some()
            })
            .count();
        assert_eq!(
            panel_count, 1,
            "exactly one Ssd1680Tricolor290 should be attached to spi3"
        );
    }

    /// S3 config wiring: `attach_esp32_external_devices` must also handle the
    /// ESP32-S3 GP-SPI model (`Esp32s3Spi`) — an S3 manifest wiring a device
    /// to `spi3_s3` previously errored with "not an ESP32 SPI peripheral"
    /// because only the classic `Esp32Spi` downcast was attempted.
    #[test]
    fn attach_esp32_external_devices_attaches_to_s3_gpspi() {
        use labwired_config::{ExternalDevice, SystemManifest};
        use std::collections::HashMap;

        let mut config = HashMap::new();
        config.insert(
            "cs_pin".to_string(),
            serde_yaml::Value::String("GPIO10".to_string()),
        );
        let manifest = SystemManifest {
            walk_deleted: false,
            schema_version: "1.0".to_string(),
            name: "test-esp32s3-epaper".to_string(),
            chip: "esp32s3.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            peripherals: vec![],
            external_devices: vec![ExternalDevice {
                id: "epaper".to_string(),
                r#type: "ssd1680_tricolor_290".to_string(),
                connection: "spi3_s3".to_string(),
                config,
            }],
            board_io: vec![],
        };

        // Register spi3_s3 exactly as the production S3 bring-up does
        // (configure_xtensa_esp32s3: Esp32s3Spi::new(22) @ 0x6002_5000),
        // without the full heavyweight S3 bus construction.
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "spi3_s3",
            0x6002_5000,
            0x100,
            None,
            Box::new(crate::peripherals::esp32s3::gpspi::Esp32s3Spi::new(22)),
        );

        attach_esp32_external_devices(&mut bus, &manifest)
            .expect("attach must succeed for an Esp32s3Spi connection");

        let idx = bus.find_peripheral_index_by_name("spi3_s3").unwrap();
        let spi = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<crate::peripherals::esp32s3::gpspi::Esp32s3Spi>()
            .expect("spi3_s3 is Esp32s3Spi");
        assert_eq!(
            spi.attached_device_count(),
            1,
            "exactly one panel attached to the S3 GP-SPI controller"
        );
    }

    /// `attach_esp32_external_devices` must return an error (not panic) when
    /// the manifest references a peripheral name that doesn't exist on the bus.
    #[test]
    fn attach_esp32_external_devices_errors_on_missing_connection() {
        use labwired_config::{ExternalDevice, SystemManifest};

        let manifest = SystemManifest {
            walk_deleted: false,
            schema_version: "1.0".to_string(),
            name: "test".to_string(),
            chip: "esp32.yaml".to_string(),
            memory_overrides: std::collections::HashMap::new(),
            peripherals: vec![],
            external_devices: vec![ExternalDevice {
                id: "epaper".to_string(),
                r#type: "ssd1680_tricolor_290".to_string(),
                connection: "spi99".to_string(), // does not exist
                config: std::collections::HashMap::new(),
            }],
            board_io: vec![],
        };

        let mut bus = SystemBus::new();
        let _cpu = configure_xtensa_esp32(&mut bus);

        let result = attach_esp32_external_devices(&mut bus, &manifest);
        assert!(
            result.is_err(),
            "should error when connection peripheral is missing"
        );
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("spi99"),
            "error message should name the missing peripheral; got: {msg}"
        );
    }
}
