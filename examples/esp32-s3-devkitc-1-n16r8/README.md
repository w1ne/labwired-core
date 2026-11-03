# ESP32-S3-DevKitC-1-N16R8 UART0 smoke

This smoke boots an Arduino sketch on LabWired's ESP32-S3-DevKitC-1-N16R8
target and checks a UART0 transmit marker. The system profile is
[`configs/systems/esp32-s3-devkitc-1-n16r8.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8.yaml).

## Scope

- `UART0`: real S3 UART peripheral model, exercised by `Serial0.println` and
  checked against the serial oracle.
- Twin pin map: GPIO43 is UART0 TX and GPIO44 is UART0 RX. This mapping comes
  from LabWired's **MODEL_ONLY** board record; it is not a vendor pinout
  verification.
- No external devices are attached. The smoke observes the UART0 console in
  the twin; it does not model the board's USB-to-UART bridge or claim USB
  serial operation.
- The board profile references a minimal chip descriptor that lists UART0 only
  and omits the broader S3 YAML's placeholder windows. The dedicated Xtensa S3
  builder still wires additional internal models in code; this smoke makes no
  claim about those peripherals. PSRAM, Wi-Fi/BLE, buttons, LEDs, and native USB
  are outside the verified path.
- The hosted verifier assumes an ideal VDD rail. It does not simulate power-on
  or brown-out reset behavior.

## Evidence

- [`evidence/part-model.json`](evidence/part-model.json) records the twin's
  model-only pin map.
- [`evidence/model-verification.json`](evidence/model-verification.json) is the
  raw hosted LabWired verification response from 2026-11-03 UTC.
- [`evidence/verify-request.json`](evidence/verify-request.json) records the
  board, diagram, source, and required marker oracle used for the run.
- [`evidence/vendor-status.md`](evidence/vendor-status.md) records the live
  manufacturer catalog and retailer availability check.

The verification is model evidence for the hosted twin, not a physical-board
test. The committed Arduino source is [`src/main.ino`](src/main.ino).
