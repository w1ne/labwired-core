# Required references

- [Linux ARM boot protocol](https://www.kernel.org/doc/html/latest/arch/arm/booting.html): zImage entry, registers, cache/MMU and DTB placement.
- [Pinned Linux v6.18.56 Generic DT fallback](https://github.com/gregkh/linux/blob/v6.18.56/arch/arm/kernel/devtree.c): `setup_machine_fdt` provides `Generic DT based system`; no invented fallback compatibility is needed.
- [Pinned ARM GIC binding](https://github.com/gregkh/linux/blob/v6.18.56/Documentation/devicetree/bindings/interrupt-controller/arm,gic.yaml), [ARM timer binding](https://github.com/gregkh/linux/blob/v6.18.56/Documentation/devicetree/bindings/timer/arm,arch_timer.yaml), and [DW UART binding](https://github.com/gregkh/linux/blob/v6.18.56/Documentation/devicetree/bindings/serial/snps-dw-apb-uart.yaml).
- [Luckfox Pico documentation](https://wiki.luckfox.com/Luckfox-Pico/): real board/BSP and future hardware workflows. Reduced direct boot is distinct from vendor SD/BSP boot.
- [Linux newc initramfs format](https://www.kernel.org/doc/html/latest/driver-api/early-userspace/buffer-format.html): deterministic archive metadata, symlinks and device nodes.
- Local backend provenance: `third_party/tlib-PROVENANCE.md`; platform limits: `crates/core/src/peripherals/armv7/README.md`.

The fixed addresses and register subsets are functional virtual models. Physical clocks, DDR training, real UART baud timing and board behavior need separate validation.
