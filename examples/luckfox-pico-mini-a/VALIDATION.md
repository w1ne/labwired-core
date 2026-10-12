# Mini A validation record

Record date: **2026-10-12**. Status: **documentation grounded; separate reduced native Linux boot passed; generic board descriptors remain below L0**.

## Evidence ledger

| Claim | Evidence available | Result |
|---|---|---|
| Exact board is Mini A | Requested product URL and official Mini A product photo | Documented |
| Mini A needs MicroSD boot, unlike NAND-populated Mini B | Official flash guide | Documented; no flash performed |
| 22 numbered pad labels and selected interfaces | Official shared Mini A/B pinout, manually inspected | Documented; no continuity test |
| Dimensions | Official shared Mini A/B dimension image, manually inspected | Documented; no physical measurement |
| Cortex-A7 core prototype | Separate [prototype document](../../docs/qemu-arm-core.md) and [`qemu_arm_core.rs`](../../crates/core/tests/qemu_arm_core.rs) | See that document for executed commands and receipts; not board proof |
| RV1103 configs and reset contract | Generic CPU/memory-only declarations; reset/ROM/SD/BSP absent; separate Linux platform does not populate this YAML | Declared; below L0 |
| Deterministic Mini A UART smoke | No board firmware, result JSON or UART capture | Pending; no L1 |
| Reduced RV1103 direct Linux boot | [Native zImage receipt](../luckfox-pico-mini-a-linux/VALIDATION.md): real MMU/GIC/timer/UART2, kernel/PID1/mounts/proc/uname and sleep/uptime | Passed locally; SD/vendor BSP/physical boot pending |
| I2C3 + ADXL345 | Existing ADXL345 I2C kit and core DEVID/XYZ test; RV1103 controller and board wiring absent | Sensor model available; basket scenario pending |
| UART4 + ESP-AT | Vendor pin references and proposed serial architecture only | Pending |
| Native-to-browser integration | No `qemu-arm` browser execution evidence | Pending |
| Physical basket / power / RF | No board, battery, charger or antenna measurements | Pending |

## Core tests are a separate claim

The older optional `qemu-arm` core exercises bounded arithmetic, ARM/Thumb interworking, shared Bus memory/MMIO and trace/builder tests. Its instruction fixture does not establish Linux boot or board reset, and still rejects unsupported Thumb IT and WFI/WFE operations. The separate `qemu-arm-system` route has privileged/MMU/interrupt/sleep fixtures and an actual production zImage/PID1 acceptance receipt. Those virtual proofs remain distinct from Wi-Fi, sensors and physical timing. Unsupported instructions and unmodeled services must stop explicitly rather than manufacture successful device state.

## Documentation checks performed

The three checked-in vendor images were downloaded from official Luckfox hosts and opened for visual inspection. The pin table follows the printed diagram, including I2C3 SDA14/SCL15 and UART4 TX10/RX11/CTS17/RTS16. SHA-256 and byte counts are preserved in the [source manifest](../../docs/assets/luckfox-pico-mini-a/sources.json). The commerce product page refused the scripted request; vendor wiki sources provide the verified facts and exact Mini A photograph.

## Evidence required before promotion

For L0, implement the declared descriptors, board reset contract and validation receipts. For L1, add reproducible Mini A smoke firmware, compiler/linker versions, source and artifact hashes, reset assertions, expected UART bytes, explicit stop reason and unmapped-access audit. For L2, preserve repeated CI evidence, an instruction audit and reviewed limitations. For L3, record scenario-specific pass/partial/blocked evidence for RV1103 clock/reset, GPIO, UART, timers, DMA and interrupt delivery as required by the rubric.

For physical acceptance, record board revision, pack part number, actual schematic/assembly revision, vendor OS image/SDK hashes, sensor register capture, ESP-AT firmware identity, serial transcript, antenna assembly, voltage/current/temperature logs and low-battery shutdown results. Do not store credentials in those artifacts.
