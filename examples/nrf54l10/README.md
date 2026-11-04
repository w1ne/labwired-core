# Nordic nRF54L10 profile

This focused profile boots an nRF54L10 application-core firmware image on the
nRF54L15-DK's documented L10 emulation target. It links against the L10 memory
geometry and exercises the DK's active-high LED0 on P2.09 plus UARTE20 VCOM on
P1.04/P1.05.

## Reproduce

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nrf54l10-on-nrf54l15-dk \
  --crate firmware-nrf54l10-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nrf54l10/onboarding-smoke.yaml \
  --system configs/systems/nrf54l10-on-nrf54l15-dk.yaml \
  --out-dir examples/nrf54l10/evidence
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nrf54l10/evidence \
  --markdown-out examples/nrf54l10/evidence/onboarding-scoreboard.md \
  --json-out examples/nrf54l10/evidence/onboarding-scoreboard.json
```

The smoke checks the UART banner and at least 16 LED GPIO edges. Committed
build and simulation logs are under [`evidence/`](evidence/); the live vendor
check is [`evidence/vendor-status.md`](evidence/vendor-status.md). The smoke ELF
is committed at `tests/fixtures/nrf54l10-smoke.elf`; `io-smoke.yaml` runs that
fixture with the installed CLI, while `onboarding-smoke.yaml` tests the freshly
built image. After changing firmware, refresh the fixture with:

```bash
cargo build -p firmware-nrf54l10-smoke --release --target thumbv8m.main-none-eabi
cp target/thumbv8m.main-none-eabi/release/firmware-nrf54l10-smoke tests/fixtures/nrf54l10-smoke.elf
```

## Model boundary

The profile includes the Cortex-M33 reset path, 1012 KiB of application RRAM,
192 KiB of SRAM, GPIO2, and UARTE20. Other nRF54L10 blocks are omitted and
fault if accessed; this profile does not model Bluetooth/802.15.4 radio,
TrustZone services, the RISC-V coprocessor, ADC, TWI, SPI, NFC, or board
peripherals beyond LED0 and the VCOM path. It declares no zero-answering
stubs. The simulation is local model evidence, not a physical-board capture.

## Sources

- Nordic's [nRF54L10 product page](https://www.nordicsemi.com/Products/nRF54L10) lists the device's memory and current package availability.
- Nordic's [nRF54L15-DK product page](https://www.nordicsemi.com/Products/Development-hardware/nRF54L15-DK) describes the DK's supported SoC options.
- Nordic's [Zephyr board guide](https://github.com/nrfconnect/sdk-zephyr/blob/main/boards/nordic/nrf54l15dk/doc/index.rst) documents the `nrf54l15dk/nrf54l10/cpuapp` emulation target.
- Board pin assignments follow Nordic's [shared nRF54L15-DK devicetree](https://github.com/nrfconnect/sdk-zephyr/blob/main/boards/nordic/nrf54l15dk/nrf54l15dk_common.dtsi) and the existing nRF54L15 DK profile in this repository.
- Live orderability evidence is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
