# Plan item #4 — Ecosystem adoption hooks: Robot Framework + Cucumber/BDD front-ends

INTERNAL design doc. No code in this pass. Lives outside the repo.
Grounded on the clean `origin/main` checkout (tip `a876d471`), read read-only at
`scratchpad/wt-main`. All file:line citations are against that checkout.

## 0. Correction vs the stale-branch draft

A prior draft read a branch 314 commits behind `main` and wrongly concluded the
CI composite action did not exist, so it planned a greenfield action. On THIS
`main` the action **exists** and is the real integration anchor:

- `.github/actions/labwired-test/action.yml` — composite action `name: "LabWired
  test"`, `using: composite`, three steps: (1) "Install LabWired" downloads the
  matching release tarball per `uname -s`/`uname -m`
  (`labwired-${ver}-${plat}.tar.gz`, platforms linux-x86_64 / linux-aarch64 /
  darwin-x86_64 / darwin-aarch64), `chmod +x`, prepends to `$GITHUB_PATH`;
  (2) "Run LabWired test" runs
  `labwired test --script … --junit … --output-dir … ${args}`; (3) "Upload
  artifacts" via `actions/upload-artifact@v4` (`always()`, gated on
  `upload-artifacts`).
- Inputs (action.yml): `script` (required), `version` (default `latest`),
  `repo` (default `w1ne/labwired`), `args`, `junit` (default
  `labwired-junit.xml`), `output-dir` (default `labwired-artifacts`),
  `upload-artifacts` (default `true`), `github-token` (default
  `${{ github.token }}`).
- `.github/actions/labwired-test/README.md` documents usage
  (`w1ne/labwired/.github/actions/labwired-test@main`) and the same input list.

The whole CI section below **extends this real action**; it does not invent one.

## 1. The single contract everything binds to (the CLI)

Both front-ends and CI converge on one process contract: `labwired test`. There
is exactly one place that implements assertion semantics, and it is the CLI; the
front-ends must NOT re-implement it.

### 1.1 CLI flags (`crates/cli/src/main.rs:527-592`, `struct TestArgs`)
- `-f/--firmware <PATH>` (`:529`), `-s/--system <PATH>` (`:533`),
  `-c/--script <PATH>` (`:537`, required).
- `--max-steps`, `--max-cycles`, `--max-uart-bytes`, `--max-vcd-bytes`,
  `--detect-stuck` (alias `--no-progress`) — all override the script's `limits`
  (`:540-574`; override precedence resolved at `commands/test.rs:109-121`).
- `--output-dir <DIR>` (`:553`) → writes `result.json`, `snapshot.json`,
  `uart.log`, `junit.xml`. `--junit <PATH>` (`:557`) → standalone JUnit copy.
- `--breakpoint`, `--no-uart-stdout`, `--trace`, `--vcd`, `--trace-max`,
  `--no-key` (`:544-591`).

### 1.2 Exit codes (`crates/cli/src/main.rs:33-36`)
- `EXIT_PASS = 0`, `EXIT_ASSERT_FAIL = 1`, `EXIT_CONFIG_ERROR = 2`,
  `EXIT_RUNTIME_ERROR = 3`. Aggregated at `:1865-1869`. Config errors (bad
  script, missing firmware, unsupported arch, oversized `max_steps`) return 2
  from `run_test` (`crates/cli/src/commands/test.rs:40,65,139,163,378,465`).

### 1.3 Output artifacts (`crates/cli/src/main.rs`, `write_outputs` `:1874`)
- `result.json` (`:1921`): `struct TestResult` (`:594-611`) —
  `result_schema_version`, `status`, `steps_executed`, `cycles`, `instructions`,
  `stop_reason`, `stop_reason_details`, `limits`, `message`, `assertions`
  (`Vec<AssertionResult>` = `{assertion, passed}`, `:626-630`), `cpu_state`,
  `firmware_hash`, `config`.
- `snapshot.json` (`:1946`), `uart.log` (`:1972`), `junit.xml` (`:1979`).
- JUnit (`write_junit_xml` `:2187`): **one `<testcase>` per assertion** plus a
  synthetic `name="run"` testcase for non-assertion / stop-condition / runtime
  failures (`:2280-2333`; per-assertion loop `:2313-2332`; `name` from
  `assertion_short_name` `:2360`). `<testsuite name="labwired">` carries
  `tests/failures/errors` counts and `firmware_hash` as a property.
- Config-error path still writes the same artifacts via
  `write_config_error_outputs` (`:2024`), so a parse failure is still a readable
  JUnit + result.json, not a bare crash.

### 1.4 Test-script v1.0 YAML schema (`crates/config/src/lib.rs`)
This is what keywords/steps generate. The schema is **frozen at "1.0"**
(`validate()` rejects anything else, `:671-688`) and every assertion/limit
struct is `#[serde(deny_unknown_fields)]`.

- `TestScript` (`:651-659`): `schema_version` (must be "1.0"), `inputs`,
  `limits`, `assertions[]`.
- `TestInputs` (`:544-549`): `firmware` (required, non-empty), `system` (opt).
- `TestLimits` (`:551-565`): `max_steps` (required, > 0), `max_cycles`,
  `max_uart_bytes`, `no_progress_steps`, `wall_time_ms`, `max_vcd_bytes`.
- `TestAssertion` (untagged enum, `:641-649`) — the **complete** assertable set:
  1. `uart_contains: <str>` (`:586`)
  2. `uart_regex: <str>` (`:592`; matched by the in-CLI `simple_regex_is_match`,
     `main.rs:~2408` — `^ $ . *` only, NOT full PCRE — front-ends must document
     this, see risks)
  3. `expected_stop_reason: <StopReason>` (`:598`); `StopReason` snake_case
     variants `:567-582`: `config_error, max_steps, max_cycles, max_uart_bytes,
     max_vcd_bytes, no_progress, wall_time, memory_violation, decode_error,
     halt, exception`.
  4. `memory_value: {address, expected_value, mask?, size?}` (`:602-619`;
     `size` accepts 1/2/4 bytes or 8/16/32 bits, default u32).
  5. `uds_tester: {id, result: done}` (`:628-639`) — niche; expose as an
     advanced keyword/step, not a headline one.
- A deprecated legacy v1 flat format also loads (`:711-735`); front-ends emit
  v1.0 only.

### 1.5 Programmatic entrypoint — verified low-level, do NOT bind to it
`crates/python/src/lib.rs` is a pyo3 module named `labwired` exposing class
`Machine` with `new(firmware, system_config)`, `step(max_steps)`,
`read_register`/`write_register`, `read_memory`/`write_memory`,
`snapshot`/`restore`, `get_pc` (`:81-241`). Its `PyStopReason` only knows
`breakpoint/step_done/max_steps_reached/manual_stop` (`:22-43`) — a DIFFERENT,
smaller enum than the test `StopReason` in §1.4. It has **no** script loader, no
assertion evaluation, no JUnit/result.json, no limits beyond `max_steps`, no
UART capture sink.

Decision: front-ends MUST drive the `labwired test` CLI, not the pyo3 module.
Binding to pyo3 would force re-implementing `uart_contains`, `uart_regex`
(including the custom matcher), `memory_value` masking/sizing, stop-reason
mapping, and JUnit emission in Python — guaranteeing drift from the Rust source
of truth. The pyo3 module stays the escape hatch for raw step/poke loops,
explicitly out of scope for these two front-ends.

## 2. Shared foundation: one Python helper, two thin front-ends

Both Robot and behave are Python; both reduce to "compose a v1.0 script + invoke
the CLI + parse result.json". Put that once in a shared library; the two
packages are thin keyword/step shims over it.

New package `labwired-test-helper` (Python, `labwired_runner/`):
- `ScriptBuilder` — accumulates `inputs`, `limits`, `assertions[]` and serializes
  v1.0 YAML matching §1.4 (incl. `deny_unknown_fields` discipline: never emit a
  key the Rust structs reject). One builder = one `labwired test` run.
- `find_labwired()` — locates the binary: `$LABWIRED_BIN` → `labwired` on PATH →
  (dev) cargo target dir. Same resolution the CI action relies on (binary on
  PATH).
- `run(builder, output_dir, junit=None, extra_args=[])` — writes the script to a
  temp/declared path, runs `labwired test -c … --output-dir … [--junit …]`,
  captures stdout/stderr, returns a `RunResult` parsed from
  `<output_dir>/result.json` (§1.3) carrying `exit_code`, `status`,
  `stop_reason`, `stop_reason_details`, `assertions[]`, `cycles`, `uart` (read
  from `uart.log`), and raw paths.
- Assertion outcome comes from result.json's per-assertion `{assertion, passed}`
  and the process exit code — the helper does NO matching itself (no second
  regex engine, no second masking). This is the anti-drift seam.
- Limit-override pass-through maps builder limits to the CLI flags in §1.1 so a
  front-end can override per-run without rewriting the script.

Both front-ends depend on this one helper. The CLI contract is touched in
exactly one Python file.

## 3. Robot Framework front-end — `robotframework-labwired`

### 3.1 Architecture
A Robot library (`LabWired/__init__.py`) in Python, `library scope = TEST CASE`
(one machine/run context per test). Thin: every keyword mutates a per-instance
`ScriptBuilder` then, at run time, calls the shared helper (§2). The library
holds no simulation logic.

Two authoring modes:
- **Path mode**: `Flash Firmware <fw.elf>` + `Use System <sys.yaml>` reference
  existing artifacts (maps to `inputs.firmware`/`inputs.system`).
- **Inline mode**: the same keywords accept ELF/manifest paths produced earlier
  in the suite (e.g. a build step). No new manifest authoring surface — manifests
  are the existing chip/system YAML.

### 3.2 Keyword surface → assertions/limits (exhaustive map to §1.4)
Setup / inputs:
- `Flash Firmware  ${elf}` → `inputs.firmware`.
- `Use System  ${yaml}` → `inputs.system`.
Limits (all of `TestLimits`):
- `Set Max Steps  ${n}` → `limits.max_steps` (required; library errors if a Run
  keyword is reached without it, mirroring `validate()` `:683`).
- `Set Max Cycles`, `Set Max Uart Bytes`, `Set No Progress Steps`,
  `Set Wall Time (ms)`, `Set Max Vcd Bytes` → matching `limits.*`.
- `Run For  ${max_steps}` — convenience: sets `max_steps` and immediately runs.
Run trigger:
- `Run Firmware` / `Run Firmware And Continue` — invokes helper; the variant
  decides whether a non-zero exit fails the keyword immediately or defers to
  explicit assertion keywords.
Assertions (each appends to `assertions[]`; the CLI evaluates, the keyword reads
the per-assertion `passed` back from result.json and fails the Robot keyword if
false):
- `UART Should Contain  ${text}` → `uart_contains`.
- `UART Should Match  ${regex}` → `uart_regex` (docstring warns: `^ $ . *` only,
  §1.4).
- `Stop Reason Should Be  ${reason}` → `expected_stop_reason` (enum-validated
  client-side against the §1.4 list to give a clear error before the run).
- `Memory At  ${addr}  Should Equal  ${value}` (optional `mask=`, `size=`) →
  `memory_value`.
- `UDS Tester  ${id}  Should Be Done` → `uds_tester` (advanced).

### 3.3 Result flow → Robot reporting
- A failing per-assertion `passed:false` raises a Robot `AssertionError` with the
  assertion's `assertion_short_name` text + `stop_reason_details` from
  result.json → shows in Robot's own `log.html`/`report.html`.
- The library also forwards the CLI's `--junit` JUnit (§1.3) as an attached
  artifact, so a CI consumer gets the per-assertion JUnit AND Robot's native
  `output.xml`. Robot's xUnit (`robot --xunit`) and LabWired's JUnit stay
  separate, non-conflicting reports.
- Exit-code semantics: a config error (CLI exit 2) raises a distinct
  `LabWiredConfigError` (fatal, fails suite setup), separating "your script is
  wrong" from "your assertion failed" (exit 1) — matching §1.2.

### 3.4 Packaging
- PyPI `robotframework-labwired`, dep on `robotframework` +
  `labwired-test-helper`. Does NOT bundle the binary; resolves via the helper.
  README documents both "binary on PATH (the CI install action already does
  this)" and `$LABWIRED_BIN`.

### 3.5 Example suite
`examples/robot/boot_smoke.robot`, modeled on
`examples/tests/stm32f103_integrated_test.yaml`:
```
*** Settings ***
Library    LabWired
*** Test Cases ***
F103 Boots And Idles
    Flash Firmware    ${FW}
    Use System        ${SYS}
    Set Max Steps     10000
    Set No Progress Steps    1000
    Run Firmware
    Stop Reason Should Be    no_progress
    UART Should Contain      READY
```

## 4. Cucumber/BDD front-end — recommend `behave` → `behave-labwired`

### 4.1 Why behave (not a JVM/JS Cucumber)
- The proven runner contract is a CLI + result.json; the shared helper (§2) is
  Python. behave is the Python-native Gherkin runner, so it reuses
  `labwired-test-helper` with zero second-language bridge — same anti-drift seam
  as Robot.
- The existing tooling surface is Python (the pyo3 crate `crates/python`,
  `pyproject.toml`, `verify.sh`). A JS/JVM Cucumber would add a Node/JVM
  toolchain to CI purely to shell out to the same binary — more moving parts, no
  capability gain.
- Gherkin `.feature` syntax is identical across Cucumber implementations, so
  authors keep standard Given/When/Then; only the step-definition language is
  Python. Buyers wanting JS/JVM Cucumber can still call `labwired test` directly;
  documented as the escape hatch.

### 4.2 Step surface → assertions/limits (same map as §3.2)
Ship a step-definition library (`behave_labwired/steps.py`) users import via one
line in their `steps/labwired.py`. Steps populate a `ScriptBuilder` in behave's
`context`; the `When` step runs the helper.
- `Given the firmware "<elf>"` → `inputs.firmware`.
- `And the system "<yaml>"` → `inputs.system`.
- `And a step limit of <n>` → `limits.max_steps` (required).
- `And a cycle limit of <n>` / `a UART byte limit of <n>` /
  `<n> no-progress steps` / `a wall-time limit of <ms> ms` → matching limits.
- `When I run the firmware` → helper run; stores `RunResult` in context.
- `Then the UART output contains "<text>"` → `uart_contains`.
- `Then the UART output matches "<regex>"` → `uart_regex` (`^ $ . *`).
- `Then the stop reason is <reason>` → `expected_stop_reason`.
- `Then memory at <addr> equals <value>` (table form for mask/size) →
  `memory_value`.
- `Then UDS tester "<id>" is done` → `uds_tester`.

Two run shapes: assert-as-you-go (each `Then` re-reads result.json's
per-assertion `passed`) and run-once-assert-many (all assertions appended before
the `When`, then `Then` steps read back). Both produce ONE `labwired test`
invocation; the helper guarantees that.

### 4.3 Result flow → behave reporting
- Failing `passed:false` raises `AssertionError` in the step → behave marks the
  scenario failed, prints assertion text + `stop_reason_details`.
- Provide a behave formatter / hook that also emits JUnit (`behave --junit`) AND
  attaches LabWired's own `--junit` per-assertion XML — same dual-report approach
  as Robot (§3.3).
- CLI exit 2 → fatal scenario error (config), exit 1 → normal scenario failure
  (assertion), matching §1.2.

### 4.4 Packaging
- PyPI `behave-labwired`, dep on `behave` + `labwired-test-helper`. Path and
  inline (build-then-run) modes both via the firmware/system path steps.

### 4.5 Example feature
`examples/behave/features/boot_smoke.feature`:
```
Feature: F103 boot smoke
  Scenario: It boots and idles
    Given the firmware "build/firmware.elf"
    And the system "configs/systems/stm32f103-integrated-test.yaml"
    And a step limit of 10000
    And 1000 no-progress steps
    When I run the firmware
    Then the stop reason is no_progress
    And the UART output contains "READY"
```

## 5. CI wiring — extend the REAL action

The existing `.github/actions/labwired-test/action.yml` installs the binary onto
`$GITHUB_PATH` and runs one script. The front-ends need the binary on PATH +
Python; reuse the install logic, add Python + front-end install + runner
invocation.

Two sibling composite actions (so install logic is not copy-pasted):
- `.github/actions/labwired-install/action.yml` — **refactor**: extract the
  current step-1 "Install LabWired" block (the `uname`-based platform switch +
  `gh release download` + `chmod` + `$GITHUB_PATH`) into its own composite
  action. Then `labwired-test/action.yml` calls `labwired-install` for step 1,
  preserving its public interface (`version`/`repo`/`github-token` unchanged).
  This is the only edit to the existing action and it is behavior-preserving.
- `.github/actions/labwired-robot/action.yml` — `using: composite`: (1) uses
  `labwired-install`; (2) `actions/setup-python`; (3) `pip install
  robotframework-labwired`; (4) `robot --xunit robot-xunit.xml <suite>`;
  (5) upload `output.xml` + LabWired artifacts. Inputs mirror the test action
  plus `suite`.
- `.github/actions/labwired-behave/action.yml` — same shape with `pip install
  behave-labwired` + `behave --junit` + artifact upload. Inputs plus
  `features-dir`.

Keep `.github/actions/labwired-test/README.md` canonical and add the two
front-end actions to it.

## 6. Golden parity guardrail (front-ends + raw CLI cannot diverge)

Risk: three surfaces (raw CLI YAML, Robot keywords, behave steps) describing the
same run could drift. Guardrail = one fixture proven equivalent three ways.

- Golden fixture set: a small ELF + system YAML + a v1.0 script exercising EVERY
  assertion kind in §1.4 (uart_contains, uart_regex, expected_stop_reason,
  memory_value, uds_tester) with known pass/fail expectations.
- Parity test (CI job) runs the SAME fixture three ways: (a) `labwired test -c
  golden.yaml`, (b) the equivalent Robot suite, (c) the equivalent behave
  feature. Asserts: identical exit code (§1.2), identical set of per-assertion
  `passed` booleans, and **byte-identical generated script YAML** from both
  front-ends vs the hand-written golden (`ScriptBuilder` output diffed against
  `golden.yaml`). Catches any keyword/step re-implementing matching or emitting a
  divergent key.
- Schema-fence test: assert the front-ends emit ONLY v1.0 keys; if
  `crates/config/src/lib.rs` adds an assertion/limit, the parity fixture fails
  until both front-ends + the golden map it — making the schema the single
  forcing function (ties into the existing rename/consistency discipline).

## 7. Phases

1. **P0 — shared helper.** `labwired-test-helper` (`ScriptBuilder` + `run` +
   `find_labwired` + `RunResult`). Unit-test YAML output against §1.4 (round-trip
   via a known-good script). No front-end yet.
2. **P1 — Robot.** `robotframework-labwired` keywords (§3.2) over the helper +
   example suite + library acceptance tests against a checked-in tiny ELF.
3. **P2 — behave.** `behave-labwired` steps (§4.2) + example feature, sharing the
   helper.
4. **P3 — golden parity guardrail** (§6) wired as a CI job.
5. **P4 — CI actions.** Refactor `labwired-install` out of the existing action
   (behavior-preserving), add `labwired-robot` + `labwired-behave` siblings,
   update README. Dog-food: run the example suites through the new actions.
6. **P5 — publish** both packages to PyPI; document install + `$LABWIRED_BIN` /
   "binary already on PATH from the install action".

## 8. New packages / files

- `labwired-test-helper` (PyPI) — shared runner + `ScriptBuilder`.
- `robotframework-labwired` (PyPI) — Robot library.
- `behave-labwired` (PyPI) — behave step library.
- `.github/actions/labwired-install/action.yml` (+ refactor of the existing test
  action to call it).
- `.github/actions/labwired-robot/action.yml`,
  `.github/actions/labwired-behave/action.yml`.
- `examples/robot/…`, `examples/behave/features/…`, golden parity fixture + CI
  job.

## 9. Risks

- **Regex mismatch.** `uart_regex` uses the CLI's custom `simple_regex_is_match`
  (`^ $ . *` only, `main.rs:~2408`), not Python `re`. Robot/behave users will
  assume PCRE. Mitigate: keyword/step docstrings state the supported subset; the
  golden fixture includes a regex case; do NOT pre-validate regexes in Python
  (a second engine — exactly the drift we forbid).
- **Stop-reason enum skew.** The test `StopReason` (§1.4, 11 variants) differs
  from the pyo3 `PyStopReason` (4 variants). Front-ends bind to the test enum via
  the CLI only; never import the pyo3 one. Client-side enum validation lists must
  be checked against `crates/config/src/lib.rs:567-582`, not hand-kept.
- **deny_unknown_fields brittleness.** Any extra key the `ScriptBuilder` emits
  fails the run at parse time (config error, exit 2). The golden parity byte-diff
  (§6) is the defense; keep the builder minimal.
- **Binary discovery in CI.** Front-end actions depend on `labwired-install`
  having run first (PATH). The composite actions enforce ordering by calling
  install internally.
- **Schema freeze.** v1.0 is frozen (`validate()` rejects non-"1.0"). If a v1.1
  lands, all three surfaces + golden must bump together — covered by the
  schema-fence test (§6).
