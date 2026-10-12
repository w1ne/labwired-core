# RV1103 smart-basket Linux circuit acceptance

> Required workflow: superpowers:subagent-driven-development. Root owns commits, pushes and merges. Continue after the real Linux boot gate; do not substitute host-only device stimulation.

**Goal:** Linux reads an attached ADXL345 and exchanges UART commands with an explicitly behavioral ESP32-C3 ESP-AT peer through the same bus/component state as the basket circuit.

## Task 1: Strict matching driver artifacts

Completed and independently reviewed on 2026-10-12: 52 preparation tests pass and two independent actual module/helper builds match. Preparation is verified; guest insertion remains NOT_RUN. See `examples/luckfox-pico-mini-a-linux/DRIVER_MODULES.md` for pins, bounds and compiler limitations.

Implement deterministic preparation of the matching Linux6.18.56 external RK3x module. Reuse the pinned upstream source SHA256 `cee9b45dfee3fb4a7168585603c88df5e3422493425ac1bd35046d509d910423`, Alpine dev package SHA256 `a2fd3f3a675e5e1c68b4dbea544fa7fe3e1ebbc0403bf1f8de6130ed6cdb1b80`, generated headers and Module.symvers. Build native host helpers from the included sources; never execute extracted ARM host utilities. Keep original .config and symbol versions unchanged. Record the compiler version and GCC plugin override, and require matching vermagic and strict guest insmod without forced version/symbol bypasses. Add the matching stock i2c-dev module to initramfs. Verify hashes, extraction path bounds and reproducible archive metadata.

## Task 2: Rockchip I2C3 controller

Completed and independently reviewed on 2026-10-12 in commit `a94b78e11`: 23 new controller tests pass in default and native/event configurations, alongside 20/28 existing device tests and 8 inspection tests. Read-only register rejection is covered. Shared ADXL345 attachment, stimulus, traces and inspection are verified; Linux basket guest execution remains NOT_RUN.

Create a bounded RK3x v1 controller implementing the registers and interrupt handshake used by the pinned real driver. Write transaction and error tests before implementation: START/address/ACK, RX/TX FIFO counts, repeated-start register read, STOP, pending/enable interrupt bits, NACK and reset. Attach existing I2cDevice implementations through the normal bus attachment seam. Route I2C3 SPI21 to GIC INTID53. Controller transfers must invoke existing select/start/write/read/stop methods on the same ADXL345 instance; do not keep duplicate axis/register state. Respect the existing unpowered-I2C wrapper so power loss produces NACK. Unsupported DMA or clock/pinmux timing must be disclosed.

## Task 3: UART4 and ESP-AT peer

Reuse the verified DW UART model at ff4e0000/SPI29 (INTID61), with UART stream-host capability attachment. Implement a new bounded ESP-AT component: AT, AT+GMR and agreed explicit mock connection/status commands, CRLF framing, bounded input buffers and deterministic RX. No SIM800L substitution and no claim of executing ESP firmware or modeling RF. Tests require bytes to pass from DW TX to component input and component RX back through UART FIFO/IRQ. Power-disabled peers consume no commands and emit no responses. Advertise unknown command behavior explicitly.

## Task 4: Shared basket guest and power acceptance

Extend the direct-boot DT with real I2C3 compatible `rockchip,rv1106-i2c`, `rockchip,rk3399-i2c`, IRQ and disclosed fixed clocks; add UART4/serial4. Include a small ARM userspace helper using actual i2c-dev I2C_RDWR and termios UART syscalls. The acceptance guest loads the strict modules, reads ADXL345 DEVID0xe5, and verifies stimulus x/y/z values through Linux transfers, then exchanges AT/GMR over ttyS4. Host assertions confirm controller and component transaction traces from the shared bus, not only matching console strings. Stimulus and battery rails use existing circuit attachment/power contracts. A power-cut scenario must suppress component transactions and document whether it halts the Linux machine or requires reloading direct boot. Do not invent ROM reboot behavior.

## Task 5: Circuit/tooling release

Run the existing PCB ERC/DRC/routing and reduced power tests only if circuit inputs change. Add native Linux basket acceptance to the opt-in CI job, with complete console and hash receipts on failure. Update model/backend availability and example documentation to distinguish direct boot, shared circuit behavior, behavioral Wi-Fi and reduced power from SD/BSP, radio, and hardware acceptance. Review default/browser independence, protect required checks, and merge current tested heads. Use the packed board-config package locally: the user explicitly skipped publication and chose local dogfooding, so no npm publication or manual deployment is required. Physical board/power/footprint/thermal/RF gates stay pending while the user has no hardware.
