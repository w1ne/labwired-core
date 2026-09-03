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
// flash_mmu types used via full paths below

/// Build an I2C-attached external device from its manifest declaration, or
/// `None` if `ext.type` is not a known I2C device (so the caller falls through
/// to the SPI path). `build_i2c_tree` also assembles a TCA9548A bus switch
/// together with every device wired behind it, so a mux reaches the bus as one
/// unit.
///
/// This is the shared factory and nothing else. It used to carry local
/// `oled-sh1107` / `oled-ssd1306` arms on top, which shadowed the kits of the
/// same name and quietly disagreed with them — the local SH1107 arm defaulted
/// to 0x3D where `SH1107_KIT` defaults to 0x3C, so the same manifest addressed
/// a different device depending on which chip ran it. The caller now consults
/// the kit registry first, which is the one place those defaults live.
fn build_i2c_external_device(
    manifest: &labwired_config::SystemManifest,
    ext: &labwired_config::ExternalDevice,
) -> anyhow::Result<Option<Box<dyn crate::peripherals::i2c::I2cDevice>>> {
    crate::peripherals::components::build_i2c_tree(manifest, ext)
}

/// Attach external devices declared in `manifest.external_devices` to an
/// ESP32-classic bus that was already set up by `configure_xtensa_esp32`.
///
/// What a device type MEANS — its pins, defaults, addresses, and how it hangs
/// off a bus — lives in that device's `PeripheralKit`, never here. This
/// function only resolves the ESP32-specific parts: which controller a
/// `connection:` names, and the legacy I²C factory for types not yet migrated
/// to a kit. Dispatch order enforces that: registry first, factory second.
/// Anything else and a type with both a kit and a factory arm would resolve
/// differently depending on which chip ran the manifest, which is exactly the
/// bug the two shadowed OLED arms used to cause.
///
/// This is the canonical implementation; `crates/wasm/src/lib.rs` delegates
/// to it (the wasm crate no longer carries its own copy).
pub fn attach_esp32_external_devices(
    bus: &mut SystemBus,
    manifest: &labwired_config::SystemManifest,
) -> anyhow::Result<()> {
    // Xtensa machines build their peripheral bank directly instead of through
    // `SystemBus::from_config`, so this is the runtime contract boundary for
    // browser/WASM manifests as well as native callers.
    crate::bus::part_pack::validate_manifest(manifest)?;
    // Classic ESP32 builds its peripheral bank in Rust and never runs
    // `SystemBus::from_config`'s peripheral loop, so it must record the
    // manifest's external-device declarations itself. Without this the devices
    // still attach and still work — they simply inspect as anonymous
    // `i2c0@0x70` entries instead of by the ids the author wrote.
    bus.record_external_devices(manifest);

    // Devices wired behind an I²C bus switch are attached as part of that
    // switch by `build_i2c_tree`, never straight onto a controller.
    let mux_children = crate::peripherals::components::i2c_mux_child_ids(manifest);

    for ext in &manifest.external_devices {
        if mux_children.contains(&ext.id.as_str()) {
            continue;
        }
        // 1. The canonical universal pass (parts → kit registry → declarative
        //    descriptors), shared with `from_config` so resolution can never
        //    diverge per chip family. See `bus::external_devices`.
        if matches!(
            crate::bus::external_devices::attach_external_device_universal(bus, manifest, ext)?,
            crate::bus::external_devices::UniversalResolution::Attached
        ) {
            continue;
        }

        // 2. The legacy I²C factory, for device types that still predate the
        //    kit contract (the TCA9548A bus switch among them). Addressed by
        //    `config.i2c_address` — a board-level fact, not a builder default
        //    — so a manifest that declares a sensor on i2c0 gets exactly that,
        //    instead of the builder hardcoding "every board always has one".
        if let Some(dev) = build_i2c_external_device(manifest, ext)? {
            bus.attach_i2c_slave(&ext.connection, dev).map_err(|_| {
                anyhow::anyhow!(
                    "External I2C device '{}' connection '{}' is not an ESP32 I2C peripheral",
                    ext.id,
                    ext.connection
                )
            })?;
            continue;
        }

        // 3. Nothing claims this type: hard error. A green run with a
        //    silently missing device is worse than no run — the simulator's
        //    worst failure mode is a pass that proves nothing.
        return Err(crate::bus::external_devices::unsupported_external_device_error("ESP32", ext));
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
    // Shared physical flash + MMU tables (partition table, SPI flash reads,
    // temporary spi_flash_mmap, cache2phys). Hybrid XIP windows keep an ELF
    // overlay for dirty pages and serve clean MMU-mapped pages from flash.
    let flash_shared = crate::peripherals::esp32::flash_mmu::Esp32FlashShared::new(4 * 1024 * 1024);
    bus.add_peripheral(
        "flash_icache",
        0x400D_0000,
        0x400000,
        None,
        Box::new(
            crate::peripherals::esp32::flash_mmu::ClassicFlashWindow::new(
                0x400D_0000,
                0x400000,
                flash_shared.clone(),
            ),
        ),
    );
    bus.add_peripheral(
        "flash_dcache",
        0x3F40_0000,
        0x400000,
        None,
        Box::new(
            crate::peripherals::esp32::flash_mmu::ClassicFlashWindow::new(
                0x3F40_0000,
                0x400000,
                flash_shared.clone(),
            ),
        ),
    );
    // PRO/APP flash MMU tables — MUST register before dport_analog_ahb
    // (0x3FF0_1000..0x3FF1_FFFF) which would otherwise shadow these windows.
    bus.add_peripheral(
        "flash_mmu_pro",
        0x3FF1_0000,
        (crate::peripherals::esp32::flash_mmu::ENTRY_NUM * 4) as u64,
        None,
        Box::new(
            crate::peripherals::esp32::flash_mmu::Esp32FlashMmuRegs::new_pro(flash_shared.clone()),
        ),
    );
    bus.add_peripheral(
        "flash_mmu_app",
        0x3FF1_2000,
        (crate::peripherals::esp32::flash_mmu::ENTRY_NUM * 4) as u64,
        None,
        Box::new(
            crate::peripherals::esp32::flash_mmu::Esp32FlashMmuRegs::new_app(flash_shared.clone()),
        ),
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
    // ets_printf is NOT thunked on classic ESP32: the real ROM implementation
    // is loaded by `install_rom_console` below, and formats through the ROM's
    // own `ets_write_char` -> putc1 -> `uart_tx_one_char` chain into UART0.
    // The Rust `rom_thunks::ets_printf` it used to share with the S3 wrote to
    // `tracing::info!` instead, so output reached the host's stderr but never
    // the UART — invisible to any capture, assertion, or timing.
    // esp_rom_spiflash_config_clk — configures flash SPI clock divider.
    // No-op in sim; returns 0 (success).
    rom_bank.register(0x4006_2bc8, rom_thunks::nop_return_zero);
    // 0x4000_9200 is `uart_tx_one_char`, not the "unnamed esp32_init helper" it
    // was once registered as here — and it is NOT thunked any more. The real
    // ROM code is loaded over this address by `install_rom_console` below.
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
    // esp_rom_crc32_le — core dump / image helpers (not in all ld scripts as a
    // named export; address from ESP-IDF rom/esp32.rom.ld + nm of app that calls it).
    rom_bank.register(0x4005_cfec, rom_thunks::rom_esp_crc32_le);
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
                                                           // ets_install_putc1 / ets_install_uart_printf / ets_install_putc2 are real
                                                           // ROM code too (loaded below). They are three-instruction routines that
                                                           // store a function pointer into the ROM's putc globals; nop'ing them meant
                                                           // firmware redirecting the console got its pointer silently dropped.
                                                           // Four console entries here used to carry INVENTED addresses under real ROM
                                                           // symbol names: uart_tx_one_char at 0x4000_8fa8, uart_tx_one_char2 at
                                                           // 0x4000_9018, uart_tx_flush at 0x4000_8fcc, and a "uart_tx_wait_idle" at
                                                           // 0x4000_9024 (the real ones are 0x9200 / 0x922c / 0x9258 / 0x9278, per
                                                           // Espressif's esp32.rom.ld). Nothing ever called them, and the mistake was
                                                           // invisible: the bank pre-fills its whole range with BREAK 1,14 and
                                                           // `get_rom_thunk` falls back to `nop_return_zero`, so a name at a dead
                                                           // address and a correctly-addressed nop behave identically — both discard
                                                           // every byte the firmware prints. They are gone; the real ROM code for the
                                                           // console runs instead (see `install_rom_console`).
    rom_bank.register(0x4000_9028, rom_thunks::nop_return_zero); // uart_tx_switch
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

    // ESP-IDF partition-table verification uses ROM MD5 (classic MD5Context
    // layout). Real implementations — CONFIG_PARTITION_TABLE_MD5 is on for
    // Arduino-ESP32 matrices, so a nop would fail load_partitions.
    rom_bank.register(0x4005_da7c, rom_thunks::rom_md5_init); // esp_rom_md5_init
    rom_bank.register(0x4005_da9c, rom_thunks::rom_md5_update); // esp_rom_md5_update
    rom_bank.register(0x4005_db1c, rom_thunks::rom_md5_final); // esp_rom_md5_final
                                                               // Load the boot ROM's REAL console routines over the BREAK bytes, taking
                                                               // the UART output path off the thunk mechanism entirely. Last, so it wins
                                                               // over any registration above.
    super::install_rom_console(&mut rom_bank);
    bus.add_peripheral("rom", 0x4000_0000, 0x70000, None, Box::new(rom_bank));
    // UART0 — STM32F1 layout for now (see caveat above).
    // UART0 (Serial) echoes to the host console; UART1/2 are capture-only.
    // Interrupt-matrix sources: ETS_UART{0,1,2}_INTR_SOURCE = 34/35/36.

    // SPI0 / SPI1 — flash SPI controllers used by the BROM and by
    // Arduino-ESP32 `esp_flash` / `bootloader_read_flash_id`. `Esp32Spi`
    // auto-clears CMD trigger bits and answers JEDEC RDID / RDSR (W0 +
    // RD_STATUS) so `esp_flash_init_main` can probe a Winbond-class 4 MiB
    // part without a firmware thunk. Optional flash-array READ can be
    // attached later via `Esp32Spi::set_flash_backing`.

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
    // DPORT (incl. APPCPU_CTRL_* and cross-core FROM_CPU IPI triggers).
    // Classic ESP32 is dual-core: callers attach a real APP_CPU via
    // `XtensaLx7::new_app_cpu` + `Machine::with_secondary_cpu`. PRO
    // releases it through ROM `ets_set_appcpu_boot_addr` (boot-ROM surface
    // already mapped above); `Machine` drains the boot address and unhalts
    // core 1 so `call_start_cpu1` runs for real — no forged `s_cpu_up`.
    //
    // Writes to the cross-core IPI region (CPU_INTR_FROM_CPU_0..3 at
    // 0xDC..0xE8 and PRO/APP_INTR_FROM_CPU_0..3 at 0x164..0x174) are
    // observable on subsequent reads; DPORT::cross_core_pending feeds
    // Machine::step interrupt delivery.
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
    // Bind I2C0's SCL/SDA wire to the classic GPIO output matrix, so a pad the
    // firmware routes to I2CEXT0_SCL/SDA (signals 29/30) carries the real
    // waveform for `read_gpio_pad` and the in-engine logic analyzer. Must come
    // AFTER both the GPIO registration and the i2c0 registration above:
    // `pad_lines_arc` CREATES the wire cell, and a controller owning a cell no
    // route reaches narrates into nothing.
    bus.wire_esp32_i2c_pads();
    // Same for VSPI (the `spi3` instance at 0x3FF6_5000 — the controller
    // arduino-esp32's `SPI` object drives) and each UART's TX, so those buses
    // are measurable on this part rather than reading as a flat line. Must come
    // AFTER `register_esp32_peripherals` above, which is what puts spi3 and
    // uart0/1/2 on the bus; `pad_lines_arc` CREATES the wire cell, and a
    // controller owning a cell no route reaches narrates into nothing.
    //
    // ⚠️ Unlike I²C these are the ONLY call sites that matter for a real lab:
    // `configs/chips/esp32.yaml` is not what a classic lab is built from.
    bus.wire_esp32_spi_pads();
    bus.wire_esp32_uart_pads();
    // AHB TX FIFO alias registered after wifi_mac_phy (see below).

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

    // APB_CTRL — clock source select etc. Read/write stub for the
    // 0x3FF6_6100..0x3FF6_6FFF TAIL of the APB-CTRL window. The 0x100 header
    // belongs to SYSCON above.
    //
    // This used to be registered at 0x3FF6_6000 with size 0x1000, overlapping
    // SYSCON completely, on the belief that "registration order wins on
    // overlap". It does not: routing.rs resolves the window with the GREATEST
    // start, and ties by the LAST registered — so this stub answered every
    // SYSCON register and the whole model was dead code reading 0xFFFFFFFF.
    //
    // The cost was not abstract. SYSCLK_CONF's PRE_DIV_CNT read 1023 instead
    // of 0, so ESP-IDF computed a CPU divider of 1024, Arduino's
    // getApbFrequency() returned 78125 Hz, and _get_effective_baudrate divided
    // by zero — which is the exception the Arduino serial thunks existed to
    // avoid. Mapping the tail where the comment always said it went removes
    // the overlap entirely rather than depending on registration order.
    bus.add_peripheral(
        "apb_ctrl",
        0x3FF6_6100,
        0x0F00,
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
        // uart1 (0x3FF5_0000) and uart2 (0x3FF6_E000) are the real Esp32Uart
        // models from ESP32_PERIPHERALS — same removal as i2c0/pwm0 below.
        // They used to ALSO appear here, and because a 0x1000 stub at the
        // SAME base registered later beats the real 0x100 model
        // (equal starts → last registered), UART1/UART2 on classic ESP32 were
        // round-trip stubs and the real models had never executed. Serial1 and
        // Serial2 therefore produced nothing — the same defect that killed
        // Serial0 via the apb_ctrl/SYSCON shadow. Guarded by
        // tests::peripheral_reachability.
        // i2c0 (0x3FF5_3000) is the real Esp32I2c model registered above.
        ("uhci0", 0x3FF5_4000),
        ("i2s1", 0x3FF6_D000),
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
    // Classic ESP32 AHB FIFO aliases (`UART_FIFO_AHB_REG`). Firmware TX writes
    // here (uart_ll_write_txfifo); STATUS/INT live on APB. Registered *after*
    // wifi_mac_phy so equal-start last-wins gives the 4-byte AHB windows
    // priority at 0x6000_0000 / 0x6001_0000 / 0x6002_E000.
    //
    // I2C0 TX FIFO AHB window: esp-idf `i2c_ll_write_txfifo` stores at
    // 0x6001_301c (not the APB DATA reg). Same last-wins priority over wifi stub.
    if let Some(idx) = bus.find_peripheral_index_by_name("i2c0") {
        if let Some(i2c) = bus.peripherals[idx]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::i2c::Esp32I2c>())
        {
            let ahb = i2c.ahb_tx_fifo_alias();
            bus.add_peripheral("i2c0_ahb_fifo", 0x6001_301c, 4, None, Box::new(ahb));
        }
    }
    for (name, ahb_base) in [
        ("uart0", 0x6000_0000u64),
        ("uart1", 0x6001_0000u64),
        ("uart2", 0x6002_E000u64),
    ] {
        if let Some(idx) = bus.find_peripheral_index_by_name(name) {
            if let Some(uart) = bus.peripherals[idx]
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::uart::Esp32Uart>())
            {
                let ahb = uart.ahb_fifo_alias();
                bus.add_peripheral(
                    &format!("{name}_ahb_fifo"),
                    ahb_base,
                    4,
                    None,
                    Box::new(ahb),
                );
            }
        }
    }

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

    // Seed the ROM's own console state — the rodata digit tables into the
    // brom_data window just registered, and putc1 into DRAM. Must come after
    // both windows exist, since it writes through the bus. Without it
    // `ets_printf` executes correctly and prints nothing, because real silicon
    // installs putc1 during a boot path we skip. See `seed_rom_console_state`.
    super::seed_rom_console_state(bus);

    // Walk-deletion decision. DERIVED, never asserted — see
    // `SystemBus::derive_walk_deletable`. The flag is only read under the
    // `event-scheduler` feature, which the browser crate enables
    // (`crates/wasm/Cargo.toml`) and the CLI deliberately does not
    // (`crates/cli/Cargo.toml`), so a wrong value here is invisible to every
    // CLI lane in this repo and shows up only in the browser.
    //
    // This line used to read `bus.legacy_walk_disabled = true;` under a comment
    // claiming "uart0, gpio, rtc_cntl, timg0/1 migrated to the event
    // scheduler". gpio / rtc_cntl / timg did migrate (`uses_scheduler() ==
    // true`). **uart0 never did.** Classic ESP32 has its own
    // `peripherals::esp32::uart::Esp32Uart`, forked from the shared
    // `peripherals::esp_uart::EspUart` that the C3/S3 use; only the shared one
    // grew `uses_scheduler` / `take_scheduled_events` / `on_event`. `Esp32Uart`
    // still drains `tx_fifo` from `tick()` and nowhere else, and declares
    // neither `uses_scheduler()` nor `needs_legacy_walk() == false`.
    //
    // So under `event-scheduler` the hand flag deleted the walk out from under
    // a model that needs it: `Esp32Uart::tick()` was never called, `tx_fifo`
    // never drained, `UART_STATUS.TXFIFO_CNT` pinned at its high-water mark,
    // and arduino-esp32's `uart_ll_write_txfifo` wait-for-space loop
    // (`while (128 - txfifo_cnt) < 2`) spun forever — the firmware booted,
    // burned billions of cycles, painted nothing and never reached `loop()`.
    // Exactly the failure mode `Peripheral::needs_legacy_walk` warns about.
    //
    // The derivation is conservative by construction and cannot make that
    // mistake: it deletes the walk only when EVERY peripheral is provably
    // walk-independent. On this bus `Esp32Uart` (uart0/1/2) forces it back on,
    // which costs classic-ESP32 browser throughput (no interval-512 batching,
    // no idle fast-forward) until `Esp32Uart` is genuinely migrated — that
    // migration additionally needs a DPORT arm in
    // `SystemBus::deliver_scheduled_irq_levels`, which today handles only the
    // C3 and S3 matrices, or the UART's TXFIFO_EMPTY interrupt would stop
    // being routed. A slow lab beats a wedged one; see the gate in
    // `crates/core/tests/esp32_classic_walk_differential.rs`.

    // Default flash image: app XIP MMU seed for cache2phys + SPI0/1 backing.
    // Callers (diag / labwired test) overlay partitions.bin at 0x8000 via
    // `seed_esp32_flash_image` before load_firmware.
    let _ = crate::peripherals::esp32::flash_mmu::seed_esp32_flash_image(bus, None);

    // Derived LAST, so it sees the final peripheral set (mirrors the rom-boot
    // path in `boot::esp32c3_rom` and the tail of `SystemBus::from_config`).
    bus.recompute_walk_deletable();

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
    // control + measurement registers. It wins the overlapping SENS sub-range
    // against the rtcio catch-all stub (0x3FF4_8400/0x1000) because routing.rs
    // picks the window with the GREATEST start, and 0x8800 > 0x8400 — NOT
    // because it is registered first. Registration order only breaks ties
    // between EQUAL starts, and there the LAST registered wins. Getting that
    // backwards is what left SYSCON dead behind apb_ctrl for a year.
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
