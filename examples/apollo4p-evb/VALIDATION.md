# Validation — Apollo4 Plus EVB (AMAP42KP)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no Apollo4 bench part, and no Renode parity claim is made (Renode's
`AmbiqApollo4_GPIO` model was used only to cross-check reset values).

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot past the SBL via the `0x0001_8000` vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x18400, SP: 0x10060000`; `test_apollo4p_evb_smoke_is_linked_after_the_bootloader` pins the fixture layout |
| Cortex-M4F core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| PWRCTRL UART0 power handshake (`DEVPWREN` → `DEVPWRSTATUS`) | smoke-tested | `pwr perf=lp uart0_pwr=1`; unit test covers the mirror and read-only status |
| PWRCTRL `MCUPERFREQ` reset LP mode / request ack | smoke-tested (reset) / unit-tested (ack) | `perf=lp`; `pwrctrl_status_follows_enable_and_perf_request_is_acked` |
| Power-domain effect on peripherals | **not modelled** | UART0 works with its domain off too |
| UART0 (PL011 layout) TX @ 115200 from a 24-MHz clock, `IBRD 13` / `FBRD 1` | smoke-tested | `uart0 clksel=1 clk=24000000 ibrd=13 fbrd=1` (IBRD/FBRD read back from the model) + heartbeat lines on the console sink |
| UART0 `CR.CLKEN` / `CR.CLKSEL` clock select | **executed, not checked** | the PL011 model does not store `CR`; the firmware derives the divisor from its own CLKSEL constant |
| GPIO `PADKEY` lock on PINCFG | unit-tested | `pincfg_resets_to_gpio_and_ignores_writes_while_locked`; the smoke unlocks/relocks around its PINCFG writes |
| GPIO push-pull output via `WTS`/`WTC` strobes + `RD` read-back | smoke-tested | `led=led0 on rd=0 lit=1` … (one strobe per change; the lit mask proves no other LED moved) |
| GPIO `WT` latch contents | smoke-tested | `wt0=1 wt2=1 wt3=1`: each LED bank's latch holds exactly its LED bit |
| Button inputs on the PULLCFG pull-up (`INPEN`, `RD`) | smoke-tested | `btn0=1 btn1=1`; unit tests cover INPEN/RDZERO gating and an external press |
| Tristate (`EN`/`ENS`/`ENC`) and open-drain output modes | unit-tested | `tristate_drives_only_when_enabled_and_open_drain_only_low` |
| Non-GPIO `FNCSEL` (UART0 on GPIO60/47) | **executed, not routed** | stored; the GPIO block does not drive such a pin, the console model does not depend on it |
| GPIO interrupts | **not modelled** | `INTnSTAT` changes only by software `SET`/`CLR` |
| Determinism (same inputs → same output) | smoke-tested | three runs: identical `uart.log`, final PC and cycle count |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Boot ROM, secure bootloader, OTP/INFO | **not modelled** | fast boot at `reset_vector_offset 0x18000` |
| CLKGEN, MCUCTRL, RSTGEN, IOM0-7, MSPI0-2, ADC, timers, STIMER, RTC, WDT, security, crypto, GPU/display, SDIO, USB, audio | declared stubs | `configs/chips/amap42kp.yaml` |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-apollo4p-evb-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./examples/apollo4p-evb/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unknown_thumb16: 0 · unhandled_thumb32: 0 · unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-apollo4p-evb-smoke \
  --system configs/systems/apollo4p-evb.yaml --max-steps 400000
#   Initial PC: 0x18400, SP: 0x10060000
#   APOLLO4P EVB SMOKE
#   cpu=cortex-m4f uart=uart0
#   pwr perf=lp uart0_pwr=1
#   uart0 clksel=1 clk=24000000 ibrd=13 fbrd=1
#   led=led0 on rd=0 lit=1
#   ...
#   hb=0 led0=0 led1=1 led2=1

# Determinism: three runs, identical UART stream, final PC and cycle count
#   run1 pass pc=105606 cycles=4000000 uart sha256 1e0003b4c3d6fdbd…
#   run2 pass pc=105606 cycles=4000000 uart sha256 1e0003b4c3d6fdbd…
#   run3 pass pc=105606 cycles=4000000 uart sha256 1e0003b4c3d6fdbd…

cargo test -p labwired-core --lib apollo4
#   12 passed (PADKEY lock, push-pull WTS/WTC + RD, INPEN/RDZERO, button
#   press, tristate/open-drain, non-GPIO FNCSEL, byte strobe, interrupt
#   STAT, node reset, snapshots, PWRCTRL mirror/ack, tick-walk opt-out)
```

## Notes

- **Toolchain.** The Apollo4 Plus is a Cortex-M4F (ARMv7E-M + FPv4-SP). The
  firmware is built for `thumbv7em-none-eabi` (soft-float ABI), so it uses no
  FPU instructions; the unsupported-instruction audit runs on that ELF.
- **Clock.** The firmware never leaves the reset LP mode and never touches
  CLKGEN. `cpu_hz` is 96 MHz. The baud arithmetic is in the firmware header
  (achieved 115,384 baud, +0.16 %).
- **RAM declaration.** The descriptor maps the whole contiguous 2816 KiB from
  `0x1000_0000` (TCM, SSRAM0, DSP0 IRAM/DRAM, SSRAM1) as one region; bank
  power (`MEMPWREN`/`SSRAMPWREN`) is stored only, so every bank is always
  usable. The smoke uses only the TCM.
- **Pins 105-127** are virtual on silicon (PINCFG without a pad); the model
  treats all 128 pins alike. Nothing in this slice uses them.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim.
- The staged `configs/chips/onboarding/ambiq-apollo4.yaml` (flash at
  `0x08000000`, 4 KiB; RAM at `0x20000000`) is the Renode bootrom-logger
  scaffold, not the Apollo4 Plus map. It is kept, marked superseded, because
  `crates/core/src/bus/known_stubs.rs` still cites it.
