# NUCLEO-F407 hardware-validation log

Every commit to the F407 chip yaml or any peripheral that F407 firmware
touches must keep the survival tests green. This file is the audit
trail: which traces have been captured against real silicon, what
revealed each bug, and which simulator commits closed each gap.

Mirrors the workflow already proven on
[`docs/boards/nucleo-l476rg.md`](../../docs/boards/nucleo-l476rg.md).

## Hardware

- Board: **NUCLEO-F407** (or STM32F4-DISCO with an external USB-UART
  on PA2/PA3 for survival traces; the I²C lane assumes Nucleo's
  on-board ST-LINK Virtual COM Port).
- Debugger: on-board ST-LINK V2-1.
- Host: Linux, `arm-none-eabi-gcc 14.x`, OpenOCD 0.12+.
- DBGMCU IDCODE @ 0xE0042000 = (to be filled by Round 1 capture).
  The chip yaml currently encodes `0x10070413` as a placeholder.

## Survival traces

Each row is a captured byte stream that the simulator must reproduce
byte-for-byte (`crates/core/tests/firmware_survival.rs::test_nucleo_f407_*`).

| Trace                   | Fixture ELF                                     | Hardware capture file                                  | Status                          |
|-------------------------|-------------------------------------------------|--------------------------------------------------------|---------------------------------|
| `nucleo_f407_smoke`     | `tests/fixtures/nucleo-f407-smoke.elf`          | [`tests/fixtures/hw_traces/nucleo_f407_smoke.txt`](../../tests/fixtures/hw_traces/nucleo_f407_smoke.txt) | ✅ Hardware-validated 2026-05-11 |
| `nucleo_f407_i2c`       | `tests/fixtures/nucleo-f407-i2c.elf`            | [`tests/fixtures/hw_traces/nucleo_f407_i2c.txt`](../../tests/fixtures/hw_traces/nucleo_f407_i2c.txt) | ✅ Hardware-validated 2026-05-11 |

## Capture-session playbook

For each trace, the bench-side workflow is:

1. **Build the firmware** (host side, no hardware needed):
   ```bash
   cargo build --release -p firmware-f407-demo
   ```
   Output: `target/thumbv7em-none-eabi/release/firmware-f407-smoke`.

2. **Stage the ELF as a test fixture**:
   ```bash
   cp target/thumbv7em-none-eabi/release/firmware-f407-smoke \
      tests/fixtures/nucleo-f407-smoke.elf
   ```
   (Already done on first round; re-do after every firmware change.)

3. **Run the sim-only assertion** to lock in the expected output:
   ```bash
   cargo test -p labwired-core --test firmware_survival \
       test_nucleo_f407_smoke_survival --release
   ```
   This must pass with the current `expected_uart_output` literal in
   `SURVIVAL_CASES` before flashing — it pins the simulator behavior.

4. **Flash the firmware to silicon**:
   ```bash
   openocd -f interface/stlink.cfg -f target/stm32f4x.cfg \
       -c "program tests/fixtures/nucleo-f407-smoke.elf verify reset exit"
   ```

5. **Capture the Virtual COM Port output**:
   ```bash
   stty -F /dev/ttyACM0 115200 cs8 -cstopb -parenb -echo raw
   timeout 3 cat /dev/ttyACM0 > tests/fixtures/hw_traces/nucleo_f407_smoke.txt
   ```
   Reset the board (NRST button on the Nucleo) once during the
   3-second window. The smoke firmware prints its payload then halts
   in `wfi`, so the byte stream is finite.

6. **Diff the silicon trace against `expected_uart_output`**:
   ```bash
   diff <(xxd tests/fixtures/hw_traces/nucleo_f407_smoke.txt) \
        <(printf 'F407 SMOKE\r\nDEV=...\r\nMUL=...\r\nDONE\r\n' | xxd)
   ```
   If they match → the trace is silicon-validated, commit the
   `hw_traces/` file as the audit artifact. If they diverge → that's
   the bug. Investigate, fix the simulator (or the chip yaml), update
   `expected_uart_output` to match silicon, re-run step 3.

## Rounds

Each round below records a sim↔silicon divergence the survival trace
surfaced and the simulator commit that closed it. Empty rounds mean
"hardware capture still pending."

### Round 1 — UART smoke (`nucleo_f407_smoke`) ✅

**Captured 2026-05-11.** Hardware: STM32F4-DISCOVERY (STM32F407VGT6),
on-board ST-LINK V2 (USB ID `0483:3748`, firmware updated mid-round
V2J24S0 → V2J43S0). Capture path: ARM semihosting via openocd `arm
semihosting enable` (dual-emit firmware writes each byte to both
USART2 DR and a `bkpt #0xAB` SYS_WRITEC, simulator only reads the
USART2 path).

Silicon byte stream (46 bytes,
[`hw_traces/nucleo_f407_smoke.txt`](../../tests/fixtures/hw_traces/nucleo_f407_smoke.txt)):

```
F407 SMOKE
DEV=10016413
MUL=369D0368
DONE
```

Matches `firmware_survival.rs::SURVIVAL_CASES[22].expected_uart_output`
byte-for-byte after the fixes below landed. The whole round took 4
sub-fixes — the survival-trace pattern surfaced each one cleanly.

**Sub-fix #1 — DBGMCU REV_ID placeholder (commit `1273981`).**
OpenOCD reported `device id = 0x10016413` from silicon. The chip
yaml placeholder was `0x10070413` (REV_ID `0x1007`). Real silicon is
REV_ID `0x1001` (Rev 1 — most common for F407V/Z/IG). Updated
`configs/chips/stm32f407.yaml::dbgmcu.config.idcode` and the
survival `expected_uart_output` to `DEV=10016413`.

**Sub-fix #2 — Vector-table garbage on any exception (commit `a435e8d`).**
The original `minimal.ld` emitted only `[SP, Reset]`. Any exception
(including the semihosting BKPT before openocd intercepts) read junk
for the HardFault vector → PC `0xf643b082` → double fault → lockup.
Rewrote `minimal.ld` with a full 16-entry Cortex-M4 vector table
where every non-Reset slot points at a `default_handler` that sits
in `wfi`. Failures now halt cleanly with PC inside `default_handler`,
making the actual fault identifiable.

**Sub-fix #3 — Simulator halted on all BKPTs (commit `a435e8d`).**
`Instruction::Bkpt` in `crates/core/src/cpu/cortex_m.rs` returned
`Halt` for every immediate. Dual-emit firmware needs `bkpt #0xAB`
(semihosting magic) to be a no-op in the simulator while still
halting on any other immediate (panics, debugger breakpoints). Now
gated on `imm8 != 0xAB`.

**Sub-fix #4 — Linker double-applying the thumb bit (this commit).**
`minimal.ld` had `LONG(Reset + 1)` modeled after the L476 demo. But
Rust emits ARM function symbols with the thumb bit *already* in the
symbol value (`readelf -s` showed `Reset = 0x08000041`), so `+ 1`
landed at `0x08000042` — thumb bit cleared. CPU loaded the vector,
switched to ARM mode, and INVSTATE-faulted on every instruction.
The L476 fixture predates this behavior (its `Reset` symbol is at
`0x08000040` per its `nm`). Fix: `LONG(Reset)` and `LONG(default_handler)`
without the `+ 1`. Hardware now boots cleanly into Reset, BKPTs trap
into openocd, semihosting forwards each byte to the host, capture
matches sim verbatim.

**ST-LINK firmware: needed but not the actual root cause.** Updated
V2J24S0 → V2J43S0 mid-round to rule it out. The fault repeated on
both firmware versions; sub-fix #4 was what unblocked it.

Other things worth re-checking on future rounds:
- **RCC bring-up timing.** The smoke firmware doesn't touch the PLL,
  so silicon stays on HSI 16 MHz. If a future round adds a clock-tree
  exercise the BRR computation needs to be re-derived for the new
  SYSCLK.
- **F4 USART_SR vs L4 USART_ISR.** This firmware uses the classic
  F4 layout (SR/DR at offsets 0/4). If silicon UART output goes silent
  on a future variant, check that the chip yaml's USART2 type still
  dispatches the V1 register layout.

### Round 2 — I²C no-slave register fingerprint (`nucleo_f407_i2c`) ✅

**Captured 2026-05-11.** Hardware: STM32F4-DISCOVERY, no slaves on
PB6/PB7 (sensors arrive separately). Capture path: ARM semihosting
via openocd (same dual-emit pattern as Round 1).

Trace shape: `I2C INIT → CR1/CR2/CCR/TRISE/OAR1/SR1/SR2 dump → START →
SR1 dump → ADDR phase (0x70 = AHT20 write, but no chip wired) → SR1/SR2
dump → STOP → SR1/SR2 dump`. 218 bytes total
([`hw_traces/nucleo_f407_i2c.txt`](../../tests/fixtures/hw_traces/nucleo_f407_i2c.txt)),
matches sim verbatim.

The round drove **3 sub-fixes**, two in the simulator and one in the
firmware:

**Sub-fix #1 — Sim assumed ACK on every address phase (this commit).**
`AddressPending` tick set `SR1.ADDR | SR2.MSL | SR2.BUSY`
unconditionally, ignoring whether `current_target` was `None`. Real
silicon raises `SR1.AF` (bit 10, Acknowledge Failure) when no slave
replies, leaves `ADDR` clear, but **keeps** `MSL+BUSY` set in `SR2`
(master mode active, waiting for STOP). Now matches: sim raises AF
on `current_target.is_none()` and sets MSL+BUSY same way silicon does.

**Sub-fix #2 — Firmware missing internal pull-ups (this commit).**
Initial silicon capture showed `SR2.BUSY=1` from the very first
register read, before any transaction. Cause: PB6/PB7 floating. The
F4-DISCO has **no external pull-ups** on the I²C lines, and the
firmware was leaving `GPIOB_PUPDR` at reset (00 = no pull). Floating
SDA/SCL look like a stuck-low bus to the I²C peripheral, which
latches BUSY and refuses to start any transaction. Fix: configure
`PUPDR` bits for PB6/PB7 to `01` (internal pull-up). After the fix,
silicon `SR2` reads `0x00` at boot, matching sim.

**Sub-fix #3 — Unit test relied on the buggy ACK assumption.**
`crates/core/src/peripherals/i2c.rs::tests::test_i2c_full_transfer_flow`
wrote address `0xA0` (slave `0x50`) with no attached devices and
asserted `SR1.ADDR` was set. With sub-fix #1 that now raises AF, not
ADDR. Updated the test to attach an `Mpu6050::new(0x50)` so the
address phase ACKs and the rest of the data-flow assertions stay
meaningful.

**No ST-LINK firmware divergence this round.** V2J43S0 from Round 1.

**What this round does NOT yet cover.** Without I²C slaves wired up,
the data phase (TXE/RXNE/BTF, multi-byte read, repeated start) isn't
exercised. The simulator's recent state-machine fixes in commit
`63b3f03` cover those paths — Round 3 will land when the AHT20 +
BMP280 sensors arrive and the firmware can drive real transactions
against modeled silicon counterparts.
