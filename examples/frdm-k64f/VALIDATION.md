# Validation — FRDM-K64F (MK64FN1M0VLL12)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x410, SP: 0x20030000` (`.text` starts after the FCF at 0x400) |
| Cortex-M4 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Flash Configuration Field at 0x400..0x40F | fixture-checked | `test_frdm_k64f_smoke_flash_config_field_is_unsecured`: key/FPROT `0xFF`, FSEC `0xFE` (unsecured, mass erase enabled), FOPT/FEPROT/FDPROT `0xFF` |
| WDOG disabled at the top of `main` (`0xC520`/`0xD928` unlock, `STCTRLH.WDOGEN=0`) | declared only | WDOG is a stub (no timeout/reset modelled); the firmware disables it as silicon requires |
| SIM clock gates (`SCGC4` UART0, `SCGC5` PORTB/PORTE) | declared only | the SIM window is a stub; the firmware writes the gates as silicon requires, the simulator does not gate on them |
| GPIOB/GPIOE output drive + PDOR readback | smoke-tested | `led=red on pin=0` … asserted in `io-smoke.yaml` |
| UART0 PTB16/PTB17 TX @ 115200 from 20.97152 MHz FEI | smoke-tested | `FRDM-K64F SMOKE` + heartbeat lines on the console sink |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported (smoke); 2,000,000 each, 0 unsupported (both Zephyr fixtures) |
| PORT pad mux/pulls | **not modelled** | PORTx PCR is a stub |
| Stock Zephyr v3.7.0 `samples/hello_world` (`frdm_k64f`) | smoke-tested | `zephyr-hello.yaml` 2/2; `firmware_survival::test_frdm_k64f_zephyr_survival` |
| Zephyr `tests/drivers/uart/uart_basic_api` (Renode `NXP-K64F.robot` UART case) | smoke-tested | `zephyr-uart-basic-api.yaml` 2/2: 7/7 ztest cases, `PROJECT EXECUTION SUCCESSFUL` |
| UART0 C2 `TIE`/`TCIE`/`RIE` interrupts (IRQ 31) | smoke-tested + unit | IRQ-driven `fifo_fill`/`fifo_read` and the `uart_irq_is_pending` test pass; `uart::tests::test_kinetis_uart_c2_interrupt_enables` |
| MCG PLL0 (`k6x` profile): FBE → PBE → PEE | smoke-tested + unit | Zephyr's `CLOCK_BootToPeeMode` settles; `mcg::tests::k6x_*`. Lock is immediate and the 120 MHz output is not derived |
| MCG FEE (the KW41Z FLL path) | unit-tested | unchanged; `mcg::tests` |
| SYSMPU / PMC / OSC0 | register files (NXP SVD) | Zephyr's `nxp_mpu`, `k6x_init` and `CLOCK_InitOsc0` writes land and read back; **no MPU protection, LVD or oscillator is modelled** |
| Zephyr `tests/net/tcp` (Renode `NXP-K64F.robot` TCP case) | smoke-tested | `zephyr-net-tcp.yaml` 2/2: 19/19 ztest cases in order, `PROJECT EXECUTION SUCCESSFUL` |
| RNGA (`kinetis_rnga`) | smoke-tested + unit | Zephyr's `entropy_mcux_rnga` init and every `sys_rand_get()` complete; `kinetis_rnga::tests` (6). OR refills instantly, samples are a fixed-seed xorshift32 perturbed by `ER` (not entropy), the error IRQ 23 and `SECV` are not modelled |
| ENET + KSZ8081 PHY | **not modelled** | the Ethernet blocks are stubs. `tests/net/tcp` runs over its own dummy L2 (`CONFIG_NET_L2_ETHERNET=n`), so it does not touch them |
| FTFE | declared stub | `configs/chips/mk64f12.yaml` (`type: "stub"`) |
| eDMA, PIT/FTM, I2C/SPI, ADC/DAC, USB, CAN, SDHC | **not mapped** | no window in `configs/chips/mk64f12.yaml`; an access faults with a memory violation |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-frdm-k64f-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./target/release/labwired test \
  --script examples/frdm-k64f/io-smoke.yaml \
  --output-dir out/frdm-k64f-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.01s

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-frdm-k64f-smoke \
  --system configs/systems/frdm-k64f.yaml --max-steps 400000
#   Initial PC: 0x410, SP: 0x20030000
#   FRDM-K64F SMOKE
#   cpu=cortex-m4 uart=uart0
#   led=red on pin=0
#   ...
#   hb=0 red=0 green=1 blue=1

# Determinism: two runs, identical UART + snapshot
cmp out/run1/uart.log out/run2/uart.log          # clean
cmp out/run1/snapshot.json out/run2/snapshot.json  # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-frdm-k64f-smoke \
  --system configs/systems/frdm-k64f.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/frdm-k64f
#   Instructions executed: 200000 · Unsupported: 0
```

### Stock Zephyr (verified 2026-10-30)

All four fixtures are unmodified upstream Zephyr v3.7.0 (commit `36940db938a8`)
for `frdm_k64f`, built with `arm-none-eabi-gcc` 13.2.1 (gnuarmemb) and
debug-stripped.

```bash
crates/firmware-frdm-k64f-zephyr/build.sh       # needs a Zephyr v3.7.0 west workspace
#   b0b136f6…ea3993  tests/fixtures/mk64f12-zephyr-hello.elf
#   6ea91e77…711f2e  tests/fixtures/mk64f12-zephyr-uart-basic-api.elf
#   8e56c2fc…b5275f  tests/fixtures/mk64f12-zephyr-sync.elf
#   0fb62bc0…ac3600  tests/fixtures/mk64f12-zephyr-net-tcp.elf   (2026-10-31)
#   (two rebuilds, identical; build.sh pins ZEPHYR_MODULES so fetching
#   more modules into the workspace does not change the bytes)

./target/release/labwired test --script examples/frdm-k64f/zephyr-hello.yaml
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! frdm_k64f/mk64f12
#   PASS  2/2 checks · zephyr-hello · 3000000 steps

./target/release/labwired test --script examples/frdm-k64f/zephyr-uart-basic-api.yaml
#   SUITE PASS - 100.00% [uart_basic_api]: pass = 6, fail = 0, skip = 0, total = 6
#   SUITE PASS - 100.00% [uart_basic_api_pending]: pass = 1, fail = 0, skip = 0, total = 1
#   PROJECT EXECUTION SUCCESSFUL
#   PASS  2/2 checks · zephyr-uart-basic-api · 45092621 steps · ~10 s

./target/release/labwired test --script examples/frdm-k64f/zephyr-sync.yaml
#   *** Booting Zephyr OS build v3.7.0 ***
#   thread_a: Hello World from cpu 0 on frdm_k64f!
#   thread_b: Hello World from cpu 0 on frdm_k64f!
#   thread_a: Hello World from cpu 0 on frdm_k64f!
#   PASS  2/2 checks · zephyr-sync · 30000000 steps            (2026-10-31)
#   negative control: max_steps 8000000 stops before the timeout round, FAIL 1/2
#   unsupported_instruction_audit.sh over 30M steps: unsupported_total 0

./target/release/labwired test --script examples/frdm-k64f/zephyr-net-tcp.yaml
#   SUITE PASS - 100.00% [net_tcp]: pass = 19, fail = 0, skip = 0, total = 19
#   PROJECT EXECUTION SUCCESSFUL
#   PASS  2/2 checks · zephyr-net-tcp · 2855528 steps · 0.17s   (2026-10-31)
#   negative control: RNGA back to `type: stub` hangs in RNGA_GetRandomData
#   (OREG_LVL poll) before the banner, FAIL 0/2 at 50M steps
#   max_cycles probes: 700M fails (suite still running), 800M passes
#   unsupported_instruction_audit.sh over 2.8M steps: unsupported_total 0

# Determinism: all scripts run twice with --output-dir; uart.log and
# snapshot.json cmp clean.
```

How it differs from the Renode robot:

- Renode waits for each `Please send characters to serial console` line, but
  a LabWired script cannot wait for a UART line. So the three
  `uart_injections` are cycle-timed (70M / 95M / 105M). The prompts land at
  ~61M, ~71–80M and ~96–100M cycles, measured with `max_cycles` probes.
- Zephyr v3.7.0 adds the third prompt (`uart_basic_api_pending`). Renode's
  older prebuilt ELF stops at two.
- The run is bounded by `max_cycles`. The pending test leaves `C2.RIE` set, so
  the UART keeps its per-cycle RX wakeup armed while the idle thread sits in
  WFI. Under idle fast-forward those cycles are not steps, so `max_steps`
  alone never ends the run. The wakeup itself is needed, because hosts such
  as the playground push typed bytes straight into the RX buffer.

Negative control: the same script on a build without the C2 interrupt
enables fails at `test_uart_fifo_fill` (0/2).

The TCP case matches Renode's robot directly: it waits for `PROJECT EXECUTION
SUCCESSFUL` and needs no input. The script also pins the 19 case names in
order. The suite's ~6.6 s of kernel time is nearly all idle, which idle
fast-forward skips, so `max_cycles` (1e9) bounds the run.

## Notes

- The heartbeat reports the actual `PDOR` bit for each LED (0 = lit, because
  the LEDs are active low), so a passing run proves the pad the firmware drove.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The K64F `core` is `cortex-m4` and the firmware is built for
  `thumbv7em-none-eabi` (integer ABI); the M4F FPU is not exercised.
- The board's UART0 pins are PTB16 (RX) / PTB17 (TX) per the Zephyr pinctrl
  file. The survey text listed them in the opposite order; the descriptor and
  the firmware follow the board dts.
