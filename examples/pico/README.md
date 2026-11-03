# Raspberry Pi Pico 1 (RP2040)

This board profile reuses the RP2040 chip model and binds its onboard LED to
GP25. The smoke builds `firmware-rp2040-demo`, which configures UART0 on GP0,
prints `RP2040_SMOKE_OK`, selects SIO for GP25, and toggles the LED through the
modeled GPIO output registers. The test asserts both the UART transcript and
at least eight GP25 edges.

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id pico \
  --crate firmware-rp2040-demo \
  --target thumbv6m-none-eabi \
  --script examples/pico/io-smoke.yaml \
  --system configs/systems/rp2040-pico.yaml \
  --out-dir examples/pico/evidence
```

Committed build and simulation logs are in [`evidence/`](evidence/).

## Vendor status

Checked 2026-11-03. Raspberry Pi's [Pico product page](https://www.raspberrypi.com/products/raspberry-pi-pico/)
listed the Pico series as **available from $4** and said Pico 1 will remain in
production until at least **January 2036**.

## Scope

- The smoke exercises the UART0 PL011 model and GPIO through IO_BANK0 and SIO.
- UART bytes are captured from the UART model. This does not assert USB CDC,
  USB boot/flash, or a physical serial connection.
- The RP2040 descriptor still has a TBMAN zero-answering stub. The smoke does
  not access it. The other board peripherals are outside this smoke's claims.
- This is a local engine simulation. No physical Pico capture is recorded.
