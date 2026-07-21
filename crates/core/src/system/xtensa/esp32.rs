// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic ESP32 (Xtensa LX6) system glue: peripheral map + external devices.
//! Split out of `system::xtensa`.

use super::RamPeripheral;
use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::peripherals::esp_xtensa_common::rom_thunks;
use crate::Bus;

/// Build an I2C-attached external device from its manifest declaration, or
/// `None` if `ext.type` is not a known I2C device (so the caller falls through
/// to the SPI path). The panel is addressed by `config.i2c_address` — a real
/// board-level fact, not a builder default — so a manifest declaring an SH1107
/// on `i2c0` at 0x3D gets exactly that, on every path that wires the manifest.
fn build_i2c_external_device(
    ext: &labwired_config::ExternalDevice,
) -> Option<Box<dyn crate::peripherals::i2c::I2cDevice>> {
    let addr = |default: u8| {
        ext.config
            .get("i2c_address")
            .and_then(|v| v.as_u64())
            .map(|a| a as u8)
            .unwrap_or(default)
    };
    match ext.r#type.as_str() {
        "oled-sh1107" => Some(Box::new(crate::peripherals::components::Sh1107::new(addr(
            0x3D,
        )))),
        "oled-ssd1306" => Some(Box::new(crate::peripherals::components::Ssd1306::new(
            addr(0x3C),
        ))),
        "tmp102" => Some(Box::new(crate::peripherals::esp32s3::tmp102::Tmp102::new())),
        "pca9685" => Some(Box::new(
            crate::peripherals::components::pca9685::Pca9685::new(),
        )),
        _ => None,
    }
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
        // I2C-attached devices are wired to an I2C controller by `connection`
        // and addressed by `config.i2c_address` — the SPI cs_pin/dc_pin framing
        // below is meaningless for them, so handle and `continue` first. This is
        // how a manifest that declares an SH1107 on i2c0 gets the panel wired,
        // instead of the builder hardcoding "every board always has one".
        if let Some(dev) = build_i2c_external_device(ext) {
            bus.attach_i2c_slave(&ext.connection, dev).map_err(|_| {
                anyhow::anyhow!(
                    "External I2C device '{}' connection '{}' is not an ESP32 I2C peripheral",
                    ext.id,
                    ext.connection
                )
            })?;
            continue;
        }

        // Potentiometer: an analog wiper on a SAR-ADC channel. It is not a bus
        // slave — it drives the ADC channel's injected level, so `analogRead()`
        // on that channel returns the wiper voltage.
        //
        // Delegated to the potentiometer kit rather than re-parsing config
        // here: the kit is what retains the model on the bus, and that
        // retention is what makes `set_input("position", …)` reach it. A second
        // copy of this wiring would silently produce a pot that reads correctly
        // at boot but cannot be driven.
        if ext.r#type == "potentiometer" {
            use crate::peripherals::kit::PeripheralKit;
            let mut ctx = crate::peripherals::kit::AttachCtx::new(bus, ext);
            crate::peripherals::components::potentiometer::POTENTIOMETER_KIT.attach(&mut ctx)?;
            continue;
        }

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

        // Funnel through the single bus choke point, which wraps `panel` in the
        // shared bus trace and dispatches to whichever SPI controller the
        // `connection:` resolves to (classic `Esp32Spi` spi2/spi3 or the GP-SPI
        // `Esp32s3Spi` spi2_s3/spi3_s3). No untraced attach path.
        bus.attach_spi_device(&ext.connection, panel).map_err(|_| {
            anyhow::anyhow!(
                "External device '{}' connection '{}' is not an ESP32 SPI peripheral",
                ext.id,
                ext.connection
            )
        })?;
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

    // I2C0 (I2C_EXT0, TRM §11) at 0x3FF5_3000 — real command-list engine
    // (`peripherals::esp32::i2c::Esp32I2c`). Built directly (not via the table
    // loop) so a board-level I2C slave is attached: a BMP280 at 0x76, the
    // canonical register-pointer device, lets firmware drive a full
    // write-pointer / repeated-start / read transaction and read back genuine
    // device data (CHIP_ID 0x58). Source 49 = ETS_I2C_EXT0_INTR_SOURCE.
    let i2c0 = crate::peripherals::esp32::i2c::Esp32I2c::new();
    bus.add_peripheral("i2c0", 0x3FF5_3000, 0x1000, None, Box::new(i2c0));
    // Register first, then attach through the single bus choke point so the
    // slave is wrapped into the shared bus trace (universal logic analyzer).
    bus.attach_i2c_slave(
        "i2c0",
        Box::new(crate::peripherals::components::Bmp280::new(0x76)),
    )
    .expect("i2c0 just registered as Esp32I2c");

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
        // i2c0 (0x3FF5_3000) is the real Esp32I2c model registered above.
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
pub(crate) fn register_esp32_peripherals(bus: &mut SystemBus) {
    use crate::peripherals::esp32::factory;
    use labwired_config::PeripheralConfig;
    use std::collections::HashMap;
    for &(id, ty, base, size, irq) in ESP32_PERIPHERALS {
        // syscon shares base with apb_ctrl (registration-order-sensitive);
        // i2c0 is built directly so board-specific I2C slaves can be attached.
        if id == "syscon" || id == "i2c0" {
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
            clock: None,
            config,
        };
        let dev = factory::try_build(ty, &cfg)
            .unwrap_or_else(|| panic!("esp32 factory missing type {ty} for {id}"));
        bus.add_peripheral(id, base, size, None, dev);
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
    ("i2c0",     "esp32_i2c",      0x3FF5_3000, 0x1000, Some(49)),
    // SENS SAR-ADC one-shot engine (RTC controller ADC1/ADC2 path the IDF
    // adc1_get_raw/adc2_get_raw drivers drive). 0x100 window over the SAR
    // control + measurement registers; registered before the rtcio catch-all
    // stub (0x3FF4_8400/0x1000) so it wins the overlapping SENS sub-range.
    ("sens_sar_adc", "esp32_sar_adc", 0x3FF4_8800, 0x0100, None),
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
