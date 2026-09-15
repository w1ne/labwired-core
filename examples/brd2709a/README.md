# BRD2709A — Silicon Labs xG26-EK2709A Explorer Kit (EFR32MG26)

Minimal deterministic smoke for the Silicon Labs **EFR32MG26B510F3200IM48-B**
(Series-2 "Mighty Gecko", Cortex-M33 @ 78 MHz, 3200 KiB flash @ `0x0800_0000`,
512 KiB SRAM) as fitted to the **BRD2709A** Explorer Kit.

Support level: **L1 smoke** (see [`../../docs/target_support_rubric.md`](../../docs/target_support_rubric.md)).

## What This Demo Proves

1. Boot/reset flow for an EFR32MG26 image (vector table at flash base `0x0800_0000` — NOT `0x0` on this Series-2 part).
2. UART logging through the board VCOM path (`USART1`, Series-2 register map: `STATUS@0x18`, `TXDATA@0x38`).
3. GPIO through the Series-2 port model: LED0/LED1 (`PC08`/`PC09`) toggled via `MODEH`+`DOUT`, pin levels read back through `DIN`, BTN0/BTN1 (`PB00`/`PB01`) sampled, and the final `DOUT` word asserted at its MMIO address.

## Quick Start

Run from the `core/` repo root.

```bash
# 1) Build the smoke firmware (bare-metal Rust, no vendor SDK)
make -C examples/brd2709a

# 2) Run the deterministic smoke checks
cargo run -p labwired-cli -- test --script examples/brd2709a/uart-smoke.yaml
cargo run -p labwired-cli -- test --script examples/brd2709a/io-smoke.yaml
```

Expected: both tests pass and the captured UART output contains

```
brd2709a: MG26 OK
MG26-IO
PC08=1 PC09=1
PC08=0 PC09=0
BTN0=1 BTN1=1
MG26-IO DONE
```

## Expected Output Signals

Emulator checks:

1. `brd2709a: MG26 OK`
2. `PC08=1 PC09=1` / `PC08=0 PC09=0` (LED pin levels read from `DIN`)
3. `BTN0=1 BTN1=1` (active-low buttons, unpressed = high)
4. `MG26-IO DONE`

## Files You Need

- `VALIDATION.md`: step-by-step reproducible runbook with observed output
- `system.yaml`: local system profile used by emulator runs
- `uart-smoke.yaml`: deterministic UART test script (marker + stop reason)
- `io-smoke.yaml`: deterministic GPIO test script (LED toggle via DIN readback + `memory_value` on `GPIOC DOUT`)
- `Makefile`: builds `firmware-mg26-demo` and writes `target/firmware`
- `REQUIRED_DOCS.md`: authoritative register/board sources
- `EXTERNAL_COMPONENTS.md`: what is (not) modelled off-chip

## Known Limitations (L1)

- GPIO covers the per-port digital path (`MODEL`/`MODEH`/`DOUT`/`DIN`); the
  ROUTE pin-mux, GPIO interrupts (EXTI/EM4WU), and the block-level SET/CLR/TGL
  aliases are not modelled.
- CMU and TIMER0 are explicit stub windows; no clock tree is modelled.
- Only USART1 is mapped (the VCOM console). No MSC/WDOG/EMU/radio.
- See the full list in [`../../configs/chips/efr32mg26.yaml`](../../configs/chips/efr32mg26.yaml).

## References

- Chip config: `../../configs/chips/efr32mg26.yaml`
- System config: `../../configs/systems/brd2709a.yaml`
- Board page: `../../docs/boards/brd2709a.md`
- Smoke firmware: `../../crates/firmware-mg26-demo/`
