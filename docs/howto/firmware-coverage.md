# Firmware coverage: lines, branches, functions

LabWired measures which parts of your firmware your tests ran, on the
**unmodified ELF**: no `--coverage` build, no gcov runtime in flash, no data
to dump over a UART. The simulator already sees every instruction, so it
counts them.

You get:

- **Line coverage** with hit counts (DWARF line table).
- **Function coverage** with call counts (ELF function symbols).
- **Branch coverage**: every conditional branch instruction in the ELF, each
  with a "jump" and a "fall-through" count. Branches the tests never reached
  are counted too, so the percentage does not flatter you.
- **Merge** of many runs into one report, with the list of tests that ran
  each line.
- **LCOV** (`genhtml`, Coverview, Codecov, VS Code Coverage Gutters) and
  **Cobertura XML** (GitLab merge-request coverage, Jenkins, Azure DevOps).

Branch coverage works on **Thumb / Thumb-2 (every Cortex-M)** and **RISC-V
(RV32/RV64, with or without the C extension)**. Line and function coverage
work on every architecture LabWired runs.

---

## Quick start

The example is `examples/coverage-brake-ecu/`: a small brake-light ECU with
AUTOSAR-style end-to-end protection (CRC-8 + alive counter) and six tests.
Its ELFs are committed, so there is nothing to build.

Run three tests with coverage:

```bash
cd examples/coverage-brake-ecu
for t in pedal-press emergency-flag crc-error; do
  labwired test --script tests/$t.yaml --coverage --output-dir out/$t
done
```

Each output directory now holds `coverage.info` (LCOV),
`coverage.cobertura.xml` and `coverage.json`.

Merge them:

```bash
labwired coverage merge out -o out/merged.info --fail-under-branches 90
```

```
Merged 3 coverage file(s), 2 source file(s)

  lines      ████████████████░░░░  81.5%  66/81
  branches   ███████████████░░░░░  76.3%  29/38
  functions  ██████████████████░░  87.5%  7/8

Branches your tests never took (8):
  src/brake_ecu.c:75    fall through never taken   (line ran in: crc-error, emergency-flag, pedal-press)
  src/brake_ecu.c:79    fall through never taken   (line ran in: crc-error, emergency-flag, pedal-press)
  src/brake_ecu.c:86    fall through never taken   (line ran in: crc-error, emergency-flag, pedal-press)
  src/brake_ecu.c:93    fall through never taken   (line ran in: crc-error)
  src/brake_ecu.c:93    never reached   (line ran in: crc-error)
  src/brake_ecu.c:105   fall through never taken   (line ran in: crc-error)
  src/brake_ecu.c:114   fall through never taken   (line ran in: crc-error)
  src/brake_ecu.c:137   jump never taken   (line ran in: crc-error, emergency-flag, pedal-press)
FAIL: branch coverage 76.3% is below --fail-under-branches 90
```

Line 75 is `if (delta == 0)`: no test ever sent a repeated alive counter.
Add `tests/e2e-sequence.yaml`, `tests/pedal-implausible.yaml` and
`tests/line-noise.yaml` and the merge reports 38/38 branches.

---

## Reading the branch list

LabWired measures branches on the machine code, so the two outcomes are the
two things the CPU can do at a conditional branch instruction:

| Kind | Meaning |
|---|---|
| `jump never taken` | The branch ran, but the condition was never true. |
| `fall through never taken` | The branch ran, but execution always jumped. |
| `never reached` | No test executed this branch at all. |

Compilers often jump *over* the body of an `if`. Then "fall through never
taken" on an `if` line means **the body never ran**. Which direction maps to
which source outcome depends on the compiler; the line number and the code
there tell you which.

In LCOV, branch `0` is the jump and branch `1` is the fall-through. A branch
that was never reached is written `BRDA:<line>,<block>,<branch>,-`, as the
LCOV format specifies.

---

## Merging

`labwired coverage merge` accepts files and directories (searched for
`*.info`). It follows `lcov -a`: counts add up; a branch stays "never
evaluated" only if no input evaluated it. Its counts equal `lcov -a`'s once
lcov's per-test records are added up (lcov keeps one record per test name;
the merge keeps one per file and remembers which tests ran each line).

Merging works on **any** LCOV file, so you can combine simulator runs with
the `gcov`/`gcovr`/`llvm-cov` coverage of your host-side unit tests:

```bash
labwired coverage merge out/ host-unit-tests.info -o all.info
```

A directory scan skips earlier merge results (and the file being written
with `-o`), so re-running the same command does not count runs twice.

| Option | |
|---|---|
| `-o, --output PATH` | Merged LCOV. |
| `--cobertura PATH` | Cobertura XML (validates against `coverage-04.dtd`). |
| `--source-root DIR` | Cobertura `<source>`; repeat for several. Default `.`. |
| `--json-out PATH` | Summary, per-file numbers, and every uncovered branch with the tests that ran its line. |
| `--fail-under-lines PCT` / `--fail-under-branches PCT` | Exit code 1 below the threshold. |

---

## CI: GitLab merge-request coverage

```yaml
firmware-coverage:
  script:
    - for t in tests/*.yaml; do labwired test --script "$t" --coverage --output-dir "out/$(basename "$t" .yaml)"; done
    - labwired coverage merge out -o coverage.info --cobertura coverage.xml --fail-under-branches 90
  coverage: '/branches\s+\S+\s+(\d+\.\d+)%/'
  artifacts:
    reports:
      coverage_report:
        coverage_format: cobertura
        path: coverage.xml
```

Source paths in the reports are the paths the compiler recorded (for
example `src/brake_ecu.c`, relative to the build directory). Run the build
from the repository root, or pass `--source-root` so GitLab can find them.

---

## How it works

1. **Count.** During the run, an observer counts how many times each
   instruction executed and how many times execution went from instruction
   A straight to instruction B. It needs no knowledge of the instruction
   set.
2. **List every branch.** After the run, LabWired decodes the ELF and lists
   every conditional branch with its target and fall-through address: Thumb
   `B<cond>` (16 and 32 bit), `CBZ`, `CBNZ`; RISC-V `BEQ BNE BLT BGE BLTU
   BGEU C.BEQZ C.BNEZ`. It walks every function symbol, every code region
   the ELF's mapping symbols mark (`$t`, `$x`, skipping `$d` literal pools),
   and every DWARF line-table sequence, so hand-written assembly with line
   info is included.
3. **Join.** "Jump" is the count of steps from the branch to its target;
   "fall-through" is the count of steps to the next instruction. DWARF maps
   each address to a file and line.

The branch decoder was checked against `llvm-objdump` on all 81 committed
firmware ELFs with line info (bare-metal C, Rust, Arduino cores, Zephyr,
ESP-IDF): it found all 18,195 two-outcome conditional branches, with no
extras and no wrong targets.

Source paths come from the line table with their directory. The build
directory most of the line table comes from is the root: its files stay
relative (`src/brake_ecu.c`); files of other units (a Cargo registry crate,
a vendor SDK) keep their absolute path, so two `src/lib.rs` never merge.

---

## Limits

- **Branch coverage is object-code coverage.** It is not MC/DC. A source
  decision the compiler turns into predicated code (a Thumb `IT` block, a
  conditional select) has no branch instruction, so it is not counted.
  Build with `-O0` or `-Og` when branch numbers matter; the same advice
  applies to every object-code tool.
- **Interrupts.** If an interrupt arrives right after a branch, that one
  outcome is recorded as a step into the handler and is not counted. The
  error is always towards *less* coverage, never more.
- **Xtensa, AVR and AArch64**: line and function coverage only for now. The
  report's `branch_isa` is `null` and no branch numbers are printed, rather
  than wrong ones.
- **Line hits** are the highest execution count among a line's addresses.
- **ARM ELFs stripped of mapping symbols** (`$t`/`$d`): literal pools inside
  functions can no longer be told from code, and a pool word can decode as
  a branch. GCC and clang emit them; run coverage on the unstripped ELF.
- **Rewinding:** `Session::restore` gives exactly a straight run's counts.
  The browser playground keeps accumulating across a rewind on purpose, so
  you see everything you explored.
- **Not a qualified tool.** LabWired has no ISO 26262 / DO-178C tool
  qualification kit. Use it to find the tests you are missing, early and in
  every CI run.
