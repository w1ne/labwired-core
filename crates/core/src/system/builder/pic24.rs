// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC24/dsPIC33 (M0) construction — dsPIC33FJ32MC204 class.
//!
//! Mirrors the AVR builder: bus from config, host console wired as the heard
//! sink, the ELF loaded into the CPU's own flash, and UART1 modelled on the
//! CPU (there is no bus UART peripheral for this chip — writing `U1TXREG`
//! reaches the console sink directly).
//!
//! M0 scope: boot + UART TX. See `crate::cpu::pic24` for the modelled
//! instruction subset and `docs/pic24-m0-reference.md` for encodings.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::bus::SystemBus;
use crate::console::ConsoleCapture;
use crate::system::node::parse_elf_image;
use crate::{Cpu, Machine};
use anyhow::anyhow;

pub(super) fn build(req: BuildRequest<'_>) -> anyhow::Result<BuiltMachine> {
    let firmware = match (&req.firmware, req.boot) {
        (FirmwareSource::Elf(elf), BootMode::FastBoot) => *elf,
        _ => {
            return Err(anyhow!(
                "not supported: PIC24 boots only from an ELF (FirmwareSource::Elf with \
                 BootMode::FastBoot); chip '{}' has no flash-image or mask-ROM boot path",
                req.chip.name
            ))
        }
    };
    let (chip, manifest) = (req.chip, req.system);

    let mut bus =
        SystemBus::from_config(chip, manifest).map_err(|e| anyhow!("Bus config error: {e:#}"))?;

    let console = ConsoleCapture::for_manifest(manifest);
    let uart_sink = console.heard_sink();
    bus.attach_host_console_echo(
        console.tapped(),
        uart_sink.clone(),
        req.options.echo_uart_stdout,
    )
    .map_err(|e| anyhow!(e))?;
    let uart_rx_bufs = super::uart_rx_sources(&bus, &req.options)?;

    let program_image = parse_elf_image(firmware).map_err(|e| anyhow!("Loader Error: {e:#}"))?;
    if program_image.arch != crate::Arch::Pic24 {
        return Err(anyhow!(
            "firmware architecture {:?} is incompatible with PIC24 chip '{}'",
            program_image.arch,
            chip.name
        ));
    }

    let mut cpu = crate::cpu::Pic24::new();
    cpu.load_program_image(&program_image);
    // UART1 TX is on the CPU, not a bus UART peripheral.
    cpu.set_serial_sink(uart_sink.clone());

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let machine = Machine::new(boxed, bus);

    Ok(BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires {
            sink: uart_sink,
            rx: uart_rx_bufs,
        },
        board_io: manifest.board_io.clone(),
        arch: labwired_config::Arch::Pic24,
        firmware_bytes: firmware.to_vec(),
    })
}
