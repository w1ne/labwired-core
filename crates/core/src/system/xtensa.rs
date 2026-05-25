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
use crate::peripherals::esp32s3::gpio::{Esp32s3Gpio, GpioObserver};
use crate::peripherals::esp32s3::i2c::{Esp32s3I2c, I2C0_BASE, I2C0_INTR_SOURCE_ID, I2C0_SIZE};
use crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix;
use crate::peripherals::esp32s3::io_mux::Esp32s3IoMux;
use crate::peripherals::esp32s3::rom_thunks::{self, RomThunkBank};
use crate::peripherals::esp32s3::system_stub::{EfuseStub, RtcCntlStub, SystemStub};
use crate::peripherals::esp32s3::systimer::Systimer;
use crate::peripherals::esp32s3::tmp102::Tmp102;
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
    // 40 (matches the RTC_APB_FREQ_REG fake we pre-write in the test).
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

    // DPORT register stub (TRM §3). Firmware writes to PERIP_CLK_EN_REG /
    // PERIP_RST_EN_REG to ungate VSPI + GPIO; sim ignores the values
    // (peripherals are always live) but absorbs the writes so they don't
    // fault. with_unwritten_ones() means any status-bit busy-wait reads
    // back high — same trick as the S3 system_stub.
    //
    // Sized 64 KiB to cover the full DPORT + analog AHB regions
    // (0x3FF00000-0x3FF1FFFF) that Arduino-ESP32's startup touches.
    //
    // Unwritten reads return ZERO (vs `with_unwritten_ones`) — Arduino-ESP32's
    // `system_early_init` reads DPORT_APPCPU_CTRL_B at 0x3FF00030 to decide
    // whether to bring up the second core. If we return all-ones, the
    // firmware enters `start_other_core` which spins forever waiting on a
    // shared flag (`s_cpu_up`) that only the APP_CPU sets — and we don't
    // model APP_CPU. Reading zero means "APP_CPU clock not enabled", so
    // the firmware skips the whole second-core bringup path.
    bus.add_peripheral(
        "dport",
        0x3FF0_0000,
        0x20000,
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

    // RTC_CNTL (TRM §31). esp_hal::init touches WDTCONFIG / WDTWPROTECT
    // registers here to disable the RTC watchdog. The S3 RtcCntlStub
    // round-trips writes and seeds PLL_LOCK as locked — same model fits
    // ESP32-classic since the firmware only reads back what it wrote.
    bus.add_peripheral(
        "rtc_cntl",
        0x3FF4_8000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::RtcCntlStub::new()),
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

    // EFUSE — esp-hal reads MAC / chip-revision bits during init.
    bus.add_peripheral(
        "efuse",
        0x3FF5_A000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::EfuseStub::new()),
    );

    // APB_CTRL — clock source select etc. Read/write stub.
    bus.add_peripheral(
        "apb_ctrl",
        0x3FF6_6000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::system_stub::SystemStub::with_unwritten_ones()),
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
        ("pwm0", 0x3FF5_E000),
        ("ledc", 0x3FF5_9000),
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

    XtensaLx7::new()
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

    // ── Flash-XIP backings — one per window (see Esp32s3Wiring docs) ──────
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
    bus.add_peripheral(
        "intmatrix",
        0x600C_2000,
        0x800,
        None,
        Box::new(Esp32s3IntMatrix::new()),
    );

    // ── I²C0 + attached TMP102 (Plan 4) ──────────────────────────────────
    let mut i2c0 = Esp32s3I2c::new();
    i2c0.attach_slave(Box::new(Tmp102::new()));
    bus.add_peripheral("i2c0", I2C0_BASE as u64, I2C0_SIZE, None, Box::new(i2c0));
    // Bind the I²C0 source ID through the intmatrix helper so esp-hal's
    // poll-then-read driver path doesn't depend on routing existing yet —
    // routing is firmware-controlled, this just leaves the source visible.
    let _ = I2C0_INTR_SOURCE_ID;

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

    // Catch-all for the rest of the high-MMIO range that esp-hal pokes
    // during init (LEDC, RMT, GPIO matrix, GDMA, LCD_CAM, EXTMEM cache
    // config, RTC calibration timer, …). Real silicon has dozens of
    // distinct peripherals in this window with bit-precise behaviour;
    // for hello-world we only need round-trip register storage and an
    // "everything's ready" default for status polls. Use the unwritten-
    // ones variant so that calibration-RDY / FIFO-empty / link-up bits
    // trip on the first iteration. The block covers
    // [0x6001_0000, 0x6004_0000) — 192 KiB.
    bus.add_peripheral(
        "mmio_rest",
        0x6001_0000,
        0x3_0000,
        None,
        Box::new(SystemStub::with_unwritten_ones()),
    );

    let mut cpu = XtensaLx7::new();
    cpu.reset(bus).expect("xtensa reset");

    Esp32s3Wiring {
        cpu,
        icache_backing,
        dcache_backing,
    }
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

    #[test]
    fn iram_writeable_and_readable() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        bus.write_u8(0x4037_0010, 0xAB).unwrap();
        assert_eq!(bus.read_u8(0x4037_0010).unwrap(), 0xAB);
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
        // 0x42000020 don't collide on the same physical offset.
        let mut bus = SystemBus::new();
        let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        wiring.icache_backing.lock().unwrap()[0] = 0xCA;
        wiring.dcache_backing.lock().unwrap()[0] = 0xFE;
        assert_eq!(bus.read_u8(0x4200_0000).unwrap(), 0xCA, "I-cache alias");
        assert_eq!(bus.read_u8(0x3C00_0000).unwrap(), 0xFE, "D-cache alias");
    }
}
