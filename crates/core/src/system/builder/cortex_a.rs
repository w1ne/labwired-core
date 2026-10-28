// SPDX-License-Identifier: MIT

use super::{BuildRequest, BuiltMachine};

pub(super) fn build(
    req: BuildRequest<'_>,
    _plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<BuiltMachine> {
    #[cfg(all(feature = "qemu-arm", not(target_arch = "wasm32")))]
    {
        use super::{BootMode, FirmwareSource, UartWires};
        use crate::bus::SystemBus;
        use crate::console::ConsoleCapture;
        if req.boot != BootMode::FastBoot {
            anyhow::bail!("Cortex-A7 ROM/Linux boot is not implemented; use a bare-metal ELF");
        }
        let FirmwareSource::Elf(elf) = req.firmware else {
            anyhow::bail!("Cortex-A7 currently takes a bare-metal ELF, not a flash image");
        };
        let mut bus = SystemBus::from_config_with_plugins(req.chip, req.system, _plugins)?;
        let console = ConsoleCapture::for_manifest(req.system);
        let sink = console.heard_sink();
        bus.attach_host_console_echo(console.tapped(), sink.clone(), req.options.echo_uart_stdout)
            .map_err(|error| anyhow::anyhow!(error))?;
        let rx = super::uart_rx_sources(&bus, &req.options)?;
        let machine = crate::system::cortex_a::load_elf(bus, elf)?;
        Ok(BuiltMachine {
            machine: Box::new(machine),
            uart: UartWires { sink, rx },
            board_io: req.system.board_io.clone(),
            arch: labwired_config::Arch::Arm,
            firmware_bytes: elf.to_vec(),
        })
    }
    #[cfg(not(all(feature = "qemu-arm", not(target_arch = "wasm32"))))]
    {
        anyhow::bail!("chip '{}': Cortex-A7 needs a native build with --features qemu-arm; browser execution is unavailable", req.chip.name)
    }
}
