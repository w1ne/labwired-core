# Validation — CC2538DK (CC2538SF53)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no CC2538 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x200400, SP: 0x20008000` |
| Cortex-M3 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| SYS_CTRL reset clock (`CLOCK_STA`: HF-RC, `SYS_DIV`/`IO_DIV` 001) | smoke-tested | `clk osc=hfrc16 sys_div=1 io_div=1 sysclk=16000000`; unit tests pin the Table 7-3 reset values |
| SYS_CTRL `RCGCUART` gate write + read-back | smoke-tested | `uart0 rcgc=1 …` |
| SYS_CTRL gating effect on other blocks | **not modelled** | gates are storage only; UART0 works with the gate clear too |
| UART0 (PL011-compatible) TX @ 115200 from 16 MHz, `IBRD 8` / `FBRD 44` | smoke-tested | `uart0 … ibrd=8 fbrd=44` + heartbeat lines on the console sink |
| GPIO address-masked `GPIO_DATA` write + read | smoke-tested | `led=yellow on pin=1 leds=02` … (one masked store per LED; the nibble read proves no other pin moved); unit tests cover the SWRU319C Figure 9-2/9-3 examples |
| GPIO `GPIO_DIR` / `GPIO_AFSEL` | smoke-tested | LEDs driven only after DIR; PA0/PA1 handed to UART0 |
| Key inputs at the reset IOC pull-up | smoke-tested | `keys select=1 left=1 right=1 up=1 down=1` |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| IOC pin mux / pad overrides | **not modelled** | the IOC window is a stub |
| GPIO interrupts, AFSEL commit lock | **not modelled** | `GPIO_RIS`/`GPIO_MIS` read 0 |
| Boot ROM, CCA evaluation | **not modelled** | fast boot; the CCA is in the image for silicon and `test_cc2538dk_smoke_customer_configuration_area_boots` pins its 44 bytes in the fixture |
| RF core, AES/PKA, flash ctrl, SSI, I2C, GPTimers, USB, uDMA, WDT/sleep timer, ADC | declared stubs | `configs/chips/cc2538sf53.yaml` |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-cc2538dk-smoke --release --target thumbv7m-none-eabi
#   Finished `release` profile — no warnings

./examples/cc2538dk/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unknown_thumb16: 0 · unhandled_thumb32: 0 · unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv7m-none-eabi/release/firmware-cc2538dk-smoke \
  --system configs/systems/cc2538dk.yaml --max-steps 400000
#   Initial PC: 0x200400, SP: 0x20008000
#   CC2538DK SMOKE
#   cpu=cortex-m3 uart=uart0
#   clk osc=hfrc16 sys_div=1 io_div=1 sysclk=16000000
#   uart0 rcgc=1 ibrd=8 fbrd=44
#   led=yellow on pin=1 leds=02
#   ...
#   hb=0 yellow=1 green=0 orange=0

# Determinism: two runs, identical UART + snapshot
cmp out/cc2538dk-run1/uart.log out/cc2538dk-run2/uart.log          # clean
cmp out/cc2538dk-run1/snapshot.json out/cc2538dk-run2/snapshot.json  # clean

cargo test -p labwired-core --lib cc2538
#   9 passed (masked read/write, byte store, DIR/pad, AFSEL, external level,
#   SYS_CTRL reset values, gate fields, CLOCK_STA tracking)
```

## Notes

- **Toolchain.** The CC2538 is a Cortex-M3 (ARMv7-M), and the firmware is built
  for `thumbv7m-none-eabi`, so the compiler cannot emit ARMv7E-M DSP or FPU
  instructions. The unsupported-instruction audit runs on that ELF.
- **Clock.** The firmware never leaves the reset clock. CLOCK_CTRL is not
  written; the divisors come from what CLOCK_STA reports, and the baud
  arithmetic is in the firmware header (achieved 115,108 baud, −0.08 %).
- **LED1 / PC0** is declared in `board_io` but never driven: it doubles as the
  CC2538 USB D+ pull-up on this board.
- **Bit 3 of `CLOCK_STA`.** Table 7-3 gives the reset value `0x0101_0109`
  while the bit-field table calls bits 7:3 reserved (0). The model returns the
  summary value verbatim.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/cc2538.yaml` (flash at `0x08000000`,
  128 KiB; 256 KiB RAM) did not match SWRU319C Table 4-1. It is kept, marked
  superseded, because `crates/core/src/bus/known_stubs.rs` still cites it.
