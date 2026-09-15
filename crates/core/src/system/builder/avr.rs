// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AVR8 (ATmega328P / classic Arduino Nano) construction, moved from
//! `labwired-wasm`'s `new_from_config_avr`.
//!
//! The body is the browser's, step for step: bus from config, host console, RX
//! feeders, the ELF loaded into the Harvard CPU's own flash, and the SPI / I²C
//! devices moved off the bus controllers onto the CPU model. Two substitutions:
//!
//! * ELF parsing uses [`parse_avr_elf_image`], core's copy of the loader's AVR
//!   segment mapping (core cannot depend on `labwired-loader`), and it refuses
//!   an ELF that is not AVR.
//! * The CPU's USART is wired to the heard console sink. The ATmega328P's
//!   USART is modelled on the CPU, not as a bus peripheral, so the browser's
//!   `attach_host_console` reaches no UART on this bus and its Serial pane
//!   stays empty. The CLI `test` path and `world` both call
//!   `Avr::set_serial_sink`; this does the same. The model has no stdout echo
//!   hook, so `BuildOptions::echo_uart_stdout` only reaches bus UARTs here.

use super::{BootMode, BuildRequest, BuiltMachine, FirmwareSource, UartWires};
use crate::bus::SystemBus;
use crate::console::ConsoleCapture;
use crate::system::node::parse_avr_elf_image;
use crate::{Cpu, Machine};
use anyhow::anyhow;

pub(super) fn build(req: BuildRequest<'_>) -> anyhow::Result<BuiltMachine> {
    let firmware = match (&req.firmware, req.boot) {
        (FirmwareSource::Elf(elf), BootMode::FastBoot) => *elf,
        _ => {
            return Err(anyhow!(
                "not supported: AVR boots only from an ELF (FirmwareSource::Elf with \
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

    let program_image =
        parse_avr_elf_image(firmware).map_err(|e| anyhow!("Loader Error: {e:#}"))?;
    let mut cpu = crate::cpu::Avr::new();
    cpu.load_program_image(&program_image);
    // USART TX is on the CPU, not a bus UART peripheral.
    cpu.set_serial_sink(uart_sink.clone());
    // SPI/I2C kits park on bus controllers; SPDR/TWCR clock them from the CPU
    // model (same as build_avr_node / CLI).
    for name in ["spi", "spi0", "spi1"] {
        for dev in bus.take_spi_devices(name) {
            cpu.push_spi_device(dev);
        }
    }
    for name in ["i2c", "i2c0", "twi"] {
        for dev in bus.take_i2c_slaves(name) {
            cpu.push_i2c_slave(dev);
        }
    }
    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let machine = Machine::new(boxed, bus);

    Ok(BuiltMachine {
        machine: Box::new(machine),
        uart: UartWires {
            sink: uart_sink,
            rx: uart_rx_bufs,
        },
        board_io: manifest.board_io.clone(),
        arch: labwired_config::Arch::Avr,
        firmware_bytes: firmware.to_vec(),
    })
}
