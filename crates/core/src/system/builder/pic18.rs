// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC18 (M0) construction.
//!
//! Mirrors the PIC24 builder: a bus from config, host console wired as the
//! heard sink, and the firmware image loaded into the CPU's own Harvard flash.
//! The core is built with the chip's own geometry
//! ([`crate::cpu::Pic18::for_chip_name`]: flash size, Access Bank split), and
//! the EUSART is CPU-internal — writing `TXREG` reaches the console sink
//! directly, as on the CLI `run`/`test` paths.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::bus::SystemBus;
use crate::console::ConsoleCapture;
use crate::system::node::parse_elf_image;
use crate::{Cpu, Machine};
use anyhow::anyhow;

pub(super) fn build(
    req: BuildRequest<'_>,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<BuiltMachine> {
    let firmware = match (&req.firmware, req.boot) {
        (FirmwareSource::Elf(elf), BootMode::FastBoot) => *elf,
        _ => {
            return Err(anyhow!(
                "not supported: PIC18 boots only from an ELF (FirmwareSource::Elf with \
                 BootMode::FastBoot); chip '{}' has no flash-image or mask-ROM boot path",
                req.chip.name
            ))
        }
    };
    let (chip, manifest) = (req.chip, req.system);

    let mut bus = SystemBus::from_config_with_plugins(chip, manifest, plugins)
        .map_err(|e| anyhow!("Bus config error: {e:#}"))?;

    let console = ConsoleCapture::for_manifest(manifest);
    let uart_sink = console.heard_sink();
    bus.attach_host_console_echo(
        console.tapped(),
        uart_sink.clone(),
        req.options.echo_uart_stdout,
    )
    .map_err(|e| anyhow!(e))?;
    let uart_rx_bufs = super::wire_host_streams(&mut bus, &req.options)?;

    let program_image = parse_elf_image(firmware).map_err(|e| anyhow!("Loader Error: {e:#}"))?;
    if program_image.arch != crate::Arch::Pic18 {
        return Err(anyhow!(
            "firmware architecture {:?} is incompatible with PIC18 chip '{}'",
            program_image.arch,
            chip.name
        ));
    }
    crate::cpu::pic18::check_pic18_entry(program_image.entry_point)
        .map_err(|e| anyhow!("Loader Error: {e}"))?;

    let mut cpu = new_core(&chip.name, uart_sink.clone());
    cpu.load_program_image(&program_image);

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let machine = Machine::new(boxed, bus);

    Ok(BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires {
            sink: uart_sink,
            rx: uart_rx_bufs,
        },
        board_io: manifest.board_io.clone(),
        arch: labwired_config::Arch::Pic18,
        firmware_bytes: firmware.to_vec(),
    })
}

/// The chip's own core with its EUSART wired to the console `sink`.
fn new_core(chip_name: &str, sink: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> crate::cpu::Pic18 {
    let mut cpu = crate::cpu::Pic18::for_chip_name(chip_name);
    // EUSART TX is on the CPU, not a bus UART peripheral.
    cpu.set_serial_sink(sink);
    cpu
}

#[cfg(test)]
mod tests {
    use super::new_core;

    #[test]
    fn pic18f4520_core_has_the_chip_geometry() {
        // DS39631E: 32 KB flash, SFRs from 0xF80 so the Access Bank splits
        // at 0x80 (the generic default core splits at 0x60).
        let cpu = new_core("pic18f4520", Default::default());
        assert_eq!(cpu.access_split(), 0x80);
        assert_eq!(cpu.flash_bytes(), 32 * 1024);
    }
}
