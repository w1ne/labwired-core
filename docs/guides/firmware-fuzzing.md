# Firmware fuzzing, silicon-confirmed

Coverage-guided fuzzing finds crashing inputs fast. The problem with doing it on
an emulator is that the emulator's peripheral models are usually *unvalidated* —
so a "crash" might be a model artifact, not a real bug. Unicorn-AFL, Fuzzware and
P2IM all spend reviewer time triaging phantom crashes that never reproduce on the
chip.

LabWired fuzzes in a **silicon-validated** simulator and then **replays every
crash on the real hardware over SWD**. A finding is reported as CONFIRMED only if
it reproduces on silicon; crashes that don't are flagged SIM-ONLY and filtered
out. That's the whole pitch: *fuzz in sim at scale, confirm on real silicon,
zero false positives.*

```
coverage-guided fuzz (sim)  ─►  distinct crashes  ─►  replay on F103 (SWD)
                                                        │
                                          ┌─────────────┴─────────────┐
                                      CONFIRMED                   SIM-ONLY
                                  (reproduces on chip)        (model artifact,
                                                                 filtered)
```

## The fuzz contract

The target firmware follows a tiny contract so the harness can inject an input
and read the outcome — the same contract works in sim and on silicon:

| Region | Default address | Meaning |
|---|---|---|
| input length | `0x20002800` | u32: number of input bytes the harness wrote |
| input data | `0x20002804` | the input bytes |
| verdict | `0x20003000` | u32 the firmware writes when it finishes |
| `DONE` marker | `0xC0DEF022` | written on clean completion |
| `FAULT` marker | `0xDEADFA17` | written by a fault/panic handler |

A crash is a CPU fault (the sim surfaces it as a step error), the firmware's
`FAULT` marker, or a hang (the step budget is exhausted without `DONE`). All of
the addresses/markers are overridable; the defaults match
`crates/firmware-f103-fuzztarget`.

See that crate for a minimal `no_std` fuzz target: a command parser with a
planted stack overflow on op `C`.

## Fuzz from the CLI

```bash
# Build the fuzz target (it follows the contract above).
cargo build -p firmware-f103-fuzztarget --target thumbv7m-none-eabi --release

# Fuzz it. Exits non-zero if a crash is found — drop it straight into CI.
labwired fuzz \
  --chip   core/configs/chips/stm32f103.yaml \
  --system core/configs/systems/stm32f103-bare.yaml \
  --firmware target/thumbv7m-none-eabi/release/firmware-f103-fuzztarget \
  --seed-input 5000 \
  --collect 8 \
  --crashes-out crashes.json
```

```
fuzzing …/firmware-f103-fuzztarget (max_iters=200000, seed=0xdeadbeef) ...
found 5 distinct crash(es):
  [43, 50, 00]
  [43, 44, BF]
  ...
first crash reproduces as: crash (fault/panic marker)
wrote 5 crash input(s) to crashes.json
```

Fuzzing is deterministic for a fixed `--seed`, so a crash always reproduces.

## Fuzzing engine: built-in or LibAFL

By default `labwired fuzz` uses a small built-in coverage-guided loop — fast to
build, no heavy dependencies, deterministic. For serious campaigns, build with
the `fuzz-libafl` feature to swap in a full [LibAFL] fuzzer (havoc/splice
mutators, a map-feedback queue scheduler, crash objectives):

```bash
cargo build --release -p labwired-cli --features fuzz-libafl
```

The simulator is wrapped as a LibAFL in-process executor: `Target::run` fills the
edge bitmap a `StdMapObserver` watches, and a sim CPU fault (or the firmware
FAULT marker) is reported as `ExitKind::Crash` and routed into the solutions
corpus by `CrashFeedback`. Both engines find the same bugs; LibAFL explores a
richer input space (it discovers multi-frame inputs the built-in mutator
doesn't) and scales better. The CLI prints which engine it used.

[LibAFL]: https://github.com/AFLplusplus/LibAFL

## Confirm on silicon (HIL)

With a board on the bench (ST-Link over SWD), the HIL-confirm harness fuzzes,
collects distinct crashes, flashes once, and replays each input on the chip —
classifying CONFIRMED vs SIM-ONLY and reporting the false-positive rate:

```bash
STM32_TARGET=stm32f1x cargo test -p labwired-hw-oracle \
  --test f103_fuzz_hil_confirm --features hw-oracle-stm32 \
  -- --ignored --test-threads=1 --nocapture
```

```
sim found 8 distinct crash input(s)
  [43, 55, 00]      silicon: CONFIRMED
  [43, AD, 10, 00]  silicon: CONFIRMED
  ...
HIL-confirm: 8/8 CONFIRMED on silicon, 0 sim-only (false-positive rate 0%)
```

The input region is zeroed on both sides before injection so an over-read crash
(the planted bug reads a length past the supplied data) is deterministic across
sim and silicon.

## Use it as a CI gate

`labwired fuzz` exits non-zero on a crash, so it gates a build like any test:

```yaml
# .github/workflows/fuzz.yml
name: fuzz
on: [pull_request]
jobs:
  fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: thumbv7m-none-eabi
      - run: cargo build -p labwired-cli --release
      - run: cargo build -p my-fuzz-target --target thumbv7m-none-eabi --release
      - name: Fuzz (fails the build on a crash)
        run: |
          ./target/release/labwired fuzz \
            --chip   configs/chips/stm32f103.yaml \
            --system configs/systems/stm32f103-bare.yaml \
            --firmware target/thumbv7m-none-eabi/release/my-fuzz-target \
            --max-iters 500000 \
            --collect 16 \
            --crashes-out crashes.json
      - uses: actions/upload-artifact@v4
        if: failure()
        with:
          name: crashes
          path: crashes.json
```

The sim run is the fast, scalable gate. When the gate trips, run the HIL-confirm
harness on a board to prove the crash is real before you spend reviewer time on
it.

## From an agent (MCP)

The `@labwired/mcp` server exposes `labwired_fuzz`: pick a board with
`labwired_list_boards`, compile your fuzz target locally, and pass the ELF. It
returns the distinct crashing inputs (hex + raw bytes) for replay or
minimization. Because the sim is silicon-validated, an agent can trust the
findings without flashing a board itself.
