# Validation — PIC16F877A (L1)

Evidence for the PIC16F877A descriptor, the new PIC16 core and its SDCC
USART/LED smoke. There is no silicon capture; everything here is
simulation-derived.

## Status

| Area | State | Evidence |
| :--- | :--- | :--- |
| Chip descriptor | ✅ | `configs/chips/pic16f877a.yaml` (DS39582B memory and core facts) |
| Device geometry | ✅ | `Pic16::for_chip_name("pic16f877a")`: 8K words, 8-level circular stack, 0x70-0x7F shared GPR (`device_table_knows_the_pic16f877a`, builder `pic16f877a_core_has_the_chip_geometry`) |
| Instruction set | ✅ | all 35 classic mid-range instructions decode, plus the legacy `OPTION`/`TRIS`; unit tests per class (`cargo test -p labwired-core --lib cpu::pic16`) |
| Banked data memory | ✅ | RP1:RP0 direct and IRP:FSR indirect addressing; core registers and the shared block mirror in every bank; TMR0/PORTB and OPTION_REG/TRISB bank 2/3 mirrors |
| Boot + C runtime | ✅ | real SDCC 4.4.0 ELF; `pic16f877a_smoke::sdcc_firmware_boots_and_prints_the_banner` |
| USART TX | ✅ | `TXREG` (0x19) to the console when `TXEN` and `SPEN` are set; `TRMT`/`TXIF` read-only |
| PORTB LED | ✅ | `pic16f877a_smoke::sdcc_firmware_drives_the_rb0_led_latch` |
| Configuration word | ✅ loaded | `sdcc_firmware_programs_the_configuration_word` (word 0x2007); not decoded |
| Session / DAP builder | ✅ | `session_builder_pic16` (banner through `Session`; the same ELF on a PIC18 chip does not print it) |
| CLI smoke | ✅ | `examples/pic16f877a/io-smoke.yaml` (ordered UART lines) |
| Determinism | ✅ | `sdcc_firmware_run_is_deterministic` (bytes, cycles, PC) |
| Unsupported instructions | ✅ 0 | 200k-step audit exits 0; a PIC16 decode error ends the run with exit 3 (`crates/cli/tests/run_exit_code.rs` uses 0x3B00, which is valid on PIC18 and unassigned on PIC16) |
| Trace contract | ✅ | `Pic16` row in `cpu_trace_conformance` |
| Silicon capture | ❌ none | simulation-derived only |

## Evidence

```bash
cargo test -p labwired-core --lib cpu::pic16
# test result: ok. 21 passed; 0 failed

cargo test -p labwired-core --test pic16f877a_smoke
# test result: ok. 5 passed; 0 failed

cargo test -p labwired-core --test session_builder_pic16
# test result: ok. 3 passed; 0 failed

cargo run -q -p labwired-cli -- test \
  --script examples/pic16f877a/io-smoke.yaml --no-uart-stdout
# PASS  2/2 checks · io-smoke · 20000 steps

bash examples/pic16f877a/ci/test.sh
# HEX->ELF re-wrap byte-identical; io-smoke PASS; audit unsupported_total 0
```

## Model notes

- **ELF machine.** PIC16 and PIC18 images are both `EM_MCHP_PIC` (204), and
  the header does not tell them apart. The loader reports PIC18; a chip
  with `arch: pic16` retargets the image (`image_arch_for_chip`). Any other
  pairing is unchanged, so an ARM image on a PIC16 chip is still refused.
- **PCL.** A read of `PCL` returns the low byte of the already-incremented
  PC, so `ADDWF PCL,F` jumps relative to the next instruction, as on
  silicon. A write loads `PCLATH<4:0>:value` and costs a second cycle.
  `GOTO`/`CALL` take `PCLATH<4:3>` for the page.
- **STATUS as a destination.** When an instruction that sets Z, DC or C
  writes STATUS, those bits take the instruction's result flags, not the
  written value. `TO`/`PD` are read-only (set by reset, `SLEEP`, `CLRWDT`).
- **Stack.** Eight levels, circular: a ninth `CALL` overwrites the oldest
  entry and there is no overflow flag.
- **PORT reads.** `PORTx` reads `latch & ~TRISx`: output pins read their
  latch and input pins read 0, because no external pin driver is modelled.
  PORTA/PORTE read as digital regardless of `ADCON1`.
- **USART.** A byte written to `TXREG` is sent at once. `TRMT` stays set,
  and `TXIF` is set whenever `TXEN` is. Baud timing and RX are not modelled.
- **Interrupts.** One vector at 0x0004. INTCON T0IF/INTF/RBIF with their
  enables, and the PIR1/PIR2 flags gated by PIE1/PIE2 and `PEIE`, all under
  `GIE`. Entry pushes the return address and clears `GIE`; `RETFIE` sets it.
  There is no hardware context save on the classic mid-range.
- **SLEEP.** Sets `TO` and clears `PD`. The core idles until an enabled
  source raises its flag, with or without `GIE`.

## Scope and limitations

Not validated, or not modelled, for this part:
- CONFIG decoding: the oscillator and `cpu_hz` come from the manifest.
- TMR1/TMR2, CCP1/2 and MSSP (SPI/I2C). TMR0 timer mode is modelled
  (Fosc/4 through the prescaler; see `examples/pic16f1829/VALIDATION.md`)
  but this smoke leaves it off.
- ADC, comparators, voltage reference and data EEPROM / flash self-write.
- WDT, USART RX and baud timing, PSP.
- PORTB interrupt-on-change and external pin drive.
- Unimplemented-register read-as-0 behaviour: unused file addresses are
  plain storage.
- The enhanced mid-range core is a separate mode of the same model, onboarded
  with the PIC16F1829 (`examples/pic16f1829`).

Known engine gaps (tracked here until fixed):
- `load_flash` still panics on an image larger than the part's flash, but
  every loader path now goes through `try_load_flash`, so `labwired run`/`test`
  report it as a config error (`EXIT_CONFIG_ERROR`) instead.
- `build_pic16_node` (the multi-node builder) attaches no serial sink, so
  USART output from a PIC16 node is not captured. Single-board `run`/`test`
  and the Session/DAP builder do capture it.

The CPU clock in the descriptor is the 5-MIPS maximum; this example's
16 MHz crystal gives 4 MIPS, and nothing in the smoke depends on time.
