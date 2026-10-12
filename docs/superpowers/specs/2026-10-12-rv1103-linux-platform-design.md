# RV1103 Linux and smart-basket simulation

The user requested Linux/RV1103 implementation on 2026-10-12 and delegated component and implementation choices. Hardware is unavailable. The existing release repairs remain a separate merge and deployment stream.

## Outcome

Run a real ARM Linux kernel and userspace on a native LabWired machine with the Mini A's 64 MiB DDR at physical address zero, RV1103 interrupt and peripheral addresses, and the existing authoritative component bus. Execute Linux I2C transactions against the ADXL345 and UART transactions against a behavioral ESP-AT module. Keep evidence for CPU execution, kernel boot, device transactions, modeled power, and hardware measurements separate.

Linux direct boot is an explicit supported boot mode. It starts after ROM, DDR training and bootloader initialization; it does not certify those stages or the Mini A SD boot chain. NPU, camera and RF propagation are outside the requested basket circuit. A minimal device tree must disclose every omitted controller and fixed-clock substitution.

## Architecture choice

1. Embed the QEMU-derived tlib CPU used by Renode, pinned to its v1.17.0 source revision. This reuses architectural MMU, privileged execution, exception entry and IRQ interfaces while LabWired retains the physical memory/device callbacks. Recommended.
2. Patch Unicorn's exception and IRQ internals. The current CPU backend already uses Unicorn, but its public API does not deliver full architectural exceptions; maintaining a private ABI introduces additional correctness and upgrade work.
3. Run a separate generic QEMU virtual machine. This can boot Linux quickly, but provides a different memory map and independently owned devices; it does not establish RV1103 or shared-circuit support.

Retain the existing optional Unicorn instruction fixture. Add an independent native system backend with explicit availability and capability reporting. Default and browser builds must not acquire native dependencies. Each backend instance owns its native state, callbacks and teardown; shared native globals must be isolated between machines. Bus callbacks are scoped, exact-width and panic-contained. No native callback may retain a Rust bus borrow beyond its execution call.

## Native system CPU

Integrate the upstream ARMv7-A Cortex-A7 model through Cpu, Machine, Bus and observer boundaries. Reuse upstream decoding, MMU/TLB and exceptions rather than translating ARM instructions in Rust. Support ARM/Thumb-2, CP15 translation, banked state, SVC and fault vectors, IRQ level assertion/deassertion, WFI wake, and explicit executed-instruction accounting. Scheduling ticks are modeled time, not measured silicon cycles.

Map authoritative RAM through a lifetime-safe native memory interface or width-preserving callbacks. Native code/data invalidation must follow guest and host writes. Physical MMIO callbacks must observe translations made by the CPU. Unsupported native host configurations, unresolved callbacks, inaccessible physical addresses and guest faults must yield explicit outcomes; they cannot silently become successful zero reads.

## RV1103 boot slice

Vendor source: LuckfoxTECH/luckfox-pico revision 824b817f889c2cbff1d48fcdb18ab494a68f69d1. Its shared RV1106 DTS and RV1103 overrides establish:

| Device | Physical map | Interrupt |
| --- | --- | --- |
| DDR | 0x00000000, 64 MiB | none |
| SRAM | 0xff6c0000, 256 KiB | none |
| GIC-400 | GICD 0xff1f1000, GICC 0xff1f2000 | CPU IRQ/FIQ |
| Architectural timer | CP15, 24 MHz | PPI 13/14 |
| Debug UART2 | 0xff4c0000, DW APB, reg-shift 2 | SPI 27 |
| Basket UART4 | 0xff4e0000, DW APB, reg-shift 2 | SPI 29 |
| Rockchip I2C3 | 0xff460000 | SPI 21 |
| SDMMC | 0xffaa0000, DW MSHC | SPI 52 |

Implement GIC distributor and CPU-interface enable, priorities, routing, acknowledge and EOI behavior needed by a single Cortex-A7 Linux guest. Implement the architectural timer's counter, compare, control/mask and IRQ behavior. UART models must include register-width/stride, DLAB, FIFOs, line status and interrupt behavior. The Rockchip I2C controller must execute address/data transfers and generate completion/error interrupts; a probe-only register stub is insufficient.

A minimal vendor-derived tree can use disclosed fixed clocks and initramfs to prove the CPU/GIC/timer/UART path before CRU and SD dependencies. Upstream mainline does not automatically match an RV1103 compatible, so the kernel machine/configuration must be explicitly supported and recorded. Do not introduce another SoC compatible that executes initialization at incorrect physical addresses.

## Linux artifacts and loader

Use pinned kernel source/configuration, device-tree source and initramfs construction with hashes and licenses. The boot receipt identifies every input byte sequence and the native backend version. ARM boot entry initializes r0=0, r1 to the documented machine identifier or DT-only value, r2 to the physical DTB address, SVC mode, masked interrupts and disabled MMU, following the kernel ARM boot ABI. Validate image format, memory bounds and non-overlapping kernel, DTB and initramfs placement before loading.

The acceptance guest emits a boot identity and a userspace sentinel, executes a syscall, and demonstrates a timer interrupt/wakeup. The basket guest additionally reads ADXL345 device identity and stimulated axis values through the Linux I2C driver, and sends AT commands through UART4 to the attached behavioral module. Stubbed devices and reduced-order power assumptions appear in the receipt and UI capability report.

## Storage and vendor boot follow-on

MicroSD is the real Mini A storage path. A separate gate requires DW MSHC FIFO, command/response, SD block protocol, interrupt and DMA behavior plus CRU/reset dependencies, then reads the root filesystem through the guest SD driver. Complete vendor-image support additionally requires ROM/SPL/U-Boot and DDR initialization evidence. Passing direct boot cannot satisfy these gates.

## Verification

- Native CPU red/green tests: architectural translation, SVC/fault entry, IRQ acknowledgement/deassertion, Thumb-2, WFI and exact physical bus accesses.
- Device tests: GIC priorities/EOI, timer deadlines/masking, UART FIFOs/IRQ, I2C transfer/error/IRQ with ADXL345 state.
- Loader negative tests: malformed images, mismatched hashes, oversized/overlapping segments, unavailable backend and unsupported board/machine.
- Real kernel acceptance: observed kernel log plus userspace/syscall/timer assertions, exact artifact and backend hashes, bounded timeout and failure output.
- Circuit acceptance: stimulated ADXL345 registers reached through Linux; UART4 exchanges reach ESP-AT; modeled power behavior is labeled and recorded.
- Existing instruction, default, browser and MCU regressions remain green. Protected merge and live deployment checks remain mandatory.

Physical USB isolation, power sequencing/backfeed, pack/NTC/polarity, module footprint, thermal/current/EMI and programming checks remain pending until hardware is available.

## Sources

- https://github.com/renode/renode/releases/tag/v1.17.0
- https://github.com/antmicro/tlib/tree/167decf9129758829762e582939128c0694e90d6
- https://github.com/LuckfoxTECH/luckfox-pico/blob/824b817f889c2cbff1d48fcdb18ab494a68f69d1/sysdrv/source/kernel/arch/arm/boot/dts/rv1106.dtsi
- https://github.com/LuckfoxTECH/luckfox-pico/blob/824b817f889c2cbff1d48fcdb18ab494a68f69d1/sysdrv/source/kernel/arch/arm/boot/dts/rv1103.dtsi
- https://www.kernel.org/doc/html/latest/arch/arm/booting.html
