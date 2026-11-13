# Validation — SAM E70 Xplained (ATSAME70Q21B)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no SAM E70 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x400400, SP: 0x20460000` |
| Cortex-M7 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| `WDT_MR.WDDIS` (watchdog disable) | declared only | the WDT window is a stub; the watchdog never fires, with or without the write |
| `MATRIX CCFG_SYSIO.SYSIO4` (PB4 from TDI to PIO) | declared only | the matrix window is a stub; the PIO model does not gate pads on SYSIO |
| PMC `PCER0` gates + `PCSR0` readback, `PMC_SR.MCKRDY` | smoke-tested | `pmc pcsr0=00005c00` (ids 10, 11, 12, 14) |
| PMC `PCER1/PCDR1/PCSR1` at +0x100, `SCER/SCSR` | unit-tested | `sam4s::tests::pmc_bank1_is_at_0x100_and_system_clocks_stay_out_of_bank0` |
| PMC clock generator: `CKGR_MOR` (0x37 key), `CKGR_PLLAR`, `CKGR_UCKR`, `PMC_MCKR` CSS/PRES/MDIV, `PMC_SR` derived | unit-tested + Zephyr-boot-tested | `sam4s::tests::pmc_status_follows_the_clock_generator` (reset SR `0x0003_0008`, keyless MOR write aborted); `same70_zephyr_boot`: MOSCSEL, MULA 24 / DIVA 1, CSS PLLA, MDIV /2, SR MOSCXTS/LOCKA/MCKRDY/LOCKU/MOSCSELS |
| Clock frequencies (start-up times, PLL output, MCK rate) | **not modelled** | every enabled source is stable at once; the core runs at the board's `cpu_hz` whatever MCKR selects |
| CHIPID `CIDR`/`EXID` | Zephyr-boot-tested | `0xA102_0E01` / `0x2` (rev B, from the hal_atmel `same70q21b.h`); unmapped, Zephyr's `atmel_same70_init` BusFaults at `0x400E_0940` (negative control) |
| Cortex-M7 L1 cache identification (CLIDR/CTR/CCSIDR/CSSELR) | unit-tested + Zephyr-boot-tested | `cortex_m7_cache::tests`, `scb::tests::cache_id_registers_follow_the_declared_m7_geometry`; CLIDR `0x0900_0003`, D CCSIDR `0xF00F_E019`, I `0xF01F_E009` |
| Cortex-M7 cache maintenance (`0xE000_EF50`..`EF78`) | Zephyr-boot-tested (accepted, no-op) | `same70_zephyr_boot`: ≥ 4 × 512 set/way ops, CCR.IC/DC set then cleared by `z_arm_init_arch_hw_at_boot`; without the window the first `DCISW` store locks the core up (negative control) |
| Cache contents / coherence | **not modelled** | memory is always coherent: a missing clean/invalidate around DMA works here and fails on silicon |
| Stock Zephyr v3.7.0 `hello_world` | smoke-tested | `zephyr-hello.yaml` 2/2; `firmware_survival::test_sam_e70_xplained_zephyr_survival` |
| Stock Zephyr v3.7.0 `synchronization` (SysTick timeouts, semaphores, PendSV) | smoke-tested | `zephyr-sync.yaml` 2/2 (`thread_a` → `thread_b`); two runs, identical `uart.log` + `snapshot.json` |
| PIO `ABCDSR1/2` + `PDR` (PA21 → A, PB4 → D) | smoke-tested (stored) | `mux pa21 psr=0 pb4 psr=0 abcd=11`; the select has no routing effect |
| USART1 async TX, 8N1, `BRGR` CD 6 / FP 4 | smoke-tested | every console line, polled on `US_CSR.TXRDY` |
| PIOC `PER/OER/SODR/CODR/ODSR/PDSR` (LED0 on PC8, active low) | smoke-tested | `led=on pin=0 odsr=00000000` … (the whole-ODSR read proves no other pin moved) |
| PIOA `PUER` + `PDSR` (SW0 on PA11) | smoke-tested | `sw0=1`: the released button reads high through the pull-up |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| USART/PIO interrupts | **not modelled** in this slice | the smoke is polled |
| TCM, MPU, FPU | **not exercised** | the smoke leaves them at reset and is built without FPU instructions; Zephyr's MPU regions are stored, not enforced |
| EEFC, RSTC, SUPC, RTT, WDT, RTC, bus matrix | declared stubs | `configs/chips/same70q21b.yaml` (`type: "stub"`) |
| GMAC, TRNG, TC0-3, SPI0/1, QSPI, TWIHS0-2, MCAN0/1, AFEC0/1, DACC, PWM0/1, HSMCI, SSC, USBHS, XDMAC, AES, ICM, ISI | **not mapped** | no window in `configs/chips/same70q21b.yaml`; an access faults with a memory violation |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-sam-e70-xplained-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./examples/sam-e70-xplained/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unknown_thumb16: 0 · unhandled_thumb32: 0 · unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-sam-e70-xplained-smoke \
  --system configs/systems/sam-e70-xplained.yaml --max-steps 400000
#   Initial PC: 0x400400, SP: 0x20460000
#   SAM E70 XPLAINED SMOKE
#   cpu=cortex-m7 uart=usart1
#   clk mainck=12000000 cd=6 fp=4 baud=115384
#   pmc pcsr0=00005c00
#   ...

# Determinism: two runs, identical UART + snapshot
cmp out/same70-run1/uart.log out/same70-run2/uart.log          # clean
cmp out/same70-run1/snapshot.json out/same70-run2/snapshot.json  # clean

cargo test -p labwired-core --test firmware_survival sam_e70
#   1 passed (test_sam_e70_xplained_smoke_survival)

python3 scripts/perf/board_perf.py --boards same70q21b
#   same70q21b  same70  step 892.0 · batch 1.4 Ir/step
```

### Stock Zephyr (verified 2026-10-30)

```bash
# Fixtures: unmodified Zephyr v3.7.0 (36940db938a8), board sam_e70_xplained/same70q21b
./crates/firmware-sam-e70-xplained-zephyr/build.sh
#   same70q21b-zephyr-hello.elf 5096c83125f750a955f7e137febc51874c425b13e4bcb39332734861321ef6be
#   same70q21b-zephyr-sync.elf  04f0604a32dae42e470433d9dabfea514fa64c17739c4e6c0d1961d19c6e2db1
#   (two rebuilds, identical bytes)

./target/release/labwired test --script examples/sam-e70-xplained/zephyr-hello.yaml --no-uart-stdout
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! sam_e70_xplained/same70q21b
#   PASS  2/2 checks · zephyr-hello · 3000000 steps

./target/release/labwired test --script examples/sam-e70-xplained/zephyr-sync.yaml --no-uart-stdout
#   thread_a: Hello World from cpu 0 on sam_e70_xplained!
#   thread_b: Hello World from cpu 0 on sam_e70_xplained!
#   PASS  2/2 checks · zephyr-sync · 50000000 steps

cargo test -p labwired-core --test same70_zephyr_boot
#   1 passed (zephyr_bring_up_walks_the_m7_caches_and_locks_the_plls)

./scripts/unsupported_instruction_audit.sh --firmware tests/fixtures/same70q21b-zephyr-sync.elf \
  --system configs/systems/sam-e70-xplained.yaml --max-steps 2000000 --fail-on-unsupported
#   unsupported_total: 0
```

Negative controls (each with only that one block removed from the chip YAML):
no `l1_cache` → core LOCKUP at the first `DCISW` store in
`SCB_InvalidateDCache`; no `chipid` → precise BusFault at `0x400E_0940` in
`atmel_same70_init`. With the old PMC (fixed `MOSCXTS|MCKRDY` status) the boot
spins forever in `soc_pmc_switch_mainck_to_fastrc` waiting for `MOSCRCS`.

## Notes

- **Zephyr runs cache-off.** `z_arm_platform_init` (SoC) invalidates and
  enables both caches, then `z_arm_init_arch_hw_at_boot`
  (`CONFIG_INIT_ARCH_HW_AT_BOOT=y`) cleans and disables them again. That is
  the stock v3.7.0 sequence on this board, and the model reproduces it; it is
  not a model artefact.
- **Busy-wait dominates `synchronization`.** Each thread calls
  `k_busy_wait(100000)`, 30 M cycles at Zephyr's 300 MHz clock, before
  `k_msleep(500)`. The sleeps are fast-forwarded, the busy-waits are not, so
  one `thread_a` → `thread_b` round is what fits in the 50 M-step CLI cap.
- **`cpu_hz`.** The board keeps `cpu_hz: 12_000_000` (the smoke's real clock).
  Zephyr programs 300 MHz and computes SysTick reloads for it; the simulated
  core runs one instruction per cycle either way, so in-guest timing is
  self-consistent, only wall-clock pacing differs.

- **Reused models.** The SAM E70 PIO and PMC keep the SAM4S register layouts,
  so this slice reuses `sam4s_pio` and `sam4s_pmc`. Two model fixes came
  with it, and they also apply to the SAM4S16C:
  - `PMC_PCER1/PCDR1/PCSR1` moved from +0x40/+0x44/+0x48 (those are
    `PMC_PCK0-2`) to +0x100/+0x104/+0x108. `PMC_SCER/SCDR` now drive their
    own `SCSR` and no longer set bits in `PCSR0`.
  - `PIO_ABCDSR2` (+0x74) is stored. Before, only `ABCDSR1` was, so the
    peripheral C/D selects were lost.
- **Perf.** Both SAM4S models are plain register banks with no `tick()`, but
  they kept the trait default `legacy_tick_active() == true`. So the bus
  walked every instance on every cycle, and the batched ARM loop never
  engaged: the first perf run on this chip measured 2415 Ir/step in batch
  mode. They now opt out the same way `ra_pfs` and the GPIO models do, and
  the chip measures 892.0 step / 1.4 batch Ir/step. The SAM4S16C has no perf
  fixture (it is waived in `board_perf.py`), so it has no measured number,
  but it gets the same fix.
- **Toolchain.** The firmware is built for `thumbv7em-none-eabi`
  (soft-float), so the compiler emits no FPU instructions and the smoke
  never has to enable CP10/CP11. The unsupported-instruction audit runs on
  that ELF.
- **Clock.** The SAM E70 resets onto the 12 MHz main RC. Zephyr's 115200
  console needs the fractional divider: CD 6 / FP 4 gives
  12 MHz / (16 × 6.5) = 115384 baud (+0.16 %).
- **Vector table size.** `cortex-m-rt` reserves 240 external vectors for
  ARMv7-M without a device crate, so `.text` starts at `0x0040_0400`.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/sam_e70.yaml` copies the Renode
  platform's 256 MiB flash at `0x08000000` and 256 MiB of RAM, which is not
  the SAM E70 map. It is kept, marked superseded, because
  `crates/core/src/bus/known_stubs.rs` still cites it.
