# BBC micro:bit V1 (Nordic nRF51822) — L1 smoke

The **micro:bit V1** target MCU: Nordic **nRF51822** (QFAA) — Cortex-M0 at
16 MHz, 256 KB flash, 16 KB RAM, 31 GPIOs, 2.4 GHz radio. LabWired models it
as the **application region of an S110 v8 part**: flash from `0x18000`, the
MBR/SoftDevice range below it an empty hole that is never loaded. The blocks
are the shared nRF52 models (UART0 in its legacy personality, 1 KB NVMC pages)
plus an nRF51 FICR.

Two ways to run firmware on it:

- **Bare firmware at the application base** (this page's gate): no
  SoftDevice, the vector table at `0x18000`.
- **Official MakeCode images** (radio, Bluetooth pairing/bonding and the
  Nordic UART service) with the SoftDevice API emulated by
  `crates/nrf-softdevice-hle`: `crates/core/examples/sd_hle_run.rs`, and the
  end-to-end bond + UART test in renode-spike-prime
  `tools/nrf-softdevice-hle/e2e_bond_uart.py`. No labwired-core lane runs
  those images.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/nrf51822.yaml`](../../configs/chips/nrf51822.yaml) |
| System | [`configs/systems/microbit-v1.yaml`](../../configs/systems/microbit-v1.yaml) |
| Smoke script | [`examples/microbit-v1/io-smoke.yaml`](../../examples/microbit-v1/io-smoke.yaml) (nightly coverage-matrix cell) |
| Committed ELF | `tests/fixtures/microbit-v1-smoke.elf` (`crates/firmware-nrf51822-demo`) |
| Survival gate | `firmware_survival::test_nrf51822_microbit_v1_smoke_survival` (PR-gated) |
| Tier | **L1 smoke**: a legacy-UART `OK`; no tier-1 fixture, no silicon bench diff, no executing-fidelity differential |

## Not modelled / not claimed

The 5x5 LED matrix (charlieplexed), the accelerometer and magnetometer,
LPCOMP, QDEC and the SPI/TWI slaves. The nRF51 ADC now has a bounded
SVD-derived channel model, but this profile has no end-to-end analog proof or
physical analog-pad aliases, and no silicon ADC validation is claimed. Bus
visibility has no nRF51 legacy UART/TWI bring-up (see
[bus visibility](../coverage/bus-visibility.md)).
