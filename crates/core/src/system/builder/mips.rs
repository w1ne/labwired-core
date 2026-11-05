// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MIPS (PIC32MZ) construction: bus from config, host console wired to the
//! chip's UARTs, the ELF loaded and the core started at its reset vector.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::bus::SystemBus;
use crate::console::ConsoleCapture;
use anyhow::anyhow;

pub(super) fn build(
    req: BuildRequest<'_>,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<BuiltMachine> {
    let firmware = match (&req.firmware, req.boot) {
        (FirmwareSource::Elf(elf), BootMode::FastBoot) => *elf,
        _ => {
            return Err(anyhow!(
                "not supported: MIPS boots only from an ELF (FirmwareSource::Elf with \
                 BootMode::FastBoot); chip '{}' has no flash-image boot path",
                req.chip.name
            ))
        }
    };
    let mut bus = SystemBus::from_config_with_plugins(req.chip, req.system, plugins)
        .map_err(|e| anyhow!("Bus config error: {e:#}"))?;
    let console = ConsoleCapture::for_manifest(req.system);
    let sink = console.heard_sink();
    bus.attach_host_console_echo(console.tapped(), sink.clone(), req.options.echo_uart_stdout)
        .map_err(|e| anyhow!(e))?;
    let rx = super::wire_host_streams(&mut bus, &req.options)?;
    let machine = crate::system::mips::load_elf(bus, firmware)?;
    Ok(BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires { sink, rx },
        board_io: req.system.board_io.clone(),
        arch: labwired_config::Arch::Mips,
        firmware_bytes: firmware.to_vec(),
    })
}
