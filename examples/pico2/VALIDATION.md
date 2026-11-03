# Pico 2 (RP2350) validation record

## Supported board slice

[`configs/systems/pico2.yaml`](../../configs/systems/pico2.yaml) uses
[`configs/chips/rp2350-pico2-smoke.yaml`](../../configs/chips/rp2350-pico2-smoke.yaml),
which contains only the memory map, UART0, IO_BANK0 function selection, and
SIO needed by this smoke. It does not inherit the POWMAN/TICKS and TBMAN stubs
from the broader `configs/chips/rp2350.yaml` profile.

The `firmware-pico2-demo` crate in `crates/firmware-pico2-demo/` selects SIO
for GP25, enables the output, and toggles it through the RP2350 SIO model. It
also writes a marker to UART0's PL011 data register. The test asserts UART
transcript text and at least eight GP25 edges; it does not assert UART bit
timing.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id pico2 \
  --crate firmware-pico2-demo \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2/io-smoke.yaml \
  --system configs/systems/pico2.yaml \
  --firmware-build-dir crates/firmware-pico2-demo \
  --out-dir examples/pico2/evidence
```

The committed evidence includes `onboarding-metrics.json`, the build and smoke
logs, and the CLI simulation artifacts. The 2026-11-03 run passed all three
assertions.

## Gaps

- `IO_BANK0` models the function selector used here; GPIO status observation
  and PADS_BANK0 electrical controls are not modeled in this board slice.
- UART0 evidence is register-level transcript capture, not a timed waveform or
  USB CDC output. USB boot/flash and ROM startup are not claimed.
- The broader RP2350 profile still declares POWMAN/TICKS and TBMAN as explicit
  zero-answering stubs. This board system omits those windows.
- No physical Pico 2 has been captured on hardware.

The separate hosted LabWired verifier has no Pico 2 target. Its closest
RP2350-Zero Arduino run returned `status: unsupported` with an undecoded T32
instruction at PC `0x1008`; that result is not an oracle pass for this board.

## Vendor status checked 2026-11-03

Raspberry Pi's live [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
said **“Available now from $5”** and stated the Pico 2 series will remain in
production until at least January 2040.
