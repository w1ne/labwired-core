// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Xtensa construction, moved from `labwired-wasm`'s
//! `new_from_config_xtensa_esp32s3`, `new_from_config_xtensa_esp32s3_flash` and
//! `new_from_config_xtensa_esp32`.
//!
//! The bodies are the browser's, step for step, with the same substitutions as
//! the RISC-V port:
//!
//! * The S3 is recognised by [`labwired_config::ChipDescriptor::is_esp32s3`], the predicate the
//!   browser dispatches on. The browser picks the S3 flash path by the presence
//!   of the `esp32s3_flash` blob; here the request says it (`FlashImage` +
//!   `RomBoot`) and the image arrives in `FlashImage::image`.
//! * The mask ROM comes from `blobs` under the browser's names, `esp32s3_irom`
//!   and `esp32s3_drom`: optional on S3 fast boot (absent, configure falls back
//!   to the native provision chain), required on S3 flash boot.
//! * Boot paths the engine has no constructor for (S3 fast boot from a flash
//!   image, any classic-ESP32 boot but an ELF fast boot) are a "not supported"
//!   error, never a silent fallback to another path.
//! * ELF parsing on the classic ESP32 uses [`parse_elf_image`] (core cannot
//!   depend on `labwired-loader`); the S3 fast boot parses its own ELF.
//! * Console attaches honour `BuildOptions::echo_uart_stdout` on the heard
//!   console; the unheard console never echoes.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::boot::esp32s3_rom::RomImages;
use crate::bus::SystemBus;
use crate::console::{ConsoleCapture, HostConsole};
use crate::system::node::parse_elf_image;
use crate::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
use crate::{Cpu, Machine};
use anyhow::anyhow;
use labwired_config::SystemManifest;

pub(super) fn build(req: BuildRequest<'_>) -> anyhow::Result<BuiltMachine> {
    if req.chip.is_esp32s3() {
        match (&req.firmware, req.boot) {
            (FirmwareSource::Elf(elf), BootMode::FastBoot) => build_esp32s3(&req, elf),
            (FirmwareSource::FlashImage { image, symbols }, BootMode::RomBoot) => {
                build_esp32s3_flash(&req, image, *symbols)
            }
            (FirmwareSource::FlashImage { .. }, BootMode::FastBoot) => Err(anyhow!(
                "not supported: ESP32-S3 fast boot from a flash image; a merged flash image \
                 boots through the mask ROM (BootMode::RomBoot)"
            )),
            (FirmwareSource::Elf(_), BootMode::RomBoot) => Err(anyhow!(
                "ESP32-S3 ROM boot needs a flash image (bootloader + partition table + app): \
                 the mask ROM loads the 2nd-stage bootloader from flash, and an ELF has none"
            )),
        }
    } else {
        match (&req.firmware, req.boot) {
            (FirmwareSource::Elf(elf), BootMode::FastBoot) => build_esp32(&req, elf),
            _ => Err(anyhow!(
                "not supported: classic ESP32 boot from a flash image or through the mask ROM \
                 (chip '{}'); only an ELF fast boot is modelled",
                req.chip.name
            )),
        }
    }
}

/// `new_from_config_xtensa_esp32`: ESP32-classic (Xtensa LX6).
/// `configure_xtensa_esp32` adds IRAM / DRAM / flash XIP / ROM / UART0; the
/// manifest's external devices attach through the core helper because this path
/// does not go through `SystemBus::from_config`.
fn build_esp32(req: &BuildRequest<'_>, firmware: &[u8]) -> anyhow::Result<BuiltMachine> {
    let manifest = req.system;
    // Drop any leftover process/thread-local aids state from a prior machine on
    // this thread. See `rom_thunks::reset_esp32_session_state`.
    crate::peripherals::esp_xtensa_common::rom_thunks::reset_esp32_session_state();

    let mut bus = SystemBus::new();
    let cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);

    // A classic ESP32 has NO USB peripheral: its devkit's CP210x sits on UART0
    // and IS the USB device the host enumerates. So `debug_uart: usb_serial_jtag`
    // here is a board-mapping error, and `attach_host_console` says so.
    let console = ConsoleCapture::for_manifest(manifest);
    let uart_sink = console.heard_sink();
    bus.attach_host_console_echo(
        console.tapped(),
        uart_sink.clone(),
        req.options.echo_uart_stdout,
    )
    .map_err(|e| anyhow!(e))?;
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    crate::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
        .map_err(|e| anyhow!("ESP32 external_devices: {e:#}"))?;
    bus.refresh_peripheral_index();

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    // Real dual-core: a second LX6 as APP_CPU (PRID 0xABAB → core 1), halted
    // until PRO_CPU releases it via ets_set_appcpu_boot_addr.
    let app_cpu: Box<dyn Cpu> = Box::new(crate::cpu::XtensaLx7::new_app_cpu());
    let mut machine = Machine::new(boxed, bus).with_secondary_cpu(app_cpu);

    let program_image = parse_elf_image(firmware).map_err(|e| anyhow!("Loader Error: {e:#}"))?;
    machine
        .load_firmware(&program_image)
        .map_err(|e| anyhow!("Simulation Error: {e}"))?;
    // XtensaLx7::reset() defaults PC to the BROM reset vector. BROM emulation is
    // skipped: jump straight to the ELF's app entry, where a 2nd-stage
    // bootloader would land.
    machine.cpu.set_pc(program_image.entry_point as u32);
    // BROM seeds SP near the top of DRAM before call_start_cpu0; seed both
    // cores' stacks (APP_CPU in a separate DRAM region below PRO_CPU's).
    machine.cpu.set_sp(0x3FFE_0000);
    if let Some(cpu1) = machine.cpu_secondary.as_mut() {
        cpu1.set_sp(0x3FFD_8000);
    }

    Ok(built(
        machine,
        console,
        uart_rx_bufs,
        manifest,
        firmware.to_vec(),
    ))
}

/// `new_from_config_xtensa_esp32s3_flash`: faithful ESP32-S3 boot from a merged
/// flash image with **no ELF** — the path every hosted S3 run needs.
///
/// The native `--rom-boot` assembly: `real_reset_boot` selects the MMU XIP
/// model, the flash image is passed as bytes, and the CPU is left at the BROM
/// reset vector so the chip's own ROM loads the app. No `fast_boot`, no
/// synthesised post-bootloader state, no thunks.
fn build_esp32s3_flash(
    req: &BuildRequest<'_>,
    flash: &[u8],
    symbols: Option<&[u8]>,
) -> anyhow::Result<BuiltMachine> {
    let (chip, manifest, blobs) = (req.chip, req.system, req.blobs);

    // The real ROM is not optional here: this path IS the ROM, so a missing
    // blob has to say so rather than boot nothing.
    let (Some(irom), Some(drom)) = (blobs.get("esp32s3_irom"), blobs.get("esp32s3_drom")) else {
        return Err(anyhow!(
            "ESP32-S3 flash boot needs the boot ROM blobs: pass esp32s3_irom + esp32s3_drom"
        ));
    };

    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts {
        real_reset_boot: true,
        rom_images: Some(RomImages {
            irom: irom.clone(),
            drom: drom.clone(),
        }),
        flash_image: Some(flash.to_vec()),
        // Size the backing from the CHIP descriptor, never from the image's own
        // byte count: the model publishes it as the JEDEC capacity, and
        // esp_flash refuses to boot when that disagrees with the image header.
        flash_size: esp32s3_flash_backing_size(chip.flash.size, flash.len()),
        // Core clock from the same descriptor: the SYSTIMER divides the CPU
        // cycle stream by it.
        ..Esp32s3Opts::for_chip(chip)
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    if wiring.boot_mode != Esp32s3BootMode::Faithful {
        return Err(anyhow!(
            "ESP32-S3 flash boot needs the real boot ROM, but the injected images did not resolve"
        ));
    }
    let mut cpu = wiring.cpu;
    // The ROM and the app install the window overflow/underflow vectors and
    // build a genuine stack save chain, so the CPU must use the real per-access
    // spill/fill path rather than the simulator's shadow stack.
    cpu.faithful_windows = true;
    // Read it back: the APP core must use the SAME window-handling mode.
    let primary_faithful_windows = cpu.faithful_windows;

    let console = attach_s3_consoles(&mut bus, manifest, req)?;
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    crate::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
        .map_err(|e| anyhow!("ESP32-S3 external_devices: {e:#}"))?;
    bus.refresh_peripheral_index();

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    // Real second core: an ESP-IDF image built dual-core stops at
    // `cpu_start: Multicore app` without one. It starts halted and is released
    // by the hardware edge the firmware drives, as in the native runner.
    let mut app_cpu_lx7 = crate::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
    // Core 1 boots the real ROM and runs the same image, so it needs
    // `faithful_windows` for the same reason core 0 does; left on the shadow
    // stack it restores a garbage SP and faults on every window overflow.
    app_cpu_lx7.faithful_windows = primary_faithful_windows;
    let app_cpu: Box<dyn Cpu> = Box::new(app_cpu_lx7);
    let mut machine = Machine::new(boxed, bus).with_secondary_cpu(app_cpu);
    crate::system::wifi::attach_configured_wifi_ap(&mut machine.bus, manifest);

    Ok(built(
        machine,
        console,
        uart_rx_bufs,
        manifest,
        symbols.map(<[u8]>::to_vec).unwrap_or_default(),
    ))
}

/// `new_from_config_xtensa_esp32s3`: the faithful S3 fast-boot path.
///
/// `configure_xtensa_esp32s3` installs IRAM/DRAM/RTC/flash-XIP plus the boot
/// ROM (injected from blobs, else the native provision chain). `fast_boot` then
/// loads the app ELF's segments (identity XIP) and synthesises post-bootloader
/// CPU state.
fn build_esp32s3(req: &BuildRequest<'_>, firmware: &[u8]) -> anyhow::Result<BuiltMachine> {
    use crate::boot::esp32s3::{fast_boot, BootOpts};
    let (chip, manifest, blobs) = (req.chip, req.system, req.blobs);

    let rom_images = match (blobs.get("esp32s3_irom"), blobs.get("esp32s3_drom")) {
        (Some(irom), Some(drom)) => Some(RomImages {
            irom: irom.clone(),
            drom: drom.clone(),
        }),
        _ => None,
    };

    let mut bus = SystemBus::new();
    // Default XIP model (fast-boot identity) + the injected faithful ROM.
    let opts = Esp32s3Opts {
        rom_images,
        // Core clock from the chip descriptor, the one home for `cpu_hz`.
        ..Esp32s3Opts::for_chip(chip)
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    let mut cpu = wiring.cpu;

    let console = attach_s3_consoles(&mut bus, manifest, req)?;
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    // Wire any devices the manifest declares (e.g. an SH1107 OLED on i2c0).
    crate::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)
        .map_err(|e| anyhow!("ESP32-S3 external_devices: {e:#}"))?;
    bus.refresh_peripheral_index();

    fast_boot(
        firmware,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(wiring.icache_backing),
            dcache_backing: Some(wiring.dcache_backing),
            factory_flash_base: None,
        },
    )
    .map_err(|e| anyhow!("ESP32-S3 fast_boot: {e}"))?;

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);
    crate::system::wifi::attach_configured_wifi_ap(&mut machine.bus, manifest);

    Ok(built(
        machine,
        console,
        uart_rx_bufs,
        manifest,
        firmware.to_vec(),
    ))
}

/// The S3 console selection both S3 constructors share, verbatim: an undeclared
/// manifest hears both consoles in one capture (the mask ROM prints on UART0, a
/// CDC-on-boot sketch on USB-Serial-JTAG); a declared one is authoritative.
fn attach_s3_consoles(
    bus: &mut SystemBus,
    manifest: &SystemManifest,
    req: &BuildRequest<'_>,
) -> anyhow::Result<ConsoleCapture> {
    let echo = req.options.echo_uart_stdout;
    let console = ConsoleCapture::for_manifest(manifest);
    let uart_sink = console.heard_sink();
    match console.tapped() {
        HostConsole::Undeclared => {
            bus.attach_usb_serial_jtag_sink_echo(uart_sink.clone(), echo);
            bus.attach_uart_tx_sink(uart_sink.clone(), echo);
        }
        tapped => {
            // Record the console with no connector FIRST, then tap the one the
            // host reads. Every console model holds ONE sink slot, so the LAST
            // writer wins.
            if tapped.is_usb_serial_jtag() {
                bus.attach_uart_tx_sink(console.unheard_sink(), false);
            } else {
                bus.attach_usb_serial_jtag_sink(console.unheard_sink());
            }
            bus.attach_host_console_echo(tapped, uart_sink.clone(), echo)
                .map_err(|e| anyhow!(e))?;
        }
    }
    Ok(console)
}

fn built(
    machine: Machine<Box<dyn Cpu>>,
    console: ConsoleCapture,
    rx: Vec<std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<u8>>>>,
    manifest: &SystemManifest,
    firmware_bytes: Vec<u8>,
) -> BuiltMachine {
    BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires {
            sink: console.heard_sink(),
            rx,
        },
        board_io: manifest.board_io.clone(),
        arch: labwired_config::Arch::Xtensa,
        firmware_bytes,
    }
}

/// Flash backing for the S3 flash boot: the part's declared capacity, never
/// less than the image (a chip YAML that understates the part cannot truncate
/// the image) and never under the 4 MiB floor.
fn esp32s3_flash_backing_size(chip_flash_size: u64, image_len: usize) -> u32 {
    let declared = u32::try_from(chip_flash_size).unwrap_or(u32::MAX);
    let image = u32::try_from(image_len).unwrap_or(u32::MAX);
    declared.max(image).max(4 * 1024 * 1024)
}
