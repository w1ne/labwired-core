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
use crate::peripherals::esp_xtensa_common::rom_thunks;
use crate::Bus;

mod esp32s3;
pub use esp32s3::*;

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
/// UART0/1/2 use the real ESP32 register layout (`peripherals::esp32::uart`):
/// TX/RX FIFO at offset 0x00, STATUS FIFO counts at `[7:0]`/`[23:16]`, the full
/// INT_RAW/ST/ENA/CLR set, and interrupt-matrix sources 34/35/36 — so
/// unmodified Espressif firmware (`uart_hal`, `HardwareSerial`, `ets_printf`)
/// runs against modeled registers instead of a thunk. (Was previously the
/// STM32F1-layout `peripherals::uart::Uart`, which only suited the demo
/// firmware that wrote to the STM32 DR offset.)
pub fn configure_xtensa_esp32(bus: &mut SystemBus) -> XtensaLx7 {
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
    // UART0 (Serial) echoes to the host console; UART1/2 are capture-only.
    // Interrupt-matrix sources: ETS_UART{0,1,2}_INTR_SOURCE = 34/35/36.

    // SPI0 / SPI1 — flash SPI controllers used by the BROM during boot.
    // Sim doesn't model the flash MMU, but Arduino-ESP32's
    // `bootloader_flash_execute_command_common` polls SPI_CMD_REG until
    // it clears (real hardware does this asynchronously). Reusing our
    // Esp32Spi controller gives us the same auto-clear CMD.USR
    // semantics, even with no attached devices — bytes streamed into
    // the FIFO just go nowhere, which is fine since we don't model
    // flash content reads.

    // GPIO controller (TRM §4.10). The e-paper lab routes CS/RST/DC/BUSY
    // through this peripheral; SCK/MOSI flow through SPI3 below.

    // SPI3 / VSPI (TRM §7). Default pinmux puts SCK on GPIO18, MOSI on
    // GPIO23, CS on GPIO5 — matches the Waveshare e-paper module wiring.
    // We don't model the IO_MUX/GPIO matrix routing; bytes flowing through
    // the SPI3 controller go straight to its attached devices.

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

    // SHA hardware accelerator (TRM §24) at 0x3FF0_3000. Real FIPS-180-4
    // SHA-1/SHA-256 block compression so firmware digests match silicon
    // instead of round-tripping zeros through the analog-AHB catch-all.
    // MUST register BEFORE dport_analog_ahb (first-registered-wins; the
    // 0x3FF0_1000..0x3FF1_FFFF stub would otherwise shadow this window).

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
        Box::new(crate::peripherals::esp_xtensa_common::system_stub::SystemStub::new()),
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
        Box::new(crate::peripherals::esp_xtensa_common::system_stub::SystemStub::new()),
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

    // TIMG0 / TIMG1 — ESP32-classic Timer Group (TRM §16). Per-group
    // 64-bit T0/T1 general-purpose counters, watchdog, RTC calibration.
    // Preserves the auto-RDY-on-START behavior of the older `TimgStub`
    // so `rtc_clk_wait_for_slow_cycle` still completes in one iteration,
    // and adds monotonic counter reads so ESP-IDF's timer-state probes
    // see forward progress. Interrupt firing is intentionally deferred.

    // EFUSE — ESP32 BROM and esp-hal read BLK0 (MAC + chip_revision)
    // during reset-handler init. Returning a coherent rev3 + non-zero
    // MAC unblocks the ILL.N stall at PC 0x4000fdd3 on cold boot.
    //
    // Documented register range ends at DEC_STATUS (0x11C in
    // ESP-IDF's efuse_reg.h), but the address-decode window is the
    // standard 4 KiB peripheral page — BROM probes beyond 0x100 and
    // a smaller size triggers a "memory access violation". Keep the
    // full 4 KiB so unmapped offsets read as 0 (== unblown fuse).

    // Classic-ESP32 peripheral models, built from the ESP32_PERIPHERALS
    // table via the esp32 factory (data-driven, mirrors esp32s3). syscon is
    // kept hand-wired below because it shares base 0x3FF6_6000 with the
    // apb_ctrl catch-all and must preserve that registration order.
    register_esp32_peripherals(bus);

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
        Box::new(
            crate::peripherals::esp_xtensa_common::system_stub::SystemStub::with_unwritten_ones(),
        ),
    );

    // LEDC — LED PWM controller (TRM §14) at 0x3FF5_9000. Real model: 8 HS +
    // 8 LS channels over 4 HS / 4 LS timers, CONF1.DUTY_START latch so
    // ledc_get_duty()/ledcRead() read back the committed duty, derived duty
    // fraction + frequency. (PWM-edge emission to GPIO deferred.) Registered
    // before the catch-all loop below so its window wins (first-registered).

    // TWAI / CAN controller (TRM §27) at 0x3FF6_B000. SJA1000-derived:
    // reset-mode handshake, single-shot TX completion + IRQ, SELF_RX, and
    // the read-and-clear interrupt register so twai_driver_install()/
    // twai_start() make forward progress instead of faulting.

    // MCPWM0 — Motor Control PWM (TRM §16) at 0x3FF5_E000. Real model of the
    // PWM-generation path: per-timer period/prescale → frequency, per-operator
    // compare-A → duty, so mcpwm_get_duty()/mcpwm_get_frequency() read back
    // what was set and a bound actuator (servo/ESC) tracks the live duty.
    // Registered before the catch-all so its window wins over the pwm0 stub.

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
            Box::new(crate::peripherals::esp_xtensa_common::system_stub::SystemStub::new()),
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
        Box::new(crate::peripherals::esp_xtensa_common::system_stub::SystemStub::new()),
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

/// Register the classic-ESP32 (LX6) peripheral models on `bus` from the
/// canonical [`ESP32_PERIPHERALS`] table via `peripherals::esp32::factory`.
/// Excludes `syscon`, which `configure_xtensa_esp32` keeps hand-wired so it
/// retains its registration order against the same-base `apb_ctrl` stub.
fn register_esp32_peripherals(bus: &mut SystemBus) {
    use crate::peripherals::esp32::factory;
    use labwired_config::PeripheralConfig;
    use std::collections::HashMap;
    for &(id, ty, base, size, irq) in ESP32_PERIPHERALS {
        if id == "syscon" {
            continue;
        }
        let mut config: HashMap<String, serde_yaml::Value> = HashMap::new();
        // uart0 echoes TX to the host console; uart1/2 are capture-only.
        if matches!(id, "uart1" | "uart2") {
            config.insert("echo_stdout".to_string(), serde_yaml::Value::Bool(false));
        }
        let cfg = PeripheralConfig {
            id: id.to_string(),
            r#type: ty.to_string(),
            base_address: base,
            size: None,
            irq,
            config,
        };
        let dev = factory::try_build(ty, &cfg)
            .unwrap_or_else(|| panic!("esp32 factory missing type {ty} for {id}"));
        bus.add_peripheral(id, base, size, None, dev);
    }
}

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

/// Canonical `(id, factory type, window base, window size, irq source)` for the
/// classic ESP32 (Xtensa LX6) peripheral models that `configure_xtensa_esp32`
/// installs by hand. The `peripherals::esp32::factory` source of truth, parallel
/// to [`ESP32S3_PERIPHERALS`]; proven equivalent to the hand-wired path by
/// `esp32_factory_descriptors_match_hardwired`.
#[allow(dead_code)]
#[rustfmt::skip]
pub(crate) const ESP32_PERIPHERALS: &[(&str, &str, u64, u64, Option<u32>)] = &[
    ("uart0",    "esp32_uart",     0x3FF4_0000, 0x0100, Some(34)),
    ("uart1",    "esp32_uart",     0x3FF5_0000, 0x0100, Some(35)),
    ("uart2",    "esp32_uart",     0x3FF6_E000, 0x0100, Some(36)),
    ("spi0",     "esp32_spi",      0x3FF4_3000, 0x1000, None),
    ("spi1",     "esp32_spi",      0x3FF4_2000, 0x1000, None),
    ("spi3",     "esp32_spi",      0x3FF6_5000, 0x1000, None),
    ("gpio",     "esp32_gpio",     0x3FF4_4000, 0x1000, None),
    ("dport",    "esp32_dport",    0x3FF0_0000, 0x1000, None),
    ("sha",      "esp32_sha",      0x3FF0_3000, 0x0100, None),
    ("rtc_cntl", "esp32_rtc_cntl", 0x3FF4_8000, 0x0200, None),
    ("timg0",    "esp32_timg",     0x3FF5_F000, 0x1000, None),
    ("timg1",    "esp32_timg",     0x3FF6_0000, 0x1000, None),
    ("efuse",    "esp32_efuse",    0x3FF5_A000, 0x1000, None),
    ("syscon",   "esp32_syscon",   0x3FF6_6000, 0x0100, None),
    ("ledc",     "esp32_ledc",     0x3FF5_9000, 0x1000, None),
    ("twai",     "esp32_twai",     0x3FF6_B000, 0x1000, None),
    ("mcpwm0",   "esp32_mcpwm",    0x3FF5_E000, 0x1000, None),
    ("host_slc", "esp32_sdio",     0x3FF5_8000, 0x1000, None),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use crate::Peripheral;
    use std::sync::{Arc, Mutex};

    /// The esp32 (LX6) factory + ESP32_PERIPHERALS table must build each
    /// peripheral with the same window (base, size) as the hand-wired
    /// `configure_xtensa_esp32`. That builder also registers memory regions and
    /// catch-all stubs, so this checks the table's peripherals by name rather
    /// than comparing whole buses. Pins the factory path as equivalent before it
    /// replaces the hand-wired registrations.
    #[test]
    fn esp32_factory_descriptors_match_hardwired() {
        use labwired_config::PeripheralConfig;
        use std::collections::HashMap;

        let mut hw = SystemBus::new();
        let _ = configure_xtensa_esp32(&mut hw);

        for &(id, ty, base, size, irq) in ESP32_PERIPHERALS {
            let cfg = PeripheralConfig {
                id: id.to_string(),
                r#type: ty.to_string(),
                base_address: base,
                size: None,
                irq,
                config: HashMap::new(),
            };
            assert!(
                crate::peripherals::esp32::factory::try_build(ty, &cfg).is_some(),
                "esp32 factory missing type {ty} for {id}"
            );
            let idx = hw
                .find_peripheral_index_by_name(id)
                .unwrap_or_else(|| panic!("hand-wired esp32 bus missing {id}"));
            let p = &hw.peripherals[idx];
            assert_eq!((p.base, p.size), (base, size), "window mismatch for {id}");
        }
    }

    /// The esp32s3 factory + canonical descriptor table must place exactly the
    /// same peripheral windows (name, base, size) as the hand-wired
    /// `register_esp32s3_peripherals`. This pins the Stage-3 data-driven path as
    /// equivalent before it replaces the hand-wired one. (i2c0's TMP102 slave is
    /// internal model state, not a window, so it does not affect this check.)
    #[test]
    fn factory_descriptors_match_hardwired_peripherals() {
        use labwired_config::PeripheralConfig;
        use std::collections::HashMap;

        let mut hw = SystemBus::new();
        hw.peripherals.clear();
        register_esp32s3_peripherals(&mut hw, &Esp32s3Opts::default());

        let mut fac = SystemBus::new();
        fac.peripherals.clear();
        for &(id, ty, base, size, irq) in ESP32S3_PERIPHERALS {
            let cfg = PeripheralConfig {
                id: id.to_string(),
                r#type: ty.to_string(),
                base_address: base,
                size: None,
                irq,
                config: HashMap::new(),
            };
            let dev = crate::peripherals::esp32s3::factory::try_build(ty, &cfg)
                .unwrap_or_else(|| panic!("esp32s3 factory missing type {ty}"));
            // Bus-entry irq is None on both paths; the source id is baked into
            // the model by the factory via cfg.irq.
            fac.add_peripheral(id, base, size, None, dev);
        }

        let windows = |b: &SystemBus| {
            let mut v: Vec<(String, u64, u64, Option<u32>)> = b
                .peripherals
                .iter()
                .map(|p| (p.name.clone(), p.base, p.size, p.irq))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            windows(&hw),
            windows(&fac),
            "factory/table path must place the same peripheral windows as the hand-wired path"
        );
    }

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
