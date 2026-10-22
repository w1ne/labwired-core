# Validation — dsPIC33FJ32MC204 (M0 + M1a + M1b)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF: PC 0 → reset `GOTO` → `__reset` | smoke-tested | `crates/core/tests/pic24_m0_smoke.rs`, `pic24_xc16_elf_boot.rs` |
| Base ISA (`MOV`/`MOV.B`, `ADD`/`AND`/`IOR`, `CLR`/`SETM`, `CP0`, `BSET`/`BCLR`/`BTG`, `SE`, `LSR`, `BRA`/`RCALL`/`CALL`/`GOTO`, `RETURN`/`RETLW`/`LNK`/`ULNK`/`RETFIE`) | unit-tested | `crates/core/src/cpu/pic24.rs` (`#[cfg(test)]`), real-XC16 boot test |
| Stack semantics: `PUSH [W15++]` / `POP [--W15]` (DS70283H Figure 4-4) | unit-tested | `call_and_return_use_the_upward_stack`, `lnk_and_ulnk_allocate_a_frame` |
| UART1 TX (`U1TXREG`) → console sink | smoke-tested | unit test `mov_w0_to_u1txreg_reaches_sink`, integration tests, io-smoke |
| Determinism (same inputs → same output) | smoke-tested | both integration tests run their firmware twice and compare |
| **Real XC16 v1.70 firmware boots** (`xc16-uart-smoke.c`, stock linker script): UART `HI` + `LATB0` toggle loop | integration | `crates/core/tests/pic24_xc16_elf_boot.rs` |
| XC16 ELF section layout (4-byte slots, PC-unit addresses, reset GOTO, IVT addresses) | integration | `xc16_elf_sections_land_at_their_pc_unit_addresses` |
| Timers T1/T2/T3: TON, TCKPS 1:1–1:256, `TMR > PR` match, 32-bit T2/T3 pair (M1a) | unit-tested | `timer1_counts_to_period_and_raises_flag`, `timer1_prescaler_1_to_8`, `timer23_pair_counts_32_bit_and_flags_timer3` |
| Interrupts: IFS0/IEC0/IPC0-2, DISI/NSTDIS gating, **IVT/AIVT vector fetch of the 24-bit ISR address**, two-word frame, RETFIE (M1a) | unit-tested | `timer1_interrupt_vectors_frames_and_returns`, `disabled_source_does_not_vector` |
| `ACCA*` / `ACCB*` 40-bit SFR storage (M1a) | unit-tested | `accumulator_sfr_words_round_trip` |
| DSP MAC (M1c), PWM/QEI (M2), ADC/SPI/I2C (M3) | **not modelled** | decode error on unsupported opcodes; no claims |
| GPIO ports A/B/C: TRISx/PORTx/LATx/ODCx, 44-pin masks (M1b) | unit-tested | `gpio_output_drives_the_pad_model`, `port_read_sees_external_levels_and_own_latch`; `peripherals::pic24_gpio` tests |
| PPS slice: RPINR7 inputs, RPOR0-12 outputs, RP0-15 = RB0-15 / RP16-25 = RC0-9 (M1b) | unit-tested | exercised by the OC/IC tests below |
| Input capture 1/2: edge modes, prescalers, 4-deep FIFO, ICI, ICOV/ICBNE (M1b) | unit-tested | `ic1_captures_tmr2_on_a_rising_edge_of_rp0`, `ic_interrupt_divider_and_overflow_rules` |
| Output compare 1/2: one-shot, toggle, delayed, continuous pulse, PWM (M1b) | unit-tested | `oc1_active_low_one_shot_pulses_rp0`, `oc1_toggle_flips_on_every_match`, `oc1_pwm_reloads_ocxrs_on_rollover` |

## Commands and evidence (verified 2026-10-18, branch `feat/pic24-m0`)

```bash
# 1. Unit tests — 8 passed
cargo test -p labwired-core --lib pic24
#   test cpu::pic24::tests::{reset_starts_at_word_zero, mov_literal_loads_w0,
#     mov_w0_to_u1txreg_reaches_sink, nop_advances_pc, bra_minus_one_stays_put,
#     u1sta_read_reports_transmit_empty, ram_round_trip,
#     unsupported_opcode_is_a_decode_error} ... ok
#   test result: ok. 8 passed; 0 failed

# 2. Integration smoke — 2 passed (boot + UART + determinism ×2)
cargo test -p labwired-core --test pic24_m0_smoke
#   test pic24_fixture_boots_and_writes_h_to_uart ... ok
#   test pic24_fixture_is_deterministic ... ok
#   test result: ok. 2 passed; 0 failed

# 3. CLI end-to-end
cargo build -p labwired-cli
./target/debug/labwired test --script examples/dspic33fj32mc204/io-smoke.yaml
#   INFO labwired_core::system::builder: Loading chip descriptor:
#     ../../configs/chips/dspic33fj32mc204.yaml
#   INFO labwired_loader: ELF Entry Point: 0x0
#   PASS  2/2 checks · io-smoke · 64 steps · 0.00s   (exit 0)
```

Expected io-smoke result: UART output contains `H`; stop reason `max_steps` — observed as above.

## Gate integration (run on this branch, 2026-10-18)

```bash
cargo test -p labwired-core --test strict_onboarding
#   Verifying Strict Onboarding for: dspic33fj32mc204
#   Found example directory: "examples/dspic33fj32mc204"
#   PASS  2/2 checks · io-smoke · 64 steps · 0.00s
#   [PASS] dspic33fj32mc204 is strictly onboarded.
cargo test -p labwired-core --test board_coverage_ratchet      # 4 passed
cargo test -p labwired-core --test session_catalog             # 1 passed (48-chip catalog incl. PIC24)
cargo test -p labwired-core --test chip_conformance            # 8 passed (declared-only board regenerated)
cargo test -p labwired-core --test cpu_trace_conformance       # 3 passed (Pic24 emits the full trace contract)
cargo test -p labwired-core --test register_compliance         # 1 passed
cargo test -p labwired-core --test register_coverage           # 1 passed
cargo test -p labwired-core --lib one_arch_policy              # 8 passed
cargo test -p labwired-core --lib cpu_hz                       # 4 passed
cargo test -p labwired-core --lib bus_proof_matrix             # 10 passed (uart proven; spi/i2c declared none)
python3 scripts/generate_firmware_exercise_matrix.py --check   # clean (entry added)
```

(Environment note: strict_onboarding also builds firmware for every other chip;
those builds need cross-targets not installed in the authoring sandbox — the
failures listed there are environmental and do not involve this chip.)

## M1a evidence (timers + interrupts, verified 2026-10-19, branch `feat/pic24-m1a`)

```bash
# 1. Unit tests — 14 passed (6 new M1a tests)
cargo test -p labwired-core --lib pic24
#   test cpu::pic24::tests::timer1_counts_to_period_and_raises_flag ... ok
#   test cpu::pic24::tests::timer1_prescaler_1_to_8 ... ok
#   test cpu::pic24::tests::timer23_pair_counts_32_bit_and_flags_timer3 ... ok
#   test cpu::pic24::tests::timer1_interrupt_vectors_frames_and_returns ... ok
#   test cpu::pic24::tests::disabled_source_does_not_vector ... ok
#   test cpu::pic24::tests::accumulator_sfr_words_round_trip ... ok
#   test result: ok. 14 passed; 0 failed

# 2. Integration + trace gates unaffected
cargo test -p labwired-core --test pic24_m0_smoke        # 2 passed (fixture unchanged)
cargo test -p labwired-core --test cpu_trace_conformance # 3 passed (per-instruction trace contract kept)

# 3. fmt / clippy
cargo fmt --all -- --check                              # clean
cargo clippy -p labwired-core --tests -- -D warnings    # clean
```

M1a scope: `T1CON/T2CON/T3CON` (TON, TCKPS, T32), `TMR1/2/3`, `PR1/2/3`,
`IFS0/IEC0/IPC0-2`, `INTCON1.NSTDIS`, `INTCON2.DISI`, exception entry with the
DS70283H two-word frame, vector `0x4 + 2*N`, `RETFIE`, and `ACCA*`/`ACCB*` SFR
storage. Encodings and addresses pinned to DS70000157G / DS70283H. IC/OC (M1b,
needs a pin/GPIO model) and the DSP MAC (M1c, PRM field tables) remain
unmodelled; unsupported opcodes still raise `DecodeError`, never a silent skip.

## M1b evidence (GPIO ports, PPS, input capture + output compare, verified 2026-10-19)

```bash
# 1. Unit tests — 27 passed (7 new M1b CPU tests + 6 port-model tests)
cargo test -p labwired-core --lib pic24
#   test cpu::pic24::tests::gpio_output_drives_the_pad_model ... ok
#   test cpu::pic24::tests::port_read_sees_external_levels_and_own_latch ... ok
#   test cpu::pic24::tests::oc1_active_low_one_shot_pulses_rp0 ... ok
#   test cpu::pic24::tests::oc1_toggle_flips_on_every_match ... ok
#   test cpu::pic24::tests::oc1_pwm_reloads_ocxrs_on_rollover ... ok
#   test cpu::pic24::tests::ic1_captures_tmr2_on_a_rising_edge_of_rp0 ... ok
#   test cpu::pic24::tests::ic_interrupt_divider_and_overflow_rules ... ok
#   test peripherals::pic24_gpio::tests::* (6) ... ok
#   test result: ok. 27 passed; 0 failed

# 2. PinPort conformance suite includes Pic24GpioPort — 5 passed
cargo test -p labwired-core --lib conformance

# 3. Integration + trace gates unaffected
cargo test -p labwired-core --test pic24_m0_smoke        # 2 passed
cargo test -p labwired-core --test cpu_trace_conformance # 3 passed

# 4. Registry boards regenerated and re-checked (chip promoted from
#    declared-only to a modelled conformance row with a behaviour gate)
UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test chip_conformance
UPDATE_BUS_VISIBILITY_BASELINE=1 cargo test -p labwired-core --test bus_visibility
cargo test -p labwired-core --test chip_conformance --test bus_visibility   # both green

# 5. fmt / clippy
cargo fmt --all -- --check                              # clean
cargo clippy -p labwired-core --tests -- -D warnings    # clean
```

M1b scope: TRISx/PORTx/LATx/ODCx ports A/B/C with the 44-pin masks, PPS
registers `RPINR7` + `RPOR0-12` consuming the IC1/IC2 inputs and OC1/OC2
outputs, input capture (modes 001-011, every-4th/16th-rising prescalers,
4-deep FIFO, ICI divider, ICOV/ICBNE), output compare (modes 001-110, PWM
`OCxRS → OCxR` reload, `OCxR` read-only while PWM runs) and their
IFS0/IEC0/IPC interrupts. Sources: DS70283H (registers, pin/RP map, PPS
codes), DS70190D §30 (port semantics), DS70198D §12 (IC), DS70209A §13 (OC).

Known approximations (also in the module docs): IC capture latency (2
instruction cycles) is not modelled — edges are sampled once per retired
step; the `OCxR = 0` "second timer cycle" case uses the plain equality
compare; output compare cannot use the 32-bit T2/T3 pair; IC mode 111
(sleep/idle wake) never captures; CNPU internal pull-ups are not modelled.

## XC16 firmware evidence (real toolchain, verified 2026-10-22)

The committed `dspic33fj32mc204-xc16-uart-smoke.elf` is built by Microchip
XC16 v1.70 (container recipe in the README) from `xc16-uart-smoke.c` with
the stock `p33FJ32MC204.gld` linker script. It exercises the compiler's own
startup code and the base ISA, not a hand-written fixture.

```bash
# 1. Unit tests — 31 passed (base ISA + stack + M0/M1a/M1b tests)
cargo test -p labwired-core --lib pic24
#   test result: ok. 31 passed; 0 failed

# 2. The real XC16 ELF boots — 3 passed
cargo test -p labwired-core --test pic24_xc16_elf_boot
#   xc16_elf_sections_land_at_their_pc_unit_addresses ... ok
#   xc16_firmware_boots_and_writes_hi_to_uart ... ok
#   xc16_firmware_is_deterministic ... ok

# 3. Fixture, trace and catalogue gates — green
cargo test -p labwired-core --test pic24_m0_smoke        # 2 passed
cargo test -p labwired-core --test cpu_trace_conformance # 3 passed
cargo test -p labwired-core --test session_catalog       # 1 passed
cargo test -p labwired-loader                            # 0 tests (parsers covered via core)

# 4. fmt / clippy
cargo fmt --all -- --check                              # clean
cargo clippy -p labwired-core -p labwired-loader --tests -- -D warnings   # clean
```

Scope added by this milestone: the base integer ISA the XC16 startup and
`main` use, the corrected stack direction (`PUSH [W15++]`), the corrected
exception entry (PC loaded from the IVT/AIVT entry as a 24-bit address),
section-based PIC24 ELF loading, and the boot-at-0 reset path.

## Unsupported-instruction audit

```bash
core/scripts/unsupported_instruction_audit.sh \
  --firmware examples/dspic33fj32mc204/firmware/dspic33fj32mc204-uart-smoke.elf \
  --system   examples/dspic33fj32mc204/system.yaml \
  --max-steps 64 --out-dir out/audit
#   unknown_thumb16: 0 · unhandled_thumb32: 0 · unknown_riscv: 0
#   unsupported_total: 0   (simulator exit 0)
```

The audit's Thumb/RISC-V counters are zero because the M0 core never degrades an
unknown opcode silently: anything outside the modelled subset raises a
`DecodeError`, which would fail the io-smoke rather than appear as a skipped
instruction. Per-instruction telemetry for the audit's `instructions_executed`
field is not emitted by the M0 core yet (M1 item).

## Console evidence (`labwired run`)

# Console evidence (`labwired run`, re-verified 2026-10-19 after the XC16 layout fix)

```bash
labwired run --chip configs/chips/dspic33fj32mc204.yaml --firmware <elf> --max-steps 200
#   UART: H
#   labwired-cli run (pic24): reached --max-steps 200; pc=0x00000210

labwired --firmware <elf> --system examples/dspic33fj32mc204/system.yaml --max-steps 64
#   UART: H
#   Final PC: 0x210
#   Total Instructions: 64
#   Total Cycles: 120   (56 self-loop BRA × 2 + 8 single-cycle instructions)
```

## Method notes

- Fixture ELF is deterministic (generated by `firmware/make-fixture-elf.py`).
- The XC16 ELF layout is **verified against a real XC16 v1.70 build**
  (committed `dspic33fj32mc204-xc16-uart-smoke.elf`): executable sections are
  4-byte slots (24-bit word + pad byte) at PC-unit addresses; the reset GOTO
  sits at PC 0; the IVT stores 24-bit ISR addresses as data. #1447's "3
  bytes per instruction" reading of DS50002071F described the assembler's
  conceptual addressing, not ELF storage; see `docs/pic24-m0-reference.md`
  §3.1 and the follow-up layout PR.
- No silicon comparison exists for this part yet; nothing here is
  hardware-compared.
