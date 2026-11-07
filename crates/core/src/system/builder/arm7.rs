// SPDX-License-Identifier: MIT

use super::{BuildRequest, BuiltMachine};

pub(super) fn build(
    req: BuildRequest<'_>,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<BuiltMachine> {
    #[cfg(all(feature = "qemu-arm-system", not(target_arch = "wasm32")))]
    {
        use super::{BootMode, FirmwareSource, UartWires};
        use crate::bus::SystemBus;
        use crate::console::ConsoleCapture;
        if req.boot != BootMode::FastBoot {
            anyhow::bail!("ARM7TDMI ROM boot is not modelled; use a bare-metal ELF");
        }
        let FirmwareSource::Elf(elf) = req.firmware else {
            anyhow::bail!("ARM7TDMI takes a bare-metal ELF, not a flash image");
        };
        let mut bus = SystemBus::from_config_with_plugins(req.chip, req.system, plugins)?;
        let console = ConsoleCapture::for_manifest(req.system);
        let sink = console.heard_sink();
        bus.attach_host_console_echo(console.tapped(), sink.clone(), req.options.echo_uart_stdout)
            .map_err(|error| anyhow::anyhow!(error))?;
        let rx = super::wire_host_streams(&mut bus, &req.options)?;
        let machine = crate::system::arm7::load_elf(bus, req.chip, elf)?;
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
        let _ = (req, plugins);
        anyhow::bail!("ARM7TDMI needs a native build with --features qemu-arm-system; browser execution is unavailable")
    }
}
