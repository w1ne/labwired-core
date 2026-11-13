# Validation — PIC16F1829 (L1)

Evidence for the PIC16F1829 descriptor, the PIC16 core's enhanced mid-range
mode and its SDCC smoke. There is no silicon capture; everything here is
simulation-derived.

## Status

| Area | State | Evidence |
| :--- | :--- | :--- |
| Chip descriptor | ✅ | `configs/chips/pic16f1829.yaml` (DS40001440E memory and core facts) |
| Device geometry | ✅ | `Pic16::for_chip_name("pic16f1829")`: enhanced core, 8K words, 32 BSR banks, PORTA/B/C masks, `PIC16E_F182X_MAP` SFR layout |
| Instruction set | ✅ | all 49 enhanced mid-range instructions plus `OPTION`/`TRIS`; one unit test per new instruction class, and the encodings the enhanced core leaves unassigned (e.g. the classic `CLRW` forms other than 0x0103) are decode errors (`cargo test -p labwired-core --lib cpu::pic16`) |
| Banked data memory | ✅ | `BSR` direct addressing; core registers 0x00-0x0B and common RAM 0x70-0x7F in every bank; `WREG` addressable at 0x09 |
| FSR spaces | ✅ | banked (0x0000-0x0FFF), linear GPR (0x2000-0x29AF, 80 B/bank), program flash (0x8000-0xFFFF, low byte, read-only); `MOVIW`/`MOVWI` pre/post inc/dec and `k[FSRn]`; `ADDFSR` |
| Stack | ✅ | 16 levels, `STKPTR`/`TOSL`/`TOSH`; overflow/underflow set `PCON.STKOVF`/`STKUNF` and reset with `CONFIG2.STVREN` (wrap without) |
| Interrupt context save | ✅ | entry copies W/STATUS/BSR/PCLATH/FSR0/FSR1 into the bank-31 shadows and `RETFIE` restores them; exercised by the SDCC ISR, which saves nothing itself |
| TMR0 | ✅ timer mode | Fosc/4 through the OPTION_REG prescaler, overflow sets `TMR0IF` (both PIC16 cores); counter mode (T0CKI) never counts |
| Boot + C runtime | ✅ | real SDCC 4.4.0 ELF; `pic16f1829_smoke::sdcc_firmware_boots_and_prints_the_banner` |
| EUSART TX | ✅ | `TXREG` (0x19A) to the console when `TXEN` and `SPEN` are set; `TRMT`/`TXIF` read-only |
| LATA / RA5 LED | ✅ | `pic16f1829_smoke::sdcc_firmware_drives_the_ra5_led_latch` |
| Configuration words | ✅ loaded | `sdcc_firmware_programs_both_configuration_words` (words 0x8007/0x8008); only STVREN decoded |
| Image/part mismatch | ✅ error | a classic image on this part (and this image on the PIC16F877A) is a loader error (exit 2), not a panic (`a_classic_image_is_rejected_by_the_enhanced_part`) |
| CLI smoke | ✅ | `examples/pic16f1829/io-smoke.yaml` (eight ordered UART lines) |
| Determinism | ✅ | `sdcc_firmware_run_is_deterministic` (bytes, cycles, PC); firmware rebuild byte-identical |
| Unsupported instructions | ✅ 0 | 200k-step audit exits 0 |
| Negative control | ✅ | with the `RETFIE` shadow restore disabled the smoke fails (`TMR0 IRQ BAD`) |
| Silicon capture | ❌ none | simulation-derived only |

## Evidence

```bash
cargo test -p labwired-core --lib cpu::pic16
# test result: ok. 44 passed; 0 failed

cargo test -p labwired-core --test pic16f1829_smoke
# test result: ok. 6 passed; 0 failed

cargo run -q -p labwired-cli -- test \
  --script examples/pic16f1829/io-smoke.yaml --no-uart-stdout
# PASS  2/2 checks · io-smoke · 200000 steps

bash examples/pic16f1829/ci/test.sh
# HEX->ELF re-wrap byte-identical; io-smoke PASS; audit unsupported_total 0
```

## Model notes

- **One core type, two modes.** The device row's `enhanced` flag selects
  the enhanced mid-range behaviour inside `Pic16`. The SFR layout is a
  per-row `Pic16SfrMap`, so the ports, PIR/PIE, TMR0, OPTION_REG, PCON,
  OSCSTAT and EUSART addresses come from the table, not from constants.
- **PC and paging.** The PC is 15 bits. `GOTO`/`CALL` take `PCLATH<6:3>`,
  `CALLW` jumps to `PCLATH<6:0>:W`, `BRW` to PC + 1 + W, and `BRA` takes a
  signed 9-bit offset from PC + 1. A `PCL` write loads `PCLATH<6:0>:value`.
- **STATUS.** Bits 7-5 are unimplemented and read 0. Only C, DC and Z are
  writable. As on the classic core, an instruction that sets those flags
  overrides the value it writes to STATUS.
- **FSR to flash.** A read through `FSRn` >= 0x8000 returns the low byte of
  program word `FSRn - 0x8000`. Writes are ignored. The extra instruction
  cycle silicon takes for this access is not modelled.
- **INDF through FSR.** An FSR that points at `INDF0`/`INDF1` reads 0 and
  ignores writes.
- **Resets.** `RESET` and a STVREN stack fault re-run the power-on register
  initialisation and set or clear the PCON flags (`RI` cleared,
  `STKOVF`/`STKUNF` set). GPR keeps its contents.
- **OSCSTAT.** Reads the HFIOFR/HFIOFL/MFIOFR/LFIOFR/HFIOFS ready bits set,
  because clock start-up is not modelled.
- **USART, ports and SLEEP** behave as on the classic core (see
  `examples/pic16f877a/VALIDATION.md`): a TX byte leaves at once, `PORTx`
  reads `LATx & ~TRISx`, and `SLEEP` idles until an enabled flag.

## Scope and limitations

Not validated, or not modelled, for this part:
- CONFIG decoding beyond STVREN, clock switching, PLL, the WDT.
- TMR1/2/4/6, CCP/ECCP, MSSP1/2 (SPI/I2C), ADC, comparators, DAC, FVR, CPS,
  SR latch, data signal modulator.
- Data EEPROM and flash self-read/write through EECON1.
- EUSART RX and baud timing, interrupt-on-change, and external pin drive.
- The two-cycle TMR0 increment inhibit after a TMR0 write.
- The 64-bank enhanced parts (PIC16F188xx, 6-bit `BSR`). They are not in
  the device table.
- Unimplemented-register read-as-0 behaviour: unused file addresses are
  plain storage.

The descriptor's CPU clock is the 8-MIPS maximum. This board runs the
16 MHz HFINTOSC, 4 MIPS (`cpu_hz: 4000000` in `system.yaml`), and the smoke
only depends on instruction counts.
