# Silent-path census

Measured on branch `chore/silent-path-census`, whose merge base is
`d503501d323fe25fb9dc2dd87481806e37beeb67` (`origin/main`); host
`aarch64-apple-darwin`, `rustc 1.97.1`.

Every number here is from a **full re-measurement** after counter (b) was corrected
(see below). Counters (a), (c) and (b1) reproduced the first run exactly, entry for
entry, which is what establishes that the (b2) figures are a correction rather than
run-to-run drift.

This document is a **measurement**. Nothing is fixed, escalated, or gated here. It
exists to scope three follow-up repairs that could not be sized without data.

> ## Correction (counter b)
>
> The first version of this document reported `nvic` on 22 labs and `scb` on 3 as
> live stubs, and called them the only entries that were "not intentional". **That
> was wrong.** Both are replaced by real models before a single instruction
> executes, and the conclusion was published without being checked.
>
> The cause was measuring at the wrong moment: counter (b) fired inside
> `bus/from_config.rs`, which is **construction time but not the end of
> construction**. `system::cortex_m::configure_cortex_m` runs afterwards on every
> ARM path — `cli/commands/test.rs`, `wasm/lib.rs`, `run.rs`, `machine.rs`,
> `debug_probe.rs`, `dap/adapter.rs`, `python/lib.rs`, `system/node.rs` — finds the
> entry matching `name == "nvic" || base == 0xE000_E100` and swaps in a real
> `Nvic`, does the same for `Scb` at `0xE000_ED00` and `Dwt` at `0xE000_1000`, and
> rewrites their names while doing it. A factory-time counter cannot see any of
> that. It was counting what the factory emitted, not what the machine ran.
>
> Counter (b) is now two clearly separated numbers: **(b1)** how often the factory
> fallthrough was taken, and **(b2)** what is *still* a `StubPeripheral` on a fully
> assembled machine. **(b2) is the actionable one**; (b1) is retained because "how
> often did the `_other =>` arm fire" is a legitimate, separate statistic about the
> factory. They are not interchangeable and must never be read off one another.
>
> A guard that proves the real `Nvic`/`Scb` are installed is
> [#877](https://github.com/w1ne/labwired-core/pull/877). The corrected mechanism
> also has a gate in this branch:
> `crates/core/tests/census_probe.rs::nvic_and_scb_trip_the_factory_counter_but_are_not_live_stubs`,
> which runs the real `from_config` on a committed chip file, asserts NVIC/SCB *are*
> stubs straight out of the factory, then asserts they are *not* after
> `configure_cortex_m`.
>
> **Generalising the lesson:** a counter placed inside a construction step measures
> that step, not construction. Before trusting one, find every pass that mutates
> the same data afterwards. The audit of those passes is in
> *[Other post-factory mutation passes](#other-post-factory-mutation-passes)*.
>
> Counters (a) and (c) instrument runtime *access*, not construction, so they should
> be unaffected. That was **checked, not assumed** — see
> *[(a) and (c) are unchanged](#a-and-c-are-unchanged)*.

## What was counted, and how it is gated

All counters live behind the `silent-census` Cargo feature
(`crates/core/src/census.rs`). It is **off by default**, is not implied by any other
feature, and even when compiled in it writes nothing unless `LABWIRED_CENSUS_OUT`
names an output path at runtime. Both the compile-time feature and the env var are
required, so it cannot be turned on by accident.

```
cargo build -p labwired-cli --features silent-census
LABWIRED_CENSUS_OUT=census.json ./target/debug/labwired test --script <lab>.yaml ...
```

| # | Path | Instrumented at | Sites |
|---|------|-----------------|-------|
| a | Dropped Cortex-M memory errors | `cpu/cortex_m.rs` - every `let _ = bus.write_*` and every discarded `bus.read_*`, wrapped in `census_bus!` | 64 (25 write, 39 read) |
| b1 | Stub-peripheral **factory fallthrough** (construction statistic) | `bus/from_config.rs` - the `_other =>` arm ending the factory chain | 1 |
| b2 | **Live** stub peripherals, **post-construction** (the actionable number) | `Machine::new` - a sweep of `bus.peripherals` after the finished bus is handed over | 1 |
| c | Undecoded register access | `peripherals/**` - catch-all `_ => {}` / `_ => 0` decode arms, via `census_reg!` | 312 |
| c'| Undecoded register access, declarative models | `peripherals/declarative.rs` - the `reg_index_at` miss fallthrough | 2 |

### How (b2) decides that something is a stub

Not by name. `PeripheralEntry::name` is rewritten by the very replacement passes
that make (b1) misleading, so a name records what the manifest asked for, never
what the machine ended up holding.

The sweep uses the type system: `Peripheral::as_any()` followed by
`dyn Any::is::<StubPeripheral>()`. That is a `TypeId` comparison against the
concrete type — it cannot false-positive on some other model, and it cannot
false-negative on a real stub. Name and base address are still *reported*,
because they are how a human finds the entry again; they are output, not the
predicate.

The one production-type change this needs is a `Peripheral::as_any` override on
`StubPeripheral`, and it is itself `#[cfg(feature = "silent-census")]`: with the
feature off the type is byte-for-byte what it was. With the feature on it returns
`Some(self)` where it previously returned `None`, and every existing consumer of
`Peripheral::as_any` in the tree immediately `downcast_ref`s to a concrete model
type that a stub is not — so `Some(stub)` and `None` are indistinguishable to all
of them.

`Machine::new` is the sweep point because it is the single choke that every
runner (CLI lab runner, multi-node environment runner, wasm, DAP, python,
`system::node`) passes through holding a *finished* bus. It already walks
`bus.peripherals` for exactly this kind of `TypeId` lookup. The census publishes
`machines_swept` alongside the table so multi-node runs and any double
construction are visible rather than silently inflating the count.

### Why c' exists, and what the audit's arm count really contains

The audit sized (c) as "~204 `_ => {}` and ~173 `_ => 0` arms". A grep at this SHA
finds 201 and 176 (377 total). Classifying each by the *subject of its enclosing
`match`* shows they are not all register decodes:

| Arm population | Count |
|---|---|
| Total `_ => {}` / `_ => 0,` arms | 377 |
| ... in `#[cfg(test)]` / `mod tests` code | 19 |
| ... matching on a register offset (`offset`, `reg`, `reg_off`, `word_off`, ...) - **instrumented** | **312** |
| ... matching on something else entirely (`cmd`, `dest`, `src`, `self.state`, `self.pointer`, `upper.as_str()`, ...) - **not instrumented, not a register decode** | 46 |

Instrumenting the last group would have produced numbers that look like register
gaps but are not. They are excluded deliberately.

Separately, the `_ =>` grep **structurally cannot see** the declarative/SVD-driven
models (`GenericPeripheral`), whose decode is
`if let Some(idx) = self.reg_index_at(offset) { ... }` with a bare `Ok(0)` / `Ok(())`
fallthrough - the same silent path in a different shape. 138 of the 1,186 peripheral
instances across the runnable corpus are declarative, so omitting them would have made
a near-zero (c) result misleading. They are counted and reported separately as
`shape: declarative_miss`.

### Read the raw counts with a 4x byte multiplier

`Peripheral::read`/`write` are **byte**-granular; `read_u32`/`write_u32` decompose into
four byte accesses. Several models (RCC among them) additionally do a read-modify-write
per byte. So **one** 32-bit write to an undecoded register costs 4 `write` hits *and* 4
`read` hits. This is pinned by a test
(`crates/core/tests/census_probe.rs::raw_counts_carry_a_four_times_byte_multiplier`).
Divide raw (c) counts by 4 to get register-level accesses.

## Coverage

| | Count |
|---|---|
| Test scripts discovered under `examples/` (any YAML with an `assertions:` block) | **97** |
| Ran | **68** |
| Skipped - firmware artifact absent | **27** |
| Ran but produced no census file | **2** |

Of the 68 runs: **25 clean** (counters a, b2 and c all zero), **43 hot**.
The 68 runs built **70 machines** — `ci-multiarch/two-riscv-test` and
`ci/two-node-inputs-env` are two-node environments and are swept twice, once per
node. Every other lab reports `machines_swept: 1`.

Assertion outcome 58 pass / 10 fail; the failures are pre-existing at this
SHA and are not caused by the census - see *Behavioural neutrality*, which proves
byte-identical output on failing labs as well as passing ones.

> Under the old, factory-time (b) the split was 19 clean / 49 hot. Six labs
> (`feather-f405` x2, `nucleo-f401re` x2, `nucleo-f767zi` x2) were counted as hot
> **solely** because their manifest declares `type: "nvic"`. They have no live
> stub, no dropped memory error and no undecoded register access, and are now
> where they belong: clean.

### On the brief's denominator

The brief describes the corpus as "89 projects and 82 committed `.elf` files" in
`examples/`. The 89 directories are right; the 82 ELFs are **repo-wide**. Only **8**
committed ELFs are under `examples/` - 67 are under `tests/fixtures/`. The runnable
unit is a *test script*, not a project: the 89 directories yield 97 scripts, and most
firmware comes from cross-compiling workspace members (33 of 38 ARM build units built
cleanly here; the 5 failures are 3 nested workspaces that do not build on this
toolchain and 2 crate names that are `[[bin]]` targets inside other packages, both of
which were then built via their real package).

## Aggregate: counter (a) - dropped Cortex-M memory errors

**2 hits, 2 distinct (pc, addr, kind), in 1 of 68 runs.**

| count | pc | addr | kind |
|---|---|---|---|
| 1 | `0x00000000` | `0x00000001` | read |
| 1 | `0x00000000` | `0x00000004` | read |

Both hits are in `examples/ci/dummy-memory-violation.yaml` - a fixture whose *purpose*
is to provoke a memory violation. **No shipped lab drops a single Cortex-M memory error.**

Cross-checked against the independent, always-on `fidelity::unmapped_mmio` log, which
records the same bus rejections one layer lower (per byte, at the point the error is
*created* rather than dropped):

| lab | fidelity `unmapped_mmio` (bytes rejected) | census (a) (errors dropped) |
|---|---|---|
| `ci/dummy-memory-violation` | 7 | 2 |
| `nucleo-l073rz/io-smoke` | 1 | 0 |
| `pico2/io-smoke` | 1 | 0 |
| `pico2/uart-smoke` | 1 | 0 |
| **total** | **10** | **2** |

The gap is the point: 8 of the 10 rejections were **propagated**, not dropped - they
hit the instruction-fetch path (`bus.read_u16(fetch_pc)?`, one of the 9 places
`cortex_m.rs` does use `?`), which already faults correctly. Counter (a) is measuring
the drop sites specifically, and they are cold.

## Aggregate: counter (b2) - live stub peripherals (the actionable number)

**147 live stubs, 34 distinct (name, base), in 41 of 68 runs, across 70 machines.**

This is what is still a `StubPeripheral` when the machine actually runs. Every one
of the 147 was traced back to the manifest entry that produced it, and **every one
is a stub the manifest or the factory deliberately asked for**:

| count | asked for by | reading |
|---|---|---|
| 120 | manifest `type: "stub"` | **Declared.** The manifest literally says `type: stub`; the factory is doing what it was asked. |
| 18 | manifest `type: "nrf54l_*_stub"` (7 strings) | **Declared by naming convention.** `dppic` x4, `wdt` x4, `ficr`/`regulators`/`rramc`/`tampc`/`uicr` x2 each. Intentional, but they still reach the factory's `_other =>` arm by *failing to match*, so the factory cannot tell them from a typo. |
| 7 | manifest `type: "icache"` | **Declared, and explicitly handled.** `peripherals/generic_factory.rs` maps `"icache" \| "dcache"` to a read-as-zero stub on purpose: Zephyr's SoC init writes `ICACHE_CR.EN` and never polls a completion flag, and the simulator has flat memory, so there is no cache behaviour to model. |
| 2 | manifest `type: "syscfg"` | **Declared, and explicitly handled.** Same file: a read-0 stub with `CCCSR` @ 0x20 seeded to `0x0000_0100` so the H7 HAL's I/O-compensation-cell READY poll exits. |
| **0** | anything else | **Nothing unaccounted for.** |

**(b2) is benign on the current corpus.** There is no live stub that anybody asked
for by accident: no unmatched manifest `type:` survives to run time, and in
particular `nvic` and `scb` — the two entries the first version of this document
flagged — do not appear here at all, because they are not stubs by the time the
machine exists.

### Aggregate: counter (b1) - factory fallthrough (construction statistic)

**163 instantiations, 10 distinct `type:` strings, in 47 of 68 runs.**

How often `bus/from_config.rs`'s `_other =>` arm was taken. Useful for scoping a
change *to the factory*, and for nothing else — it is not a statement about the
running machine.

| count | `type:` string | still a stub at run time? |
|---|---|---|
| 120 | `stub` | yes - all 120 |
| 22 | `nvic` | **no - all 22 replaced** by a real `Nvic` in `configure_cortex_m` |
| 4 | `nrf54l_dppic_stub` | yes |
| 4 | `nrf54l_wdt_stub` | yes |
| 3 | `scb` | **no - all 3 replaced** by a real `Scb` in `configure_cortex_m` |
| 2 | `nrf54l_ficr_stub` | yes |
| 2 | `nrf54l_regulators_stub` | yes |
| 2 | `nrf54l_rramc_stub` | yes |
| 2 | `nrf54l_tampc_stub` | yes |
| 2 | `nrf54l_uicr_stub` | yes |

### Reconciling (b1) and (b2)

The two numbers differ by 16, and the difference is exactly accounted for:

```
(b1) factory fallthrough                                    163
   - nvic  (22)  replaced by a real Nvic before run time    -22
   - scb   ( 3)  replaced by a real Scb  before run time     -3
   + icache( 7)  an explicit generic_factory arm, so it
                 never reaches `_other =>` and is invisible
                 to (b1) - but it IS a live stub             + 7
   + syscfg( 2)  same                                        + 2
                                                            ----
(b2) live stubs                                             147
```

That the difference runs in **both** directions is the point. (b1) over-reports by
counting entries that were replaced afterwards, and *also* under-reports by missing
stubs the factory installs from a named arm rather than by falling off the end.
Neither number is a proxy for the other.

### Delta against the first published table

| | first version | corrected | change |
|---|---|---|---|
| headline count | 163 "live stubs" | 147 live stubs (b2); 163 factory fallthroughs (b1) | the 163 was never a live-stub count |
| entries called "not intentional" | `nvic` x22, `scb` x3 | **none** | both are real models at run time |
| labs with a live stub | 47 | 41 | 6 labs were hot only because of `nvic` |
| clean labs | 19 | 25 | same 6 labs |
| stub sources not previously visible | - | `icache` x7, `syscfg` x2 | explicit factory arms, missed by a fallthrough-only counter |
| (a) dropped memory errors | 2 | 2 | unchanged |
| (c) undecoded register access | 11 raw / 5 distinct | 11 raw / 5 distinct | unchanged |

**Scoping consequence.** The follow-up repair is unchanged in shape but smaller in
substance, and it is a *factory hygiene* change rather than a fidelity gap:

- Nothing on the current corpus runs with an accidentally stubbed peripheral, so
  there is no chip-model repair to schedule from (b2). No datasheet look is owed.
- Turning `_other =>` into a hard error still breaks 47 of 68 labs, because 138 of
  the 163 fallthroughs are intentional stubbing that is *inferred from falling off
  the end of the match* rather than declared. The fix remains a migration: make the
  factory explicitly match `type: stub` and the `*_stub` convention, then
  hard-error the remainder — which, after that migration, is the empty set on this
  corpus.
- `nvic` and `scb` want a tidy-up for a different reason than the first version
  claimed: they are not a fidelity problem, they are *dead manifest entries*. The
  factory builds a stub that `configure_cortex_m` immediately throws away. Removing
  them from the chip yamls, or giving the factory a real arm for them, would make
  the fallthrough histogram mean what a reader assumes it means.

### Other post-factory mutation passes

`configure_cortex_m` is not the only thing that touches peripherals after the
factory runs. Every pass found in the tree, and whether it can produce the same
false positive:

| pass | what it does | affected? |
|---|---|---|
| `system/cortex_m.rs::configure_cortex_m` | **replaces** the entries at `0xE000_ED00` (SCB), `0xE000_E100` (NVIC), `0xE000_1000` (DWT) with real models, rewriting `name`, `base`, `size` and `irq` | **yes - this is the one that caused the wrong number.** Runs before `Machine::new`, so (b2) sees the result. |
| `cli/commands/esp32_boot_state.rs::install_esp32c3_fast_boot` | `replace_or_add_peripheral` for `systimer`, `rmt`, `usb_serial_jtag`, plus 6 `add_peripheral` installs | **yes, same mechanism** - a chip-yaml entry can be replaced by a behavioural twin. Runs at `test.rs:1352`, before the machine is built at `test.rs:1467`, so (b2) already covers it. |
| `system/xtensa/{mod,esp32,esp32s3}.rs` | ~40 `add_peripheral` calls building the ESP32/S3 bus | no - additive, and these buses are assembled before `Machine::new`. |
| `boot/esp32c3_rom::inject_rom_regions` | injects ROM windows, then `build_rom_boot_machine` | no - additive, and it constructs the machine itself, so the sweep still runs. |
| `system/riscv.rs::configure_riscv` | 14 lines; mandates no peripherals at all | no. There is no RISC-V equivalent of the Cortex-M replacement pass. |
| `bus/tick.rs` (3 sites) and `peripherals/esp32s3/gdma.rs` (2 sites) | temporarily swap `entry.dev` out for a placeholder while lending a model to a bus-aware tick, then swap it back | no - transient, mid-run, and restored. Not construction. |
| `bus/construct.rs::replace_or_add_peripheral` | the generic replacement primitive the passes above use | covered by whoever calls it, all of which run before `Machine::new`. |

The sweep point is therefore after every construction-time pass in the tree today.
It is not immune by *design*: a pass that replaced a peripheral after `Machine::new`
would be missed. `machines_swept` and this table are what a future reader should
re-check before trusting the number again.

### (a) and (c) are unchanged

Counters (a) and (c) instrument runtime *access*, so they should be unaffected by a
construction-time error. That was verified rather than assumed: the corpus was
re-run in full and both counters reproduced **exactly**, entry for entry —

- (a): 2 hits, the same 2 distinct `(pc, addr, kind)` triples, in the same single
  lab, and the same `fidelity::unmapped_mmio` cross-check (7 + 1 + 1 + 1 = 10 bytes
  rejected against 2 errors dropped).
- (c): 11 raw hits, the same 5 distinct `(peripheral, offset, kind)` triples, in the
  same 5 labs, with `declarative_miss` still at zero.
- (b1) also reproduced exactly (163 / 10 distinct / 47 labs), which is what
  establishes that this re-run and the original measured the same corpus under the
  same conditions — so the (b2) delta is the correction and not run-to-run drift.

## Aggregate: counter (c) - undecoded register access

**11 raw hits, 5 distinct (peripheral, offset, kind), in 5 of 68 runs.**

Applying the multiplier per entry rather than in bulk: the two 4-hit entries are one
32-bit write each; the three 1-hit entries are single **byte** writes. So the corpus
performs **5 register-level undecoded accesses in total**, all writes.

Fewer than 20 distinct pairs exist, so this is the complete list, not a top-20:

| count | peripheral | offset | kind | shape |
|---|---|---|---|---|
| 4 | `fdcan:Fdcan` | `0x0010` | write | `match_arm` |
| 4 | `iwdg:Iwdg` | `0x0008` | write | `match_arm` |
| 1 | `nrf54l.twim:Nrf54lTwim` | `0x0508` | write | `match_arm` |
| 1 | `nrf54l.twim:Nrf54lTwim` | `0x050c` | write | `match_arm` |
| 1 | `nrf52.twim:Nrf52Twim` | `0x0510` | write | `match_arm` |

Shape split: `match_arm` 11 hits, `declarative_miss` 0 hits.

**The declarative path recorded zero hits across the entire corpus.** That is a
measured zero from live instrumentation, not an untested assumption - the counter is
proven capable of firing by `census_probe.rs`.

**Scoping consequence.** Three findings are `twim` `0x508`/`0x50c`/`0x510` (nRF52 and
nRF54L), each a single byte write; one is `fdcan` `0x010` and one is `iwdg` `0x008`,
each a single 32-bit write. The WB55 precedent in `rcc.rs` is real, but
nothing in the current shipped corpus reproduces it: **no RCC/clock-enable offset is
undecoded on any lab that runs.** Escalating undecoded writes to a fault would break 5
labs, each for one register.

## Per-lab table

`a` = dropped Cortex-M memory errors, `b1` = factory stub fallthroughs (construction
statistic), `b2` = **live stubs on the assembled machine** (the actionable column),
`c` = undecoded register hits (raw, pre-divide). Sorted hot first.

Where `b1` and `b2` differ the difference is always `nvic`/`scb` (counted by `b1`,
replaced before run time) or `icache`/`syscfg` (a live stub `b1` cannot see).

| script | status | steps | a | b1 | b2 | c | detail |
|---|---|---|---|---|---|---|---|
| `examples/stm32f411ceu6-blackpill/io-smoke.yaml` | pass | 5,000,000 | 0 | 13 | 13 | 4 | live stub: `crc`@0x40023000, `dbg`@0xe0042000, `dma1`@0x40026000, `dma2`@0x40026400, `i2s2ext`@0x40003400, `i2s3ext`@0x40004000, `otg_fs_device`@0x50000800, `otg_fs_global`@0x50000000, `otg_fs_host`@0x50000400, `otg_fs_pwrclk`@0x50000e00, `sdio`@0x40012c00, `syscfg`@0x40013800, `wwdg`@0x40002c00; reg: `iwdg:Iwdg`@0x0008 writex4 |
| `examples/stm32f401cdu6-blackpill/i2c-smoke.yaml` | fail | 4,096 | 0 | 15 | 15 | 0 | live stub: `crc`@0x40023000, `dbg`@0xe0042000, `dma1`@0x40026000, `dma2`@0x40026400, `i2s2ext`@0x40003400, `i2s3ext`@0x40004000, `iwdg`@0x40003000, `otg_fs_device`@0x50000800, `otg_fs_global`@0x50000000, `otg_fs_host`@0x50000400, `otg_fs_pwrclk`@0x50000e00, `rtc`@0x40002800, `sdio`@0x40012c00, `syscfg`@0x40013800, `wwdg`@0x40002c00 |
| `examples/stm32f401cdu6-blackpill/io-smoke.yaml` | pass | 64 | 0 | 15 | 15 | 0 | live stub: `crc`@0x40023000, `dbg`@0xe0042000, `dma1`@0x40026000, `dma2`@0x40026400, `i2s2ext`@0x40003400, `i2s3ext`@0x40004000, `iwdg`@0x40003000, `otg_fs_device`@0x50000800, `otg_fs_global`@0x50000000, `otg_fs_host`@0x50000400, `otg_fs_pwrclk`@0x50000e00, `rtc`@0x40002800, `sdio`@0x40012c00, `syscfg`@0x40013800, `wwdg`@0x40002c00 |
| `examples/stm32f401cdu6-blackpill/trace-smoke.yaml` | pass | 64 | 0 | 15 | 15 | 0 | live stub: `crc`@0x40023000, `dbg`@0xe0042000, `dma1`@0x40026000, `dma2`@0x40026400, `i2s2ext`@0x40003400, `i2s3ext`@0x40004000, `iwdg`@0x40003000, `otg_fs_device`@0x50000800, `otg_fs_global`@0x50000000, `otg_fs_host`@0x50000400, `otg_fs_pwrclk`@0x50000e00, `rtc`@0x40002800, `sdio`@0x40012c00, `syscfg`@0x40013800, `wwdg`@0x40002c00 |
| `examples/stm32f401cdu6/uart-smoke.yaml` | pass | 64 | 0 | 15 | 15 | 0 | live stub: `crc`@0x40023000, `dbg`@0xe0042000, `dma1`@0x40026000, `dma2`@0x40026400, `i2s2ext`@0x40003400, `i2s3ext`@0x40004000, `iwdg`@0x40003000, `otg_fs_device`@0x50000800, `otg_fs_global`@0x50000000, `otg_fs_host`@0x50000400, `otg_fs_pwrclk`@0x50000e00, `rtc`@0x40002800, `sdio`@0x40012c00, `syscfg`@0x40013800, `wwdg`@0x40002c00 |
| `examples/nrf54l15-smart-ring/io-smoke.yaml` | pass | 500,000 | 0 | 9 | 9 | 2 | live stub: `dppic20`@0x500c2000, `dppic30`@0x50102000, `ficr`@0x00ffc000, `regulators`@0x50120000, `rramc`@0x5004b000, `tampc`@0x500dc000, `uicr`@0x00ffd000, `wdt30`@0x50108000, `wdt31`@0x50109000; reg: `nrf54l.twim:Nrf54lTwim`@0x0508 writex1, `nrf54l.twim:Nrf54lTwim`@0x050c writex1 |
| `examples/nrf54l15-dk/io-smoke.yaml` | pass | 200,000 | 0 | 9 | 9 | 0 | live stub: `dppic20`@0x500c2000, `dppic30`@0x50102000, `ficr`@0x00ffc000, `regulators`@0x50120000, `rramc`@0x5004b000, `tampc`@0x500dc000, `uicr`@0x00ffd000, `wdt30`@0x50108000, `wdt31`@0x50109000 |
| `examples/h563-uds-ecu/uds-session-smoke.yaml` | pass | 2,000,000 | 0 | 1 | 1 | 2 | live stub: `icache`@0x40030400; reg: `fdcan:Fdcan`@0x0010 writex2 |
| `examples/h563-uds-ecu/uds-smoke.yaml` | pass | 2,000,000 | 0 | 1 | 1 | 2 | live stub: `icache`@0x40030400; reg: `fdcan:Fdcan`@0x0010 writex2 |
| `examples/nucleo-l073rz/io-smoke.yaml` | fail | 0 | 0 | 5 | 3 | 0 | live stub: `lcd`@0x40002400, `syscfg`@0x40010000, `usb_fs`@0x40005c00 |
| `examples/ads1115-adc-lab/io-smoke.yaml` | pass | 500,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ads1115-adc-lab/stimuli-smoke.yaml` | pass | 2,000,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/adxl345-sensor-lab/io-smoke.yaml` | pass | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/bme280-weather-lab/io-smoke.yaml` | pass | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ci/dummy-memory-violation.yaml` | pass | 0 | 2 | 0 | 0 | 0 | arm: pc=0x00000000 addr=0x00000001 readx1, pc=0x00000000 addr=0x00000004 readx1 |
| `examples/demo-blinky/io-smoke.yaml` | pass | 10,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ds3231-rtc-lab/io-smoke.yaml` | pass | 500,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ds3231-rtc-lab/stimuli-smoke.yaml` | pass | 2,000,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/f103-fidelity-bench/gpiobug-smoke.yaml` | fail | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/f103-i2c-silicon/io-smoke.yaml` | pass | 50,000,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ili9341-tft-lab/io-smoke.yaml` | pass | 20,000,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ina219-power-lab/io-smoke.yaml` | pass | 500,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ina219-power-lab/stimuli-smoke.yaml` | pass | 2,000,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/max31855-thermocouple-lab/io-smoke.yaml` | fail | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/mpu6050-sensor-lab/io-smoke.yaml` | pass | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/neo6m-gps-lab/io-smoke.yaml` | pass | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ntc-thermistor-lab/io-smoke.yaml` | fail | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/pico2/io-smoke.yaml` | fail | 0 | 0 | 3 | 2 | 0 | live stub: `powman`@0x40100000, `tbman`@0x40160000 |
| `examples/pico2/uart-smoke.yaml` | fail | 0 | 0 | 3 | 2 | 0 | live stub: `powman`@0x40100000, `tbman`@0x40160000 |
| `examples/ssd1306-hello-lab/io-smoke.yaml` | pass | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/vl53l1x-tof-lab/io-smoke.yaml` | pass | 200,000 | 0 | 2 | 2 | 0 | live stub: `bkp`@0x40006c00, `usb_dev`@0x40005c00 |
| `examples/ci/l476-bldc-stall.yaml` | pass | 1,265,348 | 0 | 3 | 1 | 0 | live stub: `syscfg`@0x40010000 |
| `examples/h735-telematics-lab/io-smoke.yaml` | pass | 840,000 | 0 | 1 | 1 | 0 | live stub: `syscfg`@0x58000400 |
| `examples/hil-displacement-showcase/io-smoke.yaml` | pass | 0 | 0 | 1 | 1 | 0 | live stub: `icache`@0x40030400 |
| `examples/hil-displacement-showcase/showcase-test.yaml` | pass | 0 | 0 | 1 | 1 | 0 | live stub: `icache`@0x40030400 |
| `examples/nokia5110-invaders-lab/io-smoke.yaml` | pass | 5,000,000 | 0 | 3 | 1 | 0 | live stub: `syscfg`@0x40010000 |
| `examples/nucleo-h563zi/fullchip-smoke.yaml` | pass | 2,000 | 0 | 1 | 1 | 0 | live stub: `icache`@0x40030400 |
| `examples/nucleo-h563zi/io-smoke.yaml` | pass | 5,000,000 | 0 | 1 | 1 | 0 | live stub: `icache`@0x40030400 |
| `examples/nucleo-h563zi/uart-smoke.yaml` | pass | 64 | 0 | 1 | 1 | 0 | live stub: `icache`@0x40030400 |
| `examples/rp2040-pio/asm-smoke.yaml` | pass | 10 | 0 | 2 | 1 | 0 | live stub: `tbman`@0x4006c000 |
| `examples/rp2040-pio/io-smoke.yaml` | fail | 10,000 | 0 | 2 | 1 | 0 | live stub: `tbman`@0x4006c000 |
| `examples/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke.yaml` | pass | 20,000 | 0 | 0 | 0 | 1 | reg: `nrf52.twim:Nrf52Twim`@0x0510 writex1 |
| `examples/stm32h735-smoke/io-smoke.yaml` | pass | 5,000,000 | 0 | 1 | 1 | 0 | live stub: `syscfg`@0x58000400 |

### Clean labs (a, b2 and c all zero)

`b1` is shown so the six labs that were previously listed as hot purely because
their chip yaml declares `type: "nvic"` are visible as exactly that.

| script | status | steps | b1 |
|---|---|---|---|
| `examples/ci-multiarch/two-riscv-test.yaml` | pass | 2,000 | 0 |
| `examples/ci/dummy-fail-uart.yaml` | fail | 10 | 0 |
| `examples/ci/dummy-max-cycles.yaml` | pass | 10 | 0 |
| `examples/ci/dummy-max-steps.yaml` | pass | 10 | 0 |
| `examples/ci/dummy-max-uart-bytes.yaml` | pass | 10,000 | 0 |
| `examples/ci/dummy-no-progress.yaml` | pass | 125 | 0 |
| `examples/ci/dummy-wall-time.yaml` | pass | 0 | 0 |
| `examples/ci/two-node-inputs-env.yaml` | pass | 10 | 0 |
| `examples/ci/uart-inject-echo.yaml` | pass | 20,000 | 0 |
| `examples/ci/uart-ok.yaml` | pass | 1,000 | 0 |
| `examples/esp32c3-blinky/test-blink.yaml` | pass | 800,000 | 0 |
| `examples/esp32c3-leo-airquality/test-fresh.yaml` | pass | 24,000,000 | 0 |
| `examples/esp32c3-leo-airquality/test-stuffy.yaml` | pass | 28,000,000 | 0 |
| `examples/esp32c3-leo-airquality/test.yaml` | pass | 28,000,000 | 0 |
| `examples/feather-f405/io-smoke.yaml` | pass | 64 | 1 |
| `examples/feather-f405/uart-smoke.yaml` | pass | 64 | 1 |
| `examples/kw41z-cow-activity/calm.yaml` | pass | 6,000,000 | 0 |
| `examples/kw41z-cow-activity/stimulus-shake.yaml` | pass | 6,000,000 | 0 |
| `examples/nrf52840-proximity-lab/proximity-smoke.yaml` | fail | 6,000,000 | 0 |
| `examples/nrf52840-secure-boot-lab/secure-boot-smoke.yaml` | pass | 30,000,000 | 0 |
| `examples/nucleo-f401re/io-smoke.yaml` | pass | 64 | 1 |
| `examples/nucleo-f401re/uart-smoke.yaml` | pass | 64 | 1 |
| `examples/nucleo-f767zi/io-smoke.yaml` | pass | 64 | 1 |
| `examples/nucleo-f767zi/uart-smoke.yaml` | pass | 64 | 1 |
| `examples/simctl-selftest/simctl-selftest.yaml` | pass | 7,210 | 0 |

### Skipped - could not run

Every script that did not run gets a row. None is omitted.

| script | missing artifact | why |
|---|---|---|
| `examples/canmod-gps-sim/canmod-smoke.yaml` | `./firmware/build/canmod_gps_sim.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/ci/riscv-uart-ok.yaml` | `../../target/riscv32i-unknown-none-elf/release/riscv-ci-fixture` | riscv32 rustup target not installed on this host |
| `examples/esp32-bay-occupancy/tests/test-debounce.yaml` | `../firmware/.pio/build/adafruit_feather_esp32_v2/firmware.elf` | PlatformIO/Arduino output; needs `pio run`, not committed |
| `examples/esp32-bay-occupancy/tests/test-fault-and-display.yaml` | `../firmware/.pio/build/adafruit_feather_esp32_v2/firmware.elf` | PlatformIO/Arduino output; needs `pio run`, not committed |
| `examples/esp32-bay-occupancy/tests/test-nonblocking.yaml` | `../firmware/.pio/build/adafruit_feather_esp32_v2/firmware.elf` | PlatformIO/Arduino output; needs `pio run`, not committed |
| `examples/esp32-bay-occupancy/tests/test-occupancy-combinations.yaml` | `../firmware/.pio/build/adafruit_feather_esp32_v2/firmware.elf` | PlatformIO/Arduino output; needs `pio run`, not committed |
| `examples/esp32-bay-occupancy/tests/test-thresholds-hysteresis.yaml` | `../firmware/.pio/build/adafruit_feather_esp32_v2/firmware.elf` | PlatformIO/Arduino output; needs `pio run`, not committed |
| `examples/esp32c3-mlx90640-thermal/test-fault.yaml` | `./firmware/thermal_fingerprint.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/esp32c3-mlx90640-thermal/test-iolink-fault.yaml` | `./firmware/thermal_fingerprint.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/esp32c3-mlx90640-thermal/test-iolink.yaml` | `./firmware/thermal_fingerprint.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/esp32c3-mlx90640-thermal/test.yaml` | `./firmware/thermal_fingerprint.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-fidelity-bench/clockbug-nogate-smoke.yaml` | `./firmware/build/clockbug.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-fidelity-bench/clockbug-smoke.yaml` | `./firmware/build/clockbug.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-fidelity-bench/control-smoke.yaml` | `./firmware/build/control.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-fidelity-bench/rambug-smoke.yaml` | `./firmware/build/rambug.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-j1939-monitor/j1939-replay.yaml` | `./firmware/build/j1939_monitor.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-uds-ecu/firmware/diff/diff-smoke.yaml` | `/tmp/lw-deploy/core/examples/f103-uds-ecu/firmware/diff/build/f103_uds_diff.elf` | absolute path into an external HIL deploy tree |
| `examples/f103-uds-ecu/uds-reset-smoke.yaml` | `./firmware/build/f103_uds_ecu.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-uds-ecu/uds-session-smoke.yaml` | `./firmware/build/f103_uds_ecu.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/f103-uds-ecu/uds-smoke.yaml` | `./firmware/build/f103_uds_ecu.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/h563-uds-bootloader/ota-smoke.yaml` | `../../../udslib/examples/h563_uds_bootloader/bootloader/build/h563_uds_bootloader_sim.elf` | sibling `udslib` checkout not present in this repo |
| `examples/iolink-dido/test.yaml` | `./firmware/iolink_dido.elf` | C/Makefile firmware (arm-none-eabi-gcc / riscv32-esp-elf-gcc); ELF not committed |
| `examples/marketplace-arduino-c3/stimuli-smoke.yaml` | `../../platformio/marketplace-arduino-c3/.pio/build/marketplace/firmware.elf` | PlatformIO/Arduino output; needs `pio run`, not committed |
| `examples/mb1355c/uart-smoke.yaml` | `./board_firmware/target/thumbv7em-none-eabi/release/firmware-mb1355c-demo` | nested cargo workspace; host build fails (`unwinding panics are not supported without std`) |
| `examples/nucleo-h563zi/golden-reference/dummy_test.yaml` | `target/thumbv7em-none-eabihf/release/firmware-h563-demo` | firmware artifact absent |
| `examples/nucleo_g474re/uart-smoke.yaml` | `./board_firmware/target/thumbv7em-none-eabi/release/firmware-nucleo_g474re-demo` | nested cargo workspace; host build fails (`unwinding panics are not supported without std`) |
| `examples/nucleo_wba52cg/uart-smoke.yaml` | `./board_firmware/target/thumbv8m.main-none-eabi/release/firmware-nucleo_wba52cg-demo` | nested cargo workspace; host build fails (`unwinding panics are not supported without std`) |

### Ran, but produced no census file

| script | why |
|---|---|
| `examples/ci/benchmark.yaml` | exited before the census dump point (pre-existing config error) |
| `examples/stm32f103-integrated-test/stm32f103_integrated_test.yaml` | exited before the census dump point (pre-existing config error) |

## Behavioural neutrality

The instrumentation must not change what the simulator does, or the census is
worthless. Two independent arguments:

**1. By construction.** With the feature off, every recording site expands back to the
code it wraps:

| site | feature off expands to |
|---|---|
| `census_bus!(self, kind, expr)` | `expr` - the bare expression |
| `census_reg!(name, off, kind)` | `()` - so `_ => { census_reg!(..); }` is `_ => {}` and `_ => { census_reg!(..); 0 }` is `_ => 0` |
| `census::record_stub` / `record_live_stubs` / `record_undecoded_reg_named` / `dump_if_requested` | empty `#[inline(always)]` fns |
| `StubPeripheral::as_any` | does not exist - the whole override is `#[cfg(feature = "silent-census")]`, so the type is unchanged |

The macro arguments are not even evaluated when the feature is off, so a site cannot
introduce a side effect, a panic, or a borrow. `record_live_stubs` takes the bus by
shared reference and the off-build never touches it, so the single call site in
`Machine::new` needs no `cfg` attribute and cannot perturb construction. With the
feature on, every arm still performs its original action and *then* records; no
control flow changed anywhere.

The one thing that *is* observable with the feature on is `StubPeripheral::as_any()`
returning `Some(self)` instead of `None`. Every consumer of `Peripheral::as_any` in
the tree — `Machine::new`'s four index scans, `Machine`'s accessors, `world.rs`'s
UART lookup, `inspect.rs`'s mux lookup, `cpu/riscv.rs`'s XIP lookup — immediately
`downcast_ref`s to a concrete model type, and a stub is none of them. `Some(stub)`
and `None` produce the same `None` from every one of those chains.

**2. Empirically.** The **entire runnable corpus - all 70 labs - was run twice**, once
with a feature-off binary and once with a feature-on binary that was *actively
recording*, and `result.json` (status, steps, cycles, assertions, full CPU register
state, UART bytes, peripheral inspection, fidelity gaps) compared byte-for-byte:

| | result |
|---|---|
| labs run under both binaries | **70** |
| `result.json` byte-identical | **70 / 70** |
| exit-code mismatches | **0** |
| ON runs that actively recorded (wrote a census file) | 68 |
| ON runs where at least one counter fired | 43 |

The proof is non-vacuous: all four counters fired somewhere in the set that was
compared — (a) on `dummy-memory-violation`, (b1) on 47 labs, (b2) on 41 labs,
(c) on 5 — and both pass and fail outcomes are represented (58 pass, 10 fail, 2
config errors), yet not one output byte moved. This is a strict superset of the
8-lab spot check the first version of this document relied on.

Both gates were also checked independently:

| binary | `LABWIRED_CENSUS_OUT` | census file written? |
|---|---|---|
| feature-off | set | **no** |
| feature-on | unset | **no** |
| feature-on | set | yes |

## What was NOT measured

- **46 catch-all arms that match on something other than a register offset** - `cmd`,
  `dest`, `self.state`, `upper.as_str()` and similar. Not register decodes; counting
  them would fabricate gaps.
- **19 catch-all arms inside `#[cfg(test)]` / `mod tests`.**
- **27 scripts whose firmware could not be produced on this host** - PlatformIO/Arduino
  builds, C/Makefile firmware needing `arm-none-eabi-gcc` or `riscv32-esp-elf-gcc`, one
  riscv32 target not installed, three nested cargo workspaces that fail to build on
  this toolchain, and two paths pointing outside the repo. Every one is a row above.
- **Xtensa and ESP32 ROM-boot paths.** No `examples/esp32s3-*` directory has a test
  script at all, and the C3 Arduino lab needs `LABWIRED_ESP32C3_*` flash/ROM images.
- **The RISC-V CPU's own error handling.** `cpu/riscv.rs` propagates ~32 bus results
  with `?` and discards none, so it has no (a)-equivalent to count. Unverified beyond
  reading the code.
- **`Machine::run` batched orchestration.** Every run here used the CLI's default
  per-instruction path. The browser's batched path was not measured.
- **Any peripheral replacement that happens *after* `Machine::new`.** (b2) sweeps at
  machine construction, which is after every construction-time pass in the tree today
  (audited above), but it is not immune by design. A future pass that swapped a
  peripheral out later would be invisible to it, exactly as `configure_cortex_m` was
  invisible to (b1). Re-check that table before trusting the number again.
- **Whether an undecoded offset is actually *wrong*.** The census says the model did
  not decode an offset the firmware touched. Deciding whether that matters needs the
  datasheet, one register at a time. That is the follow-up work this table scopes.

## Belongs to another task

- Fixing any of the three paths. Explicitly out of scope here.
- Removing the dead `type: "nvic"` / `type: "scb"` entries from the chip yamls (or
  giving the factory a real arm for them). They cost a `StubPeripheral` allocation
  that `configure_cortex_m` immediately discards, and they are the reason the
  fallthrough histogram does not mean what a reader assumes. Not a fidelity bug.
- `examples/ci/benchmark.yaml` declares `max_steps: 1000000000`, above the CLI's
  `MAX_ALLOWED_STEPS` of 50000000, so it cannot run as committed.
- `examples/stm32f103-integrated-test` points at a system manifest missing a `chip:`
  field and fails to load.
- `examples/f103-uds-ecu/firmware/diff/diff-smoke.yaml` hardcodes absolute
  `/tmp/lw-deploy/...` paths and can never run from a clean checkout.
- 10 of the 68 runnable labs fail their own assertions at this SHA. Pre-existing,
  unrelated to this work, and not investigated.
- `scripts/example_smokes.sh` globs `examples/*/*smoke*.yaml examples/*/test*.yaml`,
  which misses ~7 scripts one level deeper plus every `examples/ci/*.yaml`. Widening
  it is a separate change.

