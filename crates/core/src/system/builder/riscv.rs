// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RISC-V construction, moved from `labwired-wasm`'s `new_from_config_riscv`,
//! `new_from_config_riscv_program_image`, `new_from_config_riscv_flash_fastboot`
//! and `new_from_config_riscv_romboot`.
//!
//! The bodies are the browser's, step for step. What changed is only where the
//! inputs come from and how errors are spelled:
//!
//! * The browser picks a path by which named blobs are present; here the
//!   request says it. `Elf` + `FastBoot` is the bare-ELF path, `FlashImage` +
//!   `FastBoot` the flash fast-start (the browser's `esp32c3_flash` blob plus
//!   its `labwired_esp32c3_flash_fast_start` marker), `FlashImage` + `RomBoot`
//!   the faithful mask-ROM boot (`esp32c3_flash` alone). The merged flash image
//!   arrives in `FlashImage::image` instead of the `esp32c3_flash` blob.
//! * The mask ROM still comes from `blobs`, under the browser's names:
//!   `esp32c3_irom` and `esp32c3_drom`. Optional on the bare-ELF path (absent,
//!   the chip's ROM regions stay zero, as before), required on both flash paths.
//! * ELF parsing uses [`parse_elf_image`]: core cannot depend on
//!   `labwired-loader`, and for non-AVR images the two place `PT_LOAD` segments
//!   identically.
//! * Console attaches honour `BuildOptions::echo_uart_stdout` on the console
//!   the board's socket is wired to; the unheard console never echoes.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use crate::boot::esp32s3_rom::RomImages;
use crate::bus::SystemBus;
use crate::console::{ConsoleCapture, HostConsole};
use crate::memory::ProgramImage;
use crate::system::node::parse_elf_image;
use crate::{Bus as _, Cpu, Machine};
use anyhow::anyhow;
use labwired_config::SystemManifest;
use std::sync::{Arc, Mutex};

const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;

pub(super) fn build(req: BuildRequest<'_>) -> anyhow::Result<BuiltMachine> {
    match (&req.firmware, req.boot) {
        (FirmwareSource::Elf(elf), BootMode::FastBoot) => build_plain(&req, elf),
        (FirmwareSource::FlashImage { image, symbols }, BootMode::FastBoot) => {
            build_flash_fastboot(&req, image, *symbols)
        }
        (FirmwareSource::FlashImage { image, symbols }, BootMode::RomBoot) => {
            build_romboot(&req, image, *symbols)
        }
        (FirmwareSource::Elf(_), BootMode::RomBoot) => Err(anyhow!(
            "RISC-V ROM boot needs a flash image (bootloader + partition table + app): the \
             mask ROM loads the 2nd-stage bootloader from flash, and an ELF has none"
        )),
    }
}

/// `new_from_config_riscv`: parse the ELF, then the program-image body.
///
/// SP is seeded at the top of RAM because fast boot skips the ROM and 2nd-stage
/// bootloader that would normally set it; the app's first prologue store would
/// otherwise fault.
fn build_plain(req: &BuildRequest<'_>, elf: &[u8]) -> anyhow::Result<BuiltMachine> {
    let program_image = parse_elf_image(elf).map_err(|e| anyhow!("Loader Error: {e:#}"))?;
    build_program_image(req, &program_image, elf.to_vec())
}

/// `new_from_config_riscv_program_image`.
fn build_program_image(
    req: &BuildRequest<'_>,
    program_image: &ProgramImage,
    firmware_bytes: Vec<u8>,
) -> anyhow::Result<BuiltMachine> {
    let (chip, manifest, blobs) = (req.chip, req.system, req.blobs);
    let echo = req.options.echo_uart_stdout;
    let mut bus =
        SystemBus::from_config(chip, manifest).map_err(|e| anyhow!("Bus config error: {e:#}"))?;

    // Inject the on-demand ESP32-C3 boot ROM blobs into the chip's still
    // zero-filled `rom`/`rom_data` regions, matching how the native `--rom-boot`
    // path provisions them. Absent blobs (non-C3 RISC-V chips, or a caller not
    // supplying them) leave the regions zero, preserving the fast-boot path.
    let faithful_c3_rom = {
        use crate::boot::esp32c3_rom::{DROM_BASE, IROM_BASE};
        let mut injected_irom: Option<Vec<u8>> = None;
        for mem in bus.extra_mem.iter_mut() {
            let src = if mem.base_addr == IROM_BASE as u64 {
                blobs.get("esp32c3_irom")
            } else if mem.base_addr == DROM_BASE as u64 {
                blobs.get("esp32c3_drom")
            } else {
                None
            };
            if let Some(src) = src {
                let n = src.len().min(mem.data.len());
                mem.data[..n].copy_from_slice(&src[..n]);
                if mem.base_addr == IROM_BASE as u64 {
                    injected_irom = Some(src.clone());
                }
            }
        }
        // Fast boot skips the ROM reset's own `.data` copy, so replicate it:
        // land the ROM's DRAM globals (ROM function tables esp-hal calls
        // dispatch through) exactly as silicon does — otherwise those calls
        // jump through a null/garbage pointer.
        if let Some(irom) = injected_irom {
            for (dst, bytes) in c3_rom_data_init_writes(&irom) {
                for (i, b) in bytes.iter().enumerate() {
                    let _ = bus.write_u8(dst as u64 + i as u64, *b);
                }
            }
            // With the real ROM present, esp-hal's clock bring-up runs the
            // genuine `rom_i2c_*Reg` helpers, which drive the analog I²C master
            // / ANA_CONFIG block (0x6000_E000) for the PLL. That block is not in
            // the chip YAML, so add it on the faithful path — otherwise the
            // first ROM PLL transaction faults on an unmapped access.
            bus.add_peripheral(
                "rtc_i2c_ana",
                0x6000_E000,
                0x400,
                None,
                Box::new(crate::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
            );
            bus.refresh_peripheral_index();
            // A peripheral added after bus assembly changes the input to
            // `derive_walk_deletable`; re-derive rather than rely on this model
            // happening to be inert.
            bus.recompute_walk_deletable();
            true
        } else {
            false
        }
    };

    let console = ConsoleCapture::for_manifest(manifest);
    let uart_sink = console.heard_sink();
    // On the faithful C3 ROM path, esp-println's `jtag-serial` feature prints
    // through USB_SERIAL_JTAG (0x6004_3000), not UART0. The chip YAML only has
    // a declarative register stub there, which never drains bytes, so install
    // the real behavioural model; a narrower, later-registered window overrides
    // the stub. `new_esp32c3()` so the CDC interrupt reaches the matrix.
    if faithful_c3_rom {
        use crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
        bus.add_peripheral(
            "usb_serial_jtag",
            0x6004_3000,
            0x100,
            None,
            Box::new(UsbSerialJtag::new_esp32c3()),
        );
        bus.refresh_peripheral_index();
        bus.recompute_walk_deletable();
    }
    // No mask ROM executes on this bare-ELF path, so nothing writes the same
    // bytes to both consoles: an undeclared manifest keeps capturing both into
    // one pane. A manifest that declares the board's console selects it.
    match console.tapped() {
        HostConsole::Undeclared => {
            if faithful_c3_rom {
                bus.attach_usb_serial_jtag_sink_echo(uart_sink.clone(), echo);
            }
            bus.attach_uart_tx_sink(uart_sink.clone(), echo);
        }
        tapped => {
            // Record the console with no connector FIRST, then tap the one the
            // host reads. Every console model holds ONE sink slot, so the LAST
            // writer wins.
            if tapped.is_usb_serial_jtag() {
                bus.attach_uart_tx_sink(console.unheard_sink(), false);
            } else if faithful_c3_rom {
                bus.attach_usb_serial_jtag_sink(console.unheard_sink());
            }
            bus.attach_host_console_echo(tapped, uart_sink.clone(), echo)
                .map_err(|e| anyhow!(e))?;
        }
    }
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    let cpu = crate::system::riscv::configure_riscv(&mut bus);
    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);

    machine
        .load_firmware(program_image)
        .map_err(|e| anyhow!("Simulation Error: {e}"))?;

    let sp_top = (chip.ram.base + chip.ram.size) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(program_image.entry_point as u32);

    Ok(built(
        machine,
        uart_sink,
        uart_rx_bufs,
        manifest,
        firmware_bytes,
    ))
}

/// `new_from_config_riscv_flash_fastboot`: the mask ROM is present but its
/// reset code is skipped; the 2nd-stage bootloader from the flash image is
/// loaded and entered directly.
fn build_flash_fastboot(
    req: &BuildRequest<'_>,
    flash: &[u8],
    symbols: Option<&[u8]>,
) -> anyhow::Result<BuiltMachine> {
    let (chip, manifest, blobs) = (req.chip, req.system, req.blobs);
    let mut bus =
        SystemBus::from_config(chip, manifest).map_err(|e| anyhow!("Bus config error: {e:#}"))?;

    let (Some(irom), Some(drom)) = (blobs.get("esp32c3_irom"), blobs.get("esp32c3_drom")) else {
        return Err(anyhow!(
            "C3 flash fast-start needs ESP32-C3 ROM blobs: pass esp32c3_irom + esp32c3_drom"
        ));
    };
    let images = RomImages {
        irom: irom.clone(),
        drom: drom.clone(),
    };
    if !inject_rom_regions(&mut bus, &images) {
        return Err(anyhow!(
            "C3 flash fast-start: chip YAML declares no IROM region at 0x40000000"
        ));
    }
    // The bootloader calls ROM helpers through DRAM tables initialized by the
    // mask ROM reset code. Because this path skips that reset code, copy those
    // ROM `.data` records before entering the second-stage bootloader.
    for (dst, bytes) in c3_rom_data_init_writes(irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(dst as u64 + i as u64, *b);
        }
    }

    let bootloader_image = esp32c3_bootloader_program_image_from_merged_flash(flash)
        .map_err(|e| anyhow!("ESP32-C3 flash fast-start: {e}"))?;

    let (console, usb_serial_sink) = attach_c3_flash_console(&mut bus, manifest, req)?;
    let uart_sink = console.heard_sink();
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    let mut machine = build_rom_boot_machine(
        bus,
        flash.to_vec(),
        RomBootOpts {
            // A new die per machine: two MCUs are two dies, with distinct WiFi
            // station MACs and BLE addresses.
            pinned_efuse_mac: None,
            usb_serial_sink: Some(usb_serial_sink),
        },
        |c| Box::new(c) as Box<dyn Cpu>,
    );
    echo_usb_serial_console(&mut machine, &console, req);
    load_program_segments_without_reset(&mut machine, &bootloader_image)
        .map_err(|e| anyhow!("C3 flash fast-start load: {e}"))?;

    let sp_top = (chip.ram.base + chip.ram.size) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(bootloader_image.entry_point as u32);

    crate::system::wifi::attach_configured_wifi_ap(&mut machine.bus, manifest);

    Ok(built(
        machine,
        uart_sink,
        uart_rx_bufs,
        manifest,
        symbols.map(<[u8]>::to_vec).unwrap_or_default(),
    ))
}

/// `new_from_config_riscv_romboot`: reset to the BROM vector `0x4000_0000` and
/// run the genuine mask ROM → 2nd-stage bootloader → `app_main()` from the
/// merged flash image. Peripheral wiring and the reset-vector boot are the
/// shared [`build_rom_boot_machine`], the same machine the native CLI builds.
fn build_romboot(
    req: &BuildRequest<'_>,
    flash: &[u8],
    symbols: Option<&[u8]>,
) -> anyhow::Result<BuiltMachine> {
    let (chip, manifest, blobs) = (req.chip, req.system, req.blobs);
    let mut bus =
        SystemBus::from_config(chip, manifest).map_err(|e| anyhow!("Bus config error: {e:#}"))?;

    // ROM boot cannot proceed without the real ROM — the reset vector executes
    // it directly.
    let (Some(irom), Some(drom)) = (blobs.get("esp32c3_irom"), blobs.get("esp32c3_drom")) else {
        return Err(anyhow!(
            "rom-boot needs the ESP32-C3 boot ROM: pass esp32c3_irom + esp32c3_drom blobs"
        ));
    };
    let images = RomImages {
        irom: irom.clone(),
        drom: drom.clone(),
    };
    if !inject_rom_regions(&mut bus, &images) {
        return Err(anyhow!(
            "rom-boot: chip YAML declares no IROM region at 0x40000000 to load the boot ROM"
        ));
    }

    let (console, usb_serial_sink) = attach_c3_flash_console(&mut bus, manifest, req)?;
    let uart_sink = console.heard_sink();
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    let mut machine = build_rom_boot_machine(
        bus,
        flash.to_vec(),
        RomBootOpts {
            pinned_efuse_mac: None,
            usb_serial_sink: Some(usb_serial_sink),
        },
        |c| Box::new(c) as Box<dyn Cpu>,
    );
    echo_usb_serial_console(&mut machine, &console, req);

    crate::system::wifi::attach_configured_wifi_ap(&mut machine.bus, manifest);

    Ok(built(
        machine,
        uart_sink,
        uart_rx_bufs,
        manifest,
        symbols.map(<[u8]>::to_vec).unwrap_or_default(),
    ))
}

fn built(
    machine: Machine<Box<dyn Cpu>>,
    sink: Arc<Mutex<Vec<u8>>>,
    rx: Vec<Arc<Mutex<std::collections::VecDeque<u8>>>>,
    manifest: &SystemManifest,
    firmware_bytes: Vec<u8>,
) -> BuiltMachine {
    BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires { sink, rx },
        board_io: manifest.board_io.clone(),
        arch: labwired_config::Arch::RiscV,
        firmware_bytes,
    }
}

/// `attach_c3_flash_console`: the one console decision for the two merged-flash
/// paths, where a real mask ROM runs and prints its banner to UART0 AND
/// USB-Serial-JTAG. Exactly one console is heard (a board's socket is soldered
/// to one of them); the other is recorded but not shown.
///
/// Returns the capture plus the sink for `RomBootOpts::usb_serial_sink`: the
/// USB-Serial-JTAG model is added by [`build_rom_boot_machine`] after the bus is
/// handed over, so it is the one console that cannot be attached here.
fn attach_c3_flash_console(
    bus: &mut SystemBus,
    manifest: &SystemManifest,
    req: &BuildRequest<'_>,
) -> anyhow::Result<(ConsoleCapture, Arc<Mutex<Vec<u8>>>)> {
    let console = ConsoleCapture::for_manifest(manifest);
    if console.tapped().is_usb_serial_jtag() {
        // `deploy.usb: native` board: the USB-C socket IS the C3's
        // USB-Serial-JTAG; UART0 comes out on header pins with nothing attached.
        bus.attach_uart_tx_sink(console.unheard_sink(), false);
        let usb = console.heard_sink();
        Ok((console, usb))
    } else {
        // Bridge-chip board, or an undeclared manifest (historical default:
        // UART0, where every Arduino/IDF lab shipped so far prints).
        bus.attach_host_console_echo(
            console.tapped(),
            console.heard_sink(),
            req.options.echo_uart_stdout,
        )
        .map_err(|e| anyhow!(e))?;
        let usb = console.unheard_sink();
        Ok((console, usb))
    }
}

/// `RomBootOpts` wires the USB-Serial-JTAG sink without an echo flag. When that
/// is the heard console and the caller asked for an echo, re-point the same
/// sink with echo on; with echo off (the browser's case) this does nothing, so
/// the machine is the one the browser builds.
fn echo_usb_serial_console(
    machine: &mut Machine<Box<dyn Cpu>>,
    console: &ConsoleCapture,
    req: &BuildRequest<'_>,
) {
    if req.options.echo_uart_stdout && console.tapped().is_usb_serial_jtag() {
        machine
            .bus
            .attach_usb_serial_jtag_sink_echo(console.heard_sink(), true);
    }
}

/// Parse an ESP image (the 24-byte header, then `(load_addr, len, data)`
/// segments) starting at `offset` of a merged flash image.
fn esp32c3_program_image_from_flash_offset(
    flash: &[u8],
    offset: usize,
    label: &str,
) -> Result<ProgramImage, String> {
    let image = flash.get(offset..).ok_or_else(|| {
        format!("ESP32-C3 flash image is smaller than {label} offset {offset:#x}")
    })?;
    if image.len() < ESP_IMAGE_HEADER_LEN {
        return Err(format!("ESP32-C3 {label} image header is truncated"));
    }
    if image[0] != ESP_IMAGE_MAGIC {
        return Err(format!(
            "ESP32-C3 {label} image has bad magic 0x{:02x} at flash offset {offset:#x}",
            image[0],
        ));
    }

    let segment_count = image[1] as usize;
    let entry = u32::from_le_bytes(image[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, crate::Arch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;

    for index in 0..segment_count {
        let header = image
            .get(cursor..cursor + 8)
            .ok_or_else(|| format!("ESP32-C3 {label} segment {index} header is truncated"))?;
        let load_addr = u32::from_le_bytes(header[0..4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        cursor += 8;
        let data = image
            .get(cursor..cursor + len)
            .ok_or_else(|| format!("ESP32-C3 {label} segment {index} data is truncated"))?;
        program.add_segment(load_addr, data.to_vec());
        cursor += len;
    }

    if program.segments.is_empty() {
        return Err(format!("ESP32-C3 {label} image has no loadable segments"));
    }

    Ok(program)
}

fn esp32c3_bootloader_program_image_from_merged_flash(
    flash: &[u8],
) -> Result<ProgramImage, String> {
    esp32c3_program_image_from_flash_offset(flash, 0, "bootloader")
}

/// Place segments into memory without the CPU reset `load_firmware` performs.
fn load_program_segments_without_reset(
    machine: &mut Machine<Box<dyn Cpu>>,
    program_image: &ProgramImage,
) -> Result<(), String> {
    for segment in &program_image.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }

        for (i, byte) in segment.data.iter().enumerate() {
            let addr = segment.start_addr + i as u64;
            machine
                .bus
                .write_u8(addr, *byte)
                .map_err(|e| format!("load segment at {addr:#x}: {e}"))?;
        }
    }

    Ok(())
}
