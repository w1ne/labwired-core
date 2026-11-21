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

/// The ADC1 channels a non-dsPIC33F part bonds out, from the descriptor's
/// `analog_pins` entries on its `pic24_adc_pads` window.
///
/// The core's default derives the channels from the port masks with the
/// dsPIC33FJ32MC20x layout (AN0-AN8 on RA0, RA1, RB0-RB3, RC0-RC2), and the
/// dsPIC descriptors agree with it, so they keep that path and its AD1PCFGL /
/// AD1CSSL masks unchanged. Other families put their inputs elsewhere: the
/// PIC24F16KA102 family has AN0-AN5 and AN10-AN12, and its 20-pin part has
/// AN4/AN5 on RA2/RA3 (DS39927C Table 1-2).
pub fn pic24_adc_channels(chip: &labwired_config::ChipDescriptor) -> Option<u16> {
    if chip.core.as_deref() == Some("dspic33f") {
        return None;
    }
    let pads = chip
        .peripherals
        .iter()
        .find(|p| p.r#type == "pic24_adc_pads")?;
    let channels = chip
        .analog_pins
        .values()
        .filter(|f| f.peripheral == pads.id && f.channel < 16)
        .fold(0u16, |mask, f| mask | (1 << f.channel));
    (channels != 0).then_some(channels)
}

pub(super) fn build(
    req: BuildRequest<'_>,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<BuiltMachine> {
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

    // M3: SPI1/I2C1 are CPU-internal, so the generic bus must not try to
    // attach their `external_devices` to bus peripherals. Build the bus
    // without those entries and attach them to the CPU below.
    let mut bus_manifest = manifest.clone();
    bus_manifest.external_devices.clear();
    let mut bus = SystemBus::from_config_with_plugins(chip, &bus_manifest, plugins)
        .map_err(|e| anyhow!("Bus config error: {e:#}"))?;
    // Keep the manifest's declarations for inspection even though the
    // devices themselves are attached to the CPU, not to bus peripherals.
    bus.record_external_devices(manifest);

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
    if program_image.arch != crate::Arch::Pic24 {
        return Err(anyhow!(
            "firmware architecture {:?} is incompatible with PIC24 chip '{}'",
            program_image.arch,
            chip.name
        ));
    }

    let mut cpu = crate::cpu::Pic24::new();
    cpu.set_port_masks(super::pic24_port_masks(chip));
    if let Some(channels) = pic24_adc_channels(chip) {
        cpu.set_adc_channels(channels);
    }
    // ADC1 converts routed AN levels only when the chip declares the window.
    cpu.set_adc_pad_window(
        chip.peripherals
            .iter()
            .any(|p| p.r#type == "pic24_adc_pads"),
    );
    cpu.load_program_image(&program_image);
    // UART1 TX is on the CPU, not a bus UART peripheral.
    cpu.set_serial_sink(uart_sink.clone());

    // M3: the SPI1/I2C1 controllers live on the CPU too, so the manifest's
    // devices are attached to it rather than to bus peripherals.
    for device in &manifest.external_devices {
        match device.connection.as_str() {
            "i2c1" => {
                let model = crate::peripherals::components::i2c_factory::build_external_i2c_device(
                    &device.r#type,
                    &device.id,
                    &device.config,
                )
                .ok_or_else(|| {
                    anyhow!(
                        "unsupported I2C device type '{}' on i2c1 (device '{}')",
                        device.r#type,
                        device.id
                    )
                })?;
                cpu.attach_i2c_device(model);
            }
            "spi1" => {
                let kind = device.r#type.to_ascii_lowercase();
                if kind == "loopback" || kind == "spi_loopback" {
                    let cs = device.route.get("cs").cloned().unwrap_or_default();
                    cpu.attach_spi_device(Box::new(
                        crate::peripherals::spi_loopback::SpiLoopback::new(cs),
                    ));
                } else {
                    return Err(anyhow!(
                        "unsupported SPI device type '{}' on spi1 (device '{}'); the PIC24 \
                         M3 model supports a wire loopback only",
                        device.r#type,
                        device.id
                    ));
                }
            }
            other => {
                return Err(anyhow!(
                    "external device '{}' uses connection '{other}', which the PIC24 \
                     model does not provide (only i2c1 and spi1 are CPU-internal)",
                    device.id
                ))
            }
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
        arch: labwired_config::Arch::Pic24,
        firmware_bytes: firmware.to_vec(),
    })
}
