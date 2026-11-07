# TMP117 on NUCLEO-F407

## Live vendor availability checked 2026-11-04

- TI's [TMP117 product page](https://www.ti.com/product/TMP117) reports
  **ACTIVE** and provides an **Order now** link. TI's page for the
  `TMP117AIDRVR` orderable variant also reports **ACTIVE**; TI's own inventory
  for that variant was out of stock when checked.
- Adafruit's [TMP117 STEMMA QT / Qwiic breakout, product 4821](https://www.adafruit.com/product/4821)
  was **In stock**, with **Order now to ship today**, at **$11.50**. The page
  identifies the mounted sensor as TMP117. This is the customer-buyable module
  represented by the TMP117 digital device in the system below.

## Exercised system

`tmp117-system.yaml` attaches the existing TMP117 device model at I²C address
`0x48` on the NUCLEO-F407's modeled I²C1 bus. The focused firmware in
`src/tmp117.rs` programs the STM32F407 I²C1 controller and reads TMP117 device
ID register `0x0F` and temperature result register `0x00`. It reports the pass
marker and drives LD2 on PA5 only when the model returns device ID `0x0117` and
the configured 25 °C input's raw result `0x0C80`.

The test exercised RCC clock enables, GPIOA/GPIOB, USART2 console output,
I²C1 transactions, the TMP117 model, and LD2 board I/O. No zero-answering stub
is used by this path. Other F407 features are outside this smoke's claims.

## Evidence

The committed smoke result is `evidence/tmp117/onboarding-summary.md` with the
build and simulator logs beside it. The simulator reported `PASS 3/3 checks`:
the TMP117 marker, an LD2 edge, and the expected max-step stop. UART output is
in `evidence/tmp117/simulation-output/uart.log`; the firmware image hash and
assertion results are in `evidence/tmp117/simulation-output/junit.xml` and
`result.json`.

Reproduce from the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f407-tmp117 \
  --crate nucleo-f407-i2c \
  --target thumbv7em-none-eabi \
  --firmware-build-dir examples/nucleo-f407-i2c \
  --script examples/nucleo-f407-i2c/tmp117-smoke.yaml \
  --system examples/nucleo-f407-i2c/tmp117-system.yaml \
  --out-dir examples/nucleo-f407-i2c/evidence/tmp117
```

`RUSTC_WRAPPER=` clears a stale workspace setting in this runner that points
to a missing `sccache` executable. A working local wrapper can be used instead.

## Model limits

The TMP117 descriptor returns a static temperature input. It does not model
conversion timing, the alert comparator or ALERT pin, temperature-offset
addition, or nonvolatile EEPROM programming; DATA_READY is set at reset rather
than scheduled per conversion. The smoke checks the register read path, not
measurement accuracy or the breakout's analog/power circuitry. The hosted
board catalog currently lists NUCLEO-F407 as `compilable: false`, so this is a
local simulator smoke, not a hosted `labwired_verify` verdict or hardware
confirmation.
