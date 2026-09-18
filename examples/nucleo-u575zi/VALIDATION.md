# NUCLEO-U575ZI-Q Validation Runbook

Run all commands from the repository root. Captured evidence below is from
**2026-09-18** on branch `feat/onboard-stm32u575` unless a command output says
otherwise.

Tier: **sim-validated** — all values are SVD / RM0456 / DS13736-derived. There
is no bench part; no silicon capture and no Renode differential is claimed.

## Prerequisites

1. Rust toolchain (pinned by `rust-toolchain.toml`) with
   `thumbv8m.main-none-eabi` installed.
2. `arm-none-eabi-gcc` 13.2 for the CubeU5 HAL firmware.
3. STM32CubeU5 checkout — see [`EXTERNAL_COMPONENTS.md`](EXTERNAL_COMPONENTS.md).
4. Zephyr 3.7.2 workspace at `~/zephyrproject` with the `gnuarmemb` toolchain
   (`ZEPHYR_TOOLCHAIN_VARIANT=gnuarmemb GNUARMEMB_TOOLCHAIN_PATH=/usr`) for the
   Zephyr matrix.
5. PlatformIO for the Arduino matrix.

## A. Rust smoke + io-smoke

```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u575zi/io-smoke.yaml
```

Captured output (UART line then the test verdict):

```
OK
PASS  2/2 checks · io-smoke · 64 steps · 0.00s
```

`strict_onboarding` runs this same script for the chip; the crate is built from
source (no prebuilt asset).

## B. STM32CubeU5 HAL firmware

Checkout/revision: `12d19a5358da129dc74aecff1adee370218ca186` (HAL driver
`0e5fefb8dc2d6afa60816ebbf8b1672cfec4595b`, CMSIS device
`624374fa1e21ca195d6f2102ac0caaa50d0ea4c8`). The firmware does the full
Cube HAL flow: `HAL_Init` → ICACHE → MSIS/PLL1 160 MHz + FLASH latency 4 +
VOS1/SMPS → GPIO PC7 → USART1 PA9/PA10 → banner + blink loop.

```bash
make -C examples/nucleo-u575zi/board_firmware
RUST_LOG=off target/release/labwired \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000
```

Captured output (20M steps ≈ 0.14 s simulated; one blink = 250 ms ≈ 36M steps):

```
U575-HAL OK
BLINK 0 LD1=1
```

At `--max-steps 50000000`:

```
U575-HAL OK
BLINK 0 LD1=1
BLINK 1 LD1=0
```

## C. Determinism

Two runs, stdout only (`RUST_LOG=off`; stderr carries progress/timing logs), then
diff:

```bash
RUST_LOG=off target/release/labwired \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000 > out/u575/hal-20m-run1.txt
RUST_LOG=off target/release/labwired \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000 > out/u575/hal-20m-run2.txt
diff out/u575/hal-20m-run1.txt out/u575/hal-20m-run2.txt && echo DETERMINISTIC
```

Captured output: `DETERMINISTIC` (byte-identical UART stream).

## D. Unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system configs/systems/nucleo-u575zi.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-u575zi
```

Captured summary:

```
Audit summary:
  unknown_thumb16: 0
  unhandled_thumb32: 0
  unknown_riscv: 0
  unsupported_total: 0
  report: out/unsupported-audit/nucleo-u575zi/report.md
```

## E. Arduino matrix (fidelity engine)

```bash
cargo build -p labwired-cli --release
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575
```

Captured result: **7 pass / 2 skip / 0 fail**, with cache-hit builds:

| Level | Result | Marker |
|-------|--------|--------|
| L0_serial_boot | ✅ pass | `LW_L0_OK` |
| L1_serial_loop | ✅ pass | `LW_L1_OK` |
| L2_blink_serial | ✅ pass | `LW_L2_OK` (GPIO edges on `gpioc:7`) |
| L3_i2c_sensor | ✅ pass | `LW_L3_OK` — INA219 exact tier, no `LW_L3_PARTIAL_NO_RX` |
| L4_spi_sensor | ✅ pass | `LW_L4_OK` — exact MAX31855 frame `0x01901600` |
| L5_adc | ⏭️ skip | ADC1 uses the closest (`stm32h7`) profile; U5 `RES[3:2]` delta documented, `analogRead` unproven |
| L6_pwm | ✅ pass | `LW_L6_OK` |
| L7_timer | ✅ pass | `LW_L7_OK` |
| L8_can | ⏭️ skip | FDCAN not declared in the U5 chip yaml (first pass) |

Per-level `uart.log`/`result.json` under
`validation/arduino-matrix/out/stm32u575/<level>/run/`.

## F. Zephyr matrix (fidelity engine)

Stock Zephyr 3.7.2 (`c66235fb7346bbe3dbedd1dd76ec5a37a8e8262b`), board
`nucleo_u575zi_q`:

```bash
PATH="$HOME/zephyrproject/.venv/bin:$PATH" \
  python3 validation/zephyr-matrix/run_matrix.py --boards stm32u575 --no-build
```

Captured result: **4/4 pass** — `LW_Z0_OK`, `LW_Z1_OK`, `LW_Z2_OK`, `LW_Z3_OK`,
each preceded by `*** Booting Zephyr OS build c66235fb7346 ***`. Per-level
outputs under `validation/zephyr-matrix/out/stm32u575/<level>/run/`.

## G. PR-gate survival fixtures

Committed fixtures (hard `assert!` on presence — absence fails, never skips):

| Fixture | sha256 | Source |
|---------|--------|--------|
| `tests/fixtures/stm32u575-zephyr-hello.elf` | `6a40d36d4e6243287b40b5525d634428b32faac33f780b9c47135b3c18fec04d` | stock Zephyr `samples/hello_world` @ `c66235fb7346` for `nucleo_u575zi_q` (stripped, like the other `*-zephyr-hello.elf` fixtures) |
| `tests/fixtures/stm32u575-arduino-serial.elf` | `e4d620734c1b8bebd091cf17d6e87decdaaa6154f35fc5b64d2425a1419255a9` | Arduino matrix L0 (`PlatformIO`, `nucleo_u575zi_q`) |

```bash
cargo test -p labwired-core --test firmware_survival stm32u575 -- --nocapture
```

Captured output:

```
test test_stm32u575_arduino_serial_survival ... ok
test test_stm32u575_zephyr_survival ... ok
test result: ok. 2 passed; 0 failed
```

The Zephyr case asserts `Hello World! nucleo_u575zi_q` (the full stock line is
`Hello World! nucleo_u575zi_q/stm32u575xx`) on a real `attach_uart_tx_sink`
capture; the Arduino case asserts the `LW_L0_OK` marker and exercises the
Cube-startup `CRC->POL` write plus the U5 CRS register surface.

`validation/bus_proof_matrix.json` carries the U575 row (`uart`/`i2c`/`spi` all
`proven`), gated by `cargo test -p labwired-core --lib bus_proof_matrix`:

```
stm32u575        -       proven   proven   proven
```

## H. I2C L3 negative control (kit present vs absent)

Same stock Zephyr L3 ELF, same 15M step budget, only the system manifest
changes. The kit-free system (`examples/nucleo-u575zi/system.yaml`) declares
`external_devices: []`; the matrix system
(`validation/zephyr-matrix/systems/stm32u575.yaml`) attaches the INA219 at
`0x40` on `i2c1`.

```bash
# NEGATIVE — no device on the bus
target/release/labwired \
  --firmware validation/zephyr-matrix/out/stm32u575/L3_i2c_sensor/zephyr.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 15000000
# → *** Booting Zephyr OS build c66235fb7346 ***
#   LW_Z3_BOOT
#   LW_Z3_FAIL err=-5        (Zephyr -EIO: the absent address NACKs)

# POSITIVE — INA219 @0x40 attached
target/release/labwired \
  --firmware validation/zephyr-matrix/out/stm32u575/L3_i2c_sensor/zephyr.elf \
  --system validation/zephyr-matrix/systems/stm32u575.yaml --max-steps 15000000
# → *** Booting Zephyr OS build c66235fb7346 ***
#   LW_Z3_BOOT
#   LW_Z3_OK
```

The negative control proves the L3 oracle is not vacuous: the identical
firmware/ELF fails when nothing answers the address and passes only with the
INA219 model attached.

## I. Manifest / generated docs

```bash
python3 scripts/generate_validation_status.py --check   # exit 0
```

`validation/manifest.yaml` carries the `stm32u575` entry (`tier: sim-validated`,
no `silicon:`) and the regenerated
[`docs/boards/VALIDATION_STATUS.md`](../../docs/boards/VALIDATION_STATUS.md)
renders it as "no silicon capture". `--check --drift` is green. The 7
silicon-verified boards it used to flag (nrf52840, seeed-xiao-nrf52840-sense,
stm32h563, nucleo-l476rg, nucleo-l073rz, stm32f103, stm32f407) were this
branch's own drift, not a pre-existing red: the CPU shift-flag edits
(`crates/core/src/cpu/cortex_m.rs`, `crates/core/src/decoder/arm.rs`) and the
V2 RCC PLL1/U5 CR-ready edits (`crates/core/src/peripherals/rcc.rs`) changed
their watched content digests past the old `drift_ack_digest`. Each was
re-acked on 2026-09-18 with a note naming the changed paths (see its
`drift_ack` block in `validation/manifest.yaml`) and re-stamped with
`python3 scripts/generate_validation_status.py --write-ack-digests`; a live
re-capture remains owed. U575 is not among them.

## J. Scoreboards (partial-run policy)

Fleet scoreboards under `docs/coverage/` are published **only by full matrix
runs**. U575 was verified locally 2026-09-18; the fleet scoreboard refresh is
pending the next full run. The partial runs above wrote only
`validation/<matrix>/out/scoreboard.md` and left
`docs/coverage/arduino-scoreboard.md` / `docs/coverage/zephyr-scoreboard.md`
at their last full-run revisions (the Arduino runner printed
"Docs scoreboard unchanged (partial run; not full 18×9)").

## K. Final sweep (2026-09-18)

Fresh Task 9 rerun at `52d2b9f1` (`feat/onboard-stm32u575`), after the Tasks
1–8 review round. Commands are reproduced verbatim from the task plan;
deviations are called out inline.

### K.1 Targeted + regression tests

```bash
cargo test -p labwired-core --lib peripherals:: -- --nocapture 2>&1 | tail -20
```

```text
test result: ok. 2589 passed; 0 failed; 0 ignored; 0 measured; 954 filtered out; finished in 0.81s
```

```bash
cargo test -p labwired-core --test chip_conformance --test svd_conformance --test strict_onboarding -- --nocapture 2>&1 | tail -30
```

The exact command stops at the known `strict_onboarding` red (cargo fail-fast),
so it was rerun once with `--no-fail-fast` to capture all three binaries:

```text
# chip_conformance
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.86s
# svd_conformance
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.53s
# strict_onboarding
  [PASS] stm32u575 is strictly onboarded.
Error: Strict Board Onboarding Failed for: ["imxrt1064 (missing io-smoke.yaml, not in allowlist)", "stm32f401 (missing io-smoke.yaml, not in allowlist)", "stm32f746 (missing io-smoke.yaml, not in allowlist)", "atsamd51 (missing io-smoke.yaml, not in allowlist)", "ra4m1 (missing io-smoke.yaml, not in allowlist)", "atsamd21 (missing io-smoke.yaml, not in allowlist)"]
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 49.24s
```

U575 passes; the six listed chips are the pre-existing reds.

```bash
cargo test -p labwired-core --test firmware_survival stm32u575 -- --nocapture 2>&1 | tail -10
```

```text
test test_stm32u575_zephyr_survival ... ok
test test_stm32u575_arduino_serial_survival ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 71 filtered out; finished in 2.24s
```

```bash
cargo test -p labwired-core --lib bus_proof_matrix -- --nocapture 2>&1 | tail -10
```

```text
bus-proof baseline: 944ee7f183230623df753f1b92eb864c4af6504b (102 cells) vs live (105 cells)
test tests::bus_proof_matrix::bus_proof_matrix_never_regresses ... ok
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 3533 filtered out; finished in 0.05s
```

```bash
cargo test -p labwired-core h563 -- --nocapture 2>&1 | tail -10
cargo test -p labwired-core wba52 -- --nocapture 2>&1 | tail -10
```

`tail -10` lands on trailing zero-match test binaries in both cases, so the
matched tests below are quoted from a full capture. H563:

```text
test cpu::cortex_m::tests::test_thumb2_vfma_decodes_the_real_h563_opcode ... ok
test test_stm32h563_arduino_serial_survival ... ok
test test_stm32h563_demo_survival ... ok
test test_stm32h563_zephyr_survival ... ok
test h563_requires_cycle_accurate ... ok
test h563_is_walk_free_and_tick_512 ... ok
```

WBA52:

```text
test test_stm32wba52_zephyr_survival ... ok
test test_stm32wba52_arduino_serial_survival ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 71 filtered out; finished in 1.78s
```

Extra L476 regression sweep (same command pattern): `board_io_button_stimulus`
2 passed (`the_l476_demo_reads_its_released_user_button_as_silicon_does`,
`pressing_b1_flips_the_byte_the_l476_demo_prints`), `firmware_survival` 17
passed, `nucleo_l476rg` 4 passed — all ok.

Full library suite:

```bash
cargo test -p labwired-core --lib 2>&1 | tail -5
```

```text
test result: ok. 3540 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 4.52s
```

### K.2 Unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system configs/systems/nucleo-u575zi.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-u575zi
```

```text
Audit summary:
  unknown_thumb16: 0
  unhandled_thumb32: 0
  unknown_riscv: 0
  unsupported_total: 0
  report: out/unsupported-audit/nucleo-u575zi/report.md
```

No unsupported instructions on the 200k-step boot path.

### K.3 Determinism (fresh)

```bash
cargo run -q -p labwired-cli -- --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000 > out/u575-run-1.txt
cargo run -q -p labwired-cli -- --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000 > out/u575-run-2.txt
diff out/u575-run-1.txt out/u575-run-2.txt && echo DETERMINISTIC
```

```text
DETERMINISTIC
```

stdout of each run (28 bytes):

```text
U575-HAL OK
BLINK 0 LD1=1
```

### K.4 Docs / gates freshness

```bash
python3 scripts/generate_validation_status.py --check
```

Exit 0, no output. `--check --drift` also exits 0: the 7 silicon-board reds
this branch produced (`nrf52840`, `seeed-xiao-nrf52840-sense`, `stm32h563`,
`nucleo-l476rg`, `nucleo-l073rz`, `stm32f103`, `stm32f407`) were NOT
pre-existing — the branch's CPU shift-flag and V2 RCC PLL1/U5 CR-ready edits
moved their watched content digests past the old `drift_ack_digest`. Each was
re-acked on 2026-09-18 with a note naming the changed files and re-stamped via
`--write-ack-digests`; a live re-capture remains owed. U575 is not among them.

Arduino matrix — the runner has no `--no-build`; its documented equivalent from
`--help` is `--sim-only`, which reuses the cached ELFs:

```bash
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575 --sim-only
```

```text
==> stm32u575 × L0_serial_boot
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L1_serial_loop
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L2_blink_serial
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L3_i2c_sensor
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L4_spi_sensor
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L5_adc: skip (ADC1 descriptor uses the closest profile (stm32h7); the U5 RES[3:2] delta is documented but analogRead is unproven on U5 yet)
==> stm32u575 × L6_pwm
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L7_timer
    compile: skipped (--sim-only)
    run: pass
==> stm32u575 × L8_can: skip (FDCAN not declared in the U5 chip yaml (first pass))

Done in 17s — 7 pass / 2 skip / 0 fail (cells 9; gate treats skip as non-fail)
Docs scoreboard unchanged (partial run; not full 18×9)
```

Zephyr matrix:

```bash
PATH="$HOME/zephyrproject/.venv/bin:$PATH" python3 validation/zephyr-matrix/run_matrix.py --boards stm32u575 --no-build
```

```text
=== stm32u575 / L0_hello (nucleo_u575zi_q) ===
  -> pass  uart='*** Booting Zephyr OS build c66235fb7346 ***\\nLW_Z0_OK\\n'

=== stm32u575 / L1_sleep (nucleo_u575zi_q) ===
  -> pass  uart='*** Booting Zephyr OS build c66235fb7346 ***\\nLW_Z1_BOOT\\nLW_Z1_OK\\n'

=== stm32u575 / L2_blink (nucleo_u575zi_q) ===
  -> pass  uart='*** Booting Zephyr OS build c66235fb7346 ***\\nLW_Z2_BOOT\\nLW_Z2_OK\\n'

=== stm32u575 / L3_i2c_sensor (nucleo_u575zi_q) ===
  -> pass  uart='*** Booting Zephyr OS build c66235fb7346 ***\\nLW_Z3_BOOT\\nLW_Z3_OK\\n'

Done: 4/4 pass/skip (0 fail).
```

```bash
cargo fmt --all -- --check                                    # exit 0
cargo clippy -p labwired-core --all-targets -- -D warnings    # exit 0
python3 scripts/ci/chip_coverage.py --report
python3 scripts/ci/docs-commands-gate.py target/release/labwired
```

```text
chip coverage: 35 chips proven by a CLI gate, 0 declared uncovered (ceiling 0)
checked 35 documented command(s) against target/release/labwired
every documented command is one the CLI accepts.
```

The docs-commands gate needs an executable CLI (`labwired` on `PATH`, or passed
as argv[1]). Before the matrices the release CLI was rebuilt
(`cargo build -p labwired-cli --release`); cargo found it already up to date
with the HEAD library sources.

### K.5 Remaining reds (unchanged, pre-existing)

| Red | Scope | Owner |
|-----|-------|-------|
| `strict_onboarding` fails chips missing `io-smoke.yaml` (pre-existing) | imxrt1064, stm32f401, stm32f746, atsamd51, ra4m1, atsamd21 | chip owners |
| `docs/coverage/{arduino,zephyr}-scoreboard.md` not republished | partial runs don't publish (documented policy, §J) | next full matrix run |

The 7 board drifts this branch caused are re-acked with rationale (§K.4), not
outstanding; no H563/WBA52/L476 regressions.
