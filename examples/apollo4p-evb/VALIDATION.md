# Validation — Apollo4 Plus EVB (AMAP42KP)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no Apollo4 bench part, and no Renode parity claim is made (Renode's
`AmbiqApollo4_GPIO` model was used only to cross-check reset values; its
`ambiq-apollo4.repl` maps the same 4 KiB boot-ROM window and answers the CPU
block's `DAXICTRL` with `DAXIREADY`).

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
| Stock Zephyr v3.7.0 `samples/hello_world` (`apollo4p_evb`, AmbiqSuite HAL via hal_ambiq) | smoke-tested | `zephyr-hello.yaml` 2/2 checks, 3,000,000 steps: `*** Booting Zephyr OS build v3.7.0 ***`, `Hello World! apollo4p_evb/apollo4p`; `firmware_survival::test_apollo4p_evb_zephyr_survival` |
| Boot-ROM helper table at `0x0800_0000` (clean-room, `crates/core/roms/apollo4`): `br_util_read_word` / `write_word` / `delay_cycles` | integration-tested on the CPU model; exercised | `apollo4_bootrom` (3 tests, with and without `event-scheduler`); Zephyr's `am_hal_delay_us` |
| `br_util_delay_cycles` timing | **2 cycles/iteration, silicon 3** | the CPU model charges a taken branch 1 cycle, so HAL delays run ~1/3 short |
| Boot-ROM MRAM helpers (`nv_*`) | **not modelled** | return -1, change nothing (`apollo4_bootrom::mram_helpers_report_failure_and_change_nothing`) |
| CPU block (`0x4800_0000`): DAXI flush/invalidate with `DAXIREADY`, `CACHECTRL.CACHEREADY` | unit-tested; exercised | `cargo test -p labwired-core --lib apollo4` (`cpu_*`, 2 tests); Zephyr's `am_hal_pwrctrl_low_power_init` |
| `SYNC_READ` bus write-flush (`0x47FF_0000`) | exercised | a read-0 stub; writes are never queued in the model |
| Negative control | fails | on main (no boot-ROM window) the Zephyr image faults at step 3,081 fetching `0x0800_009C`; without the CPU block it BusFaults at `0x4800_0050` in `daxi_flush_invalidate` |
| Determinism of the Zephyr run | smoke-tested | two runs: identical `uart.log` and `snapshot.json` |
| Instruction support for the Zephyr image | audited | `unsupported_instruction_audit.sh`: 3,000,000 steps, 0 unsupported |
| Rest of the boot ROM, secure bootloader, OTP/INFO | **not modelled** | fast boot at `reset_vector_offset 0x18000` |
| Cache / DAXI buffering | **not modelled** | memory is coherent; actions complete at once |
| CLKGEN, MCUCTRL, RSTGEN, IOM0-7, MSPI0-2, ADC, timers, STIMER, RTC, WDT, security, crypto, GPU/display, SDIO, USB, audio | declared stubs | `configs/chips/amap42kp.yaml` |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-apollo4p-evb-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./examples/apollo4p-evb/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unknown_thumb16: 0 · unhandled_thumb32: 0 · unsupported_total: 0
#   PASS  2/2 checks · zephyr-hello · 3000000 steps
#   Audit summary: unsupported_total: 0

# Stock Zephyr (2026-10-30): rebuild byte-identical, then run
PATH=~/tools/zephyr-venv/bin:$PATH crates/firmware-apollo4p-zephyr/build.sh
#   dbf18ce4d3f2c53f4b5fec947664c00c93ebc4948cb8cbb3f2ffbeb377d6443f  amap42kp-zephyr-hello.elf
./target/release/labwired test --script examples/apollo4p-evb/zephyr-hello.yaml \
  --output-dir out/apollo4p-zephyr --no-uart-stdout
#   PASS  2/2 checks · zephyr-hello · 3000000 steps
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! apollo4p_evb/apollo4p
cargo test -p labwired-core --test apollo4_bootrom
cargo test -p labwired-core --features event-scheduler --test apollo4_bootrom
#   3 passed each

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
