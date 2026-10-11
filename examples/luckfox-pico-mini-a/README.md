# Luckfox Pico Mini A onboarding package

This directory records the exact hardware, official sources, basket interfaces and implementation gates for **Luckfox Pico Mini A / RV1103**. It is a documentation package, not a runnable board target. The [RV1103 descriptor](../../configs/chips/rv1103.yaml) and [Mini A manifest](system.yaml) declare CPU/memory geometry; peripherals, reset/boot firmware and Linux simulation remain unimplemented. The target is **below L0** in the [support rubric](../../docs/target_support_rubric.md).

Start with the [board page](../../docs/boards/luckfox-pico-mini-a.md), then [HARDWARE.md](HARDWARE.md), [REQUIRED_DOCS.md](REQUIRED_DOCS.md), [EXTERNAL_COMPONENTS.md](EXTERNAL_COMPONENTS.md) and [VALIDATION.md](VALIDATION.md). Official image originals and SHA-256 provenance are under [`docs/assets/luckfox-pico-mini-a`](../../docs/assets/luckfox-pico-mini-a/).

## Selected basket architecture

| Role | Selected component / connection |
|---|---|
| Host | Mini A, MicroSD-backed vendor Linux on physical hardware |
| Wi-Fi | ESP32-C3-MINI-1U with external 2.4 GHz antenna; ESP-AT over UART4 |
| Motion | ADXL345 over I2C3 |
| Battery | Protected 1S 3.7 V nominal 2000 mAh LiPo; exact pack not selected |
| Charging | BQ24074 with power-path management |
| Board supply | TPS61023 boost, regulated 5 V |
| External parts supply | TPS62160 buck from the boosted 5 V rail, regulated 3.3 V |

The ESP owns Wi-Fi association and network sockets; the host application sends AT commands over serial. This design does **not** create a Linux `wlan0` device. Choosing these parts does not establish electrical, thermal, firmware or emulator compatibility. See the hardware notes for constraints.

## Current executable foundation

[`examples/cortex-a7-core`](../cortex-a7-core/) is the separate ARM instruction fixture. Its `chip.yaml`/`system.yaml`/`firmware.S` use a synthetic platform and no RV1103 peripherals. Follow [the native core prototype document](../../docs/qemu-arm-core.md) for actual commands and test receipts. Do not flash that synthetic fixture to a Mini A or reuse its addresses as a board definition.

## Implementation gates

1. **Ground the silicon:** pin the official SDK revision, Mini A schematic revision and RV1103 reference-manual revision. Recover real reset entry, SRAM/DDR ranges, peripheral windows, clock/reset behavior, register semantics and interrupt numbers. Resolve missing register documentation before implementing peripherals or promoting the declaration.
2. **Define a minimal boot contract:** start with an explicitly described bare-metal SRAM/initialized-memory entry and polled UART smoke. Document which earlier boot stages are provided externally. Add only audited real RV1103 clock/reset, pinmux and UART models; avoid generic register banks that merely let drivers continue.
3. **Implement a genuine target:** implement the declared RV1103/Mini A descriptors and add reproducible board smoke firmware. State exact compiler, linker map, binary hashes and unsupported-instruction audit. Only validated descriptors qualify for L0.
4. **Prove deterministic smoke:** assert reset PC/SP, an explicit stop reason, expected UART bytes and absence of unexplained unmapped MMIO. Store result JSON, trace, UART log and fingerprints. Repeat after reset before claiming L1.
5. **Model the selected peripherals:** implement RV1103 I2C3/UART4 behavior, the existing ADXL345 I2C register kit with deterministic acceleration replay, plus an ESP-AT protocol peer with bounded response timing and fault cases. Keep this peer distinct from a radio or Internet simulation.
6. **Qualify the basket behavior:** test stationary/moving transitions, missing accelerometer, malformed AT responses, reconnects, serial backpressure and network outage without losing locally recorded events. Physical bench captures must independently establish sensor orientation, real serial flow control and Wi-Fi behavior.
7. **Treat Linux as a separate milestone:** MMU/CP15, exception modes, interrupt-controller/timer delivery, boot chain, DDR, SD storage and Linux driver behavior are prerequisite work. Do not promote arithmetic tests to Linux-boot evidence.
8. **Close hardware gates:** qualify rail sequencing, USB power isolation, charge termination, thermal rise, pack cutoff, low-battery shutdown and worst-case radio/CPU load. Measure energy/runtime and verify carrier/enclosure dimensions against physical hardware.

These are implementation gates, not a promised schedule. The [validation record](VALIDATION.md) identifies which gates have evidence.
