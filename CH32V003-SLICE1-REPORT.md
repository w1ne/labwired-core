# CH32V003 bring-up — Slice 1 report

## Changes

The current full-file line counts are:

| File | Lines | Changes |
| --- | ---: | --- |
| `crates/core/src/cpu/riscv.rs` | 2,281 | Added the RV32E profile selection, register-operand validation before instruction effects, and standard cycle CSR sharing. |
| `crates/core/src/cpu/riscv_rv32e_tests.rs` | 83 | Added profile-selection, legal/illegal ADDI, and compressed full-register-field tests. |
| `crates/core/src/peripherals/pfic.rs` | 356 | Added a standalone PFIC model with enable/pending/active/priority registers, arbitration, nesting and vector-free handler APIs. |
| `crates/core/src/peripherals/rcc.rs` | 4,420 | Added `RccRegisterLayout::Wch`, the WCH RCC model and its PLL-ready/SYSCLK test. |
| `crates/core/src/peripherals/mod.rs` | 285 | Exported the PFIC module. |
| `CH32V003-SLICE1-REPORT.md` | 163 | Recorded changes, test commands and raw outputs, unverified facts, and next-slice work. |

RV32E checks all decoded integer-register operands, including rd/rs fields in atomic instructions and full-width register fields in compressed instructions. Illegal use traps with `mcause = 2`, `mtval = 0`, and leaves instruction execution and PC advancement to the trap path. The RV32E profile uses the same `0xC00`/`0xC80` cycle CSR behavior as `StandardRv32`.

PFIC register offsets listed in the brief are implemented relative to a PFIC base mapping at `0xE000E000`. The model uses source IDs 0–26 as a simulator convention; the hardware IRQ number assignment was not available to verify. Priorities use the low four bits of one per-source word, with larger values ranked higher as a model choice. Nesting is exposed through `set_nesting_enabled`, `pending_source`, `acknowledge`, and `complete`. Vector-free handler lookup is provided as an API because the brief did not give VTF MMIO offsets or slot count.

The WCH RCC supports the PLL-ready polling and clock switching path, an HSE frequency setter, AHB prescaling, and storage for the three peripheral-enable registers. The WCH model is reachable by `RccRegisterLayout::Wch`; chip mapping remains deferred.

## Datasheet facts — verified against the public CH32V003RM

The LabWired Part Knowledge lookup for `CH32V003` returned `NOT_FOUND`, so the
original slice-1 facts were marked provisional. They were then verified
out-of-band against the public sources:

- CH32V003 Reference Manual (V1.5 / V1.9), §6.5.2 `R32_PFIC_*` register table.
- Official WCH peripheral library `ch32v00x.h` / `ch32v00x_rcc.c` (WCH V1.0.0):
  `RCC_TypeDef`, `APBAHBPrescTable`, and the `IRQn_Type` interrupt numbers.

### Verified (the brief values were correct)
- PFIC offsets: `ISR1 0xE000E000`, `IENR1 0xE000E100`, `IRER1 0xE000E180`,
  `IPSR1 0xE000E200`, `IPRR1 0xE000E280`, `IACTR1 0xE000E300`,
  `IPRIORx 0xE000E400`, `SCTLR 0xE000ED10` — all match the RM.
- RCC offsets: `CTLR 0x00`, `CFGR0 0x04`, `AHBPCENR 0x14`, `APB2PCENR 0x18`,
  `APB1PCENR 0x1C` — match the official `RCC_TypeDef`.
- Bit fields: `PLLSRC` CFGR0 bit 16, `PLLON` CTLR bit 24, `HSEON` CTLR bit 16,
  `SW[1:0]` CFGR0 bits 0–1, `SWS[1:0]` bits 2–3.
- `HSI = 24 MHz`, `HSE = 24 MHz` default (header `HSI_VALUE`/`HSE_VALUE`),
  `PLL = source × 2`.

### Newly known (previously "unknown" in this report)
- VTF: `PFIC_VTFIDR 0xE000E050`, `PFIC_VTFADDRR0 0xE000E060`,
  `PFIC_VTFADDRR1 0xE000E064`; `PFIC_CFGR 0xE000E048`, `PFIC_GISR 0xE000E04C`,
  `PFIC_ITHRESDR 0xE000E040`.
- IRQ map (`IRQn_Type`): `WWDG 16, PVD 17, FLASH 18, RCC 19, EXTI7_0 20,
  AWU 21, DMA1_CH1..7 22–28, ADC 29, I2C1_EV 30, I2C1_ER 31, USART1 32,
  SPI1 33, TIM1_BRK 34, TIM1_UP 35, TIM1_TRG_COM 36, TIM1_CC 37, TIM2 38`;
  SysTick 12, software 14.

### Corrected (one real bug)
- `WchRcc::hclk_hz()` used a power-of-two-log HPRE divisor table
  (`0..=7 => 1, 8 => 2, 9 => 4, … 512`) that was wrong on nearly every entry
  and invented a `/512` encoding that does not exist. The verified table is the
  WCH `APBAHBPrescTable` `{1,2,3,4,5,6,7,8,1,2,3,4,5,6,7,8}`: field `0x1` is
  `/2`, `0x2` is `/3`, and the upper half repeats the lower (`0x8` is `/1`).
  Fixed in commit `291821aa5`; the test now asserts the verified encodings.

### Still provisional
- PFIC priority encoding/order and the exact 2-level nesting semantics are a
  model choice, not yet read bit-by-bit from the RM. Source IDs 0–26 are a
  simulator convention; reconcile to the verified IRQ map when PFIC is wired
  into a chip. `LSI = 128 kHz` and the `HSE 4–25 MHz` range remain unconfirmed
  detail.

## Tests

The following are the final focused commands. They used an isolated temporary target directory because simultaneous builds from other sessions caused a shared-target linker failure (`undefined hidden symbol`). The output below is the raw test-runner portion; Cargo also printed its existing workspace profile warnings.

### RV32E

Command:

```text
CARGO_TARGET_DIR=/tmp/codex-ch32v003-target cargo test -p labwired-core --lib rv32e_ -- --quiet
```

Output:

```text
Finished `test` profile [optimized + debuginfo] target(s) in 0.17s
Running unittests src/lib.rs (/tmp/codex-ch32v003-target/debug/deps/labwired_core-2749f3dfd141bd8f)

running 5 tests
.....
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5411 filtered out; finished in 0.00s
```

### PFIC

Command:

```text
CARGO_TARGET_DIR=/tmp/codex-ch32v003-target cargo test -p labwired-core --lib pfic -- --quiet
```

Output:

```text
Finished `test` profile [optimized + debuginfo] target(s) in 3m 06s
Running unittests src/lib.rs (/tmp/codex-ch32v003-target/debug/deps/labwired_core-2749f3dfd141bd8f)

running 5 tests
.....
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5411 filtered out; finished in 0.00s
```

### WCH RCC

Command:

```text
CARGO_TARGET_DIR=/tmp/codex-ch32v003-target cargo test -p labwired-core --lib wch_rcc_pll_ready_switches_sysclk -- --quiet
```

Output:

```text
Finished `test` profile [optimized + debuginfo] target(s) in 0.21s
Running unittests src/lib.rs (/tmp/codex-ch32v003-target/debug/deps/labwired_core-2749f3dfd141bd8f)

running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5415 filtered out; finished in 0.00s
```

### Negative controls

Each behavior was temporarily disabled, its focused test failed, and the implementation was restored before the final passing runs.

RV32E command:

```text
CARGO_TARGET_DIR=/tmp/codex-ch32v003-target cargo test -p labwired-core --lib rv32e_rejects_addi_destination_x16_before_writing_or_advancing -- --quiet
```

Output:

```text
running 1 test
cpu::riscv::rv32e_tests::rv32e_rejects_addi_destination_x16_before_writing_or_advancing --- FAILED

thread 'cpu::riscv::rv32e_tests::rv32e_rejects_addi_destination_x16_before_writing_or_advancing' panicked at crates/core/src/cpu/riscv_rv32e_tests.rs:23:5:
assertion `left == right` failed
  left: 0
 right: 2

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 5415 filtered out; finished in 0.00s
error: test failed, to rerun pass `-p labwired-core --lib`
```

PFIC command:

```text
CARGO_TARGET_DIR=/tmp/codex-ch32v003-target cargo test -p labwired-core --lib enable_set_and_clear_registers_update_status -- --quiet
```

Output:

```text
running 1 test
peripherals::pfic::tests::enable_set_and_clear_registers_update_status --- FAILED

thread 'peripherals::pfic::tests::enable_set_and_clear_registers_update_status' panicked at crates/core/src/peripherals/pfic.rs:295:9:
assertion `left == right` failed
  left: 0
 right: 72

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 5415 filtered out; finished in 0.00s
error: test failed, to rerun pass `-p labwired-core --lib`
```

RCC command:

```text
CARGO_TARGET_DIR=/tmp/codex-ch32v003-target cargo test -p labwired-core --lib wch_rcc_pll_ready_switches_sysclk -- --quiet
```

Output:

```text
running 1 test
peripherals::rcc::tests::wch_rcc_pll_ready_switches_sysclk --- FAILED

thread 'peripherals::rcc::tests::wch_rcc_pll_ready_switches_sysclk' panicked at crates/core/src/peripherals/rcc.rs:3202:9:
assertion `left != right` failed
  left: 0
 right: 0

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 5415 filtered out; finished in 0.00s
error: test failed, to rerun pass `-p labwired-core --lib`
```

## Next slice

The next slice still needs the CH32V003 peripheral map, chip descriptor and firmware fixture. Before wiring those up, verify the provisional RCC details, PFIC source mapping and priority/nesting semantics, and the VTF MMIO layout against CH32V003RM.
