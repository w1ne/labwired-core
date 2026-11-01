// SPDX-License-Identifier: MIT

use super::{BuildRequest, BuiltMachine};

pub(super) fn build(
    req: BuildRequest<'_>,
    _plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<BuiltMachine> {
    #[cfg(all(feature = "qemu-arm-system", not(target_arch = "wasm32")))]
    {
        use super::{BootMode, FirmwareSource, UartWires};
        use crate::bus::SystemBus;
        use crate::console::ConsoleCapture;
        if req.boot != BootMode::FastBoot {
            anyhow::bail!("Cortex-R5F boot ROM / FSBL boot is not modelled; use an ELF");
        }
        let FirmwareSource::Elf(elf) = req.firmware else {
            anyhow::bail!("Cortex-R5F takes an initialized-memory ELF, not a flash image");
        };
        let mut bus = SystemBus::from_config_with_plugins(req.chip, req.system, _plugins)?;
        // The RPU UARTs must exist before the console attaches to them.
        let cpu = crate::system::cortex_r::attach(&mut bus, req.chip)?;
        let console = ConsoleCapture::for_manifest(req.system);
        let sink = console.heard_sink();
        bus.attach_host_console_echo(console.tapped(), sink.clone(), req.options.echo_uart_stdout)
            .map_err(|error| anyhow::anyhow!(error))?;
        let rx = super::wire_host_streams(&mut bus, &req.options)?;
        let machine = crate::system::cortex_r::load(bus, cpu, elf)?;
        Ok(BuiltMachine {
            machine: Box::new(machine),
            uart: UartWires { sink, rx },
            board_io: req.system.board_io.clone(),
            arch: labwired_config::Arch::Arm,
            firmware_bytes: elf.to_vec(),
        })
    }
    #[cfg(not(all(feature = "qemu-arm-system", not(target_arch = "wasm32"))))]
    {
        anyhow::bail!("chip '{}': Cortex-R5F needs a native build with --features qemu-arm-system; browser execution is unavailable", req.chip.name)
    }
}
