# ci-fixture-riscv (synthetic CI fixture, not a real board)

This is not a real chip or board. It is a minimal synthetic RV32I profile
used only to exercise the RISC-V core path in
`firmware_survival::riscv_ci_fixture`. It is listed in the manifest so the
cross-registry consistency test (`board_registry_consistency.rs`) has a
board entry to resolve it against, honestly labeled as having no silicon
backing.

## Status at a glance

| Aspect      | Status                                                                        |
|-------------|----------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/ci-fixture-riscv.yaml`](../../configs/chips/ci-fixture-riscv.yaml) |
| Validation  | `firmware_survival::riscv_ci_fixture`                                            |
| Tier        | **structural** — synthetic fixture, no silicon claim of any kind                 |

## What is modeled (from the chip yaml)

- 128 KiB flash at `0x8000_0000`, 64 KiB RAM at `0x8002_0000`
- One UART peripheral at `0x4000_C000`

## What is NOT proven

Everything about this "chip" is synthetic — it does not correspond to any
real silicon and no claim should ever be read from it beyond "the RISC-V CPU
core survives N cycles of this fixture ELF."
