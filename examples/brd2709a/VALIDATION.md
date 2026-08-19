# BRD2709A (xG26-EK2709A) Validation Runbook

Run all commands from `core/`. Observed output below was captured on
2026-08-18 on macOS (aarch64), rustc 1.97.1, worktree `feat/onboard-brd2709a`.

## Prerequisites

1. Rust toolchain with the `thumbv7m-none-eabi` target
   (`rustup target add thumbv7m-none-eabi`).
2. No hardware required for any step here. (The physical board was used only
   for chip identification during onboarding: probe-rs over the on-board
   J-Link OB, 1366:0105 — Energy Micro DP part 0x1013, CPUID = Cortex-M33.)

## A) Build the smoke firmware

```bash
make -C examples/brd2709a
# or directly:
cargo build -p firmware-mg26-demo --target thumbv7m-none-eabi --release
```

Pass criteria: `target/thumbv7m-none-eabi/release/firmware-mg26-demo` exists.

## B) Descriptor validation

```bash
cargo build -p labwired-cli
./target/debug/labwired asset validate --chip configs/chips/efr32mg26.yaml
./target/debug/labwired asset validate --system configs/systems/brd2709a.yaml
./target/debug/labwired asset list-chips | grep EFR32MG26
```

Observed:

```
{ "valid": true, "issues": [], "context": "ChipDescriptor: \"configs/chips/efr32mg26.yaml\"", ... }   # exit 0
{ "valid": true, "issues": [], "context": "SystemManifest: \"configs/systems/brd2709a.yaml\"", ... } # exit 0
- EFR32MG26 (Arch: Arm)
```

## C) Deterministic UART smoke (the L1 gate)

```bash
cargo run -p labwired-cli -- test --script examples/brd2709a/uart-smoke.yaml
```

Observed:

```
INFO labwired_loader: ELF Entry Point: 0x8000009
PASS  2/2 checks · uart-smoke · 512 steps · 0.00s
brd2709a: MG26 OK
```

Pass criteria: exit code 0, `PASS 2/2`, UART output contains `brd2709a: MG26 OK`.

## C2) Deterministic IO smoke (GPIO: LEDs + buttons)

```bash
cargo run -p labwired-cli -- test --script examples/brd2709a/io-smoke.yaml
```

Observed:

```
INFO labwired_loader: ELF Entry Point: 0x8000009
PASS  7/7 checks · io-smoke · 2048 steps · 0.00s
brd2709a: MG26 OK
MG26-IO
PC08=1 PC09=1
PC08=0 PC09=0
BTN0=1 BTN1=1
MG26-IO DONE
```

The `PC08=/PC09=` values are read by the firmware from `GPIOC DIN` — the pin
path, not the `DOUT` latch — so a model where DOUT never reaches the pins
fails these lines. `BTN0=1 BTN1=1` is correct: the buttons are active-low
(UG594), so unpressed reads high. The script also asserts the final
`GPIOC DOUT` word directly (`memory_value` at 0x4003C0A0, mask 0x300).

Pass criteria: exit code 0, `PASS 7/7`.

## D) Interactive run against the shipped system manifest

```bash
./target/debug/labwired run \
  --chip configs/chips/efr32mg26.yaml \
  --firmware target/thumbv7m-none-eabi/release/firmware-mg26-demo \
  --system configs/systems/brd2709a.yaml --max-steps 512
```

Observed:

```
INFO labwired_loader: ELF Entry Point: 0x8000009
brd2709a: MG26 OK
```

## E) Unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/firmware-mg26-demo \
  --system configs/systems/brd2709a.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/brd2709a
```

Observed:

```
Audit summary:
  unknown_thumb16: 0
  unhandled_thumb32: 0
  unknown_riscv: 0
  unsupported_total: 0
  report: out/unsupported-audit/brd2709a/report.md
```

## E2) Silicon smoke (physical BRD2709A, 2026-08-18)

Board on USB: J-Link OB probe `1366:0105:000440338937`, VCOM at
`/dev/cu.usbmodem0004403389371` (115200 8N1).

```bash
# Baseline: with clock gates untouched, the APB blocks bus-fault.
~/.labwired/bin/probe-rs read --chip EFR32MG26B510F3200IM48 \
  --probe 1366:0105:000440338937 b32 0x400A4018 1
# Observed: "Target device responded with a FAULT response to the request."

# Flash the sim-built ELF, then capture VCOM across a reset.
~/.labwired/bin/probe-rs download --chip EFR32MG26B510F3200IM48 \
  --probe 1366:0105:000440338937 \
  target/thumbv7m-none-eabi/release/firmware-mg26-demo     # Finished in 0.94s
stty -f /dev/cu.usbmodem0004403389371 115200 raw -echo
(cat /dev/cu.usbmodem0004403389371 > /tmp/cap.log & P=$!; sleep 0.3; \
 ~/.labwired/bin/probe-rs reset --chip EFR32MG26B510F3200IM48 \
   --probe 1366:0105:000440338937 >/dev/null 2>&1; sleep 2; kill $P)
cat /tmp/cap.log
```

Observed on VCOM (clean ASCII — this also confirms the 19 MHz out-of-reset
EM01GRPA / HFRCO-startup-band baud basis behind `CLKDIV = 2384`):

```
brd2709a: MG26 OK
MG26-IO
PC08=1 PC09=1
PC08=0 PC09=0
BTN0=0 BTN1=0
MG26-IO DONE
```

Post-run probe reads (both bus-FAULTED before the firmware's CMU clock
enables — the Series-2 clock-gating wall):

```
0x400A4018 (USART1_STATUS): 00002062   # TXENS|TXC|TXBL|TXIDLE — sane, no FAULT
0x4003C0A0 (GPIOC_DOUT):    00000300   # both LEDs driven on
0x40008064 (CMU_CLKEN0):    04000000   # GPIO clock-enable latched (bit 26)
```

Note the one sim/silicon divergence this run pinned: on the bench the buttons
read `BTN0=0 BTN1=0` (PB00/PB01 left in DISABLED mode — input buffer off reads
0), while the sim's board_io model drives the DIN latch to the released
active-low level and the sim prints `BTN0=1 BTN1=1`. The io-smoke asserts the
sim behaviour; the line above is silicon's.

## F) Gate tests touched by this onboarding

```bash
cargo test -p labwired-core --lib uart                 # 149 passed (incl. Efr32s2 pins)
cargo test -p labwired-core --lib gpio                 # 103 passed (incl. Efr32s2 port pins)
cargo test -p labwired-core --test chip_conformance    # estate row for efr32mg26
cargo test -p labwired-core --test board_coverage_ratchet
cargo test -p labwired-core --test bus_visibility
cargo test -p labwired-core --test cpu_hz_single_source
cargo test -p labwired-core --test unknown_peripheral_type
cargo test -p labwired-config --test chip_pins_ratchet
python3 scripts/generate_validation_status.py --check --drift
```

All pass (exit 0) on this tree.

## Troubleshooting

1. A firmware built for Series-1 EFR32 (STATUS@0x10 / TXDATA@0x34) hangs on
   this chip: that is the correct simulation of the wrong silicon, not a bug.
