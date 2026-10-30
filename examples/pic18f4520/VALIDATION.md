# Validation — PIC18F4520 (L1)

Evidence for the PIC18F4520 descriptor and its SDCC EUSART/LED smoke. There
is no silicon capture; everything here is simulation-derived.

## Status

| Area | State | Evidence |
| :--- | :--- | :--- |
| Chip descriptor | ✅ | `configs/chips/pic18f4520.yaml` (DS39631E memory and core facts) |
| Device geometry | ✅ | `Pic18::for_chip_name("pic18f4520")`: 32 KB flash, 31-level stack, Access Bank split `0x80` (`pic18f4520_access_bank_splits_at_0x80`) |
| Standard instruction set | ✅ | all 75 standard PIC18 instructions decode; per-class unit tests use the DS39631 instruction-reference examples |
| Boot + C runtime | ✅ | real SDCC 4.4.0 ELF; `pic18f4520_smoke::sdcc_firmware_boots_and_prints_the_banner` |
| EUSART TX | ✅ | `TXREG` (0xFAD) to the console when `TXEN` and `SPEN` are set; `TRMT`/`TXIF` read-only |
| PORTB LED | ✅ | `pic18f4520_smoke::sdcc_firmware_drives_the_rb0_led_latch` |
| CLI smoke | ✅ | `examples/pic18f4520/io-smoke.yaml` (ordered UART lines) |
| Determinism | ✅ | `sdcc_firmware_run_is_deterministic` (bytes, cycles, PC) |
| Unsupported instructions | ✅ 0 | 200k-step audit exits 0 (a PIC18 decode error would abort) |
| Silicon capture | ❌ none | simulation-derived only |

## Evidence

```bash
cargo test -p labwired-core --lib cpu::pic18
# test result: ok. 52 passed; 0 failed

cargo test -p labwired-core --test pic18f4520_smoke
# test result: ok. 4 passed; 0 failed

cargo run -q -p labwired-cli -- test \
  --script examples/pic18f4520/io-smoke.yaml --no-uart-stdout
# PASS  2/2 checks · io-smoke · 20000 steps

bash examples/pic18f4520/ci/test.sh
# HEX->ELF re-wrap byte-identical; io-smoke PASS; audit unsupported_total 0
```

## Model notes

- **MOVFF and the indirect aliases.** `MOVFF` takes 12-bit absolute operands
  and expands `INDFn`/`POSTINCn`/`POSTDECn`/`PREINCn`/`PLUSWn` on either side.
  SDCC's software stack depends on this; without it every string reads as
  `0x00`.
- **PORT reads.** `PORTx` reads `LATx & ~TRISx`: output pins read their latch
  and input pins read 0, because no external pin driver is modelled yet. A
  read-modify-write on `PORTx` therefore copies those zeros into `LATx`,
  which is the same hazard silicon has with undriven inputs.
- **EUSART.** A byte written to `TXREG` is sent at once. `TRMT` stays set,
  and `TXIF` is set whenever `TXEN` is. Baud-rate timing and RX are not
  modelled.
- **Table reads.** `TBLRD` reads flash, the user ID words and the CONFIG
  words the image programmed (`0xFF` where nothing was programmed).
  Unimplemented memory reads 0. The device ID words are not modelled.
  `TBLWT` only moves `TBLPTR`; no EECON1 write sequence is modelled, so flash
  never changes.
- **SLEEP.** Sets `TO` and clears `PD`. The core then idles until an
  individually enabled interrupt flag is raised, with or without `GIE`.

## Scope and limitations

Not validated, or not modelled, for this part:
- CONFIG decoding: the oscillator and `cpu_hz` come from the manifest, not
  from CONFIG1H.
- Timers 0–3, CCP/ECCP and MSSP (SPI/I2C).
- ADC, comparators, HLVD and data EEPROM.
- WDT, EUSART RX and baud timing.
- PORTB interrupt-on-change and external pin drive.
- The extended instruction set (XINST) and interrupt latency.

The CPU clock in the descriptor is the 10-MIPS maximum; this example's
16 MHz crystal gives 4 MIPS, and nothing in the smoke depends on time.
