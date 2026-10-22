# External Components — RISC-V `virt` machine

None. The smoke firmware exercises the machine's own surface only: CPU, RAM
and the NS16550 UART console. The UART is the modelled console and needs no
external device — the engine captures its byte stream directly.

A `virt` machine can be extended with virtio devices (block, net, console),
a PCI host bridge, an RTC and a GPIO controller; none of these are modelled
or attached in this example:

| Component | Machine wiring | State |
| :--- | :--- | :--- |
| Virtio block/net/console | PCI or MMIO transport | not modelled |
| PCI host bridge | ECAM + MMIO windows | not modelled |
| PLIC / CLINT | platform interrupt controllers | declared `type: stub` (the smoke polls) |
| RTC / GPIO | machine options | not modelled |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
