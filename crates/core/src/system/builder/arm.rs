// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Cortex-M construction, moved from `labwired-wasm`'s `new_from_config_arm`.
//!
//! The body mirrors the browser constructor step for step: bus from config,
//! host console through [`ConsoleCapture`] (so a `debug_uart:` naming a UART
//! the bus does not have is an error, not a silent fallback), every UART RX
//! queue collected as a feeder, `configure_cortex_m`, a boxed CPU, and the ELF
//! loaded without a reset. The one substitution is the ELF parser: core cannot
//! depend on `labwired-loader` (the loader depends on core), so it uses
//! [`parse_elf_image`], which places `PT_LOAD` segments at `p_paddr` exactly as
//! `labwired_loader::load_elf_bytes` does for every non-AVR image.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::bus::SystemBus;
use crate::console::ConsoleCapture;
use crate::system::cortex_m::configure_cortex_m;
use crate::system::node::parse_elf_image;
use crate::{Cpu, Machine};
use anyhow::anyhow;

pub(super) fn build(req: BuildRequest<'_>) -> anyhow::Result<BuiltMachine> {
    // The browser constructor has one Cortex-M path, an ELF loaded without a
    // reset. A ROM-boot request must say so, not silently get that path.
    if req.boot == BootMode::RomBoot {
        return Err(anyhow!(
            "not supported: Cortex-M ROM boot (chip '{}'); only an ELF load \
             (BootMode::FastBoot) is modelled",
            req.chip.name
        ));
    }
    let mut bus = SystemBus::from_config(req.chip, req.system)
        .map_err(|e| anyhow!("Bus config error: {e:#}"))?;

    let console = ConsoleCapture::for_manifest(req.system);
    let sink = console.heard_sink();
    bus.attach_host_console_echo(console.tapped(), sink.clone(), req.options.echo_uart_stdout)
        .map_err(|e| anyhow!(e))?;
    let rx = super::uart_rx_sources(&bus, &req.options)?;

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);

    let elf = match req.firmware {
        FirmwareSource::Elf(bytes) => bytes,
        FirmwareSource::FlashImage { .. } => {
            return Err(anyhow!("ARM targets take an ELF, not a flash image"))
        }
    };
    let image = parse_elf_image(elf).map_err(|e| anyhow!("Loader Error: {e:#}"))?;
    machine
        .load_firmware(&image)
        .map_err(|e| anyhow!("Simulation Error: {e}"))?;

    Ok(BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires { sink, rx },
        board_io: req.system.board_io.clone(),
        arch: labwired_config::Arch::Arm,
        firmware_bytes: elf.to_vec(),
    })
}
