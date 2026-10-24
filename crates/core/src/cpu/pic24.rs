// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC24 / dsPIC33F CPU — **M0 + M1a + M1b + M1c + M2 + M3 subset** for the
//! dsPIC33FJ32MC204 class.
//!
//! Scope:
//! * **M0** — reset/boot, the MOV/NOP/BRA subset needed for the UART smoke,
//!   CPU-internal UART1 transmit, deterministic stepping.
//! * **M1a** — Timers T1/T2/T3 (incl. the 32-bit T2/T3 pair), interrupt
//!   flags (IFS0), enables (IEC0), priorities (IPC0-IPC2), NSTDIS/DISI
//!   gating, exception entry with the datasheet's two-word stack frame, and
//!   RETFIE.
//! * **M1b** — the three GPIO ports (TRISx/PORTx/LATx/ODCx, pads modelled by
//!   `peripherals::pic24_gpio`), the PPS slice the capture/compare units
//!   need (RPINR7 input map, RPOR0-12 output map; RP0-15 = RB0-15,
//!   RP16-25 = RC0-9 on the 44-pin package), Input Capture 1/2 (edge and
//!   prescaler capture modes, 4-deep FIFO, ICI interrupt divider,
//!   ICOV/ICBNE) and Output Compare 1/2 (one-shot, toggle, delayed
//!   one-shot, continuous pulse, PWM modes driving their routed RPn pin).
//! * **M1c** — the DSP MAC class: `MAC`/`MPY`/`MPY.N`/`MSC` (incl. the
//!   same-register square forms), `CLR`, `MOVSAC`, `LAC`, `SAC`, `SAC.R`,
//!   the X/Y prefetches (Tables 5-7…5-10), accumulator write-back
//!   (Table 5-13), CORCON fractional/integer, signed/unsigned, rounding and
//!   saturation modes, and the OA/OB/SA/SB/OAB/SAB status bits. Verified
//!   against the DS70000157G worked examples and real XC16 firmware.
//! * **M2** — Motor Control PWM 1 (free-running/single-event/up-down time
//!   base, PTOPS/PTCKPS, three duty generators with buffering and IUE, the
//!   complementary/independent output pairs on PWM1H1/L1 = RB14/RB15,
//!   PWM1H2/L2 = RB12/RB13, PWM1H3/L3 = RB10/RB11, dead-time units A/B,
//!   P1OVDCON override, FLTA1 fault states with latched/cycle-by-cycle
//!   restore, PWM1IF postscaling) and the Quadrature Encoder Interface 1
//!   (x2/x4 decoding, POS1CNT/MAX1CNT, match and index resets with IMV,
//!   UPDN/UPDN pin, CNTERR/CEID, the digital filter and the 16-bit
//!   timer/counter mode). IFS3/IEC3/IPC14/IPC15 are wired into the interrupt
//!   fabric (PWM1 = vector 65, QEI = 66, FLTA1 = 71).
//! * **M3** — ADC1 (10/12-bit, FORM formats, SAMP/DONE/ASAM, SSRC manual /
//!   TMR3 / PWM1 / internal counter, CHPS sequences, CSCNA scan list,
//!   BUFM split buffer, SMPI interrupts, TAD/SAMC timing), SPI1 (master and
//!   slave register model, 8/16-bit, SPRE/PPRE timing, SPIROV, framed-mode
//!   registers, loopback/attached `SpiDevice`s) and I2C1 (master
//!   start/restart/stop, 7/10-bit address decode, transmit/receive/ACK,
//!   ACKSTAT, BRG timing, attached `I2cDevice`s such as the TMP102, plus
//!   slave address-match/test seams). IFS1/IEC1/IPC3/IPC4 are wired into
//!   the fabric (SPI1E = 17, SPI1 = 18, ADC1 = 21, SI2C1 = 24, MI2C1 = 25).
//!
//! Not yet modelled — unsupported opcodes return
//! [`SimulationError::DecodeError`] rather than guessing: PSV/EDS windows,
//! the DSP *accumulator* arithmetic
//! (`ADD`/`SUB`/`MAX`/`MIN`/`SFTAC`/`FBCL` on ACCA/ACCB). Change
//! notification (CNENx/CNPUx) and the remaining PPS functions (UART/SPI/
//! INT/QEI routing) are not modelled: the PPS registers store values, but
//! only the IC1/IC2 inputs, OC1/OC2 outputs, PWM outputs, QEI inputs and the
//! UPDN output are consumed.
//!
//! Known M2 approximations (documented, not guessed; the PWM/QEI state
//! machines follow DS70187E §14 and DS70208C §15):
//! * PWM output polarity is active-high and PWMPIN = 1 is assumed (the
//!   erased-device FPOR values), so PWM1CON1 resets to 0 and the pins are
//!   GPIO until PENyH/PENyL are set.
//! * Center-aligned counting keeps both endpoint ticks (the 2*(PTPER+1)
//!   period of Equation 14-2); the compare is evaluated with the phase
//!   direction before the endpoint flip. The FRM's text does not pin the
//!   exact endpoint tick order.
//! * QEI index recognition uses the rising edge of the filtered INDX input
//!   with the IMV match, rather than latching a quadrature-state transition
//!   (DS70208C §15.5.3.1); diagonal (illegal) quadrature transitions are
//!   ignored.
//! * The QEI digital filter advances once per instruction cycle with the
//!   QECK divider and accepts a level after three equal samples.
//! * PWM/QEI inputs are sampled once per retired instruction step (the same
//!   granularity as the M1b capture units).
//!
//! Known M1c approximations (documented, not guessed; every other M1c
//! behaviour is pinned by a DS70000157G example — see the tests):
//! * `LAC`'s shifter is 32-bit, which the PRM's examples 1-2 pin exactly;
//!   with super saturation enabled and an overflowing left shift the exact
//!   40-bit result is kept (no PRM example covers this combination).
//! * `MPY.N` lists only OA/OB/OAB as affected, so it stores the negated
//!   product without saturation or SA/SB (the page is followed literally).
//! * Accumulator write-back applies SATDW like `SAC.R` (the PRM mentions
//!   saturation only for SAC/SAC.R; no write-back example saturates).
//!
//! Known M1b approximations (documented, not guessed):
//! * IC capture latency (2 instruction cycles, DS70198D Figure 12-2) is not
//!   modelled: pad edges are sampled once per retired step and capture the
//!   timer value after that step's tick.
//! * The OCxR = 0 "match on the second timer cycle" case (DS70209A
//!   Figure 13-6) is approximated by the plain equality compare.
//! * Output Compare cannot use the T2/T3 32-bit pair (T32 = 1).
//! * IC mode 111 (sleep/idle wake only, DS70198D Register 12-1) is stored
//!   but never captures — there is no sleep mode to wake from.
//!
//! Conventions (verified against a real XC16 v1.70-built ELF, committed at
//! `examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-uart-smoke.elf`):
//! * `pc` is a **PC-unit address**: two units per instruction word, so
//!   instructions sit at even addresses and the PC advances by 2. This is
//!   the datasheet's addressing (`PC<0>` always 0; interrupt vectors at
//!   `0x4 + 2*N`) and the XC16 guide's "Program Counter Unit".
//! * Internally each instruction occupies one 4-byte little-endian slot
//!   (`INSTR_SLOT_BYTES`, slot index = `pc / 2`). XC16 **ELF executable
//!   sections store exactly this 4-byte slot form** — the 24-bit word plus
//!   one pad byte per instruction — and the parsers copy them verbatim
//!   (verified: every executable section of the real ELF has a size that is
//!   a multiple of 4, and the disassembler decodes with a 4-byte stride).
//!   DS50002071F's "in an executable section, 2 program counter units are
//!   equivalent to 3 bytes" describes the 16-bit assembler's *conceptual*
//!   address arithmetic, not ELF file storage; "phantom bytes" are the pad
//!   bytes of that 4-byte container (DS50002071F glossary).
//! * Interrupt vectors are **data**: each vector location holds the 24-bit
//!   starting address of its ISR (DS70283H §7.1: "Each interrupt vector
//!   contains a 24-bit-wide address"). Exception entry loads the PC *from*
//!   the vector; it does not execute code placed at the vector address.
//! * Non-executable PIC24 program-memory sections (read-only data reached
//!   through PSV/table reads) are skipped at load — PSV is not modelled.
//! * Data space: `0x0000–0x07FF` SFR (UART1, timers, interrupt registers,
//!   ports, capture/compare modelled; the rest is a shadow latch),
//!   `0x0800–0x0FFF` 2 KB SRAM. Anything else is a
//!   [`SimulationError::MemoryViolation`] — never silently dropped.
//! * Instruction encodings are transcribed from DS70000157G (16-Bit MCU and
//!   DSC Programmer's Reference Manual); see the reference doc for the table.
//! * I/O-port semantics are pinned to the dsPIC33F FRM §30 "I/O Ports"
//!   (DS70190D): writes to `PORTx` write the latch, reads read the pin;
//!   `ODCx` makes the pad open-drain. The CPU keeps the registers and
//!   mirrors `TRIS`/`LAT`/`ODC` (plus its Output Compare drive) to the
//!   bus-side pad model at `PIC24_IO_MIRROR_BASE + address`.
//! * Input capture semantics are pinned to FRM §12 (DS70198D) and output
//!   compare to FRM §13 (DS70209A); ICxIF/OCxIF share the IFS0/IEC0/IPC1-2
//!   fabric of M1a.
//!
//! ## Modelled instructions
//!
//! Base integer ISA needed by real XC16 code (encodings transcribed from
//! DS70000157G; `MOV #lit16`/`MOV Ws,f`/`MOV f,Wd` predate this list):
//!
//! | Instruction | Notes |
//! | :--- | :--- |
//! | `NOP`, `MOV #lit16,Wn`, `MOV Ws,f`, `MOV f,Wn` | M0 |
//! | `MOV.B #lit8,Wn`, `MOV{.B} Ws,Wd` | all ggg/hhh modes except `[Ws+Wb]` |
//! | `CLR{.B} f`, `SETM{.B} f` | D=0/1 (WREG or file register) |
//! | `ADD{.B} Wb,#lit5,Wd`, `ADD{.B} Wb,Ws,Wd` | |
//! | `SUB{.B} Wb,#lit5,Wd`, `SUB{.B} Wb,Ws,Wd` | |
//! | `CLR Wd`, `SETM Wd` (WREG forms) | no flags (Table 22-2) |
//! | `INC{.B} Ws,Wd` | C/DC/N/OV/Z |
//! | `AND{.B} Wb,#lit5,Wd`, `AND{.B} Wb,Ws,Wd`, `AND #lit10,Wn` | |
//! | `IOR{.B} Wb,#lit5,Wd`, `IOR{.B} Wb,Ws,Wd`, `IOR #lit10,Wn` | |
//! | `CP0{.B} Ws`, `CP0{.B} f` | |
//! | `BSET/BCLR/BTG{.B} f,#bit`, `BSET/BCLR/BTG{.B} Ws,#bit` | |
//! | `SE [Ws],Wd` (and register form) | |
//! | `LSR Wb,#lit4,Wnd` | |
//! | `BRA cond,Expr` (all 15 condition codes), `RCALL Expr` | |
//! | `CALL Expr`, `GOTO Expr` | 2-word instructions, PC advances by 4 |
//! | `RETURN`, `RETLW{.B} #lit10,Wn`, `LNK #lit14`, `ULNK` | |
//! | `RETFIE` | M1a |
//!
//! DSP MAC class (M1c, DS70000157G §4.15/§5.4), all with the X/Y prefetch
//! tables, write-back table and CORCON modes:
//!
//! | Instruction | Notes |
//! | :--- | :--- |
//! | `MAC/MPY/MPY.N/MSC Wm*Wn, Acc {,AWB}` | `1100 0mmm`, Tables 5-12/5-13 |
//! | `MAC/MPY Wm*Wm, Acc` | `1111 00mm`, Table 5-11, no write-back |
//! | `CLR Acc {,AWB}` | clears the accumulator and its OA/SA or OB/SB |
//! | `MOVSAC Acc {,AWB}` | prefetch + write-back only, no status |
//! | `LAC Ws/[...], {#Slit4,} Acc` | 32-bit shifter (PRM examples 1-2) |
//! | `SAC`/`SAC.R Acc, {#Slit4,} Wd/[...]` | 40-bit shift, SATDW, RND |
//!
//! Not yet implemented — these return [`SimulationError::DecodeError`]:
//! `TBLRD*`/`TBLW*` (PSV is not modelled), `ADDC`/`SUBB`/`CPB` and their
//! sticky Z flag, the `[Ws+Wb]` register-offset mode for the base ISA, the
//! DSP accumulator arithmetic (`ADD`/`SUB`/`MAX`/`MIN`/`SFTAC`/`FBCL`),
//! `REPEAT`/`DO`, `PUSH`/`POP`, `RESET`/`BREAK`.
//!
//! UART1 registers (DS70283H): `U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224`.
//! M2 peripherals (DS70283H Tables 4-8/4-10): `P1TCON 0x01C0` …
//! `P1DC3 0x01DA`, `QEI1CON 0x01E0` … `MAX1CNT 0x01E6`; the QEI inputs are
//! mapped by `RPINR14/15` and the FLTA1 fault input by `RPINR12`.

use crate::peripherals::device::{I2cDevice, SpiDevice};
use crate::snapshot::{CpuSnapshot, Pic24CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Flash size of the dsPIC33FJ32MC204 (DS70283H): 32 KB.
pub const FLASH_BYTES: usize = 32 * 1024;
/// Internal storage per instruction: 24-bit word in a 4-byte little-endian
/// slot. This **is** the XC16 ELF layout: executable sections store one such
/// slot per instruction and the parsers copy them verbatim
/// ([`check_xc16_section`]).
pub const INSTR_SLOT_BYTES: usize = 4;
/// PC units per instruction: the PC advances by 2 (DS70283H §4; the
/// "Program Counter Unit" in DS50002071F).
pub const PC_UNITS_PER_INSTRUCTION: u32 = 2;
/// End of user program flash in PC units: 0x000000–0x0057FE on the
/// dsPIC33FJ32MC204 (DS70283H Figure 4-1), i.e. 11264 instructions, which
/// XC16 packs into 33,792 bytes (3 bytes each).
pub const PROGRAM_PC_END: u32 = 0x5800;
/// Instruction slots available in flash.
pub const INSTR_SLOTS: usize = (PROGRAM_PC_END / PC_UNITS_PER_INSTRUCTION) as usize;
/// Internal flash storage: one 4-byte slot per instruction (the XC16 ELF
/// form). Larger than [`FLASH_BYTES`]: program space is 0x5800 PC units =
/// 11264 instructions, and each occupies a 4-byte slot here.
pub const FLASH_STORAGE_BYTES: usize = INSTR_SLOTS * INSTR_SLOT_BYTES;

/// Reject an odd XC16 program address. Program addresses are PC units and an
/// instruction always starts on an even one (DS70283H §4); rounding an odd
/// address down would silently load or start one instruction early.
pub fn check_xc16_pc_address(what: &str, addr: u64) -> Result<(), String> {
    if addr % u64::from(PC_UNITS_PER_INSTRUCTION) != 0 {
        return Err(format!(
            "PIC24 {what} {addr:#x} is odd; XC16 program addresses are even PC units (DS70283H §4)"
        ));
    }
    Ok(())
}

/// Reject an XC16 entry point that is odd or not in program flash
/// (0x0..0x5800, DS70283H Figure 4-1): the CPU would start fetching nothing.
pub fn check_xc16_entry(addr: u64) -> Result<(), String> {
    check_xc16_pc_address("entry point", addr)?;
    if addr >= u64::from(PROGRAM_PC_END) {
        return Err(format!(
            "PIC24 entry point {addr:#x} is past program flash 0x0..{PROGRAM_PC_END:#x} \
             (DS70283H Figure 4-1)"
        ));
    }
    Ok(())
}

/// Reject an XC16 executable **section** that is not a whole number of
/// 4-byte slots, is odd, starts past program flash, or runs off its end
/// (0x0057FE, DS70283H Figure 4-1). `data` is the raw section from the ELF
/// (already in the CPU's slot form — see [`INSTR_SLOT_BYTES`]). Loading would
/// otherwise truncate it silently.
///
/// Verified against a real XC16 v1.70 ELF: every executable section
/// (`.text`, `.reset`, `.ivt.*`, `.aivt.*`) has a size that is a multiple
/// of 4, and the 24-bit word sits in the low three bytes of each slot.
pub fn check_xc16_section(addr: u64, data: &[u8]) -> Result<(), String> {
    if data.is_empty() || data.len() % INSTR_SLOT_BYTES != 0 {
        return Err(format!(
            "PIC24 executable section at {addr:#x} is {} bytes — not a whole number of \
             4-byte XC16 slots (DS50002071F)",
            data.len()
        ));
    }
    check_xc16_span(addr, (data.len() / INSTR_SLOT_BYTES) as u64)
}

/// [`check_xc16_section`] from the instruction count alone, so a section can
/// be checked from its header before its bytes are read.
pub fn check_xc16_span(addr: u64, instructions: u64) -> Result<(), String> {
    check_xc16_pc_address("executable section", addr)?;
    let end = addr.saturating_add(instructions * u64::from(PC_UNITS_PER_INSTRUCTION));
    if addr >= u64::from(PROGRAM_PC_END) || end > u64::from(PROGRAM_PC_END) {
        return Err(format!(
            "PIC24 executable section {addr:#x}..{end:#x} does not fit program flash \
             0x0..{PROGRAM_PC_END:#x} (DS70283H Figure 4-1)"
        ));
    }
    Ok(())
}
/// Data-space SRAM base (X RAM), DS70283H Figure 4-3.
pub const RAM_BASE: u32 = 0x0800;
/// 2 KB SRAM.
pub const RAM_BYTES: usize = 2 * 1024;
/// SFR space size (0x0000–0x07FF).
pub const SFR_BYTES: usize = 0x0800;

/// UART1 mode register.
pub const SFR_U1MODE: u16 = 0x0220;
/// UART1 status register.
pub const SFR_U1STA: u16 = 0x0222;
/// UART1 transmit register (write a byte here).
pub const SFR_U1TXREG: u16 = 0x0224;

/// U1STA.TRMT — transmit shift register empty (DS70283H U1STA bit 8).
pub const U1STA_TRMT: u16 = 0x0100;
/// U1STA.UTXBF — transmit buffer full (DS70283H U1STA bit 9).
pub const U1STA_UTXBF: u16 = 0x0200;

// ── DS70283H SFR map (M1a additions) ────────────────────────────────────────

/// Accumulator A low / high / upper words.
pub const SFR_ACCAL: u16 = 0x0022;
pub const SFR_ACCAH: u16 = 0x0024;
/// Accumulator A upper word (bits 39:32).
pub const SFR_ACCAU: u16 = 0x0026;
pub const SFR_ACCBL: u16 = 0x0028;
pub const SFR_ACCBH: u16 = 0x002A;
pub const SFR_ACCBU: u16 = 0x002C;
/// CPU status register.
pub const SFR_SR: u16 = 0x0042;
/// Core control register (IPL3 lives in bit 3).
pub const SFR_CORCON: u16 = 0x0044;
/// Interrupt control registers.
pub const SFR_INTCON1: u16 = 0x0080;
pub const SFR_INTCON2: u16 = 0x0082;
/// Interrupt flag status 0.
pub const SFR_IFS0: u16 = 0x0084;
/// Interrupt enable control 0.
pub const SFR_IEC0: u16 = 0x0094;
/// Interrupt priority control 0/1/2.
pub const SFR_IPC0: u16 = 0x00A4;
pub const SFR_IPC1: u16 = 0x00A6;
pub const SFR_IPC2: u16 = 0x00A8;
/// Timer registers (DS70283H Table 4-x).
pub const SFR_TMR1: u16 = 0x0100;
pub const SFR_PR1: u16 = 0x0102;
pub const SFR_T1CON: u16 = 0x0104;
pub const SFR_TMR2: u16 = 0x0106;
pub const SFR_TMR3HLD: u16 = 0x0108;
pub const SFR_TMR3: u16 = 0x010A;
pub const SFR_PR2: u16 = 0x010C;
pub const SFR_PR3: u16 = 0x010E;
pub const SFR_T2CON: u16 = 0x0110;
pub const SFR_T3CON: u16 = 0x0112;

/// TCON.TON — timer on.
pub const TCON_TON: u16 = 0x8000;
/// TCON.TCKPS mask / shift (`00`=1:1, `01`=1:8, `10`=1:64, `11`=1:256).
pub const TCON_TCKPS_MASK: u16 = 0x0030;
pub const TCON_TCKPS_SHIFT: u16 = 4;
/// T2CON.T32 — Timer2/3 32-bit pair mode.
pub const T2CON_T32: u16 = 0x0008;

/// IFS0 flag bit and vector number for each modelled source (DS70283H
/// Table 7-1). The vector number is the table's *Vector Number* column
/// (`IRQ + 8`); IFS0 bit 4 is reserved, so IC2/OC2/T2/T3 are one vector
/// higher than the bit index + 7 (fixed in M2: the previous table was off
/// by one for those four sources).
pub const IRQ_INT0: (u16, u32) = (0, 8);
pub const IRQ_IC1: (u16, u32) = (1, 9);
pub const IRQ_OC1: (u16, u32) = (2, 10);
pub const IRQ_T1: (u16, u32) = (3, 11);
pub const IRQ_IC2: (u16, u32) = (5, 13);
pub const IRQ_OC2: (u16, u32) = (6, 14);
pub const IRQ_T2: (u16, u32) = (7, 15);
pub const IRQ_T3: (u16, u32) = (8, 16);

/// IFS3 flag bit and vector number for the M2 sources (DS70283H Table 7-1:
/// PWM1 = vector 65, QEI = vector 66, FLTA1 = vector 71). Their IRQ numbers
/// (57/58/63) drive the IPC mapping: IPC14 fields 1/2 and IPC15 field 3.
pub const IRQ_PWM1: (u16, u32) = (8, 65);
pub const IRQ_QEI: (u16, u32) = (9, 66);
pub const IRQ_FLTA1: (u16, u32) = (15, 71);

/// M3 interrupt sources (DS70283H Table 7-1): SPI1E = vector 17 / IRQ 9,
/// SPI1 = 18 / 10, ADC1 = 21 / 13, SI2C1 = 24 / 16, MI2C1 = 25 / 17.
pub const IRQ_SPI1E: (u16, u32) = (9, 17);
pub const IRQ_SPI1: (u16, u32) = (10, 18);
pub const IRQ_AD1: (u16, u32) = (13, 21);
pub const IRQ_SI2C1: (u16, u32) = (0, 24);
pub const IRQ_MI2C1: (u16, u32) = (1, 25);

// ── M3: ADC1 / SPI1 / I2C1 (DS70283H Tables 4-13/4-15/4-11) ─────────────────

/// ADC1 result buffer 0 (buffers 0-15 at 0x0300-0x031E).
pub const SFR_ADC1BUF0: u16 = 0x0300;
/// Number of 16-bit ADC1 result buffers.
pub const ADC1_BUF_COUNT: usize = 16;
/// AD1CON1: ADON/AD12B/FORM/SSRC/SIMSAM/ASAM/SAMP/DONE.
pub const SFR_AD1CON1: u16 = 0x0320;
/// AD1CON2: VCFG/CSCNA/CHPS/BUFS/SMPI/BUFM/ALTS.
pub const SFR_AD1CON2: u16 = 0x0322;
/// AD1CON3: ADRC/SAMC/ADCS.
pub const SFR_AD1CON3: u16 = 0x0324;
/// AD1CHS123: CH1/2/3 positive and negative input selects.
pub const SFR_AD1CHS123: u16 = 0x0326;
/// AD1CHS0: CH0 positive and negative input selects.
pub const SFR_AD1CHS0: u16 = 0x0328;
/// AD1PCFGL: PCFG8..PCFG0 analog-input enables.
pub const SFR_AD1PCFGL: u16 = 0x032C;
/// AD1CSSL: CSS8..CSS0 scan list.
pub const SFR_AD1CSSL: u16 = 0x0330;

/// SPI1 status register.
pub const SFR_SPI1STAT: u16 = 0x0240;
/// SPI1 control register 1.
pub const SFR_SPI1CON1: u16 = 0x0242;
/// SPI1 control register 2 (framed mode).
pub const SFR_SPI1CON2: u16 = 0x0244;
/// SPI1 transmit/receive buffer.
pub const SFR_SPI1BUF: u16 = 0x0248;

/// I2C1 receive register.
pub const SFR_I2C1RCV: u16 = 0x0200;
/// I2C1 transmit register.
pub const SFR_I2C1TRN: u16 = 0x0202;
/// I2C1 baud-rate generator.
pub const SFR_I2C1BRG: u16 = 0x0204;
/// I2C1 control register.
pub const SFR_I2C1CON: u16 = 0x0206;
/// I2C1 status register.
pub const SFR_I2C1STAT: u16 = 0x0208;
/// I2C1 slave address.
pub const SFR_I2C1ADD: u16 = 0x020A;
/// I2C1 slave address mask.
pub const SFR_I2C1MSK: u16 = 0x020C;

/// IFS1 / IEC1 / IPC3 / IPC4 — ADC1, SPI1 and I2C1 flags and priorities.
pub const SFR_IFS1: u16 = 0x0086;
pub const SFR_IEC1: u16 = 0x0096;
pub const SFR_IPC3: u16 = 0x00AA;
pub const SFR_IPC4: u16 = 0x00AC;

/// AD1CON1 bits.
pub const AD1CON1_ADON: u16 = 0x8000;
pub const AD1CON1_AD12B: u16 = 0x0400;
pub const AD1CON1_SAMP: u16 = 0x0002;
pub const AD1CON1_DONE: u16 = 0x0001;
/// AD1CON2 bits.
pub const AD1CON2_CSCNA: u16 = 0x0400;
pub const AD1CON2_BUFS: u16 = 0x0080;
pub const AD1CON2_BUFM: u16 = 0x0002;
pub const AD1CON2_ALTS: u16 = 0x0001;
/// SPI1STAT bits.
pub const SPI1STAT_SPIEN: u16 = 0x8000;
pub const SPI1STAT_SPIROV: u16 = 0x0040;
pub const SPI1STAT_SPITBF: u16 = 0x0002;
pub const SPI1STAT_SPIRBF: u16 = 0x0001;
/// SPI1CON1 bits.
pub const SPI1CON1_MSTEN: u16 = 0x0020;
pub const SPI1CON1_MODE16: u16 = 0x0400;
/// I2C1CON bits.
pub const I2C1CON_I2CEN: u16 = 0x8000;
pub const I2C1CON_A10M: u16 = 0x0400;
pub const I2C1CON_ACKDT: u16 = 0x0020;
pub const I2C1CON_ACKEN: u16 = 0x0010;
pub const I2C1CON_RCEN: u16 = 0x0008;
pub const I2C1CON_PEN: u16 = 0x0004;
pub const I2C1CON_RSEN: u16 = 0x0002;
pub const I2C1CON_SEN: u16 = 0x0001;
/// I2C1STAT bits.
pub const I2C1STAT_ACKSTAT: u16 = 0x8000;
pub const I2C1STAT_TRSTAT: u16 = 0x4000;
pub const I2C1STAT_BCL: u16 = 0x0400;
pub const I2C1STAT_ADD10: u16 = 0x0100;
pub const I2C1STAT_IWCOL: u16 = 0x0080;
pub const I2C1STAT_I2COV: u16 = 0x0040;
pub const I2C1STAT_DA: u16 = 0x0020;
pub const I2C1STAT_P: u16 = 0x0010;
pub const I2C1STAT_S: u16 = 0x0008;
pub const I2C1STAT_RW: u16 = 0x0004;
pub const I2C1STAT_RBF: u16 = 0x0002;
pub const I2C1STAT_TBF: u16 = 0x0001;

/// INTCON1.NSTDIS — nesting disable (bit 15).
pub const INTCON1_NSTDIS: u16 = 0x8000;
/// INTCON2.DISI — disable interrupts 1-6 (bit 14).
pub const INTCON2_DISI: u16 = 0x4000;

/// Exception entry costs two extra instruction cycles (DS70283H §7).
pub const EXCEPTION_ENTRY_CYCLES: u32 = 2;
/// RETFIE takes three instruction cycles (DS70000157G).
pub const RETFIE_CYCLES: u32 = 3;

/// RETFIE — 0000 0110 0100 0000 0000 0000 (DS70000157G): bits 19-16 = 0110,
/// bits 15-12 = 0100, i.e. `0x0006_4000` as a 24-bit word.
pub const INSN_RETFIE: u32 = 0x0006_4000;

// ── SR flags (DS70283H Register 7-1 / DS70000157G §2.16) ────────────────────

/// SR.C — carry (for subtracts: 1 = no borrow, DS70000157G Table 4-8).
pub const SR_C: u16 = 0x0001;
/// SR.Z — zero.
pub const SR_Z: u16 = 0x0002;
/// SR.OV — overflow.
pub const SR_OV: u16 = 0x0004;
/// SR.N — negative.
pub const SR_N: u16 = 0x0008;
/// SR.DC — digit carry.
pub const SR_DC: u16 = 0x0100;

/// INTCON2.ALTIVT — use the Alternate IVT at `0x104` (DS70283H §7.1).
pub const INTCON2_ALTIVT: u16 = 0x8000;

// ── CORCON (DS70283H Register 3-2) ──────────────────────────────────────────

/// CORCON.IF — 1 = integer multiplies, 0 = fractional (left-shifted by one,
/// DS70000157G §4.12.1).
pub const CORCON_IF: u16 = 0x0001;
/// CORCON.RND — 1 = biased (conventional) rounding, 0 = unbiased
/// (convergent) rounding.
pub const CORCON_RND: u16 = 0x0002;
/// CORCON.ACCSAT — 1 = 9.31 super saturation, 0 = 1.31 normal saturation.
pub const CORCON_ACCSAT: u16 = 0x0010;
/// CORCON.SATDW — data-space write saturation from the DSP engine.
pub const CORCON_SATDW: u16 = 0x0020;
/// CORCON.SATB / CORCON.SATA — per-accumulator saturation enables.
pub const CORCON_SATB: u16 = 0x0040;
pub const CORCON_SATA: u16 = 0x0080;
/// CORCON.US — 1 = unsigned multiplies.
pub const CORCON_US: u16 = 0x1000;

// ── DSP ALU status bits, SR high byte (DS70000157G §2.16.4) ─────────────────

/// SR.OA — accumulator A overflowed into the guard bits.
pub const SR_OA: u16 = 0x8000;
/// SR.OB — accumulator B overflowed into the guard bits.
pub const SR_OB: u16 = 0x4000;
/// SR.SA — accumulator A saturated (sticky).
pub const SR_SA: u16 = 0x2000;
/// SR.SB — accumulator B saturated (sticky).
pub const SR_SB: u16 = 0x1000;
/// SR.OAB — OA || OB (derived).
pub const SR_OAB: u16 = 0x0800;
/// SR.SAB — SA || SB (derived; clearing it clears SA and SB).
pub const SR_SAB: u16 = 0x0400;

/// Signed 32-bit range — the 1.31 normal-saturation limits.
const ACC32_MAX: i64 = (1 << 31) - 1;
const ACC32_MIN: i64 = -(1 << 31);
/// Signed 40-bit range — the 9.31 super-saturation limits.
const ACC40_MAX: i64 = (1 << 39) - 1;
const ACC40_MIN: i64 = -(1 << 39);

/// `true` when `value` fits the signed 32-bit range.
fn fits_signed32(value: i64) -> bool {
    (ACC32_MIN..=ACC32_MAX).contains(&value)
}

/// `true` when `value` fits the signed 40-bit range.
fn fits_signed40(value: i64) -> bool {
    (ACC40_MIN..=ACC40_MAX).contains(&value)
}

/// Wrap an i64 to the 40-bit accumulator width, sign-extended.
fn wrap40(value: i64) -> i64 {
    let masked = value & 0xFF_FFFF_FFFF;
    if masked >= (1 << 39) {
        masked - (1 << 40)
    } else {
        masked
    }
}

// ── DS70283H / DS70190D / DS70198D / DS70209A (M1b additions) ───────────────

/// The window the CPU mirrors port-register writes to
/// (`peripherals::pic24_gpio`): `PIC24_IO_MIRROR_BASE + data-space address`.
pub const PIC24_IO_MIRROR_BASE: u64 = 0x0001_0000;

/// I/O port registers (DS70283H Table 4-19 … 4-22).
pub const SFR_TRISA: u16 = 0x02C0;
pub const SFR_PORTA: u16 = 0x02C2;
pub const SFR_LATA: u16 = 0x02C4;
pub const SFR_ODCA: u16 = 0x02C6;
pub const SFR_TRISB: u16 = 0x02C8;
pub const SFR_PORTB: u16 = 0x02CA;
pub const SFR_LATB: u16 = 0x02CC;
pub const SFR_ODCB: u16 = 0x02CE;
pub const SFR_TRISC: u16 = 0x02D0;
pub const SFR_PORTC: u16 = 0x02D2;
pub const SFR_LATC: u16 = 0x02D4;
pub const SFR_ODCC: u16 = 0x02D6;

/// Pads on the 44-pin dsPIC33FJ32MC204 (DS70283H Table 4-20/21/22 TRISx
/// reset values: RA0-4/RA7-10, RB0-15, RC0-9).
pub const PORT_A_MASK: u16 = 0x079F;
pub const PORT_B_MASK: u16 = 0xFFFF;
pub const PORT_C_MASK: u16 = 0x03FF;

/// Input capture registers (DS70283H Table 4-x, addresses 0x0140-0x0146).
pub const SFR_IC1BUF: u16 = 0x0140;
pub const SFR_IC1CON: u16 = 0x0142;
pub const SFR_IC2BUF: u16 = 0x0144;
pub const SFR_IC2CON: u16 = 0x0146;

/// Output compare registers (0x0180-0x018A).
pub const SFR_OC1RS: u16 = 0x0180;
pub const SFR_OC1R: u16 = 0x0182;
pub const SFR_OC1CON: u16 = 0x0184;
pub const SFR_OC2RS: u16 = 0x0186;
pub const SFR_OC2R: u16 = 0x0188;
pub const SFR_OC2CON: u16 = 0x018A;

/// PPS input map (IC1R<4:0>, IC2R<12:8>) and the first output register;
/// RPOR0-12 are consecutive 16-bit registers (0x06C0-0x06D8).
pub const SFR_RPINR7: u16 = 0x068E;
pub const SFR_RPOR0: u16 = 0x06C0;
/// Number of RPORn registers on this part (RP0-RP25 = 26 pins, 2 per reg).
pub const RPOR_COUNT: usize = 13;

/// ICxCON bits (DS70198D Register 12-1).
pub const ICCON_ICSIDL: u16 = 1 << 13;
pub const ICCON_ICTMR: u16 = 1 << 7;
pub const ICCON_ICI_MASK: u16 = 0x0060;
pub const ICCON_ICI_SHIFT: u16 = 5;
pub const ICCON_ICOV: u16 = 1 << 4;
pub const ICCON_ICBNE: u16 = 1 << 3;
pub const ICCON_ICM_MASK: u16 = 0x0007;

/// OCxCON bits (DS70209A Register 13-1; OCFLT is read-only).
pub const OCCON_OCSIDL: u16 = 1 << 13;
pub const OCCON_OCFLT: u16 = 1 << 4;
pub const OCCON_OCTSEL: u16 = 1 << 3;
pub const OCCON_OCM_MASK: u16 = 0x0007;

/// PPS output function codes (DS70283H Table 10-2).
pub const PPS_OUT_OC1: u16 = 0b1_0010;
pub const PPS_OUT_OC2: u16 = 0b1_0011;
/// UPDN (QEI direction) output function (DS70283H Table 10-2: 11010).
pub const PPS_OUT_UPDN: u16 = 0b1_1010;

// ── M2: Motor Control PWM (DS70283H Table 4-8) ──────────────────────────────

pub const SFR_P1TCON: u16 = 0x01C0;
pub const SFR_P1TMR: u16 = 0x01C2;
pub const SFR_P1TPER: u16 = 0x01C4;
pub const SFR_P1SECMP: u16 = 0x01C6;
pub const SFR_PWM1CON1: u16 = 0x01C8;
pub const SFR_PWM1CON2: u16 = 0x01CA;
pub const SFR_P1DTCON1: u16 = 0x01CC;
pub const SFR_P1DTCON2: u16 = 0x01CE;
pub const SFR_P1FLTACON: u16 = 0x01D0;
pub const SFR_P1OVDCON: u16 = 0x01D4;
pub const SFR_P1DC1: u16 = 0x01D6;
pub const SFR_P1DC2: u16 = 0x01D8;
pub const SFR_P1DC3: u16 = 0x01DA;

/// P1TCON bits (DS70187E Register 14-1).
pub const PTCON_PTEN: u16 = 0x8000;
pub const PTCON_PTSIDL: u16 = 0x2000;
pub const PTCON_PTOPS_MASK: u16 = 0x00F0;
pub const PTCON_PTCKPS_MASK: u16 = 0x000C;
pub const PTCON_PTMOD_MASK: u16 = 0x0003;

/// PWM1CON2 bits (DS70187E Register 14-6).
pub const PWM2CON_IUE: u16 = 0x0004;
pub const PWM2CON_OSYNC: u16 = 0x0002;
pub const PWM2CON_UDIS: u16 = 0x0001;

/// The six PWM1 output pins on the 44-pin package (DS70283H Table 1-1):
/// (port index, bit) for pairs 1-3, high then low.
pub const PWM1_PINS: [((usize, u8), (usize, u8)); 3] = [
    ((1, 14), (1, 15)), // PWM1H1/L1 = RB14/RB15
    ((1, 12), (1, 13)), // PWM1H2/L2 = RB12/RB13
    ((1, 10), (1, 11)), // PWM1H3/L3 = RB10/RB11
];

// ── M2: Quadrature Encoder Interface (DS70283H Table 4-10) ─────────────────

pub const SFR_QEI1CON: u16 = 0x01E0;
pub const SFR_DFLT1CON: u16 = 0x01E2;
pub const SFR_POS1CNT: u16 = 0x01E4;
pub const SFR_MAX1CNT: u16 = 0x01E6;

/// QEI1CON bits (DS70208C Register 15-1).
pub const QEICON_CNTERR: u16 = 0x8000;
pub const QEICON_INDEX: u16 = 0x1000;
pub const QEICON_UPDN: u16 = 0x0800;
pub const QEICON_QEIM_MASK: u16 = 0x0700;
pub const QEICON_SWPAB: u16 = 0x0080;
pub const QEICON_PCDOUT: u16 = 0x0040;
pub const QEICON_TQGATE: u16 = 0x0020;
pub const QEICON_TQCKPS_MASK: u16 = 0x0018;
pub const QEICON_POSRES: u16 = 0x0004;
pub const QEICON_TQCS: u16 = 0x0002;
pub const QEICON_UDSRC: u16 = 0x0001;

/// DFLT1CON bits (DS70208C Register 15-2).
pub const DFLT_IMV_MASK: u16 = 0x0600;
pub const DFLT_CEID: u16 = 0x0100;
pub const DFLT_QEOUT: u16 = 0x0080;
pub const DFLT_QECK_MASK: u16 = 0x0070;

/// PPS inputs used by M2 (DS70283H Table 4-2).
pub const SFR_RPINR12: u16 = 0x0698; // FLTA1R<4:0>
pub const SFR_RPINR14: u16 = 0x069C; // QEB1R<12:8>, QEA1R<4:0>
pub const SFR_RPINR15: u16 = 0x069E; // INDX1R<4:0>

/// IFS3/IEC3/IPC14/IPC15 (DS70283H Table 7-2).
pub const SFR_IFS3: u16 = 0x008A;
pub const SFR_IEC3: u16 = 0x009A;
pub const SFR_IPC14: u16 = 0x00C0;
pub const SFR_IPC15: u16 = 0x00C2;

/// Input capture FIFO depth (DS70198D §12.4: four levels).
pub const IC_FIFO_DEPTH: usize = 4;

/// One input capture channel (DS70198D §12).
#[derive(Debug, Clone, Default)]
struct IcChannel {
    /// `ICxCON` (ICSIDL/ICTMR/ICI/ICOV/ICBNE/ICM).
    con: u16,
    /// The 4-deep capture FIFO (§12.4).
    fifo: std::collections::VecDeque<u16>,
    /// Capture events counted since the last interrupt (the ICI divider).
    ici_count: u8,
    /// Rising edges counted for the ICM = 100/101 input prescalers.
    prescale: u8,
}

/// One output compare channel (DS70209A §13).
#[derive(Debug, Clone, Default)]
struct OcChannel {
    /// `OCxCON` (OCSIDL/OCFLT/OCTSEL/OCM).
    con: u16,
    /// `OCxR` — the active compare value (read-only while a PWM mode runs).
    r: u16,
    /// `OCxRS` — PWM shadow, or the second compare value of the pulse modes.
    rs: u16,
    /// The level the unit drives on its routed OCx pad; `None`: released.
    level: Option<bool>,
    /// A finished one-shot waits for the next `OCxCON` write.
    halted: bool,
}

/// The shared fields of the 3-operand MCU form
/// (`0www wBqq qddd dxxx xxxx`, DS70000157G `ADD Wb,Ws,Wd` and friends).
struct ThreeOp {
    /// `B` — byte operation.
    byte: bool,
    /// `Wb` — base register (4 bits).
    wb: usize,
    /// `qqq` — destination addressing mode.
    qqq: u32,
    /// `Wd` — destination register.
    wd: usize,
    /// `#lit5` when bits 6-5 select the literal form.
    lit: Option<u16>,
    /// `ppp` — source addressing mode (0 when `lit` is present).
    ppp: u32,
    /// `Ws` — source register.
    ws: usize,
}

/// M3 ADC1 sampling/conversion phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdcState {
    /// ADON = 0.
    Off,
    /// The sample-and-hold is sampling (`SAMP = 1`).
    Sampling,
    /// A conversion (or the next channel of the current sequence) is running.
    Converting,
}

/// A pending I2C1 master operation, completed after its wire time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum I2cOp {
    Start,
    RepeatedStart,
    Transmit,
    Receive,
    Ack,
    Stop,
}

/// dsPIC33F CPU core (M0 + M1a + M1b subset).
pub struct Pic24 {
    /// W0–W15. W15 is the stack pointer.
    pub w: [u16; 16],
    /// Program counter — **word address** (see module docs).
    pub pc: u32,
    /// Status register (SR). Flags are not yet evaluated by the M0 subset.
    pub sr: u16,
    /// Retired instruction cycles (1 per single-cycle instruction).
    pub cycles: u64,
    flash: Vec<u8>,
    sfr: Vec<u8>,
    ram: Vec<u8>,
    u1mode: u16,
    u1sta: u16,
    // M1a: timers (with prescaler phase counters), interrupt latches, accumulators.
    t1con: u16,
    tmr1: u16,
    pr1: u16,
    t1_frac: u16,
    t2con: u16,
    tmr2: u16,
    pr2: u16,
    t2_frac: u16,
    t3con: u16,
    tmr3: u16,
    pr3: u16,
    t3_frac: u16,
    ifs0: u16,
    ifs1: u16,
    iec0: u16,
    iec1: u16,
    ipc0: u16,
    ipc1: u16,
    ipc2: u16,
    ipc3: u16,
    ipc4: u16,
    intcon1: u16,
    intcon2: u16,
    corcon: u16,
    acc_a: i64,
    acc_b: i64,
    // M1b: ports, PPS, capture/compare.
    tris: [u16; 3],
    lat: [u16; 3],
    odc: [u16; 3],
    /// Last sampled live pad levels (`PORTx`), one word per port.
    pad_in: [u16; 3],
    /// The drive the Output Compare units present on their routed pads.
    pdrv: [u16; 3],
    /// Pads the Output Compare units own (driver taken from the GPIO stage).
    pdrv_mask: [u16; 3],
    /// `RPINR7`: IC1R<4:0> / IC2R<12:8> (0 = RP0 … 25 = RP25, 31 = VSS).
    rpinr7: u16,
    /// `RPOR0-12`: two 5-bit output-function fields each.
    rpor: [u16; RPOR_COUNT],
    ic: [IcChannel; 2],
    oc: [OcChannel; 2],
    // M2: Motor Control PWM (MCPWM1) and QEI1.
    p1tcon: u16,
    /// P1TMR: bit 15 = PTDIR (read-only), bits 14-0 = the count.
    p1tmr: u16,
    /// P1TPER buffer (loaded into `p1tper_active` at the period boundary).
    p1tper: u16,
    p1tper_active: u16,
    p1secmp: u16,
    pwm1con1: u16,
    pwm1con2: u16,
    p1dtcon1: u16,
    p1dtcon2: u16,
    p1fltacon: u16,
    p1ovdcon: u16,
    /// P1DC1-3 buffers.
    p1dc: [u16; 3],
    p1dc_active: [u16; 3],
    /// PWM input-prescaler phase (TCY counts).
    p1_frac: u16,
    /// PWM output postscaler counter.
    p1_post: u8,
    /// Generator levels (logical active), one per complementary/independent pair.
    p1_gen: [bool; 3],
    /// Remaining dead-time cycles before the high/low pin may go active.
    p1_dt_hi: [u16; 3],
    p1_dt_lo: [u16; 3],
    /// FLTA1 currently forces the programmed fault states.
    p1_fault_force: bool,
    /// Latched-mode fault that has not been cleared yet.
    p1_fault_latched: bool,
    /// Fault released; the outputs resume at the next period/half boundary.
    p1_fault_release: bool,
    /// Last sampled FLTA1 *asserted* state (the pin is active low).
    flta_prev: bool,
    /// `RPINR12/14/15`: FLTA1 / QEB1,QEA1 / INDX1 input maps.
    rpinr12: u16,
    rpinr14: u16,
    rpinr15: u16,
    /// IFS3/IEC3/IPC14/IPC15 — PWM1, QEI and FLTA1 interrupts.
    ifs3: u16,
    iec3: u16,
    ipc14: u16,
    ipc15: u16,
    qei1con: u16,
    dflt1con: u16,
    pos1cnt: u16,
    max1cnt: u16,
    /// Last decoded filtered (A<<1)|B quadrature state.
    qei_prev_ab: u8,
    /// Last filtered index level (rising-edge detection).
    qei_index_prev: bool,
    /// Digital filter: 3-bit histories, clock-divider counters and outputs
    /// for QEA, QEB and INDX.
    qei_filt: [u8; 3],
    qei_filt_cnt: [u16; 3],
    qei_filt_out: [bool; 3],
    /// QEI timer-mode input prescaler phase.
    qei_frac: u16,
    /// Last raw QEA level (timer external clock / gate edge detection).
    qei_timer_prev: bool,
    /// QEI external-clock prescaler counter.
    qei_ext_psc: u16,
    // M3: ADC1 (10/12-bit, 16-word buffer), SPI1 and I2C1.
    ad1con1: u16,
    ad1con2: u16,
    ad1con3: u16,
    ad1chs123: u16,
    ad1chs0: u16,
    ad1pcfgl: u16,
    ad1cssl: u16,
    /// ADC1BUF0-15 — the 16-word result buffer.
    adc_buf: [u16; ADC1_BUF_COUNT],
    /// Injected analog codes (raw 10/12-bit) for AN0-AN8 and the spare pins.
    adc_analog: [u16; 16],
    adc_state: AdcState,
    /// Remaining TAD cycles of the current conversion.
    adc_conv_remaining: u32,
    /// Remaining TAD cycles of an SSRC = 111 sample window.
    adc_sample_remaining: u32,
    /// Completed sample/convert sequences since the last interrupt.
    adc_conv_count: u16,
    /// Channel position inside the current CHPS sequence.
    adc_seq_pos: u8,
    /// Next word within the 16-word buffer (0-7 inside a half when BUFM = 1).
    adc_buf_pos: u8,
    /// BUFS: the half currently being filled when BUFM = 1.
    adc_half: u8,
    /// ALTS sweep select (false = MUX A, true = MUX B).
    adc_alt: bool,
    /// TMR3 period match latched for SSRC = 010.
    adc_t3_trigger: bool,
    /// PWM1 period match latched for SSRC = 011.
    adc_pwm_trigger: bool,
    /// Scan-list cursor for CSCNA (AD1CSSL bit index).
    adc_scan_next: u8,
    // SPI1.
    spi1stat: u16,
    spi1con1: u16,
    spi1con2: u16,
    /// Last received word (SPI1BUF read).
    spi1_rx: u16,
    /// Word loaded into the transmit buffer but not yet started.
    spi1_tx: u16,
    /// A one-deep transmit holding slot used when SPI1BUF is written while
    /// the previous word is still shifting.
    spi1_tx_pending: Option<u16>,
    /// Cycles left in the current shift.
    spi1_busy: u32,
    spi1_shifting: bool,
    /// Devices on the SPI1 wires (first device answers; see `attach_spi_device`).
    spi_devices: Vec<Box<dyn SpiDevice>>,
    // I2C1.
    i2c1con: u16,
    i2c1stat: u16,
    i2c1add: u16,
    i2c1msk: u16,
    i2c1brg: u16,
    i2c1rcv: u16,
    i2c1trn: u16,
    /// Selected slave for the current master transaction.
    i2c_target: Option<usize>,
    i2c_reading: bool,
    i2c_addressed: bool,
    /// True after a 10-bit address prefix (11110xx) was sent.
    i2c_10bit_prefix: Option<u16>,
    /// Cycles left in the current I2C operation.
    i2c_busy: u32,
    i2c_op: Option<I2cOp>,
    /// Devices on the I2C1 wires.
    i2c_devices: Vec<Box<dyn I2cDevice>>,
    /// Port state changed and must be pushed to the pad models.
    pads_dirty: bool,
    serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
    exception_pending: Option<u32>,
}

impl Default for Pic24 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic24 {
    pub fn new() -> Self {
        Self {
            w: [0; 16],
            pc: 0,
            sr: 0,
            cycles: 0,
            flash: vec![0xFF; FLASH_STORAGE_BYTES],
            sfr: vec![0; SFR_BYTES],
            ram: vec![0; RAM_BYTES],
            u1mode: 0,
            u1sta: 0,
            t1con: 0,
            tmr1: 0,
            pr1: 0,
            t1_frac: 0,
            t2con: 0,
            tmr2: 0,
            pr2: 0,
            t2_frac: 0,
            t3con: 0,
            tmr3: 0,
            pr3: 0,
            t3_frac: 0,
            ifs0: 0,
            ifs1: 0,
            iec0: 0,
            iec1: 0,
            ipc0: 0,
            ipc1: 0,
            ipc2: 0,
            ipc3: 0,
            ipc4: 0,
            intcon1: 0,
            intcon2: 0,
            corcon: 0,
            acc_a: 0,
            acc_b: 0,
            tris: [PORT_A_MASK, PORT_B_MASK, PORT_C_MASK],
            lat: [0; 3],
            odc: [0; 3],
            pad_in: [0; 3],
            pdrv: [0; 3],
            pdrv_mask: [0; 3],
            // Reset: every remappable input tied to VSS (RPINR7 R/W-1 = 1F1F).
            rpinr7: 0x1F1F,
            rpor: [0; RPOR_COUNT],
            ic: [IcChannel::default(), IcChannel::default()],
            oc: [OcChannel::default(), OcChannel::default()],
            p1tcon: 0,
            p1tmr: 0,
            p1tper: 0,
            p1tper_active: 0,
            p1secmp: 0,
            // PWMPIN = 1, HPOL = LPOL = 1 (erased-device configuration):
            // PWM pins are GPIO until PEN is set, and active is high.
            pwm1con1: 0,
            pwm1con2: 0,
            p1dtcon1: 0,
            p1dtcon2: 0,
            p1fltacon: 0,
            p1ovdcon: 0x3F00,
            p1dc: [0; 3],
            p1dc_active: [0; 3],
            p1_frac: 0,
            p1_post: 0,
            p1_gen: [false; 3],
            p1_dt_hi: [0; 3],
            p1_dt_lo: [0; 3],
            p1_fault_force: false,
            p1_fault_latched: false,
            p1_fault_release: false,
            flta_prev: false,
            rpinr12: 0x001F,
            rpinr14: 0x1F1F,
            rpinr15: 0x001F,
            ifs3: 0,
            iec3: 0,
            ipc14: 0,
            ipc15: 0x4000,
            qei1con: 0,
            dflt1con: 0,
            pos1cnt: 0,
            max1cnt: 0xFFFF,
            qei_prev_ab: 0,
            qei_index_prev: false,
            qei_filt: [0; 3],
            qei_filt_cnt: [0; 3],
            qei_filt_out: [false; 3],
            qei_frac: 0,
            qei_timer_prev: false,
            qei_ext_psc: 0,
            ad1con1: 0,
            ad1con2: 0,
            ad1con3: 0,
            ad1chs123: 0,
            ad1chs0: 0,
            ad1pcfgl: 0x0000,
            ad1cssl: 0,
            adc_buf: [0; ADC1_BUF_COUNT],
            adc_analog: [0; 16],
            adc_state: AdcState::Off,
            adc_conv_remaining: 0,
            adc_sample_remaining: 0,
            adc_conv_count: 0,
            adc_seq_pos: 0,
            adc_buf_pos: 0,
            adc_half: 0,
            adc_alt: false,
            adc_t3_trigger: false,
            adc_pwm_trigger: false,
            adc_scan_next: 0,
            spi1stat: 0,
            spi1con1: 0,
            spi1con2: 0,
            spi1_rx: 0,
            spi1_tx: 0,
            spi1_tx_pending: None,
            spi1_busy: 0,
            spi1_shifting: false,
            spi_devices: Vec::new(),
            i2c1con: 0x1000,
            i2c1stat: 0,
            i2c1add: 0,
            i2c1msk: 0,
            i2c1brg: 0,
            i2c1rcv: 0,
            i2c1trn: 0x00FF,
            i2c_target: None,
            i2c_reading: false,
            i2c_addressed: false,
            i2c_10bit_prefix: None,
            i2c_busy: 0,
            i2c_op: None,
            i2c_devices: Vec::new(),
            pads_dirty: true,
            serial_sink: None,
            exception_pending: None,
        }
    }

    /// Load instruction words into flash from slot 0 (tests / fixtures).
    ///
    /// Words are 24-bit; the top byte of each slot is ignored.
    pub fn load_words(&mut self, words: &[u32]) {
        assert!(
            words.len() <= INSTR_SLOTS,
            "fixture exceeds flash instruction slots"
        );
        for (i, word) in words.iter().enumerate() {
            let off = i * INSTR_SLOT_BYTES;
            self.flash[off..off + 3].copy_from_slice(&word.to_le_bytes()[..3]);
        }
    }

    /// Load raw instruction-slot bytes at a PC address (Harvard flash), the
    /// PIC24 counterpart of `Avr::load_flash` for fixture harnesses. `bytes`
    /// are already in this CPU's 4-byte slot form.
    pub fn load_flash(&mut self, addr_pc: u32, bytes: &[u8]) {
        let off = (addr_pc as usize / PC_UNITS_PER_INSTRUCTION as usize) * INSTR_SLOT_BYTES;
        assert!(
            off + bytes.len() <= FLASH_STORAGE_BYTES,
            "fixture exceeds flash"
        );
        self.flash[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// Load a program image (engine path). XC16 addresses are PC units and
    /// the parsers hand over executable sections already in 4-byte slot
    /// form; place each slot at `addr / 2`.
    ///
    /// The PC starts at 0: dsPIC33F reset vectors through the GOTO at
    /// address 0 (`Reset – GOTO Instruction`/`Reset – GOTO Address`,
    /// DS70283H Figure 7-1), which real XC16 images contain.
    pub fn load_program_image(&mut self, image: &crate::memory::ProgramImage) {
        for seg in &image.segments {
            let slot = (seg.start_addr / u64::from(PC_UNITS_PER_INSTRUCTION)) as usize;
            let start = slot.saturating_mul(INSTR_SLOT_BYTES);
            let end = start
                .saturating_add(seg.data.len())
                .min(FLASH_STORAGE_BYTES);
            if start < FLASH_STORAGE_BYTES {
                self.flash[start..end].copy_from_slice(&seg.data[..end - start]);
            }
        }
        self.pc = 0;
    }

    /// Attach the console sink that receives UART1 TX bytes (same type the AVR
    /// core and the session builders use).
    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    // ── M1a: timers, stack, interrupts ──────────────────────────────────────

    /// Timer prescaler from TCKPS (`00`=1:1, `01`=1:8, `10`=1:64, `11`=1:256).
    fn prescale(con: u16) -> u32 {
        match (con & TCON_TCKPS_MASK) >> TCON_TCKPS_SHIFT {
            0 => 1,
            1 => 8,
            2 => 64,
            _ => 256,
        }
    }

    /// Advance all running timers by `cycles` instruction cycles.
    ///
    /// Period match is `TMR > PR` (period `PR + 1` ticks): the timer wraps to
    /// zero and the IFS0 flag is set — the datasheet's "match → reset → flag"
    /// behaviour. In 32-bit mode Timer2/3 is controlled by Timer2's TON and
    /// prescaler with the interrupt reported as Timer3 (DS70283H §12).
    fn advance_timers(&mut self, cycles: u32) {
        if self.t1con & TCON_TON != 0 {
            let psc = Self::prescale(self.t1con);
            let acc = u32::from(self.t1_frac) + cycles;
            self.t1_frac = (acc % psc) as u16;
            for _ in 0..(acc / psc) {
                self.tmr1 = self.tmr1.wrapping_add(1);
                if self.tmr1 > self.pr1 {
                    self.tmr1 = 0;
                    self.ifs0 |= 1 << IRQ_T1.0;
                }
            }
        }
        if self.t2con & T2CON_T32 == 0 {
            if self.t2con & TCON_TON != 0 {
                let psc = Self::prescale(self.t2con);
                let acc = u32::from(self.t2_frac) + cycles;
                self.t2_frac = (acc % psc) as u16;
                for _ in 0..(acc / psc) {
                    self.tmr2 = self.tmr2.wrapping_add(1);
                    let mut wrapped = false;
                    if self.tmr2 > self.pr2 {
                        self.tmr2 = 0;
                        self.ifs0 |= 1 << IRQ_T2.0;
                        wrapped = true;
                    }
                    // The Output Compare units clock off the same ticks.
                    self.oc_tick(2, wrapped);
                }
            }
            if self.t3con & TCON_TON != 0 {
                let psc = Self::prescale(self.t3con);
                let mut acc = u32::from(self.t3_frac) + cycles;
                self.t3_frac = (acc % psc) as u16;
                acc /= psc;
                for _ in 0..acc {
                    self.tmr3 = self.tmr3.wrapping_add(1);
                    let mut wrapped = false;
                    if self.tmr3 > self.pr3 {
                        self.tmr3 = 0;
                        self.ifs0 |= 1 << IRQ_T3.0;
                        // ADC1 SSRC = 010: the TMR3 period match ends sampling.
                        self.adc_t3_trigger = true;
                        wrapped = true;
                    }
                    self.oc_tick(3, wrapped);
                }
            }
        } else if self.t2con & TCON_TON != 0 {
            let psc = Self::prescale(self.t2con);
            let mut acc = u32::from(self.t2_frac) + cycles;
            self.t2_frac = (acc % psc) as u16;
            acc /= psc;
            let period = (u32::from(self.pr3) << 16) | u32::from(self.pr2);
            let mut cnt = (u32::from(self.tmr3) << 16) | u32::from(self.tmr2);
            for _ in 0..acc {
                cnt = cnt.wrapping_add(1);
                if cnt > period {
                    cnt = 0;
                    self.ifs0 |= 1 << IRQ_T3.0;
                    self.adc_t3_trigger = true;
                }
            }
            self.tmr2 = cnt as u16;
            self.tmr3 = (cnt >> 16) as u16;
        }
    }

    // ── M1b: ports, PPS, input capture, output compare ──────────────────────

    /// `(port index, register role)` for an I/O-port SFR address. Role:
    /// 0 TRIS, 1 PORT, 2 LAT, 3 ODC.
    fn port_sfr(addr: u16) -> Option<(usize, u8)> {
        match addr {
            0x02C0..=0x02C6 => Some((0, ((addr - 0x02C0) / 2) as u8)),
            0x02C8..=0x02CE => Some((1, ((addr - 0x02C8) / 2) as u8)),
            0x02D0..=0x02D6 => Some((2, ((addr - 0x02D0) / 2) as u8)),
            _ => None,
        }
    }

    /// The base of a port's mirror window: `TRISA + 16*port`, best effort.
    /// The 16-byte stride leaves room for the two non-silicon drive
    /// registers (offsets 8 and 10) without overlapping the next port —
    /// the real SFRs are only 8 bytes apart.
    fn port_base(port: usize) -> u16 {
        SFR_TRISA + 0x10 * port as u16
    }

    fn port_mask(port: usize) -> u16 {
        match port {
            0 => PORT_A_MASK,
            1 => PORT_B_MASK,
            _ => PORT_C_MASK,
        }
    }

    /// The `(port, bit)` an RPn pin is on the 44-pin package (DS70283H pin
    /// diagrams): RP0-15 are RB0-15, RP16-25 are RC0-9.
    fn rp_pin(rp: u8) -> Option<(usize, u8)> {
        match rp {
            0..=15 => Some((1, rp)),
            16..=25 => Some((2, rp - 16)),
            _ => None,
        }
    }

    /// The RPn the ICx input is routed from (`RPINR7`, DS70283H Register
    /// 10-4): 0-25 = RP0-RP25, 31 = tied to VSS. 26-30 are reserved.
    fn ic_rp(&self, ch: usize) -> Option<u8> {
        let field = if ch == 0 {
            (self.rpinr7 & 0x1F) as u8
        } else {
            ((self.rpinr7 >> 8) & 0x1F) as u8
        };
        (field <= 25).then_some(field)
    }

    /// The RPn an OCx output is mapped to (`RPOR0-12`), by function code
    /// (DS70283H Table 10-2: 10010 = OC1, 10011 = OC2).
    fn oc_rp(&self, ch: usize) -> Option<u8> {
        let code = if ch == 0 { PPS_OUT_OC1 } else { PPS_OUT_OC2 };
        self.pps_out_rp(code)
    }

    /// Find the RPn whose RPOR field carries `code`.
    fn pps_out_rp(&self, code: u16) -> Option<u8> {
        for (i, reg) in self.rpor.iter().enumerate() {
            if reg & 0x1F == code {
                return Some((2 * i) as u8);
            }
            if (reg >> 8) & 0x1F == code {
                return Some((2 * i + 1) as u8);
            }
        }
        None
    }

    /// The live pad level `PORTx` reads: the chip's own stages drive, the
    /// last sample fills the inputs (DS70190D §30.2.2 — a read of PORTx
    /// reads the pin).
    fn port_read(&self, port: usize) -> u16 {
        let own = !self.tris[port] & !(self.odc[port] & self.lat[port]);
        let pm = self.pdrv_mask[port];
        let driven = own | pm;
        let levels = (self.lat[port] & own & !pm) | (self.pdrv[port] & pm);
        ((levels & driven) | (self.pad_in[port] & !driven)) & Self::port_mask(port)
    }

    /// Push the port registers and the Output Compare drive to the bus-side
    /// pad models (`peripherals::pic24_gpio`); best effort — a bus with no
    /// pad models answers nothing and must not fail a step.
    fn sync_pads(&mut self, bus: &mut dyn Bus) {
        if !self.pads_dirty {
            return;
        }
        self.pads_dirty = false;
        let mut pdrv = [0u16; 3];
        let mut mask = [0u16; 3];
        for ch in 0..2 {
            if self.oc[ch].con & OCCON_OCM_MASK == 0 {
                continue;
            }
            let Some(Some((port, bit))) = self.oc_rp(ch).map(Self::rp_pin) else {
                continue;
            };
            mask[port] |= 1 << bit;
            if self.oc[ch].level == Some(true) {
                pdrv[port] |= 1 << bit;
            }
        }
        let (pwm_levels, pwm_mask) = self.pwm_pad_state();
        for port in 0..3 {
            pdrv[port] = (pdrv[port] & !pwm_mask[port]) | pwm_levels[port];
            mask[port] |= pwm_mask[port];
        }
        if self.qei1con & QEICON_PCDOUT != 0 {
            if let Some(rp) = self.pps_out_rp(PPS_OUT_UPDN) {
                if let Some((port, bit)) = Self::rp_pin(rp) {
                    mask[port] |= 1 << bit;
                    if self.qei1con & QEICON_UPDN != 0 {
                        pdrv[port] |= 1 << bit;
                    } else {
                        pdrv[port] &= !(1 << bit);
                    }
                }
            }
        }
        self.pdrv = pdrv;
        self.pdrv_mask = mask;
        for port in 0..3 {
            let base = PIC24_IO_MIRROR_BASE + u64::from(Self::port_base(port));
            _ = bus.write_u16(base, self.tris[port]);
            _ = bus.write_u16(base + 0x04, self.lat[port]);
            _ = bus.write_u16(base + 0x06, self.odc[port]);
            _ = bus.write_u16(base + 0x08, self.pdrv[port]);
            _ = bus.write_u16(base + 0x0A, self.pdrv_mask[port]);
        }
    }

    /// Sample the ports once per retired step and run the input capture
    /// channels on the edges (see the module-level latency note).
    fn sample_capture(&mut self, bus: &dyn Bus, cycles: u32) {
        let mut sampled = [0u16; 3];
        for (port, slot) in sampled.iter_mut().enumerate() {
            let addr = PIC24_IO_MIRROR_BASE + u64::from(Self::port_base(port)) + 0x02;
            *slot = bus.read_u16(addr).unwrap_or(self.pad_in[port]);
        }
        let old = self.pad_in;
        self.pad_in = sampled;
        for ch in 0..2 {
            let con = self.ic[ch].con;
            let icm = con & ICCON_ICM_MASK;
            if !(1..=5).contains(&icm) {
                continue; // Off, unused, or sleep-wake-only: no captures.
            }
            let Some(rp) = self.ic_rp(ch) else { continue };
            let Some((port, bit)) = Self::rp_pin(rp) else {
                continue;
            };
            let was = old[port] & (1 << bit) != 0;
            let now = sampled[port] & (1 << bit) != 0;
            if was == now {
                continue;
            }
            let fire = match icm {
                1 => true, // Every edge.
                2 => !now, // Falling.
                3 => now,  // Rising.
                4 => {
                    // Every 4th rising edge.
                    now && {
                        self.ic[ch].prescale = (self.ic[ch].prescale + 1) & 3;
                        self.ic[ch].prescale == 0
                    }
                }
                _ => {
                    // Every 16th rising edge (ICM = 101).
                    now && {
                        self.ic[ch].prescale = (self.ic[ch].prescale + 1) & 15;
                        self.ic[ch].prescale == 0
                    }
                }
            };
            if fire {
                self.ic_capture(ch);
            }
        }
        self.sample_qei(cycles);
    }

    // ── M2: Motor Control PWM (DS70187E §14) ────────────────────────────────

    /// Dead time for unit A (`a = true`) or unit B, in TCY: a 6-bit count
    /// times the 1/2/4/8 prescaler (DS70187E Register 14-7).
    fn pwm_dead_time(&self, a: bool) -> u16 {
        let (ps, val) = if a {
            ((self.p1dtcon1 >> 6) & 0x3, self.p1dtcon1 & 0x3F)
        } else {
            ((self.p1dtcon1 >> 14) & 0x3, (self.p1dtcon1 >> 8) & 0x3F)
        };
        val.saturating_mul(1u16 << ps)
    }

    /// `true` when the pair's active-edge dead time comes from unit A.
    fn pwm_dt_unit_a(&self, pair: usize, active_edge: bool) -> bool {
        let bit = if active_edge { 2 * pair + 1 } else { 2 * pair };
        self.p1dtcon2 & (1 << bit) == 0
    }

    fn pwm_pair_complementary(&self, pair: usize) -> bool {
        self.pwm1con1 & (1 << (8 + pair)) == 0
    }

    fn pwm_pair_enabled(&self, pair: usize) -> (bool, bool) {
        (
            self.pwm1con1 & (1 << (4 + pair)) != 0,
            self.pwm1con1 & (1 << pair) != 0,
        )
    }

    /// Drive one generator and arm the dead-time counter for the pin that is
    /// about to go active (complementary pairs only, DS70187E §14.14).
    fn pwm_drive(&mut self, pair: usize, active: bool) {
        if self.p1_gen[pair] == active {
            return;
        }
        self.p1_gen[pair] = active;
        if !self.pwm_pair_complementary(pair) {
            return;
        }
        if active {
            let a = self.pwm_dt_unit_a(pair, true);
            self.p1_dt_hi[pair] = self.pwm_dead_time(a);
            self.p1_dt_lo[pair] = 0;
        } else {
            let a = self.pwm_dt_unit_a(pair, false);
            self.p1_dt_lo[pair] = self.pwm_dead_time(a);
            self.p1_dt_hi[pair] = 0;
        }
    }

    /// PWM1IF with the PTOPS postscaler (DS70187E §14.9). Single-event and
    /// double-update modes ignore the postscaler.
    fn pwm_interrupt(&mut self) {
        let mode = self.p1tcon & PTCON_PTMOD_MASK;
        if mode == 1 || mode == 3 {
            self.ifs3 |= 1 << IRQ_PWM1.0;
            return;
        }
        let post = ((self.p1tcon & PTCON_PTOPS_MASK) >> 4) + 1;
        self.p1_post = self.p1_post.wrapping_add(1);
        if u16::from(self.p1_post) >= post {
            self.p1_post = 0;
            self.ifs3 |= 1 << IRQ_PWM1.0;
        }
    }

    /// A period/zero boundary: latch the buffered period and duty values and
    /// resume from a fault (DS70187E §14.12, §14.15.3.2).
    fn pwm_boundary(&mut self) {
        self.p1tper_active = self.p1tper;
        if self.pwm1con2 & PWM2CON_UDIS == 0 {
            self.p1dc_active = self.p1dc;
        }
        if self.p1_fault_release {
            self.p1_fault_release = false;
            self.p1_fault_force = false;
        }
    }

    /// Apply the duty-cycle special cases: 0 is inactive for the whole
    /// period, values above PxTPER are active for the whole period.
    fn pwm_apply_duty_extremes(&mut self, tper: u16) {
        for pair in 0..3 {
            let dc = self.p1dc_active[pair];
            if dc == 0 {
                self.pwm_drive(pair, false);
            } else if dc > tper {
                self.pwm_drive(pair, true);
            }
        }
    }

    /// One PWM time-base tick (DS70187E §14.7/§14.8/§14.11).
    fn pwm_tick(&mut self) {
        let mode = self.p1tcon & PTCON_PTMOD_MASK;
        let tper = self.p1tper_active & 0x7FFF;
        let mut count = self.p1tmr & 0x7FFF;
        let mut down = self.p1tmr & 0x8000 != 0;
        self.pwm_apply_duty_extremes(tper);
        match mode {
            0 | 1 => {
                if count == tper {
                    count = 0;
                    self.pwm_boundary();
                    self.pwm_interrupt();
                    // ADC1 SSRC = 011: the PWM1 special event trigger.
                    self.adc_pwm_trigger = true;
                    for pair in 0..3 {
                        let dc = self.p1dc_active[pair];
                        self.pwm_drive(pair, dc != 0);
                    }
                    if mode == 1 {
                        self.p1tcon &= !PTCON_PTEN;
                        for pair in 0..3 {
                            self.pwm_drive(pair, false);
                        }
                    }
                } else {
                    count += 1;
                    for pair in 0..3 {
                        let dc = self.p1dc_active[pair];
                        if dc != 0 && dc <= tper && count == dc {
                            self.pwm_drive(pair, false);
                        }
                    }
                }
            }
            2 | 3 => {
                if !down {
                    for pair in 0..3 {
                        let dc = self.p1dc_active[pair];
                        if dc != 0 && dc <= tper && count == dc {
                            self.pwm_drive(pair, false);
                        }
                    }
                    if count >= tper {
                        down = true;
                        if mode == 3 {
                            self.pwm_boundary();
                            self.pwm_interrupt();
                            self.adc_pwm_trigger = true;
                        }
                    } else {
                        count += 1;
                    }
                } else {
                    for pair in 0..3 {
                        let dc = self.p1dc_active[pair];
                        if dc != 0 && count == dc {
                            self.pwm_drive(pair, true);
                        }
                    }
                    if count == 0 {
                        down = false;
                        self.pwm_boundary();
                        self.pwm_interrupt();
                        self.adc_pwm_trigger = true;
                    } else {
                        count -= 1;
                    }
                }
            }
            _ => {}
        }
        self.p1tmr = (if down { 0x8000 } else { 0 }) | (count & 0x7FFF);
    }

    /// Sample FLTA1 (active low) and update the fault force/latch state.
    fn pwm_fault_sample(&mut self, bus: &dyn Bus) {
        let old_force = self.p1_fault_force;
        let level = self.rp_input_level(bus, self.rpinr12 & 0x1F);
        let asserted = !level;
        let enabled = self.p1fltacon & 0x7 != 0;
        let latched_mode = self.p1fltacon & 0x0080 == 0;
        if enabled && asserted && !self.flta_prev {
            self.ifs3 |= 1 << IRQ_FLTA1.0;
            if latched_mode {
                self.p1_fault_latched = true;
            }
        }
        if asserted {
            // A new fault cancels a pending boundary release.
            self.p1_fault_release = false;
        } else if self.p1_fault_latched && self.ifs3 & (1 << IRQ_FLTA1.0) == 0 {
            // Latched mode: pin high and flag cleared → release at the next
            // period/half-period boundary.
            self.p1_fault_latched = false;
            self.p1_fault_release = true;
        } else if !self.p1_fault_latched && self.p1_fault_force {
            // Cycle-by-cycle: released when the pin goes high.
            self.p1_fault_release = true;
        }
        self.p1_fault_force = if !enabled {
            false
        } else {
            asserted || self.p1_fault_latched || self.p1_fault_release
        };
        if !enabled {
            self.p1_fault_latched = false;
            self.p1_fault_release = false;
        }
        if self.p1tcon & PTCON_PTEN == 0 {
            self.p1_fault_force = false;
            self.p1_fault_latched = false;
            self.p1_fault_release = false;
        }
        self.flta_prev = asserted;
        if self.p1_fault_force != old_force {
            self.pads_dirty = true;
        }
    }

    /// Advance the PWM time base by `cycles` TCY (called once per retired
    /// step, before the pad drive is published).
    fn advance_mcpwm(&mut self, cycles: u32, bus: &dyn Bus) {
        self.pwm_fault_sample(bus);
        if self.p1tcon & PTCON_PTEN == 0 {
            for pair in 0..3 {
                self.p1_gen[pair] = false;
                self.p1_dt_hi[pair] = 0;
                self.p1_dt_lo[pair] = 0;
            }
            return;
        }
        let psc = 1u32 << (2 * u32::from((self.p1tcon & PTCON_PTCKPS_MASK) >> 2));
        for _ in 0..cycles {
            for pair in 0..3 {
                self.p1_dt_hi[pair] = self.p1_dt_hi[pair].saturating_sub(1);
                self.p1_dt_lo[pair] = self.p1_dt_lo[pair].saturating_sub(1);
            }
            self.p1_frac += 1;
            if u32::from(self.p1_frac) >= psc {
                self.p1_frac = 0;
                self.pwm_tick();
            }
        }
        // The generator/dead-time state may have changed this step.
        self.pads_dirty = true;
    }

    /// The six PWM pin levels and their driver mask, after dead time,
    /// override and fault logic (DS70187E §14.10/§14.11/§14.15).
    fn pwm_pad_state(&self) -> ([u16; 3], [u16; 3]) {
        let mut levels = [0u16; 3];
        let mut mask = [0u16; 3];
        for (pair, (high_pin, low_pin)) in PWM1_PINS.iter().enumerate() {
            let (en_h, en_l) = self.pwm_pair_enabled(pair);
            if !en_h && !en_l {
                continue;
            }
            let complementary = self.pwm_pair_complementary(pair);
            let gen = self.p1_gen[pair];
            let running = self.p1tcon & PTCON_PTEN != 0;
            // With the time base disabled both pins are inactive; only an
            // override or fault can still drive them (DS70187E §14.11.1).
            let mut act_h = running && gen;
            let mut act_l = running && if complementary { !gen } else { gen };
            if complementary && running {
                if gen {
                    act_h = self.p1_dt_hi[pair] == 0;
                    act_l = false;
                } else {
                    act_l = self.p1_dt_lo[pair] == 0;
                    act_h = false;
                }
            }
            // Override: POVD = 0 drives the pin from POUT.
            if self.p1ovdcon & (1 << (9 + 2 * pair)) == 0 {
                act_h = self.p1ovdcon & (1 << (1 + 2 * pair)) != 0;
            }
            if self.p1ovdcon & (1 << (8 + 2 * pair)) == 0 {
                act_l = self.p1ovdcon & (1 << (2 * pair)) != 0;
            }
            // Fault A has priority over override and the generator.
            if self.p1_fault_force && self.p1fltacon & (1 << pair) != 0 {
                act_h = self.p1fltacon & (1 << (9 + 2 * pair)) != 0;
                act_l = self.p1fltacon & (1 << (8 + 2 * pair)) != 0;
                if complementary && act_h && act_l {
                    act_l = false;
                }
            }
            let (hp, hb) = *high_pin;
            let (lp, lb) = *low_pin;
            if en_h {
                mask[hp] |= 1 << hb;
                if act_h {
                    levels[hp] |= 1 << hb;
                }
            }
            if en_l {
                mask[lp] |= 1 << lb;
                if act_l {
                    levels[lp] |= 1 << lb;
                }
            }
        }
        (levels, mask)
    }

    /// `P1TCON` write: enable/disable, mode and prescaler changes restart
    /// the time-base counters (DS70187E §14.8.2).
    fn pwm_configure_tcon(&mut self, val: u16) {
        let old = self.p1tcon;
        let new = val
            & (PTCON_PTEN | PTCON_PTSIDL | PTCON_PTOPS_MASK | PTCON_PTCKPS_MASK | PTCON_PTMOD_MASK);
        self.p1tcon = new;
        self.p1_frac = 0;
        self.p1_post = 0;
        if new & PTCON_PTEN == 0 {
            for pair in 0..3 {
                self.p1_gen[pair] = false;
                self.p1_dt_hi[pair] = 0;
                self.p1_dt_lo[pair] = 0;
            }
            self.p1tper_active = self.p1tper;
            self.p1dc_active = self.p1dc;
            self.p1_fault_force = false;
            self.p1_fault_latched = false;
            self.p1_fault_release = false;
        } else if old & PTCON_PTEN == 0 && new & PTCON_PTMOD_MASK <= 1 {
            // Edge-aligned modes start the period (and the active outputs)
            // when the time base is enabled at TMR = 0.
            for pair in 0..3 {
                let dc = self.p1dc_active[pair];
                self.pwm_drive(pair, dc != 0);
            }
        }
        self.pads_dirty = true;
    }

    /// `P1DCy` write: the buffer updates immediately when the time base is
    /// off or IUE = 1, otherwise at the next duty update point (§14.12).
    fn pwm_duty_write(&mut self, pair: usize, val: u16) {
        self.p1dc[pair] = val;
        if self.p1tcon & PTCON_PTEN == 0 || self.pwm1con2 & PWM2CON_IUE != 0 {
            self.p1dc_active[pair] = val;
        }
    }

    /// `QEI1CON` write: INDEX is read-only, UPDN is read-only in QEI modes,
    /// and a mode change clears the edge/filter state (DS70208C Register 15-1).
    fn qei_configure(&mut self, val: u16) {
        let old_mode = (self.qei1con >> 8) & 7;
        let mut new = (val & !QEICON_INDEX) | (self.qei1con & QEICON_INDEX);
        if (new & QEICON_QEIM_MASK) >> 8 >= 4 {
            // QEI modes: UPDN is the read-only direction status.
            new = (new & !QEICON_UPDN) | (self.qei1con & QEICON_UPDN);
        }
        self.qei1con = new;
        let new_mode = (new >> 8) & 7;
        if new_mode != old_mode {
            self.qei_prev_ab = 0;
            self.qei_index_prev = false;
            self.qei_filt = [0; 3];
            self.qei_filt_cnt = [0; 3];
            self.qei_frac = 0;
            self.qei_ext_psc = 0;
            self.qei_timer_prev = false;
        }
        // TQCS = TQGATE = 1: no counting, one interrupt on configuration.
        if new_mode == 1 && new & (QEICON_TQCS | QEICON_TQGATE) == (QEICON_TQCS | QEICON_TQGATE) {
            self.qei_interrupt();
        }
        self.pads_dirty = true;
    }

    // ── M2: Quadrature Encoder Interface (DS70208C §15) ─────────────────────

    fn qei_interrupt(&mut self) {
        self.ifs3 |= 1 << IRQ_QEI.0;
    }

    fn pad_level(&self, rp: u16) -> bool {
        Self::rp_field(rp)
            .and_then(Self::rp_pin)
            .map(|(p, b)| self.pad_in[p] & (1 << b) != 0)
            .unwrap_or(false)
    }

    fn rp_field(field: u16) -> Option<u8> {
        (field <= 25).then_some(field as u8)
    }

    /// Read one external pad level from the bus pad mirror (used for FLTA1,
    /// which is sampled before `sample_capture` runs).
    fn rp_input_level(&self, bus: &dyn Bus, rp: u16) -> bool {
        let Some((port, bit)) = Self::rp_field(rp).and_then(Self::rp_pin) else {
            return false;
        };
        let addr = PIC24_IO_MIRROR_BASE + u64::from(Self::port_base(port)) + 0x02;
        bus.read_u16(addr)
            .map(|v| v & (1 << bit) != 0)
            .unwrap_or(self.pad_in[port] & (1 << bit) != 0)
    }

    fn qei_raw_inputs(&self) -> [bool; 3] {
        [
            self.pad_level(self.rpinr14 & 0x1F),
            self.pad_level((self.rpinr14 >> 8) & 0x1F),
            self.pad_level(self.rpinr15 & 0x1F),
        ]
    }

    /// Digital filter: a level is accepted after three consecutive equal
    /// filter clocks (`FCY / QECK`, DS70208C §15.3). Returns the filtered
    /// QEA, QEB and INDX levels.
    fn qei_filter(&mut self, raw: [bool; 3], cycles: u32) -> [bool; 3] {
        if self.dflt1con & DFLT_QEOUT == 0 {
            self.qei_filt = [0; 3];
            self.qei_filt_cnt = [0; 3];
            self.qei_filt_out = raw;
            return raw;
        }
        let div = match (self.dflt1con & DFLT_QECK_MASK) >> 4 {
            0 => 1u16,
            1 => 2,
            2 => 4,
            3 => 16,
            4 => 32,
            5 => 64,
            6 => 128,
            _ => 256,
        };
        let added = cycles.min(u32::from(u16::MAX)) as u16;
        for (ch, &raw_ch) in raw.iter().enumerate() {
            self.qei_filt_cnt[ch] = self.qei_filt_cnt[ch].saturating_add(added);
            while self.qei_filt_cnt[ch] >= div {
                self.qei_filt_cnt[ch] -= div;
                self.qei_filt[ch] = ((self.qei_filt[ch] << 1) | u8::from(raw_ch)) & 0x7;
                if self.qei_filt[ch] == 0b111 {
                    self.qei_filt_out[ch] = true;
                } else if self.qei_filt[ch] == 0 {
                    self.qei_filt_out[ch] = false;
                }
            }
        }
        self.qei_filt_out
    }

    /// x2/x4 quadrature decoding (DS70208C §15.4, Table 15-1). State is
    /// `(A << 1) | B`; forward is 00→10→11→01→00.
    fn qei_decode(&mut self, old_ab: u8, new_ab: u8) {
        let qeim = (self.qei1con >> 8) & 7;
        if qeim < 4 {
            return;
        }
        let swap = self.qei1con & QEICON_SWPAB != 0;
        let map = |ab: u8| -> u8 {
            if swap {
                ((ab & 1) << 1) | ((ab >> 1) & 1)
            } else {
                ab
            }
        };
        let old = map(old_ab);
        let new = map(new_ab);
        let delta = if qeim & 2 != 0 {
            // x4: every edge; diagonal transitions are ignored.
            const QUAD: [i8; 16] = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];
            QUAD[usize::from((old << 2) | new)]
        } else if (old & 2) != (new & 2) {
            // x2: count on Phase A edges only (Phase B gives the direction).
            let a = (new >> 1) & 1;
            let b = new & 1;
            if a != b {
                1
            } else {
                -1
            }
        } else {
            0
        };
        if delta != 0 {
            self.qei_count(delta);
        }
    }

    /// Apply one quadrature count to POS1CNT, including the match-reset and
    /// index-mode count-error rules (DS70208C §15.5.2/§15.5.3.4).
    fn qei_count(&mut self, delta: i8) {
        let qeim = (self.qei1con >> 8) & 7;
        let up = delta > 0;
        self.qei1con = if up {
            self.qei1con | QEICON_UPDN
        } else {
            self.qei1con & !QEICON_UPDN
        };
        let index_mode = qeim & 1 == 0; // 100 or 110
        if index_mode {
            self.pos1cnt = if up {
                self.pos1cnt.wrapping_add(1)
            } else {
                self.pos1cnt.wrapping_sub(1)
            };
            if self.qei1con & QEICON_CNTERR == 0 {
                let error = if up {
                    self.pos1cnt == self.max1cnt.wrapping_add(1)
                } else {
                    self.pos1cnt == 0xFFFF
                };
                if error {
                    // CNTERR is set even when CEID masks the interrupt
                    // (DS70208C §15.5.3.4); further errors are ignored until
                    // the flag is cleared by software.
                    self.qei1con |= QEICON_CNTERR;
                    if self.dflt1con & DFLT_CEID == 0 {
                        self.qei_interrupt();
                    }
                }
            }
        } else if up {
            if self.pos1cnt == self.max1cnt {
                self.pos1cnt = 0;
                self.qei_interrupt();
            } else {
                self.pos1cnt = self.pos1cnt.wrapping_add(1);
            }
        } else if self.pos1cnt == 0 {
            self.pos1cnt = self.max1cnt;
            self.qei_interrupt();
        } else {
            self.pos1cnt = self.pos1cnt.wrapping_sub(1);
        }
    }

    /// A filtered index rising edge: apply the IMV match and reset POS1CNT
    /// in the index modes (DS70208C §15.5.3).
    fn qei_index_event(&mut self, a: bool, b: bool) {
        let qeim = (self.qei1con >> 8) & 7;
        if qeim < 4 {
            return;
        }
        let imv = (self.dflt1con & DFLT_IMV_MASK) >> 9;
        let matched = if qeim & 2 != 0 {
            // x4: IMV1 = required B, IMV0 = required A.
            (u16::from(b) == (imv >> 1)) && (u16::from(a) == (imv & 1))
        } else {
            // x2: IMV1 selects the phase (0 = A, 1 = B), IMV0 is its state.
            let value = if imv & 2 != 0 { b } else { a };
            u16::from(value) == (imv & 1)
        };
        if !matched {
            return;
        }
        self.qei_interrupt();
        if self.qei1con & QEICON_POSRES != 0 {
            let up = self.qei1con & QEICON_UPDN != 0;
            self.pos1cnt = if up { 0 } else { self.max1cnt };
        }
    }

    /// QEI timer/counter mode (QEIM = 001, DS70208C §15.6).
    fn qei_timer_step(&mut self, cycles: u32, qea: bool, qeb: bool) {
        let tqcs = self.qei1con & QEICON_TQCS != 0;
        let tqgate = self.qei1con & QEICON_TQGATE != 0;
        if tqcs && tqgate {
            // The timer does not increment; the interrupt is raised when the
            // configuration is written (qei_configure).
            self.qei_timer_prev = qea;
            return;
        }
        let up = if self.qei1con & QEICON_UDSRC != 0 {
            qeb
        } else {
            self.qei1con & QEICON_UPDN != 0
        };
        let psc = match (self.qei1con & QEICON_TQCKPS_MASK) >> 3 {
            0 => 1u16,
            1 => 8,
            2 => 64,
            _ => 256,
        };
        if tqcs {
            if qea && !self.qei_timer_prev {
                self.qei_ext_psc = self.qei_ext_psc.wrapping_add(1);
                if self.qei_ext_psc >= psc {
                    self.qei_ext_psc = 0;
                    self.qei_timer_count(up);
                }
            }
            self.qei_timer_prev = qea;
            return;
        }
        if tqgate {
            if self.qei_timer_prev && !qea {
                self.qei_interrupt();
            }
            self.qei_timer_prev = qea;
            if !qea {
                return;
            }
        }
        self.qei_frac = self
            .qei_frac
            .saturating_add(cycles.min(u32::from(u16::MAX)) as u16);
        while self.qei_frac >= psc {
            self.qei_frac -= psc;
            self.qei_timer_count(up);
        }
    }

    fn qei_timer_count(&mut self, up: bool) {
        self.qei1con = if up {
            self.qei1con | QEICON_UPDN
        } else {
            self.qei1con & !QEICON_UPDN
        };
        if up {
            let next = u32::from(self.pos1cnt) + 1;
            if next > u32::from(self.max1cnt) {
                self.pos1cnt = 0;
                self.qei_interrupt();
            } else {
                self.pos1cnt = next as u16;
            }
        } else {
            let next = self.pos1cnt.wrapping_sub(1);
            if next == self.max1cnt {
                self.pos1cnt = 0;
                self.qei_interrupt();
            } else {
                self.pos1cnt = next;
            }
        }
    }

    /// Sample the QEI inputs once per retired step and run the decoder or
    /// timer (DS70208C §15.3-§15.6).
    fn sample_qei(&mut self, cycles: u32) {
        let raw = self.qei_raw_inputs();
        let qeim = (self.qei1con >> 8) & 7;
        if qeim == 1 {
            // Timer mode: the digital filter is inactive (§15.6).
            self.qei1con = (self.qei1con & !QEICON_INDEX) | if raw[2] { QEICON_INDEX } else { 0 };
            self.qei_timer_step(cycles, raw[0], raw[1]);
            return;
        }
        if qeim < 4 {
            self.qei_prev_ab = (u8::from(raw[0]) << 1) | u8::from(raw[1]);
            self.qei_index_prev = raw[2];
            return;
        }
        let filt = self.qei_filter(raw, cycles);
        self.qei1con = (self.qei1con & !QEICON_INDEX) | if filt[2] { QEICON_INDEX } else { 0 };
        let ab = (u8::from(filt[0]) << 1) | u8::from(filt[1]);
        let old_ab = self.qei_prev_ab;
        self.qei_prev_ab = ab;
        if ab != old_ab {
            self.qei_decode(old_ab, ab);
        }
        if filt[2] && !self.qei_index_prev {
            self.qei_index_event(filt[0], filt[1]);
        }
        self.qei_index_prev = filt[2];
        if self.qei1con & QEICON_PCDOUT != 0 {
            // The UPDN output pin follows the direction status.
            self.pads_dirty = true;
        }
    }

    /// One capture event on channel `ch` (DS70198D §12.4/§12.5).
    fn ic_capture(&mut self, ch: usize) {
        let con = self.ic[ch].con;
        let icm = con & ICCON_ICM_MASK;
        let ici = (con & ICCON_ICI_MASK) >> ICCON_ICI_SHIFT;
        let bit = if ch == 0 {
            1 << IRQ_IC1.0
        } else {
            1 << IRQ_IC2.0
        };
        let full = self.ic[ch].fifo.len() >= IC_FIFO_DEPTH;
        if !full {
            let tmr = if con & ICCON_ICTMR != 0 {
                self.tmr2
            } else {
                self.tmr3
            };
            self.ic[ch].fifo.push_back(tmr);
            self.ic[ch].con |= ICCON_ICBNE;
        }
        let every = icm == 1 || ici == 0;
        if full {
            if every {
                // In these modes interrupts continue and ICOV stays clear
                // even after an overflow (DS70198D §12.4.2).
                self.ifs0 |= bit;
            } else {
                // A later interrupt cannot be honoured until the buffer is
                // drained; the overrun sets ICOV instead.
                self.ic[ch].con |= ICCON_ICOV;
            }
        } else {
            self.ic[ch].ici_count += 1;
            if every || self.ic[ch].ici_count >= (ici + 1) as u8 {
                self.ic[ch].ici_count = 0;
                self.ifs0 |= bit;
            }
        }
    }

    /// One Output Compare tick on timer `which` (2 or 3). `rollover` is true
    /// on the tick the timer wrapped to 0 (DS70209A §13.3.2).
    fn oc_tick(&mut self, which: u8, rollover: bool) {
        for ch in 0..2 {
            let con = self.oc[ch].con;
            let ocm = con & OCCON_OCM_MASK;
            if ocm == 0 || self.oc[ch].halted {
                continue;
            }
            let sel = if con & OCCON_OCTSEL != 0 { 3 } else { 2 };
            if sel != which {
                continue;
            }
            let tmr = if which == 2 { self.tmr2 } else { self.tmr3 };
            let r = self.oc[ch].r;
            let rs = self.oc[ch].rs;
            match ocm {
                // Active-low one-shot: match → high, interrupt on rising.
                1 => {
                    if tmr == r {
                        self.oc_set_level(ch, true);
                        self.oc[ch].halted = true;
                        self.oc_irq(ch);
                    }
                }
                // Active-high one-shot: match → low, interrupt on falling.
                2 => {
                    if tmr == r {
                        self.oc_set_level(ch, false);
                        self.oc[ch].halted = true;
                        self.oc_irq(ch);
                    }
                }
                // Toggle: every match flips the pin; both edges interrupt.
                3 => {
                    if tmr == r {
                        let now = self.oc[ch].level.unwrap_or(false);
                        self.oc_set_level(ch, !now);
                        self.oc_irq(ch);
                    }
                }
                // Delayed one-shot / continuous pulse: OCxR → high,
                // OCxRS → low with the falling-edge interrupt.
                4 | 5 => {
                    if tmr == r {
                        self.oc_set_level(ch, true);
                    }
                    if tmr == rs {
                        self.oc_set_level(ch, false);
                        self.oc_irq(ch);
                        if ocm == 4 {
                            self.oc[ch].halted = true;
                        }
                    }
                }
                // PWM (fault pin not modelled, so mode 111 behaves as mode
                // 110 with OCFLT clear): match drives low; on rollover the
                // OCxRS shadow loads into OCxR and the pin follows.
                _ => {
                    if rollover {
                        self.oc[ch].r = rs;
                        let high = rs != 0;
                        self.oc_set_level(ch, high);
                    } else if tmr == self.oc[ch].r {
                        self.oc_set_level(ch, false);
                    }
                }
            }
        }
    }

    /// Set the OCx pad level, marking the mirror dirty only on a change.
    fn oc_set_level(&mut self, ch: usize, level: bool) {
        if self.oc[ch].level != Some(level) {
            self.oc[ch].level = Some(level);
            self.pads_dirty = true;
        }
    }

    /// Raise the OCx interrupt flag (both sets live in IFS0).
    fn oc_irq(&mut self, ch: usize) {
        self.ifs0 |= if ch == 0 {
            1 << IRQ_OC1.0
        } else {
            1 << IRQ_OC2.0
        };
    }

    /// Apply an `OCxCON` write: store the writable bits and initialise the
    /// pin per the selected mode (DS70209A Table 13-1). Every write also
    /// re-arms a completed one-shot — that is the documented re-trigger.
    fn oc_configure(&mut self, ch: usize, val: u16) {
        let old_level = self.oc[ch].level;
        self.oc[ch].con = val & (OCCON_OCSIDL | OCCON_OCTSEL | OCCON_OCM_MASK);
        self.oc[ch].halted = false;
        let ocm = self.oc[ch].con & OCCON_OCM_MASK;
        self.oc[ch].level = match ocm {
            0 => None,
            1 | 4 | 5 => Some(false),
            2 => Some(true),
            // Toggle keeps the pin where it was (a one-shot sets the state).
            3 => Some(old_level.unwrap_or(false)),
            _ => Some(self.oc[ch].r != 0),
        };
        self.pads_dirty = true;
    }

    /// Apply an `ICxCON` write. Disabling the module clears the FIFO and the
    /// ICBNE/ICOV flags (DS70198D §12.4).
    fn ic_configure(&mut self, ch: usize, val: u16) {
        self.ic[ch].con = val & (ICCON_ICSIDL | ICCON_ICTMR | ICCON_ICI_MASK | ICCON_ICM_MASK);
        if self.ic[ch].con & ICCON_ICM_MASK == 0 {
            self.ic[ch].fifo.clear();
            self.ic[ch].ici_count = 0;
            self.ic[ch].prescale = 0;
        }
    }

    /// Read (pop) `ICxBUF`: an empty buffer reads 0 (not modelled: the
    /// silicon value of a read from an empty FIFO is not documented).
    /// Draining the buffer clears ICBNE and, with it, the overrun ICOV
    /// (DS70198D §12.4.1/§12.4.2).
    fn ic_buf_read(&mut self, ch: usize) -> u16 {
        let val = self.ic[ch].fifo.pop_front().unwrap_or(0);
        if self.ic[ch].fifo.is_empty() {
            self.ic[ch].con &= !(ICCON_ICBNE | ICCON_ICOV);
        }
        val
    }

    /// Push one 16-bit word on the W15 stack (SRAM only, as on silicon).
    ///
    /// DS70283H Figure 4-4: "PUSH: [W15++]" — the stack grows toward higher
    /// addresses.
    fn push_word(&mut self, val: u16) -> SimResult<()> {
        let a = self.w[15];
        let end = RAM_BASE as u16 + RAM_BYTES as u16;
        if a >= RAM_BASE as u16 && a + 2 <= end {
            let o = a as usize - RAM_BASE as usize;
            self.ram[o..o + 2].copy_from_slice(&val.to_le_bytes());
            self.w[15] = self.w[15].wrapping_add(2);
            Ok(())
        } else {
            Err(SimulationError::MemoryViolation(u64::from(a)))
        }
    }

    /// Pop one 16-bit word from the W15 stack.
    ///
    /// DS70283H Figure 4-4: "POP: [--W15]".
    fn pop_word(&mut self) -> SimResult<u16> {
        self.w[15] = self.w[15].wrapping_sub(2);
        let a = self.w[15];
        let end = RAM_BASE as u16 + RAM_BYTES as u16;
        if a < RAM_BASE as u16 || a + 2 > end {
            return Err(SimulationError::MemoryViolation(u64::from(a)));
        }
        let o = a as usize - RAM_BASE as usize;
        Ok(u16::from_le_bytes([self.ram[o], self.ram[o + 1]]))
    }

    /// Read the 24-bit instruction word at a PC-unit address (program space).
    ///
    /// Used for the interrupt vector table, which stores ISR addresses as
    /// data (DS70283H §7.1).
    fn program_word(&self, addr_pc: u32) -> SimResult<u32> {
        if addr_pc & (PC_UNITS_PER_INSTRUCTION - 1) != 0 {
            return Err(SimulationError::DecodeError(u64::from(addr_pc)));
        }
        let off = (addr_pc as usize / PC_UNITS_PER_INSTRUCTION as usize) * INSTR_SLOT_BYTES;
        if off + 3 > FLASH_STORAGE_BYTES {
            return Err(SimulationError::MemoryViolation(off as u64));
        }
        Ok(u32::from_le_bytes([
            self.flash[off],
            self.flash[off + 1],
            self.flash[off + 2],
            0,
        ]))
    }

    /// Current CPU priority level: IPL<2:0> (SR<7:5>) plus IPL<3> (CORCON<3>).
    fn current_ipl(&self) -> u16 {
        ((self.sr >> 5) & 0x7) | (((self.corcon >> 3) & 1) << 3)
    }

    /// The highest-priority pending, enabled interrupt: (vector, priority).
    ///
    /// Natural order = IFS0 bit order (DS70283H §7.3.1). Simplified gating:
    /// DISI blocks everything we model (levels 1-6), NSTDIS blocks nesting.
    /// The priority field for `irq` from IPC0-2 or IPC14/15
    /// (DS70283H §7.3: four priority fields per register).
    fn ipc_priority(&self, irq: u32) -> u16 {
        let reg = irq / 4;
        let shift = 4 * (irq % 4);
        let ipc = match reg {
            0 => self.ipc0,
            1 => self.ipc1,
            2 => self.ipc2,
            3 => self.ipc3,
            4 => self.ipc4,
            14 => self.ipc14,
            15 => self.ipc15,
            _ => 0,
        };
        (ipc >> shift) & 0x7
    }

    fn pending_interrupt(&self) -> Option<(u32, u16)> {
        if self.intcon2 & INTCON2_DISI != 0 {
            return None;
        }
        let ipl = self.current_ipl();
        if self.intcon1 & INTCON1_NSTDIS != 0 && ipl > 0 {
            return None;
        }
        // (IFS register index, flag bit, IRQ number, vector number).
        let sources: [(u8, u16, u32, u32); 16] = [
            (0, IRQ_INT0.0, 0, IRQ_INT0.1),
            (0, IRQ_IC1.0, 1, IRQ_IC1.1),
            (0, IRQ_OC1.0, 2, IRQ_OC1.1),
            (0, IRQ_T1.0, 3, IRQ_T1.1),
            (0, IRQ_IC2.0, 5, IRQ_IC2.1),
            (0, IRQ_OC2.0, 6, IRQ_OC2.1),
            (0, IRQ_T2.0, 7, IRQ_T2.1),
            (0, IRQ_T3.0, 8, IRQ_T3.1),
            (0, IRQ_SPI1E.0, 9, IRQ_SPI1E.1),
            (0, IRQ_SPI1.0, 10, IRQ_SPI1.1),
            (0, IRQ_AD1.0, 13, IRQ_AD1.1),
            (1, IRQ_SI2C1.0, 16, IRQ_SI2C1.1),
            (1, IRQ_MI2C1.0, 17, IRQ_MI2C1.1),
            (3, IRQ_PWM1.0, 57, IRQ_PWM1.1),
            (3, IRQ_QEI.0, 58, IRQ_QEI.1),
            (3, IRQ_FLTA1.0, 63, IRQ_FLTA1.1),
        ];
        let mut best: Option<(u32, u16)> = None;
        for (reg, bit, irq, vector) in sources {
            let (flags, enables) = match reg {
                0 => (self.ifs0, self.iec0),
                1 => (self.ifs1, self.iec1),
                _ => (self.ifs3, self.iec3),
            };
            if flags & (1 << bit) == 0 || enables & (1 << bit) == 0 {
                continue;
            }
            let prio = self.ipc_priority(irq);
            if prio == 0 || prio <= ipl {
                continue;
            }
            // Highest priority wins; on a tie the lower vector number wins
            // (the hardware's fixed priority order).
            match best {
                None => best = Some((vector, prio)),
                Some((bv, bp)) if prio > bp || (prio == bp && vector < bv) => {
                    best = Some((vector, prio));
                }
                _ => {}
            }
        }
        best
    }

    /// Take the pending interrupt, if any: push the DS70283H §7 frame
    /// (PC<15:0>, then SR<7:0>|IPL3|PC<22:16>), raise IPL, then load the PC
    /// from the vector table.
    ///
    /// The vector location holds the 24-bit ISR address as **data**
    /// (DS70283H §7.1) — it is not executed. `INTCON2.ALTIVT` selects the
    /// Alternate IVT at `0x104` (DS70283H Figure 7-1).
    fn take_interrupt(&mut self) -> SimResult<bool> {
        let Some((vector, prio)) = self.pending_interrupt() else {
            return Ok(false);
        };
        let base = if self.intcon2 & INTCON2_ALTIVT != 0 {
            0x104
        } else {
            0x4
        };
        let target = self.program_word(base + 2 * vector)?;
        self.push_word(self.pc as u16)?;
        let packed = ((self.sr & 0x00FF) << 8)
            | (((self.corcon >> 3) & 1) << 7)
            | ((self.pc >> 16) as u16 & 0x007F);
        self.push_word(packed)?;
        self.sr = (self.sr & !0x00E0) | ((prio & 0x7) << 5);
        self.pc = target;
        Ok(true)
    }

    fn fetch(&self) -> SimResult<u32> {
        if self.pc & (PC_UNITS_PER_INSTRUCTION - 1) != 0 {
            // PC<0> is always 0 on silicon (DS70283H §4).
            return Err(SimulationError::DecodeError(self.pc as u64));
        }
        let off = (self.pc as usize / PC_UNITS_PER_INSTRUCTION as usize) * INSTR_SLOT_BYTES;
        if off + INSTR_SLOT_BYTES > FLASH_STORAGE_BYTES {
            return Err(SimulationError::MemoryViolation(off as u64));
        }
        Ok(u32::from_le_bytes([
            self.flash[off],
            self.flash[off + 1],
            self.flash[off + 2],
            0,
        ]))
    }

    /// Data-space read (word). DS70283H Figure 4-3 layout.
    ///
    /// `&mut self`: reading `ICxBUF` pops the capture FIFO (DS70198D §12.4).
    pub fn data_read(&mut self, addr: u16) -> SimResult<u16> {
        match addr {
            a if a < SFR_BYTES as u16 => match a {
                SFR_U1MODE => Ok(self.u1mode),
                // TRMT reads set once the (instant) shift completes; UTXBF never sticks.
                SFR_U1STA => Ok((self.u1sta | U1STA_TRMT) & !U1STA_UTXBF),
                SFR_U1TXREG => Ok(0),
                SFR_ACCAL => Ok((self.acc_a & 0xFFFF) as u16),
                SFR_ACCAH => Ok(((self.acc_a >> 16) & 0xFFFF) as u16),
                SFR_ACCAU => Ok(((self.acc_a >> 32) & 0x00FF) as u16),
                SFR_ACCBL => Ok((self.acc_b & 0xFFFF) as u16),
                SFR_ACCBH => Ok(((self.acc_b >> 16) & 0xFFFF) as u16),
                SFR_ACCBU => Ok(((self.acc_b >> 32) & 0x00FF) as u16),
                SFR_SR => Ok(self.sr),
                SFR_CORCON => Ok(self.corcon),
                SFR_INTCON1 => Ok(self.intcon1),
                SFR_INTCON2 => Ok(self.intcon2),
                SFR_IFS0 => Ok(self.ifs0),
                SFR_IEC0 => Ok(self.iec0),
                SFR_IPC0 => Ok(self.ipc0),
                SFR_IPC1 => Ok(self.ipc1),
                SFR_IPC2 => Ok(self.ipc2),
                SFR_TMR1 => Ok(self.tmr1),
                SFR_PR1 => Ok(self.pr1),
                SFR_T1CON => Ok(self.t1con),
                SFR_TMR2 => Ok(self.tmr2),
                SFR_TMR3 => Ok(self.tmr3),
                SFR_PR2 => Ok(self.pr2),
                SFR_PR3 => Ok(self.pr3),
                SFR_T2CON => Ok(self.t2con),
                SFR_T3CON => Ok(self.t3con),
                SFR_IC1CON => Ok(self.ic[0].con),
                SFR_IC2CON => Ok(self.ic[1].con),
                // Reading ICxBUF pops the FIFO.
                SFR_IC1BUF => Ok(self.ic_buf_read(0)),
                SFR_IC2BUF => Ok(self.ic_buf_read(1)),
                SFR_OC1CON => Ok(self.oc[0].con),
                SFR_OC2CON => Ok(self.oc[1].con),
                SFR_OC1R => Ok(self.oc[0].r),
                SFR_OC2R => Ok(self.oc[1].r),
                SFR_OC1RS => Ok(self.oc[0].rs),
                SFR_OC2RS => Ok(self.oc[1].rs),
                SFR_RPINR7 => Ok(self.rpinr7),
                SFR_P1TCON => Ok(self.p1tcon),
                SFR_P1TMR => Ok(self.p1tmr),
                SFR_P1TPER => Ok(self.p1tper),
                SFR_P1SECMP => Ok(self.p1secmp),
                SFR_PWM1CON1 => Ok(self.pwm1con1),
                SFR_PWM1CON2 => Ok(self.pwm1con2),
                SFR_P1DTCON1 => Ok(self.p1dtcon1),
                SFR_P1DTCON2 => Ok(self.p1dtcon2),
                SFR_P1FLTACON => Ok(self.p1fltacon),
                SFR_P1OVDCON => Ok(self.p1ovdcon),
                SFR_P1DC1 => Ok(self.p1dc[0]),
                SFR_P1DC2 => Ok(self.p1dc[1]),
                SFR_P1DC3 => Ok(self.p1dc[2]),
                SFR_QEI1CON => Ok(self.qei1con),
                SFR_DFLT1CON => Ok(self.dflt1con & 0x07F0),
                SFR_POS1CNT => Ok(self.pos1cnt),
                SFR_MAX1CNT => Ok(self.max1cnt),
                SFR_RPINR12 => Ok(self.rpinr12),
                SFR_RPINR14 => Ok(self.rpinr14),
                SFR_RPINR15 => Ok(self.rpinr15),
                SFR_IFS3 => Ok(self.ifs3),
                SFR_IEC3 => Ok(self.iec3),
                SFR_IPC14 => Ok(self.ipc14),
                SFR_IPC15 => Ok(self.ipc15),
                a if (SFR_RPOR0..SFR_RPOR0 + 2 * RPOR_COUNT as u16).contains(&a) => {
                    Ok(self.rpor[((a - SFR_RPOR0) / 2) as usize])
                }
                a if (SFR_ADC1BUF0..SFR_ADC1BUF0 + 2 * ADC1_BUF_COUNT as u16).contains(&a) => {
                    Ok(self.adc_buf[((a - SFR_ADC1BUF0) / 2) as usize])
                }
                SFR_AD1CON1 => Ok(self.ad1con1),
                // BUFS (bit 7) is read-only status, not a stored control bit.
                SFR_AD1CON2 => Ok((self.ad1con2 & !AD1CON2_BUFS) | (u16::from(self.adc_half) << 7)),
                SFR_AD1CON3 => Ok(self.ad1con3),
                SFR_AD1CHS123 => Ok(self.ad1chs123),
                SFR_AD1CHS0 => Ok(self.ad1chs0),
                SFR_AD1PCFGL => Ok(self.ad1pcfgl),
                SFR_AD1CSSL => Ok(self.ad1cssl),
                SFR_SPI1STAT => Ok(self.spi1stat),
                SFR_SPI1CON1 => Ok(self.spi1con1),
                SFR_SPI1CON2 => Ok(self.spi1con2),
                // Reading SPI1BUF returns RXB and clears SPIRBF.
                SFR_SPI1BUF => {
                    let v = self.spi1_rx;
                    self.spi1stat &= !SPI1STAT_SPIRBF;
                    Ok(v)
                }
                // Reading I2C1RCV clears RBF (DS70283H Register 18-2).
                SFR_I2C1RCV => {
                    let v = self.i2c1rcv;
                    self.i2c1stat &= !I2C1STAT_RBF;
                    Ok(v)
                }
                SFR_I2C1TRN => Ok(self.i2c1trn),
                SFR_I2C1BRG => Ok(self.i2c1brg),
                SFR_I2C1CON => Ok(self.i2c1con),
                SFR_I2C1STAT => Ok(self.i2c1stat),
                SFR_I2C1ADD => Ok(self.i2c1add),
                SFR_I2C1MSK => Ok(self.i2c1msk),
                SFR_IFS1 => Ok(self.ifs1),
                SFR_IEC1 => Ok(self.iec1),
                SFR_IPC3 => Ok(self.ipc3),
                SFR_IPC4 => Ok(self.ipc4),
                a if Self::port_sfr(a).is_some() => {
                    let (port, role) = Self::port_sfr(a).expect("guarded by port_sfr");
                    Ok(match role {
                        0 => self.tris[port],
                        1 => self.port_read(port),
                        2 => self.lat[port],
                        _ => self.odc[port],
                    })
                }
                _ => {
                    let o = a as usize;
                    Ok(u16::from_le_bytes([self.sfr[o], self.sfr[o + 1]]))
                }
            },
            a if (RAM_BASE as u16..(RAM_BASE as u16 + RAM_BYTES as u16)).contains(&a) => {
                let o = a as usize - RAM_BASE as usize;
                Ok(u16::from_le_bytes([self.ram[o], self.ram[o + 1]]))
            }
            a => Err(SimulationError::MemoryViolation(a as u64)),
        }
    }

    /// Data-space write (word). UART1 TX latches into the console sink.
    pub fn data_write(&mut self, addr: u16, val: u16) -> SimResult<()> {
        match addr {
            a if a < SFR_BYTES as u16 => {
                match a {
                    SFR_U1MODE => self.u1mode = val,
                    SFR_U1STA => self.u1sta = val,
                    SFR_U1TXREG => {
                        if let Some(sink) = &self.serial_sink {
                            if let Ok(mut buf) = sink.lock() {
                                buf.push((val & 0x00FF) as u8);
                            }
                        }
                    }
                    SFR_ACCAL => self.acc_a = (self.acc_a & !0xFFFF) | i64::from(val),
                    SFR_ACCAH => self.acc_a = (self.acc_a & !0xFFFF_0000) | (i64::from(val) << 16),
                    SFR_ACCAU => {
                        self.acc_a =
                            (self.acc_a & !0x00FF_0000_0000) | (i64::from(val & 0x00FF) << 32)
                    }
                    SFR_ACCBL => self.acc_b = (self.acc_b & !0xFFFF) | i64::from(val),
                    SFR_ACCBH => self.acc_b = (self.acc_b & !0xFFFF_0000) | (i64::from(val) << 16),
                    SFR_ACCBU => {
                        self.acc_b =
                            (self.acc_b & !0x00FF_0000_0000) | (i64::from(val & 0x00FF) << 32)
                    }
                    SFR_SR => {
                        // SA/SB/OA/OB are sticky, set by hardware and cleared
                        // only by software: a written 0 clears a flag, a
                        // written 1 keeps its old value (a write cannot set
                        // it). SAB/OAB are derived; writing 0 to SAB also
                        // clears SA and SB (DS70000157G §2.16.4 Notes 2-3).
                        let old = self.sr;
                        let keep = |bit: u16| if val & bit != 0 { old & bit } else { 0 };
                        let mut v = val & !(SR_OA | SR_OB | SR_SA | SR_SB | SR_OAB | SR_SAB);
                        v |= keep(SR_OA) | keep(SR_OB) | keep(SR_SA) | keep(SR_SB);
                        if val & SR_SAB == 0 {
                            v &= !(SR_SA | SR_SB);
                        }
                        if val & SR_OAB == 0 {
                            v &= !(SR_OA | SR_OB);
                        }
                        self.sr = v;
                        self.refresh_dsp_status();
                    }
                    SFR_CORCON => self.corcon = val,
                    SFR_INTCON1 => self.intcon1 = val,
                    SFR_INTCON2 => self.intcon2 = val,
                    SFR_IFS0 => self.ifs0 = val,
                    SFR_IEC0 => self.iec0 = val,
                    SFR_IPC0 => self.ipc0 = val,
                    SFR_IPC1 => self.ipc1 = val,
                    SFR_IPC2 => self.ipc2 = val,
                    SFR_TMR1 => self.tmr1 = val,
                    SFR_PR1 => self.pr1 = val,
                    SFR_T1CON => self.t1con = val,
                    SFR_TMR2 => self.tmr2 = val,
                    SFR_TMR3 => self.tmr3 = val,
                    SFR_PR2 => self.pr2 = val,
                    SFR_PR3 => self.pr3 = val,
                    SFR_T2CON => self.t2con = val,
                    SFR_T3CON => self.t3con = val,
                    SFR_IC1CON => self.ic_configure(0, val),
                    SFR_IC2CON => self.ic_configure(1, val),
                    // ICxBUF is a FIFO port; writes are not part of its use.
                    SFR_IC1BUF | SFR_IC2BUF => {}
                    SFR_OC1CON => self.oc_configure(0, val),
                    SFR_OC2CON => self.oc_configure(1, val),
                    SFR_OC1R => {
                        // Read-only while a PWM mode runs: OCxRS is its shadow
                        // (DS70209A §13.3.2.8).
                        if self.oc[0].con & OCCON_OCM_MASK < 0b110 {
                            self.oc[0].r = val;
                        }
                    }
                    SFR_OC2R => {
                        if self.oc[1].con & OCCON_OCM_MASK < 0b110 {
                            self.oc[1].r = val;
                        }
                    }
                    SFR_OC1RS => self.oc[0].rs = val,
                    SFR_OC2RS => self.oc[1].rs = val,
                    SFR_RPINR7 => self.rpinr7 = val & 0x1F1F,
                    SFR_P1TCON => self.pwm_configure_tcon(val),
                    SFR_P1TMR => {
                        self.p1tmr = (self.p1tmr & 0x8000) | (val & 0x7FFF);
                        self.p1_frac = 0;
                        self.p1_post = 0;
                    }
                    SFR_P1TPER => {
                        self.p1tper = val & 0x7FFF;
                        if self.p1tcon & PTCON_PTEN == 0 {
                            self.p1tper_active = self.p1tper;
                        }
                    }
                    SFR_P1SECMP => self.p1secmp = val & 0x7FFF,
                    SFR_PWM1CON1 => {
                        self.pwm1con1 = val & 0x0777;
                        self.pads_dirty = true;
                    }
                    SFR_PWM1CON2 => self.pwm1con2 = val & 0x0F07,
                    SFR_P1DTCON1 => self.p1dtcon1 = val,
                    SFR_P1DTCON2 => self.p1dtcon2 = val & 0x003F,
                    SFR_P1FLTACON => {
                        let old = self.p1fltacon;
                        self.p1fltacon = val & 0x3F87;
                        // A fault already asserted when FAEN is first enabled
                        // is a new fault event (the flagged edge happened
                        // while the input was disabled).
                        if self.p1fltacon & 0x7 != 0 && old & 0x7 == 0 && self.flta_prev {
                            self.ifs3 |= 1 << IRQ_FLTA1.0;
                            if self.p1fltacon & 0x0080 == 0 {
                                self.p1_fault_latched = true;
                            }
                            self.p1_fault_force = true;
                            self.pads_dirty = true;
                        }
                    }
                    SFR_P1OVDCON => {
                        self.p1ovdcon = val & 0x3FFF;
                        self.pads_dirty = true;
                    }
                    SFR_P1DC1 => self.pwm_duty_write(0, val),
                    SFR_P1DC2 => self.pwm_duty_write(1, val),
                    SFR_P1DC3 => self.pwm_duty_write(2, val),
                    SFR_QEI1CON => self.qei_configure(val),
                    SFR_DFLT1CON => self.dflt1con = val & 0x07F0,
                    SFR_POS1CNT => self.pos1cnt = val,
                    SFR_MAX1CNT => self.max1cnt = val,
                    SFR_RPINR12 => self.rpinr12 = val & 0x001F,
                    SFR_RPINR14 => self.rpinr14 = val & 0x1F1F,
                    SFR_RPINR15 => self.rpinr15 = val & 0x001F,
                    SFR_IFS3 => self.ifs3 = val,
                    SFR_IEC3 => self.iec3 = val,
                    SFR_IPC14 => self.ipc14 = val,
                    SFR_IPC15 => self.ipc15 = val,
                    SFR_AD1CON1 => {
                        let old = self.ad1con1;
                        // ADON, ADSIDL, AD12B, FORM, SSRC, SIMSAM and ASAM are
                        // writable; SAMP/DONE follow the state machine
                        // (DS70183D Register 16-1).
                        let writable = 0x8000 | 0x2000 | 0x0400 | 0x0300 | 0x00E0 | 0x0008 | 0x0004;
                        let mut v = (old & !writable) | (val & writable);
                        if val & AD1CON1_DONE != 0 {
                            v |= old & AD1CON1_DONE;
                        } else {
                            v &= !AD1CON1_DONE;
                        }
                        let was_on = old & AD1CON1_ADON != 0;
                        let now_on = v & AD1CON1_ADON != 0;
                        if !now_on {
                            v &= !(AD1CON1_SAMP | AD1CON1_DONE);
                            self.ad1con1 = v;
                            self.adc_state = AdcState::Off;
                            self.adc_conv_remaining = 0;
                            self.adc_sample_remaining = 0;
                        } else if !was_on {
                            // Enabling the module does not start sampling in
                            // manual mode; ADON just powers the ADC.
                            self.adc_t3_trigger = false;
                            self.adc_pwm_trigger = false;
                            if v & 0x0004 != 0 {
                                v |= AD1CON1_SAMP;
                                self.ad1con1 = v;
                                self.adc_start_sampling();
                            } else {
                                self.ad1con1 = v & !AD1CON1_SAMP;
                                self.adc_state = AdcState::Off;
                            }
                        } else {
                            let old_samp = old & AD1CON1_SAMP != 0;
                            let want_samp = val & AD1CON1_SAMP != 0;
                            let ssrc = (v >> 5) & 0x7;
                            if old_samp && !want_samp && ssrc == 0 {
                                self.ad1con1 = v & !AD1CON1_SAMP;
                                self.adc_start_conversion();
                            } else if !old_samp && want_samp {
                                self.ad1con1 = v | AD1CON1_SAMP;
                                self.adc_start_sampling();
                            } else {
                                self.ad1con1 = v | (old & AD1CON1_SAMP);
                            }
                        }
                    }
                    SFR_AD1CON2 => {
                        self.ad1con2 = val
                            & (0xE000
                                | AD1CON2_CSCNA
                                | 0x0300
                                | 0x003C
                                | AD1CON2_BUFM
                                | AD1CON2_ALTS);
                    }
                    SFR_AD1CON3 => self.ad1con3 = val & 0x9FFF,
                    SFR_AD1CHS123 => self.ad1chs123 = val & 0x0707,
                    SFR_AD1CHS0 => self.ad1chs0 = val & 0x9F9F,
                    SFR_AD1PCFGL => self.ad1pcfgl = val & 0x01FF,
                    SFR_AD1CSSL => self.ad1cssl = val & 0x01FF,
                    // ADC1BUF0-15 are read-only.
                    a if (SFR_ADC1BUF0..SFR_ADC1BUF0 + 2 * ADC1_BUF_COUNT as u16).contains(&a) => {}
                    SFR_SPI1STAT => {
                        let mut v = (self.spi1stat
                            & (SPI1STAT_SPITBF | SPI1STAT_SPIRBF | SPI1STAT_SPIROV))
                            | (val & (SPI1STAT_SPIEN | 0x2000));
                        if val & SPI1STAT_SPIROV == 0 {
                            v &= !SPI1STAT_SPIROV;
                        }
                        if v & SPI1STAT_SPIEN == 0 {
                            self.spi1_shifting = false;
                            self.spi1_busy = 0;
                            self.spi1_tx_pending = None;
                            v &= !(SPI1STAT_SPITBF | SPI1STAT_SPIRBF | SPI1STAT_SPIROV);
                        }
                        self.spi1stat = v;
                    }
                    SFR_SPI1CON1 => self.spi1con1 = val & 0x1FFF,
                    SFR_SPI1CON2 => self.spi1con2 = val & 0xE002,
                    SFR_SPI1BUF => {
                        if self.spi1stat & SPI1STAT_SPIEN == 0 {
                            // Module off: the write is ignored.
                        } else if self.spi1con1 & SPI1CON1_MSTEN != 0 {
                            if self.spi1_shifting {
                                self.spi1_tx_pending = Some(val);
                                self.spi1stat |= SPI1STAT_SPITBF;
                            } else {
                                self.spi_start_transfer(val);
                            }
                        } else {
                            // Slave: load TXB and wait for an external clock.
                            self.spi1_tx = val;
                            self.spi1stat |= SPI1STAT_SPITBF;
                        }
                    }
                    SFR_I2C1RCV => {}
                    SFR_I2C1TRN => {
                        if self.i2c1con & I2C1CON_I2CEN != 0 {
                            if self.i2c_op.is_some() {
                                self.i2c1stat |= I2C1STAT_IWCOL;
                            } else {
                                self.i2c1trn = val & 0x00FF;
                                self.i2c1stat |= I2C1STAT_TBF | I2C1STAT_TRSTAT;
                                self.i2c_op = Some(I2cOp::Transmit);
                                self.i2c_busy = self.i2c_op_cycles(9);
                            }
                        }
                    }
                    SFR_I2C1BRG => self.i2c1brg = val & 0x0FFF,
                    SFR_I2C1CON => {
                        let old = self.i2c1con;
                        let was_enabled = old & I2C1CON_I2CEN != 0;
                        // Static writable bits (I2CEN..ACKDT); the HC request
                        // bits are latched below and cleared on completion.
                        let mut v = (val & 0xBFE0) | (old & 0x001F);
                        if val & I2C1CON_I2CEN == 0 {
                            self.i2c1con = v & !0x001F;
                            self.i2c_op = None;
                            self.i2c_busy = 0;
                            if let Some(idx) = self.i2c_target.take() {
                                self.i2c_devices[idx].stop();
                            }
                            self.i2c_addressed = false;
                            self.i2c_reading = false;
                            self.i2c_10bit_prefix = None;
                            self.i2c1stat = 0;
                        } else {
                            if !was_enabled {
                                self.i2c1stat = 0;
                            }
                            if self.i2c_op.is_none() {
                                let request = if val & I2C1CON_SEN != 0 {
                                    Some((I2cOp::Start, I2C1CON_SEN, 1))
                                } else if val & I2C1CON_RSEN != 0 {
                                    Some((I2cOp::RepeatedStart, I2C1CON_RSEN, 1))
                                } else if val & I2C1CON_PEN != 0 {
                                    Some((I2cOp::Stop, I2C1CON_PEN, 1))
                                } else if val & I2C1CON_RCEN != 0 {
                                    Some((I2cOp::Receive, I2C1CON_RCEN, 9))
                                } else if val & I2C1CON_ACKEN != 0 {
                                    Some((I2cOp::Ack, I2C1CON_ACKEN, 9))
                                } else {
                                    None
                                };
                                if let Some((op, bit, bits)) = request {
                                    v |= bit;
                                    self.i2c_op = Some(op);
                                    self.i2c_busy = self.i2c_op_cycles(bits);
                                }
                            }
                            self.i2c1con = v;
                        }
                    }
                    SFR_I2C1STAT => {
                        // ACKSTAT/TRSTAT/ADD10/DA/P/S/R_W/RBF/TBF are
                        // read-only; BCL/IWCOL/I2COV are clear-on-0 (a
                        // written 0 clears, a written 1 keeps the old bit).
                        let sticky = I2C1STAT_BCL | I2C1STAT_IWCOL | I2C1STAT_I2COV;
                        self.i2c1stat &= !sticky | val;
                    }
                    SFR_I2C1ADD => self.i2c1add = val & 0x03FF,
                    SFR_I2C1MSK => self.i2c1msk = val & 0x03FF,
                    SFR_IFS1 => self.ifs1 = val,
                    SFR_IEC1 => self.iec1 = val,
                    SFR_IPC3 => self.ipc3 = val,
                    SFR_IPC4 => self.ipc4 = val,
                    a if (SFR_RPOR0..SFR_RPOR0 + 2 * RPOR_COUNT as u16).contains(&a) => {
                        self.rpor[((a - SFR_RPOR0) / 2) as usize] = val;
                        self.pads_dirty = true;
                    }
                    a if Self::port_sfr(a).is_some() => {
                        let (port, role) = Self::port_sfr(a).expect("guarded by port_sfr");
                        let mask = Self::port_mask(port);
                        match role {
                            0 => self.tris[port] = val & mask,
                            // Writing PORTx writes the port data latch
                            // (DS70190D §30.2.2), the same as LATx.
                            1 | 2 => self.lat[port] = val & mask,
                            _ => self.odc[port] = val & mask,
                        }
                        self.pads_dirty = true;
                    }
                    _ => {
                        let o = a as usize;
                        self.sfr[o..o + 2].copy_from_slice(&val.to_le_bytes());
                    }
                }
                Ok(())
            }
            a if (RAM_BASE as u16..(RAM_BASE as u16 + RAM_BYTES as u16)).contains(&a) => {
                let o = a as usize - RAM_BASE as usize;
                self.ram[o..o + 2].copy_from_slice(&val.to_le_bytes());
                Ok(())
            }
            a => Err(SimulationError::MemoryViolation(a as u64)),
        }
    }

    // ── Base ISA helpers (DS70000157G) ──────────────────────────────────────

    /// Set N/Z from a word or byte result. DS70000157G §4.4: for byte
    /// operations "the STATUS Register is updated to reflect the result of
    /// the byte operation".
    fn set_nz(&mut self, value: u16, byte: bool) {
        let v = if byte { value & 0x00FF } else { value };
        let sign = if byte { 0x0080 } else { 0x8000 };
        self.sr &= !(SR_N | SR_Z);
        if v & sign != 0 {
            self.sr |= SR_N;
        }
        if v == 0 {
            self.sr |= SR_Z;
        }
    }

    /// Add with the MCU ALU flags (DS70000157G §2.16.1); returns the result.
    fn alu_add(&mut self, a: u16, b: u16, cin: u16, byte: bool) -> u16 {
        let (a32, b32, mask, sign) = if byte {
            (u32::from(a & 0xFF), u32::from(b & 0xFF), 0xFF, 0x80)
        } else {
            (u32::from(a), u32::from(b), 0xFFFF, 0x8000)
        };
        let c32 = u32::from(cin);
        let sum = a32 + b32 + c32;
        let res = (sum & mask) as u16;
        let carry = sum > mask;
        let digit = (a32 & 0xF) + (b32 & 0xF) + c32 > 0xF;
        let ov = ((a32 ^ b32) & sign == 0) && ((a32 ^ u32::from(res)) & sign != 0);
        self.sr &= !(SR_C | SR_DC | SR_OV);
        if carry {
            self.sr |= SR_C;
        }
        if digit {
            self.sr |= SR_DC;
        }
        if ov {
            self.sr |= SR_OV;
        }
        self.set_nz(res, byte);
        res
    }

    /// Subtract with the MCU ALU flags; returns the result. C reads as
    /// "Carry (not Borrow)": it is **set when no borrow occurred**
    /// (DS70000157G Table 4-8; CPB example: `0x20 - 0x20` leaves C = 1).
    fn alu_sub(&mut self, a: u16, b: u16, cin: u16, byte: bool) -> u16 {
        let (a32, b32, mask, sign) = if byte {
            (u32::from(a & 0xFF), u32::from(b & 0xFF), 0xFF, 0x80)
        } else {
            (u32::from(a), u32::from(b), 0xFFFF, 0x8000)
        };
        let c32 = u32::from(cin);
        let borrow_in = b32 + c32;
        let res = (a32.wrapping_sub(borrow_in) & mask) as u16;
        let no_borrow = a32 >= borrow_in;
        let digit = (a32 & 0xF) >= (b32 & 0xF) + c32;
        let ov = ((a32 ^ b32) & sign != 0) && ((a32 ^ u32::from(res)) & sign != 0);
        self.sr &= !(SR_C | SR_DC | SR_OV);
        if no_borrow {
            self.sr |= SR_C;
        }
        if digit {
            self.sr |= SR_DC;
        }
        if ov {
            self.sr |= SR_OV;
        }
        self.set_nz(res, byte);
        res
    }

    /// Evaluate a `BRA` condition code (DS70000157G Table 4-8). Codes
    /// `0x1`/`0x9` are the `GEU`/`LTU` aliases of `C`/`NC`.
    fn branch_taken(&self, cond: u32) -> bool {
        let c = self.sr & SR_C != 0;
        let z = self.sr & SR_Z != 0;
        let n = self.sr & SR_N != 0;
        let ov = self.sr & SR_OV != 0;
        match cond {
            0x0 => ov,
            0x1 => c,
            0x2 => z,
            0x3 => n,
            0x4 => z || (n != ov),
            0x5 => n != ov,
            0x6 => c || z,
            0x8 => !ov,
            0x9 => !c,
            0xA => !z,
            0xB => !n,
            0xC => !z && (n == ov),
            0xD => n == ov,
            0xE => c && !z,
            _ => false,
        }
    }

    /// Resolve a `ppp`/`ggg` source operand (DS70000157G Tables 5-2/5-5).
    /// Post/pre-modified registers step by 1 for byte operations and 2 for
    /// word operations (DS70000157G Example 4-4).
    fn read_operand(&mut self, reg: usize, mode: u32, byte: bool) -> SimResult<u16> {
        let step = if byte { 1 } else { 2 };
        match mode {
            0 => Ok(self.w[reg]),
            1 => self.read_indirect(self.w[reg], byte),
            2 => {
                let v = self.read_indirect(self.w[reg], byte)?;
                self.w[reg] = self.w[reg].wrapping_sub(step);
                Ok(v)
            }
            3 => {
                let v = self.read_indirect(self.w[reg], byte)?;
                self.w[reg] = self.w[reg].wrapping_add(step);
                Ok(v)
            }
            4 => {
                self.w[reg] = self.w[reg].wrapping_sub(step);
                self.read_indirect(self.w[reg], byte)
            }
            5 => {
                self.w[reg] = self.w[reg].wrapping_add(step);
                self.read_indirect(self.w[reg], byte)
            }
            _ => Err(SimulationError::DecodeError(u64::from(self.pc))),
        }
    }

    /// Store a `qqq`/`hhh` destination operand (DS70000157G Tables 5-3/5-6).
    fn write_operand(&mut self, reg: usize, mode: u32, val: u16, byte: bool) -> SimResult<()> {
        let step = if byte { 1 } else { 2 };
        match mode {
            0 => {
                if byte {
                    self.w[reg] = (self.w[reg] & 0xFF00) | (val & 0x00FF);
                } else {
                    self.w[reg] = val;
                }
                Ok(())
            }
            1 => self.write_indirect(self.w[reg], val, byte),
            2 => {
                let a = self.w[reg];
                self.write_indirect(a, val, byte)?;
                self.w[reg] = self.w[reg].wrapping_sub(step);
                Ok(())
            }
            3 => {
                let a = self.w[reg];
                self.write_indirect(a, val, byte)?;
                self.w[reg] = self.w[reg].wrapping_add(step);
                Ok(())
            }
            4 => {
                self.w[reg] = self.w[reg].wrapping_sub(step);
                let a = self.w[reg];
                self.write_indirect(a, val, byte)
            }
            5 => {
                self.w[reg] = self.w[reg].wrapping_add(step);
                let a = self.w[reg];
                self.write_indirect(a, val, byte)
            }
            _ => Err(SimulationError::DecodeError(u64::from(self.pc))),
        }
    }

    fn read_indirect(&mut self, addr: u16, byte: bool) -> SimResult<u16> {
        if byte {
            Ok(u16::from(self.data_read_byte(addr)?))
        } else {
            self.data_read(addr)
        }
    }

    fn write_indirect(&mut self, addr: u16, val: u16, byte: bool) -> SimResult<()> {
        if byte {
            self.data_write_byte(addr, val as u8)
        } else {
            self.data_write(addr, val)
        }
    }

    /// Byte read at a data-space byte address (little-endian: even = low).
    pub fn data_read_byte(&mut self, addr: u16) -> SimResult<u8> {
        let word = self.data_read(addr & !1)?;
        Ok(if addr & 1 == 0 {
            word as u8
        } else {
            (word >> 8) as u8
        })
    }

    /// Byte write at a data-space byte address. File registers are updated
    /// with a read-modify-write of the containing word, so word-only side
    /// effects (U1STA's status reads, ICxBUF pops) keep their semantics.
    pub fn data_write_byte(&mut self, addr: u16, val: u8) -> SimResult<()> {
        let word_addr = addr & !1;
        let word = self.data_read(word_addr)?;
        let new = if addr & 1 == 0 {
            (word & 0xFF00) | u16::from(val)
        } else {
            (word & 0x00FF) | (u16::from(val) << 8)
        };
        self.data_write(word_addr, new)
    }

    /// The `0www wBqq qddd dxxx xxxx` fields shared by the 3-operand
    /// MCU instructions (DS70000157G: `ADD Wb,Ws,Wd` and friends).
    fn decode_3op(word: u32) -> ThreeOp {
        let lit = if (word >> 5) & 0x3 == 0b11 {
            Some((word & 0x1F) as u16)
        } else {
            None
        };
        ThreeOp {
            byte: word & 0x0000_4000 != 0,
            wb: ((word >> 15) & 0xF) as usize,
            qqq: (word >> 11) & 0x7,
            wd: ((word >> 7) & 0xF) as usize,
            lit,
            ppp: (word >> 4) & 0x7,
            ws: (word & 0xF) as usize,
        }
    }

    /// Keep the derived OAB/SAB bits equal to OA|OB and SA|SB
    /// (DS70000157G §2.16.4).
    fn refresh_dsp_status(&mut self) {
        let oab = self.sr & (SR_OA | SR_OB) != 0;
        let sab = self.sr & (SR_SA | SR_SB) != 0;
        self.sr = (self.sr & !(SR_OAB | SR_SAB))
            | if oab { SR_OAB } else { 0 }
            | if sab { SR_SAB } else { 0 };
    }

    // ── DSP engine: MAC class (DS70000157G §4.15, §5.4) ─────────────────────

    /// The DSP multiplier result for `Wm*Wn`: signed or unsigned per
    /// CORCON.US, left-shifted by one in fractional mode (CORCON.IF = 0)
    /// because the 2.30 product of two 1.15 values is kept in the 1.31
    /// accumulator format (DS70000157G §4.12.1).
    fn dsp_multiply(&self, wm: u16, wn: u16) -> i64 {
        let product = if self.corcon & CORCON_US != 0 {
            i64::from(wm) * i64::from(wn)
        } else {
            i64::from(wm as i16) * i64::from(wn as i16)
        };
        if self.corcon & CORCON_IF == 0 {
            product << 1
        } else {
            product
        }
    }

    /// Store a result to an accumulator with the CORCON saturation rules and
    /// the OA/OB/SA/SB status updates (DS70000157G §2.16.4, §4.12.1, §4.13):
    ///
    /// * saturation disabled — the 40-bit adder result is kept, a guard-bit
    ///   overflow (outside the signed 32-bit range) sets OA/OB, an overflow
    ///   out of the accumulator MSb sets SA/SB;
    /// * normal (1.31) — a result outside signed 32-bit clamps to
    ///   0x7FFF_FFFF / 0x8000_0000 and sets SA/SB (no OA/OB: the clamped
    ///   value never uses the guard bits);
    /// * super (9.31) — a result outside signed 40-bit clamps to
    ///   0x7F_FFFF_FFFF / 0x80_0000_0000 and sets SA/SB; guard-bit overflow
    ///   sets OA/OB.
    fn dsp_store_acc(&mut self, acc: usize, value: i64) {
        let sat_enabled = self.corcon & if acc == 0 { CORCON_SATA } else { CORCON_SATB } != 0;
        let super_sat = self.corcon & CORCON_ACCSAT != 0;
        let fits32 = fits_signed32(value);
        let fits40 = fits_signed40(value);
        // OA/OB report a *guard-bit* overflow (the exact result no longer
        // fits the signed 32-bit range) while SA/SB report *saturation* (the
        // result left the active accumulator range). That is why the two
        // flags use different predicates (DS70000157G §2.16.4).
        let (stored, saturated, guard_overflow) = if !sat_enabled {
            (wrap40(value), !fits40, !fits32)
        } else if super_sat {
            let stored = value.clamp(ACC40_MIN, ACC40_MAX);
            (stored, !fits40, !fits32)
        } else {
            let stored = value.clamp(ACC32_MIN, ACC32_MAX);
            (stored, !fits32, false)
        };
        if guard_overflow {
            self.sr |= if acc == 0 { SR_OA } else { SR_OB };
        }
        if saturated {
            self.sr |= if acc == 0 { SR_SA } else { SR_SB };
        }
        self.refresh_dsp_status();
        if acc == 0 {
            self.acc_a = stored;
        } else {
            self.acc_b = stored;
        }
    }

    /// One X or Y data prefetch (DS70000157G Tables 5-7/5-9, §4.15.1-4.15.2).
    /// The X path uses W8/W9, the Y path W10/W11; `op == 0b1100` is the
    /// W12 register-offset mode (16-bit signed offset, no updates).
    fn dsp_prefetch(&mut self, x_space: bool, op: u32, dest: usize) -> SimResult<()> {
        if op == 0b0100 {
            return Ok(());
        }
        let base = match (x_space, op & 0b1000 != 0) {
            (true, false) => 8,
            (true, true) => 9,
            (false, false) => 10,
            (false, true) => 11,
        };
        let (ea, delta) = if op == 0b1100 {
            let offset = i32::from(self.w[12] as i16);
            ((i32::from(self.w[base]) + offset) as u16, 0i32)
        } else {
            let delta = match op & 0b111 {
                0 => 0,
                1 => 2,
                2 => 4,
                3 => 6,
                5 => -6,
                6 => -4,
                _ => -2,
            };
            (self.w[base], delta)
        };
        let value = self.data_read(ea)?;
        self.w[dest] = value;
        if delta != 0 {
            self.w[base] = (i32::from(self.w[base]) + delta) as u16;
        }
        Ok(())
    }

    /// Rounding decision for SAC.R and accumulator write-back
    /// (DS70000157G CORCON.RND; Microchip "Rounding" note): conventional
    /// (biased) rounds up when the low word is >= 0x8000; convergent
    /// (unbiased) rounds a 0x8000 tie to even.
    fn dsp_round_up(&self, hi: u16, lo: u16) -> bool {
        if self.corcon & CORCON_RND != 0 {
            lo >= 0x8000
        } else {
            lo > 0x8000 || (lo == 0x8000 && hi & 1 == 1)
        }
    }

    /// SATDW (CORCON<5>): a DSP data-space store saturates to 0x7FFF/0x8000
    /// when the stored value needs the guard bits. The PRM's SAC examples
    /// pin the test: `FF C891 8F4C << 4` (40-bit `FC 8918 F4C0`) stores
    /// 0x8000, while a value whose bits 39:32 are sign extension keeps its
    /// high word (`00 120F FF00 >> 4` stores 0x0120).
    fn dsp_satdw(&self, value: i64, stored: u16) -> u16 {
        if self.corcon & CORCON_SATDW != 0 && !fits_signed32(value) {
            if value < 0 {
                0x8000
            } else {
                0x7FFF
            }
        } else {
            stored
        }
    }

    /// Accumulator write-back of the *other* accumulator (DS70000157G
    /// Table 5-13): `00` stores the rounded ACCxH to W13, `01` to [W13] with
    /// a post-increment of two, `10`/`11` do nothing (no write-back).
    fn dsp_write_back(&mut self, acc: usize, awb: u32) -> SimResult<()> {
        if awb >= 2 {
            return Ok(());
        }
        // The operation uses `acc`; the write-back stores the other one.
        let value = if acc == 0 { self.acc_b } else { self.acc_a };
        let hi = ((value >> 16) & 0xFFFF) as u16;
        let lo = (value & 0xFFFF) as u16;
        // The accumulator is 40-bit and the rounding carry adds at most
        // 2^16, so `rounded` still fits 41 bits: SATDW compares the true
        // value (a value above 2^39 is positive and saturates to 0x7FFF) and
        // the stored word is its ACCxH extract.
        let rounded = if self.dsp_round_up(hi, lo) {
            value.wrapping_add(0x1_0000)
        } else {
            value
        };
        let stored = self.dsp_satdw(rounded, ((rounded >> 16) & 0xFFFF) as u16);
        if awb == 0 {
            self.w[13] = stored;
        } else {
            let ea = self.w[13];
            self.data_write(ea, stored)?;
            self.w[13] = self.w[13].wrapping_add(2);
        }
        Ok(())
    }

    /// MAC / MPY / MPY.N / MSC (DS70000157G §5.4). `square` selects the
    /// same-register form (`1111 00mm ...`, no write-back).
    fn exec_dsp_mac(&mut self, word: u32, square: bool) -> SimResult<()> {
        let acc = ((word >> 15) & 1) as usize;
        let (wm, wn) = if square {
            let w = 4 + ((word >> 16) & 0x3) as usize;
            (self.w[w], self.w[w])
        } else {
            let (a, b) = match (word >> 16) & 0x7 {
                0 => (4, 5),
                1 => (4, 6),
                2 => (4, 7),
                4 => (5, 6),
                5 => (5, 7),
                6 => (6, 7),
                _ => return Err(SimulationError::DecodeError(u64::from(self.pc))),
            };
            (self.w[a], self.w[b])
        };
        let negate = word & 0x0000_4000 != 0;
        let store = if square {
            word & 0x3 == 0b01
        } else {
            word & 0x3 == 0b11
        };
        let product = self.dsp_multiply(wm, wn);
        if store && negate {
            // MPY.N lists OA/OB/OAB only (DS70000157G MPY.N page): the
            // negated product is stored without saturation and without SA/SB.
            let negated = -product;
            if !fits_signed32(negated) {
                self.sr |= if acc == 0 { SR_OA } else { SR_OB };
            }
            self.refresh_dsp_status();
            if acc == 0 {
                self.acc_a = wrap40(negated);
            } else {
                self.acc_b = wrap40(negated);
            }
        } else if store {
            self.dsp_store_acc(acc, product);
        } else {
            let current = if acc == 0 { self.acc_a } else { self.acc_b };
            // MAC adds the product, MSC subtracts it.
            let result = if negate {
                current - product
            } else {
                current + product
            };
            self.dsp_store_acc(acc, result);
        }
        let x_op = (word >> 6) & 0xF;
        let y_op = (word >> 2) & 0xF;
        let x_dest = 4 + ((word >> 12) & 0x3) as usize;
        let y_dest = 4 + ((word >> 10) & 0x3) as usize;
        self.dsp_prefetch(true, x_op, x_dest)?;
        self.dsp_prefetch(false, y_op, y_dest)?;
        if !square {
            self.dsp_write_back(acc, word & 0x3)?;
        }
        Ok(())
    }

    /// `CLR Acc {,prefetch} {,AWB}` (DS70000157G): clear the 40-bit
    /// accumulator and its OA/SA (or OB/SB) status bits, optionally
    /// prefetching and writing back the other accumulator.
    fn exec_dsp_clr(&mut self, word: u32) -> SimResult<()> {
        let acc = ((word >> 15) & 1) as usize;
        if acc == 0 {
            self.acc_a = 0;
            self.sr &= !(SR_OA | SR_SA);
        } else {
            self.acc_b = 0;
            self.sr &= !(SR_OB | SR_SB);
        }
        self.refresh_dsp_status();
        let x_op = (word >> 6) & 0xF;
        let y_op = (word >> 2) & 0xF;
        let x_dest = 4 + ((word >> 12) & 0x3) as usize;
        let y_dest = 4 + ((word >> 10) & 0x3) as usize;
        self.dsp_prefetch(true, x_op, x_dest)?;
        self.dsp_prefetch(false, y_op, y_dest)?;
        self.dsp_write_back(acc, word & 0x3)?;
        Ok(())
    }

    /// `MOVSAC Acc {,prefetch} {,AWB}` — prefetches and write-back only; no
    /// accumulator operation and no status (DS70000157G).
    fn exec_dsp_movsac(&mut self, word: u32) -> SimResult<()> {
        let acc = ((word >> 15) & 1) as usize;
        let x_op = (word >> 6) & 0xF;
        let y_op = (word >> 2) & 0xF;
        let x_dest = 4 + ((word >> 12) & 0x3) as usize;
        let y_dest = 4 + ((word >> 10) & 0x3) as usize;
        self.dsp_prefetch(true, x_op, x_dest)?;
        self.dsp_prefetch(false, y_op, y_dest)?;
        self.dsp_write_back(acc, word & 0x3)?;
        Ok(())
    }

    /// `LAC Ws|[...], {#Slit4,} Acc` (DS70000157G): load a 1.15 value into
    /// the accumulator with an optional shift (negative = left).
    ///
    /// The PRM's examples 1-2 pin the shifter as **32-bit**: the value is
    /// placed in ACCxH:ACCxL, the shift wraps/extends within 32 bits and
    /// ACCxU is the sign extension of the result — example 1 (`0x1221 << 3`)
    /// stores `FF 9108 0000` with OB set, not the 40-bit `00 9108 0000`.
    /// The overflow test uses the exact shifted value, so a right shift never
    /// flags (example 2 has SR = 0).
    fn exec_dsp_lac(&mut self, word: u32) -> SimResult<()> {
        let acc = ((word >> 15) & 1) as usize;
        let wb = ((word >> 11) & 0xF) as usize;
        let shift = ((word >> 7) & 0xF) as u8;
        let mode = (word >> 4) & 0x7;
        let ws = (word & 0xF) as usize;
        let src = match mode {
            0 => self.w[ws],
            6 => {
                let ea = self.w[ws].wrapping_add(self.w[wb]);
                self.data_read(ea)?
            }
            _ => self.read_operand(ws, mode, false)?,
        };
        let slit = i32::from(shift) - if shift >= 8 { 16 } else { 0 }; // -8..=7
        let base = i64::from(src as i16) << 16;
        let true_value = if slit < 0 {
            base << (-slit)
        } else {
            base >> slit
        };
        let shifted32 = if slit < 0 {
            (base as i32).wrapping_shl((-slit) as u32)
        } else {
            (base as i32) >> slit
        };
        let sat_enabled = self.corcon & if acc == 0 { CORCON_SATA } else { CORCON_SATB } != 0;
        let super_sat = self.corcon & CORCON_ACCSAT != 0;
        let fits32 = fits_signed32(true_value);
        let stored = if !fits32 && sat_enabled && !super_sat {
            if true_value > 0 {
                ACC32_MAX
            } else {
                ACC32_MIN
            }
        } else if !fits32 && sat_enabled && super_sat {
            // Super saturation keeps the guard bits: store the exact 40-bit
            // result (a 32-bit source shifted by at most 8 always fits).
            true_value
        } else {
            i64::from(shifted32)
        };
        if !fits32 && (!sat_enabled || super_sat) {
            self.sr |= if acc == 0 { SR_OA } else { SR_OB };
        }
        if !fits32 && sat_enabled && !super_sat {
            self.sr |= if acc == 0 { SR_SA } else { SR_SB };
        }
        self.refresh_dsp_status();
        if acc == 0 {
            self.acc_a = stored;
        } else {
            self.acc_b = stored;
        }
        Ok(())
    }

    /// `SAC`/`SAC.R` (DS70000157G): store ACCxH after an optional 40-bit
    /// arithmetic shift, with SATDW saturation (status is not affected).
    fn exec_dsp_sac(&mut self, word: u32, rounded: bool) -> SimResult<()> {
        let acc = ((word >> 15) & 1) as usize;
        let wb = ((word >> 11) & 0xF) as usize;
        let shift = ((word >> 7) & 0xF) as u8;
        let mode = (word >> 4) & 0x7;
        let wd = (word & 0xF) as usize;
        let value = if acc == 0 { self.acc_a } else { self.acc_b };
        let slit = i32::from(shift) - if shift >= 8 { 16 } else { 0 };
        let shifted = wrap40(if slit < 0 {
            value << (-slit)
        } else {
            value >> slit
        });
        let hi = ((shifted >> 16) & 0xFFFF) as u16;
        let lo = (shifted & 0xFFFF) as u16;
        let store_value = if rounded && self.dsp_round_up(hi, lo) {
            shifted + 0x1_0000
        } else {
            shifted
        };
        let stored = self.dsp_satdw(store_value, ((store_value >> 16) & 0xFFFF) as u16);
        match mode {
            0 => self.w[wd] = stored,
            1..=5 => self.write_operand(wd, mode, stored, false)?,
            6 => {
                let ea = self.w[wd].wrapping_add(self.w[wb]);
                self.data_write(ea, stored)?;
            }
            _ => return Err(SimulationError::DecodeError(u64::from(self.pc))),
        }
        Ok(())
    }

    /// Execute one decoded word. Returns the instruction cycles consumed.
    fn execute(&mut self, word: u32) -> SimResult<u32> {
        let pc = self.pc;

        // NOP — DS70000157G: 0000 0000 xxxx xxxx xxxx xxxx.
        if word & 0x00FF_FFFF == 0 {
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV #lit16, Wnd — 0010 kkkk kkkk kkkk kkkk dddd.
        if word & 0x00F0_0000 == 0x0020_0000 {
            let k = ((word >> 4) & 0xFFFF) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = k;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV.B #lit8, Wnd — 1011 0011 1100 kkkk kkkk dddd.
        if word & 0x00FF_F000 == 0x00B3_C000 {
            let k = ((word >> 4) & 0xFF) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = k;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV Wns, f — 1000 1fff ffff ffff ffff ssss.
        if word >> 19 == 0b1_0001 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let s = (word & 0xF) as usize;
            self.data_write(f, self.w[s])?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV f, Wnd — 1000 0fff ffff ffff ffff dddd.
        if word >> 19 == 0b1_0000 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let d = (word & 0xF) as usize;
            let v = self.data_read(f)?;
            self.w[d] = v;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV{.B} Ws, Wd — 0111 1www wBhh hddd dggg ssss (all modes).
        if word & 0x00F8_0000 == 0x0078_0000 {
            let byte = word & 0x0000_4000 != 0;
            let hhh = (word >> 11) & 0x7;
            let d = ((word >> 7) & 0xF) as usize;
            let ggg = (word >> 4) & 0x7;
            let s = (word & 0xF) as usize;
            let val = self.read_operand(s, ggg, byte)?;
            self.write_operand(d, hhh, val, byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV{.B} f {,WREG} — 1011 1111 1BDf ffff ffff ffff.
        if word & 0x00FF_8000 == 0x00BF_8000 {
            let byte = word & 0x0000_4000 != 0;
            let to_file = word & 0x0000_2000 != 0;
            let f = (word & 0x1FFF) as u16;
            let val = self.read_indirect(f, byte)?;
            if to_file {
                self.write_indirect(f, val, byte)?;
            } else if byte {
                self.w[0] = (self.w[0] & 0xFF00) | (val & 0x00FF);
                self.set_nz(self.w[0], true);
            } else {
                self.w[0] = val;
                self.set_nz(val, false);
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV{.B} WREG, f — 1011 0111 1B1f ffff ffff ffff.
        if word & 0x00FF_A000 == 0x00B7_A000 {
            let byte = word & 0x0000_4000 != 0;
            let f = (word & 0x1FFF) as u16;
            let val = if byte { self.w[0] & 0x00FF } else { self.w[0] };
            self.write_indirect(f, val, byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // ADD{.B} f {,WREG} — 1011 0100 0BDf ffff ffff ffff.
        if word & 0x00FF_8000 == 0x00B4_0000 {
            let byte = word & 0x0000_4000 != 0;
            let to_file = word & 0x0000_2000 != 0;
            let f = (word & 0x1FFF) as u16;
            let a = self.read_indirect(f, byte)?;
            let r = self.alu_add(a, self.w[0], 0, byte);
            if to_file {
                self.write_indirect(f, r, byte)?;
            } else if byte {
                self.w[0] = (self.w[0] & 0xFF00) | (r & 0x00FF);
            } else {
                self.w[0] = r;
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // AND{.B} f {,WREG} — 1011 0110 0BDf ffff ffff ffff.
        if word & 0x00FF_8000 == 0x00B6_0000 {
            let byte = word & 0x0000_4000 != 0;
            let to_file = word & 0x0000_2000 != 0;
            let f = (word & 0x1FFF) as u16;
            let a = self.read_indirect(f, byte)?;
            let r = a & self.w[0];
            self.set_nz(r, byte);
            if to_file {
                self.write_indirect(f, r, byte)?;
            } else if byte {
                self.w[0] = (self.w[0] & 0xFF00) | (r & 0x00FF);
            } else {
                self.w[0] = r;
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // IOR{.B} f {,WREG} — 1011 0111 0BDf ffff ffff ffff.
        if word & 0x00FF_8000 == 0x00B7_0000 {
            let byte = word & 0x0000_4000 != 0;
            let to_file = word & 0x0000_2000 != 0;
            let f = (word & 0x1FFF) as u16;
            let a = self.read_indirect(f, byte)?;
            let r = a | self.w[0];
            self.set_nz(r, byte);
            if to_file {
                self.write_indirect(f, r, byte)?;
            } else if byte {
                self.w[0] = (self.w[0] & 0xFF00) | (r & 0x00FF);
            } else {
                self.w[0] = r;
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // AND/IOR #lit10, Wn — 1011 001x 0Bkk kkkk kkkk dddd.
        if word & 0x00FF_8000 == 0x00B2_0000 || word & 0x00FF_8000 == 0x00B3_0000 {
            let k = ((word >> 4) & 0x3FF) as u16;
            let d = (word & 0xF) as usize;
            let r = if word & 0x0001_0000 == 0 {
                k & self.w[d]
            } else {
                k | self.w[d]
            };
            self.set_nz(r, false);
            self.w[d] = r;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // ADD #lit10, Wn — 1011 0000 0Bkk kkkk kkkk dddd.
        if word & 0x00FF_8000 == 0x00B0_0000 {
            let k = ((word >> 4) & 0x3FF) as u16;
            let d = (word & 0xF) as usize;
            let r = self.alu_add(k, self.w[d], 0, false);
            self.w[d] = r;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // AND{.B} Wb,#lit5,Wd / Wb,Ws,Wd — 0110 0www wBqq qddd dxxx xxxx.
        if word & 0x00F8_0000 == 0x0060_0000 {
            let op = Self::decode_3op(word);
            let rhs = match op.lit {
                Some(k) => k,
                None => self.read_operand(op.ws, op.ppp, op.byte)?,
            };
            let r = self.w[op.wb] & rhs;
            self.set_nz(r, op.byte);
            self.write_operand(op.wd, op.qqq, r, op.byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // IOR{.B} Wb,#lit5,Wd / Wb,Ws,Wd — 0111 0www wBqq qddd dxxx xxxx.
        if word & 0x00F8_0000 == 0x0070_0000 {
            let op = Self::decode_3op(word);
            let rhs = match op.lit {
                Some(k) => k,
                None => self.read_operand(op.ws, op.ppp, op.byte)?,
            };
            let r = self.w[op.wb] | rhs;
            self.set_nz(r, op.byte);
            self.write_operand(op.wd, op.qqq, r, op.byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // SUB{.B} Wb,#lit5,Wd / Wb,Ws,Wd — 0101 0www wBqq qddd dxxx xxxx.
        if word & 0x00F8_0000 == 0x0050_0000 {
            let op = Self::decode_3op(word);
            let rhs = match op.lit {
                Some(k) => k,
                None => self.read_operand(op.ws, op.ppp, op.byte)?,
            };
            let r = self.alu_sub(self.w[op.wb], rhs, 0, op.byte);
            self.write_operand(op.wd, op.qqq, r, op.byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // ADD{.B} Wb,#lit5,Wd / Wb,Ws,Wd — 0100 0www wBqq qddd dxxx xxxx.
        if word & 0x00F8_0000 == 0x0040_0000 {
            let op = Self::decode_3op(word);
            let rhs = match op.lit {
                Some(k) => k,
                None => self.read_operand(op.ws, op.ppp, op.byte)?,
            };
            let r = self.alu_add(self.w[op.wb], rhs, 0, op.byte);
            self.write_operand(op.wd, op.qqq, r, op.byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // CLR/SETM Wd — 1110 1011 0xxx xddd d000 0000. (The WREG forms;
        // the 1110 111x file forms are decoded below.) Table 22-2 #15/#69
        // lists Status Affected = None for both, matching the file form.
        if word & 0x00FF_0000 == 0x00EB_0000 && word & 0x0000_787F == 0 {
            let set = word & 0x0000_8000 != 0;
            let d = ((word >> 7) & 0xF) as usize;
            self.w[d] = if set { 0xFFFF } else { 0 };
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // INC{.B} Ws,Wd — 1110 1000 0Bqq qddd dppp ssss. The dsPIC33F
        // assembler also spells the indirect operands as `INC [Ws],[Wd]`.
        // (The file form is not emitted by XC16 for this subset; a WREG
        // form with ppp/qqq = direct covers the compiler's `INC f` cases,
        // which it actually writes as `INC [Wn],[Wn]`.)
        if word & 0x00FF_0000 == 0x00E8_0000 {
            let byte = word & 0x0000_4000 != 0;
            // Bit 15 separates INC (clear) from INC2 (set); the two share
            // the rest of the encoding (DS70157D §5).
            let increment = if word & 0x0000_8000 != 0 { 2 } else { 1 };
            let qqq = (word >> 11) & 0x7;
            let d = ((word >> 7) & 0xF) as usize;
            let ppp = (word >> 4) & 0x7;
            let sreg = (word & 0xF) as usize;
            let v = self.read_operand(sreg, ppp, byte)?;
            // INC/INC2 affect C, DC, N, OV and Z (DS70283H Table 22-2 #39);
            // XC16 relies on C for its 32-bit increment idiom.
            let r = self.alu_add(v, increment, 0, byte);
            self.write_operand(d, qqq, r, byte)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // BSET/BCLR/BTG{.B} f,#bit — 1010 100x bbbf ffff ffff ffff.
        if matches!(word >> 16, 0xA8..=0xAA) {
            let bit = ((word >> 13) & 0x7) as u8;
            let f = (word & 0x1FFF) as u16;
            let mut b = self.data_read_byte(f)?;
            b = match word >> 16 {
                0xA8 => b | (1 << bit),
                0xA9 => b & !(1 << bit),
                _ => b ^ (1 << bit),
            };
            self.data_write_byte(f, b)?;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // BSET/BCLR/BTG{.B} Ws,#bit — 1010 000x bbbb 0B00 0ppp ssss.
        if matches!(word >> 16, 0xA0..=0xA2) {
            let byte = word & 0x0000_0400 != 0;
            let bit = (word >> 12) & 0xF;
            let reg = (word & 0xF) as usize;
            let mode = (word >> 4) & 0x7;
            let step = if byte { 1 } else { 2 };
            // Source and destination are the same operand: resolve the
            // effective address with a single post/pre-modification.
            let (val, addr) = match mode {
                0 => (self.w[reg], None),
                1 => (self.read_indirect(self.w[reg], byte)?, Some(self.w[reg])),
                2 => {
                    let a = self.w[reg];
                    let v = self.read_indirect(a, byte)?;
                    self.w[reg] = self.w[reg].wrapping_sub(step);
                    (v, Some(a))
                }
                3 => {
                    let a = self.w[reg];
                    let v = self.read_indirect(a, byte)?;
                    self.w[reg] = self.w[reg].wrapping_add(step);
                    (v, Some(a))
                }
                4 => {
                    self.w[reg] = self.w[reg].wrapping_sub(step);
                    let a = self.w[reg];
                    (self.read_indirect(a, byte)?, Some(a))
                }
                5 => {
                    self.w[reg] = self.w[reg].wrapping_add(step);
                    let a = self.w[reg];
                    (self.read_indirect(a, byte)?, Some(a))
                }
                _ => return Err(SimulationError::DecodeError(u64::from(pc))),
            };
            let r = match word >> 16 {
                0xA0 => val | (1 << bit),
                0xA1 => val & !(1 << bit),
                _ => val ^ (1 << bit),
            };
            match addr {
                None => {
                    if byte {
                        self.w[reg] = (self.w[reg] & 0xFF00) | (r & 0x00FF);
                    } else {
                        self.w[reg] = r;
                    }
                }
                Some(a) => self.write_indirect(a, r, byte)?,
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // CP0{.B} Ws — 1110 0000 0000 0B00 0ppp ssss.
        if word & 0x00FF_F800 == 0x00E0_0000 {
            let byte = word & 0x0000_0400 != 0;
            let reg = (word & 0xF) as usize;
            let mode = (word >> 4) & 0x7;
            let v = self.read_operand(reg, mode, byte)?;
            self.alu_sub(v, 0, 0, byte);
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // CP0{.B} f — 1110 0010 0B0f ffff ffff ffff.
        if word & 0x00FF_A000 == 0x00E2_0000 {
            let byte = word & 0x0000_4000 != 0;
            let f = (word & 0x1FFF) as u16;
            let v = self.read_indirect(f, byte)?;
            self.alu_sub(v, 0, 0, byte);
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // CLR/SETM{.B} f (or WREG) — 1110 111x 0BDf ffff ffff ffff.
        if word & 0x00FF_0000 == 0x00EF_0000 {
            let byte = word & 0x0000_4000 != 0;
            let to_file = word & 0x0000_2000 != 0;
            let set = word & 0x0000_8000 != 0;
            let f = (word & 0x1FFF) as u16;
            let val = if set {
                if byte {
                    0x00FF
                } else {
                    0xFFFF
                }
            } else {
                0
            };
            if to_file {
                self.write_indirect(f, val, byte)?;
            } else if byte {
                self.w[0] = (self.w[0] & 0xFF00) | (val & 0x00FF);
            } else {
                self.w[0] = val;
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // SE [Ws], Wnd / SE Ws, Wnd — 1111 1011 0000 0ddd dppp ssss.
        if word & 0x00FF_F800 == 0x00FB_0000 {
            let d = ((word >> 7) & 0xF) as usize;
            let mode = (word >> 4) & 0x7;
            let s = (word & 0xF) as usize;
            let v = self.read_operand(s, mode, true)?;
            let ext = if v & 0x0080 != 0 {
                v | 0xFF00
            } else {
                v & 0x00FF
            };
            self.w[d] = ext;
            self.set_nz(ext, false);
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // LSR Wb, #lit4, Wnd — 1101 1110 0www wddd d100 kkkk.
        if word & 0x00FF_8000 == 0x00DE_0000 {
            let wb = ((word >> 11) & 0xF) as usize;
            let wd = ((word >> 7) & 0xF) as usize;
            let k = word & 0xF;
            let r = self.w[wb] >> k.min(15);
            self.w[wd] = r;
            self.set_nz(r, false);
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // LNK #lit14 — 1111 1010 00kk kkkk kkkk kkk0 (DS70000157G §5.4).
        if word & 0x00FF_C000 == 0x00FA_0000 {
            // The 13 stored bits are lit14/2 (bit 0 is fixed 0).
            let lit14 = (((word >> 1) & 0x3FFF) as u16) << 1;
            self.push_word(self.w[14])?;
            self.w[14] = self.w[15];
            self.w[15] = self.w[15].wrapping_add(lit14);
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // ULNK — 1111 1010 1000 0000 0000 0000.
        if word == 0x00FA_8000 {
            self.w[15] = self.w[14];
            let v = self.pop_word()?;
            self.w[14] = v;
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // RETLW{.B} #lit10, Wn — 0000 0101 0Bkk kkkk kkkk dddd.
        if word & 0x00FF_8000 == 0x0005_0000 {
            let byte = word & 0x0000_4000 != 0;
            let k = ((word >> 4) & 0x3FF) as u16;
            let d = (word & 0xF) as usize;
            let hi = self.pop_word()?;
            let lo = self.pop_word()?;
            self.pc = (u32::from(hi & 0x007F) << 16) | u32::from(lo);
            if byte {
                self.w[d] = (self.w[d] & 0xFF00) | (k & 0x00FF);
            } else {
                self.w[d] = k;
            }
            return Ok(3);
        }

        // RETURN — 0000 0110 0000 0000 0000 0000.
        if word & 0x00FF_FFFF == 0x0006_0000 {
            let hi = self.pop_word()?;
            let lo = self.pop_word()?;
            self.pc = (u32::from(hi & 0x007F) << 16) | u32::from(lo);
            return Ok(3);
        }

        // RETFIE — 0000 0110 0100 0000 0000 0000 (DS70000157G).
        if word == INSN_RETFIE {
            let packed = self.pop_word()?;
            let low = self.pop_word()?;
            self.sr = (self.sr & 0xFF00) | ((packed >> 8) & 0x00FF);
            self.corcon = (self.corcon & !0x0008) | (((packed >> 7) & 1) << 3);
            self.pc = (u32::from(packed & 0x007F) << 16) | u32::from(low);
            return Ok(RETFIE_CYCLES);
        }

        // RCALL Expr — 0000 0111 nnnn nnnn nnnn nnnn.
        if word >> 16 == 0x07 {
            let off = (word & 0xFFFF) as u16 as i16;
            let ret = pc.wrapping_add(2);
            self.push_word(ret as u16)?;
            self.push_word(((ret >> 16) & 0x7F) as u16)?;
            let next = pc as i64 + 2 + 2 * i64::from(off);
            if next < 0 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            self.pc = next as u32;
            return Ok(2);
        }

        // CALL Expr — 2 words, 0000 0010 nnnn... (DS70000157G).
        if word >> 16 == 0x02 {
            let word2 = self.program_word(pc + 2)?;
            let n = ((word >> 1) & 0x7FFF) | ((word2 & 0x7F) << 15);
            let ret = pc.wrapping_add(4);
            self.push_word(ret as u16)?;
            self.push_word(((ret >> 16) & 0x7F) as u16)?;
            self.pc = n << 1;
            return Ok(2);
        }

        // GOTO Expr — 2 words, 0000 0100 nnnn... (DS70000157G).
        if word >> 16 == 0x04 {
            let word2 = self.program_word(pc + 2)?;
            let n = ((word >> 1) & 0x7FFF) | ((word2 & 0x7F) << 15);
            self.pc = n << 1;
            return Ok(2);
        }

        // BRA Expr — 0011 0111 nnnn nnnn nnnn nnnn (slit16 words).
        if word >> 16 == 0x37 {
            let off = (word & 0xFFFF) as u16 as i16;
            let next = pc as i64 + 2 + 2 * i64::from(off);
            if next < 0 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            self.pc = next as u32;
            return Ok(2);
        }

        // BRA cond, Expr — 0011 cccc nnnn... (DS70000157G Table 4-8).
        if word & 0x00F0_0000 == 0x0030_0000 {
            let cond = (word >> 16) & 0xF;
            let off = (word & 0xFFFF) as u16 as i16;
            if self.branch_taken(cond) {
                let next = pc as i64 + 2 + 2 * i64::from(off);
                if next < 0 {
                    return Err(SimulationError::DecodeError(u64::from(pc)));
                }
                self.pc = next as u32;
                return Ok(2);
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // ── DSP MAC class (DS70000157G §4.15, §5.4) ─────────────────────────
        // Square MAC/MPY — 1111 00mm A0xx yyii iijj jj0b.
        // 0b1x in bits 19-18 is an unimplemented encoding (DecodeError).
        if word & 0x00F0_0000 == 0x00F0_0000 {
            if word & 0x000C_0000 == 0 {
                self.exec_dsp_mac(word, true)?;
                self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
                return Ok(1);
            }
        } else if word & 0x00F0_0000 == 0x00C0_0000 {
            if word & 0x0008_0000 == 0 {
                // MAC/MPY/MPY.N/MSC/CLR/MOVSAC — 1100 0mmm ...
                match (word >> 16) & 0x7 {
                    0b011 => self.exec_dsp_clr(word)?,
                    0b111 => self.exec_dsp_movsac(word)?,
                    _ => self.exec_dsp_mac(word, false)?,
                }
            } else {
                // LAC/SAC/SAC.R — 1100 1010 / 1100 1100 / 1100 1101.
                match (word >> 16) & 0xF {
                    0xA => self.exec_dsp_lac(word)?,
                    0xC => self.exec_dsp_sac(word, false)?,
                    0xD => self.exec_dsp_sac(word, true)?,
                    _ => return Err(SimulationError::DecodeError(u64::from(pc))),
                }
            }
            self.pc = pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // Everything else is deliberately unimplemented (see module scope).
        Err(SimulationError::DecodeError(self.pc as u64))
    }
}

// ── M3: ADC1, SPI1, I2C1 ────────────────────────────────────────────────────

impl Pic24 {
    /// Inject a raw 10/12-bit code for analog channel `channel` (AN0-AN8).
    /// This is the harness seam for an analog source; the session builder
    /// leaves unconnected inputs at 0. AN0-AN8 map to RA0, RA1, RB0-RB3,
    /// RC0-RC2 (DS70283H Table 1-1).
    pub fn set_adc_analog(&mut self, channel: u8, code: u16) {
        if let Some(slot) = self.adc_analog.get_mut(usize::from(channel)) {
            *slot = code & 0x0FFF;
        }
    }

    /// Attach a device to the SPI1 wires. The first attached device answers
    /// each transfer: this model has no SS/CS routing (documented in the M3
    /// reference). The device is called once per 8-bit frame, high byte
    /// first in MODE16.
    pub fn attach_spi_device(&mut self, device: Box<dyn SpiDevice>) {
        self.spi_devices.push(device);
    }

    /// Attach an I2C1 slave. Address matching goes through
    /// [`I2cDevice::claims_address`] (7-bit addresses; the trait's address is
    /// a `u8`).
    pub fn attach_i2c_device(&mut self, device: Box<dyn I2cDevice>) {
        self.i2c_devices.push(device);
    }

    /// TAD in instruction cycles: `(ADCS + 1) * TCY` (DS70183D §16.4.4).
    /// ADRC = 1 selects the internal RC oscillator; its frequency is not
    /// modelled, so the same formula is used (documented approximation).
    fn adc_tad(&self) -> u32 {
        u32::from(self.ad1con3 & 0x00FF) + 1
    }

    fn adc_channels_per_sample(&self) -> u8 {
        // CHPS: 00 = CH0, 01 = CH0/CH1, 1x = CH0-CH3 (DS70183D Table 16-6).
        match (self.ad1con2 >> 8) & 0x3 {
            0 => 1,
            1 => 2,
            _ => 4,
        }
    }

    /// The next channel in the AD1CSSL scan list (CSCNA), or CH0SA when the
    /// list is empty.
    fn adc_scan_channel(&self) -> u8 {
        let list = self.ad1cssl & 0x01FF;
        if list == 0 {
            return (self.ad1chs0 & 0x1F) as u8;
        }
        for offset in 0..9u8 {
            let ch = (self.adc_scan_next + offset) % 9;
            if list & (1 << ch) != 0 {
                return ch;
            }
        }
        0
    }

    /// MUX A/B positive input for one channel position in the sequence.
    /// CH1/2/3 are AN0/1/2 or AN3/4/5 selected by CH123SA (DS70183D
    /// Table 16-10); CH0 scans when CSCNA = 1.
    fn adc_positive_channel(&self, pos: u8) -> u8 {
        let alt = self.adc_alt;
        let ch0 = if alt {
            (self.ad1chs0 >> 8) & 0x1F
        } else {
            self.ad1chs0 & 0x1F
        };
        let ch123 = if alt {
            (self.ad1chs123 >> 8) & 0x1
        } else {
            self.ad1chs123 & 0x1
        };
        match pos {
            0 if self.ad1con2 & AD1CON2_CSCNA != 0 => self.adc_scan_channel(),
            0 => ch0 as u8,
            1 => {
                if ch123 != 0 {
                    3
                } else {
                    0
                }
            }
            2 => {
                if ch123 != 0 {
                    4
                } else {
                    1
                }
            }
            _ => {
                if ch123 != 0 {
                    5
                } else {
                    2
                }
            }
        }
    }

    /// MUX A/B negative input for one channel position (`None` = VREF-).
    fn adc_negative_channel(&self, pos: u8) -> Option<u8> {
        let alt = self.adc_alt;
        if pos == 0 {
            let na = if alt {
                (self.ad1chs0 >> 15) & 1
            } else {
                (self.ad1chs0 >> 7) & 1
            };
            (na != 0).then_some(1)
        } else if self.ad1con1 & AD1CON1_AD12B != 0 {
            // 12-bit mode: the CH1-3 negative selects are reserved.
            None
        } else {
            let na = if alt {
                (self.ad1chs123 >> 9) & 0x3
            } else {
                (self.ad1chs123 >> 1) & 0x3
            };
            // 10 = AN6/AN7/AN8; 00/01 = VREF-; 11 = reserved.
            match na {
                2 => Some(6 + (pos - 1)),
                _ => None,
            }
        }
    }

    fn adc_read_channel(&self, ch: u8) -> u16 {
        // PCFG = 1 (or an absent channel) is a digital pin, not an analog
        // source; it reads 0 here (documented approximation).
        if ch > 8 || self.ad1pcfgl & (1 << ch) != 0 {
            return 0;
        }
        let pos = self.adc_analog[usize::from(ch)];
        let neg = self
            .adc_negative_channel(self.adc_seq_pos)
            .map(|n| {
                if n <= 8 {
                    self.adc_analog[usize::from(n)]
                } else {
                    0
                }
            })
            .unwrap_or(0);
        let max = if self.ad1con1 & AD1CON1_AD12B != 0 {
            4095
        } else {
            1023
        };
        pos.saturating_sub(neg).min(max)
    }

    /// FORM<1:0> output formatting (DS70183D §16.4.5 / Figure 16-14):
    /// 00 unsigned integer, 01 signed integer, 10 unsigned fraction,
    /// 11 signed fraction. Fractional results are left-justified to 16 bits.
    fn adc_format(&self, raw: u16) -> u16 {
        let twelve = self.ad1con1 & AD1CON1_AD12B != 0;
        let max: i32 = if twelve { 4095 } else { 1023 };
        let mid = (max + 1) / 2;
        let shift = if twelve { 4 } else { 6 };
        let code = i32::from(raw.min(max as u16));
        match (self.ad1con1 >> 8) & 0x3 {
            0 => code as u16,
            1 => (code - mid) as u16,
            2 => ((code << shift) & 0xFFFF) as u16,
            _ => (((code - mid) << shift) & 0xFFFF) as u16,
        }
    }

    fn adc_start_sampling(&mut self) {
        if self.ad1con1 & AD1CON1_ADON == 0 {
            return;
        }
        self.ad1con1 |= AD1CON1_SAMP;
        self.adc_state = AdcState::Sampling;
        self.adc_seq_pos = 0;
        self.adc_sample_remaining = 0;
        // A period match while the ADC was off or converting is stale.
        self.adc_t3_trigger = false;
        self.adc_pwm_trigger = false;
        // SSRC = 111: the internal counter ends sampling after SAMC * TAD.
        if (self.ad1con1 >> 5) & 0x7 == 0b111 {
            let samc = u32::from((self.ad1con3 >> 8) & 0x1F).max(1);
            self.adc_sample_remaining = samc * self.adc_tad();
        }
    }

    fn adc_start_conversion(&mut self) {
        if self.ad1con1 & AD1CON1_ADON == 0 {
            return;
        }
        self.ad1con1 &= !(AD1CON1_SAMP | AD1CON1_DONE);
        self.adc_state = AdcState::Converting;
        self.adc_seq_pos = 0;
        let tconv = if self.ad1con1 & AD1CON1_AD12B != 0 {
            14
        } else {
            12
        };
        self.adc_conv_remaining = tconv * self.adc_tad();
    }

    fn adc_store_result(&mut self, value: u16) {
        // The 16-bit buffer holds the FORMATTED value: fractional formats
        // are left-justified, so masking to 10/12 bits here would truncate
        // them (DS70183D Figure 16-14).
        let bufm = self.ad1con2 & AD1CON2_BUFM != 0;
        let idx = if bufm {
            usize::from(self.adc_half) * 8 + usize::from(self.adc_buf_pos & 0x7)
        } else {
            usize::from(self.adc_buf_pos & 0x0F)
        };
        self.adc_buf[idx] = value;
        self.adc_buf_pos = (self.adc_buf_pos + 1) & if bufm { 0x7 } else { 0x0F };
    }

    fn adc_complete_conversion(&mut self) {
        let pos = self.adc_seq_pos;
        let ch = self.adc_positive_channel(pos);
        let raw = self.adc_read_channel(ch);
        let result = self.adc_format(raw);
        self.adc_store_result(result);
        self.adc_seq_pos = self.adc_seq_pos.saturating_add(1);
        if self.adc_seq_pos < self.adc_channels_per_sample() {
            let tconv = if self.ad1con1 & AD1CON1_AD12B != 0 {
                14
            } else {
                12
            };
            self.adc_conv_remaining = tconv * self.adc_tad();
            self.adc_state = AdcState::Converting;
            return;
        }
        // Sequence complete: DONE, SMPI interrupt, buffer half, ASAM.
        self.adc_state = AdcState::Off;
        self.ad1con1 |= AD1CON1_DONE;
        self.ad1con1 &= !AD1CON1_SAMP;
        self.adc_conv_count = self.adc_conv_count.saturating_add(1);
        let smpi = (self.ad1con2 >> 2) & 0xF;
        if self.adc_conv_count > smpi {
            self.adc_conv_count = 0;
            self.ifs0 |= 1 << IRQ_AD1.0;
            if self.ad1con2 & AD1CON2_BUFM != 0 {
                self.adc_half ^= 1;
                self.adc_buf_pos = 0;
            }
        }
        if self.ad1con2 & AD1CON2_CSCNA != 0 {
            // Move past the channel this sequence used, not just to the next
            // bit index: with a sparse list (0,2,4) the latter repeats AN2.
            let used = self.adc_scan_channel();
            self.adc_scan_next = (used + 1) % 9;
        }
        // ALTS: this sequence used its MUX; the next one uses the other.
        if self.ad1con2 & AD1CON2_ALTS != 0 {
            self.adc_alt = !self.adc_alt;
        }
        if self.ad1con1 & 0x0004 != 0 {
            // ASAM: sampling begins immediately after the last conversion.
            self.adc_start_sampling();
        }
    }

    fn advance_adc(&mut self, cycles: u32) {
        if self.ad1con1 & AD1CON1_ADON == 0 {
            self.adc_state = AdcState::Off;
            return;
        }
        match self.adc_state {
            AdcState::Sampling => match (self.ad1con1 >> 5) & 0x7 {
                // SSRC = 001 (INT0) needs an external INT0 edge, which is
                // not modelled; the sample stays open.
                0b010 if self.adc_t3_trigger => {
                    self.adc_t3_trigger = false;
                    self.adc_start_conversion();
                }
                0b011 if self.adc_pwm_trigger => {
                    self.adc_pwm_trigger = false;
                    self.adc_start_conversion();
                }
                0b111 => {
                    if self.adc_sample_remaining <= cycles {
                        self.adc_start_conversion();
                    } else {
                        self.adc_sample_remaining -= cycles;
                    }
                }
                _ => {}
            },
            AdcState::Converting => {
                if self.adc_conv_remaining <= cycles {
                    self.adc_conv_remaining = 0;
                    self.adc_complete_conversion();
                } else {
                    self.adc_conv_remaining -= cycles;
                }
            }
            AdcState::Off => {}
        }
    }

    // ── SPI1 ────────────────────────────────────────────────────────────────

    /// SCK divider: PPRE (1/4/16/64) times SPRE (1..8). DS70206C Eq. 18-1
    /// gives `FSCK = FCY / (PPRE * SPRE)`, so one SCK period is `divider`
    /// TCY (not `2 * divider`).
    fn spi_divider(&self) -> u32 {
        let ppre = u32::from(self.spi1con1 & 0x3);
        let spre = u32::from((self.spi1con1 >> 2) & 0x7);
        let primary = match ppre {
            3 => 1,
            2 => 4,
            1 => 16,
            _ => 64,
        };
        primary * (8 - spre)
    }

    fn spi_transfer_word(&mut self, word: u16) -> u16 {
        if self.spi_devices.is_empty() {
            return if self.spi1con1 & SPI1CON1_MODE16 != 0 {
                0xFFFF
            } else {
                0x00FF
            };
        }
        if self.spi1con1 & SPI1CON1_MODE16 != 0 {
            let hi = self.spi_devices[0].transfer((word >> 8) as u8);
            let lo = self.spi_devices[0].transfer(word as u8);
            (u16::from(hi) << 8) | u16::from(lo)
        } else {
            u16::from(self.spi_devices[0].transfer(word as u8))
        }
    }

    fn spi_start_transfer(&mut self, word: u16) {
        self.spi1_tx = word;
        // TXB -> SPIxSR happens as soon as the module is idle, so the
        // transmit buffer is no longer full (DS70283H Register 17-1).
        self.spi1stat &= !SPI1STAT_SPITBF;
        self.spi1_shifting = true;
        let bits = if self.spi1con1 & SPI1CON1_MODE16 != 0 {
            16
        } else {
            8
        };
        self.spi1_busy = bits * self.spi_divider();
    }

    fn advance_spi(&mut self, cycles: u32) {
        if !self.spi1_shifting {
            return;
        }
        if self.spi1_busy > cycles {
            self.spi1_busy -= cycles;
            return;
        }
        self.spi1_busy = 0;
        self.spi1_shifting = false;
        self.spi1stat &= !SPI1STAT_SPITBF;
        let word = self.spi1_tx;
        let rx = self.spi_transfer_word(word);
        if self.spi1stat & SPI1STAT_SPIRBF != 0 {
            // The previous word was never read: this one is discarded and
            // SPIROV reports the overrun (DS70283H Register 17-1).
            self.spi1stat |= SPI1STAT_SPIROV;
            self.ifs0 |= 1 << IRQ_SPI1E.0;
        } else {
            self.spi1_rx = rx;
            self.spi1stat |= SPI1STAT_SPIRBF;
            self.ifs0 |= 1 << IRQ_SPI1.0;
        }
        if let Some(next) = self.spi1_tx_pending.take() {
            self.spi_start_transfer(next);
        }
    }

    /// Shift one word through SPI1 in slave mode (test seam; no external SPI
    /// master model exists). Returns the word the slave presents on SDO.
    pub fn spi_slave_clock(&mut self, mosi: u16) -> u16 {
        let miso = self.spi1_tx;
        self.spi1_rx = mosi;
        self.spi1stat &= !SPI1STAT_SPITBF;
        self.spi1stat |= SPI1STAT_SPIRBF;
        self.ifs0 |= 1 << IRQ_SPI1.0;
        miso
    }

    // ── I2C1 ────────────────────────────────────────────────────────────────

    /// One SCL period is `(BRG + 1)` TCY (DS70000195 §19; the 1.111 MHz
    /// high-time correction term is omitted — a documented approximation).
    /// Byte operations charge nine SCL periods; start/stop conditions one.
    fn i2c_op_cycles(&self, bits: u32) -> u32 {
        bits * (u32::from(self.i2c1brg) + 1)
    }

    fn i2c_select(&mut self, addr: u8, read: bool) {
        self.i2c_target = self.i2c_devices.iter().position(|d| d.claims_address(addr));
        self.i2c_addressed = true;
        self.i2c_reading = read;
        self.i2c1stat &= !(I2C1STAT_ACKSTAT | I2C1STAT_DA | I2C1STAT_ADD10);
        if read {
            self.i2c1stat |= I2C1STAT_RW;
        } else {
            self.i2c1stat &= !I2C1STAT_RW;
        }
        if let Some(idx) = self.i2c_target {
            self.i2c_devices[idx].select_address(addr);
            self.i2c_devices[idx].start();
        } else {
            self.i2c1stat |= I2C1STAT_ACKSTAT;
        }
    }

    fn i2c_transmit_complete(&mut self) {
        let byte = (self.i2c1trn & 0x00FF) as u8;
        if !self.i2c_addressed {
            if let Some(_prefix) = self.i2c_10bit_prefix.take() {
                // 10-bit address low byte. The slave contract is 7-bit, so
                // the address cannot match: NACK it (do not truncate to 7
                // bits, which could accidentally select a different slave).
                self.i2c_addressed = true;
                self.i2c_target = None;
                self.i2c_reading = false;
                self.i2c1stat &= !(I2C1STAT_ACKSTAT | I2C1STAT_DA | I2C1STAT_ADD10);
                self.i2c1stat |= I2C1STAT_ACKSTAT;
            } else if byte & 0xF8 == 0xF0 {
                // 10-bit address prefix (11110 A9 A8 R/W).
                self.i2c_10bit_prefix = Some(u16::from(byte));
                self.i2c1stat &= !(I2C1STAT_ACKSTAT | I2C1STAT_DA | I2C1STAT_RW);
            } else {
                self.i2c_select(byte >> 1, byte & 1 != 0);
            }
        } else if self.i2c_reading {
            // A data write while the address phase selected a read is NACKed.
            self.i2c1stat |= I2C1STAT_ACKSTAT;
        } else if let Some(idx) = self.i2c_target {
            self.i2c_devices[idx].write(byte);
            self.i2c1stat &= !I2C1STAT_ACKSTAT;
            self.i2c1stat |= I2C1STAT_DA;
        } else {
            self.i2c1stat |= I2C1STAT_ACKSTAT | I2C1STAT_DA;
        }
    }

    fn i2c_receive_complete(&mut self) {
        let byte = match self.i2c_target {
            Some(idx) if self.i2c_reading => self.i2c_devices[idx].read(),
            _ => 0xFF,
        };
        if self.i2c1stat & I2C1STAT_RBF != 0 {
            self.i2c1stat |= I2C1STAT_I2COV;
        } else {
            self.i2c1rcv = u16::from(byte);
            self.i2c1stat |= I2C1STAT_RBF | I2C1STAT_DA;
        }
    }

    fn advance_i2c(&mut self, cycles: u32) {
        if self.i2c_op.is_none() {
            return;
        }
        if self.i2c_busy > cycles {
            self.i2c_busy -= cycles;
            return;
        }
        self.i2c_busy = 0;
        let op = self.i2c_op.take().expect("checked above");
        match op {
            I2cOp::Start | I2cOp::RepeatedStart => {
                self.i2c1con &= !(I2C1CON_SEN | I2C1CON_RSEN);
                self.i2c1stat =
                    (self.i2c1stat & !(I2C1STAT_P | I2C1STAT_TBF | I2C1STAT_TRSTAT)) | I2C1STAT_S;
                self.i2c_target = None;
                self.i2c_addressed = false;
                self.i2c_reading = false;
                self.i2c_10bit_prefix = None;
                self.ifs1 |= 1 << IRQ_MI2C1.0;
            }
            I2cOp::Transmit => {
                self.i2c1stat &= !(I2C1STAT_TBF | I2C1STAT_TRSTAT);
                self.i2c_transmit_complete();
                self.ifs1 |= 1 << IRQ_MI2C1.0;
            }
            I2cOp::Receive => {
                self.i2c1con &= !I2C1CON_RCEN;
                self.i2c_receive_complete();
                self.ifs1 |= 1 << IRQ_MI2C1.0;
            }
            I2cOp::Ack => {
                self.i2c1con &= !I2C1CON_ACKEN;
                self.ifs1 |= 1 << IRQ_MI2C1.0;
            }
            I2cOp::Stop => {
                self.i2c1con &= !I2C1CON_PEN;
                self.i2c1stat = (self.i2c1stat
                    & !(I2C1STAT_S | I2C1STAT_RW | I2C1STAT_TBF | I2C1STAT_TRSTAT))
                    | I2C1STAT_P;
                if let Some(idx) = self.i2c_target.take() {
                    self.i2c_devices[idx].stop();
                }
                self.i2c_addressed = false;
                self.i2c_reading = false;
                self.i2c_10bit_prefix = None;
                self.ifs1 |= 1 << IRQ_MI2C1.0;
            }
        }
    }

    /// Test seam: deliver an external master's address phase to the I2C1
    /// slave. Returns whether the address matched (SI2C1IF is raised on a
    /// match). There is no external master model on the bus.
    pub fn i2c_slave_address(&mut self, addr: u16, read: bool) -> bool {
        if self.i2c1con & I2C1CON_I2CEN == 0 {
            return false;
        }
        let mask = if self.i2c1con & I2C1CON_A10M != 0 {
            0x03FF
        } else {
            0x007F
        };
        let msk = self.i2c1msk & mask;
        if (addr & mask & !msk) != (self.i2c1add & mask & !msk) {
            return false;
        }
        self.i2c1stat = (self.i2c1stat & !(I2C1STAT_P | I2C1STAT_TBF | I2C1STAT_RBF | I2C1STAT_DA))
            | I2C1STAT_S;
        if read {
            self.i2c1stat |= I2C1STAT_RW;
        } else {
            self.i2c1stat &= !I2C1STAT_RW;
        }
        if self.i2c1con & I2C1CON_A10M != 0 {
            self.i2c1stat |= I2C1STAT_ADD10;
        }
        self.ifs1 |= 1 << IRQ_SI2C1.0;
        true
    }

    /// Test seam: an external master writes a byte to the I2C1 slave.
    pub fn i2c_slave_write(&mut self, byte: u8) {
        if self.i2c1stat & I2C1STAT_RBF != 0 {
            self.i2c1stat |= I2C1STAT_I2COV;
            return;
        }
        self.i2c1rcv = u16::from(byte);
        self.i2c1stat |= I2C1STAT_RBF | I2C1STAT_DA;
        self.ifs1 |= 1 << IRQ_SI2C1.0;
    }

    /// Test seam: the byte the I2C1 slave presents to an external master.
    pub fn i2c_slave_read(&mut self) -> u8 {
        self.i2c1stat &= !I2C1STAT_TBF;
        (self.i2c1trn & 0x00FF) as u8
    }

    /// Test seam: an external master issues a stop.
    pub fn i2c_slave_stop(&mut self) {
        self.i2c1stat = (self.i2c1stat & !(I2C1STAT_S | I2C1STAT_ADD10)) | I2C1STAT_P;
        self.ifs1 |= 1 << IRQ_SI2C1.0;
    }

    /// Reset the M3 peripheral registers. Attached devices stay attached
    /// (hardware does not unplug a sensor on a device reset).
    fn reset_m3(&mut self) {
        self.ad1con1 = 0;
        self.ad1con2 = 0;
        self.ad1con3 = 0;
        self.ad1chs123 = 0;
        self.ad1chs0 = 0;
        self.ad1pcfgl = 0x0000;
        self.ad1cssl = 0;
        self.adc_buf = [0; ADC1_BUF_COUNT];
        // `adc_analog` is the harness's pin voltage, not an SFR: a device
        // reset does not change what is connected to the pin.
        self.adc_state = AdcState::Off;
        self.adc_conv_remaining = 0;
        self.adc_sample_remaining = 0;
        self.adc_conv_count = 0;
        self.adc_seq_pos = 0;
        self.adc_buf_pos = 0;
        self.adc_half = 0;
        self.adc_alt = false;
        self.adc_t3_trigger = false;
        self.adc_pwm_trigger = false;
        self.adc_scan_next = 0;
        self.spi1stat = 0;
        self.spi1con1 = 0;
        self.spi1con2 = 0;
        self.spi1_rx = 0;
        self.spi1_tx = 0;
        self.spi1_tx_pending = None;
        self.spi1_busy = 0;
        self.spi1_shifting = false;
        self.i2c1con = 0x1000;
        self.i2c1stat = 0;
        self.i2c1add = 0;
        self.i2c1msk = 0;
        self.i2c1brg = 0;
        self.i2c1rcv = 0;
        self.i2c1trn = 0x00FF;
        self.i2c_target = None;
        self.i2c_reading = false;
        self.i2c_addressed = false;
        self.i2c_10bit_prefix = None;
        self.i2c_busy = 0;
        self.i2c_op = None;
    }
}

impl std::fmt::Debug for Pic24 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pic24")
            .field("pc", &self.pc)
            .field("sr", &self.sr)
            .field("cycles", &self.cycles)
            .field("w", &self.w)
            .finish_non_exhaustive()
    }
}

impl Cpu for Pic24 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.w = [0; 16];
        self.pc = 0;
        self.sr = 0;
        self.cycles = 0;
        self.u1mode = 0;
        self.u1sta = 0;
        self.t1con = 0;
        self.tmr1 = 0;
        self.pr1 = 0;
        self.t1_frac = 0;
        self.t2con = 0;
        self.tmr2 = 0;
        self.pr2 = 0;
        self.t2_frac = 0;
        self.t3con = 0;
        self.tmr3 = 0;
        self.pr3 = 0;
        self.t3_frac = 0;
        self.ifs0 = 0;
        self.ifs1 = 0;
        self.iec0 = 0;
        self.iec1 = 0;
        self.ipc0 = 0;
        self.ipc1 = 0;
        self.ipc2 = 0;
        self.ipc3 = 0;
        self.ipc4 = 0;
        self.intcon1 = 0;
        self.intcon2 = 0;
        self.corcon = 0;
        self.acc_a = 0;
        self.acc_b = 0;
        self.tris = [PORT_A_MASK, PORT_B_MASK, PORT_C_MASK];
        self.lat = [0; 3];
        self.odc = [0; 3];
        self.pad_in = [0; 3];
        self.pdrv = [0; 3];
        self.pdrv_mask = [0; 3];
        self.rpinr7 = 0x1F1F;
        self.rpor = [0; RPOR_COUNT];
        self.ic = [IcChannel::default(), IcChannel::default()];
        self.oc = [OcChannel::default(), OcChannel::default()];
        self.p1tcon = 0;
        self.p1tmr = 0;
        self.p1tper = 0;
        self.p1tper_active = 0;
        self.p1secmp = 0;
        self.pwm1con1 = 0;
        self.pwm1con2 = 0;
        self.p1dtcon1 = 0;
        self.p1dtcon2 = 0;
        self.p1fltacon = 0;
        self.p1ovdcon = 0x3F00;
        self.p1dc = [0; 3];
        self.p1dc_active = [0; 3];
        self.p1_frac = 0;
        self.p1_post = 0;
        self.p1_gen = [false; 3];
        self.p1_dt_hi = [0; 3];
        self.p1_dt_lo = [0; 3];
        self.p1_fault_force = false;
        self.p1_fault_latched = false;
        self.p1_fault_release = false;
        self.flta_prev = false;
        self.rpinr12 = 0x001F;
        self.rpinr14 = 0x1F1F;
        self.rpinr15 = 0x001F;
        self.ifs3 = 0;
        self.iec3 = 0;
        self.ipc14 = 0;
        self.ipc15 = 0x4000;
        self.qei1con = 0;
        self.dflt1con = 0;
        self.pos1cnt = 0;
        self.max1cnt = 0xFFFF;
        self.qei_prev_ab = 0;
        self.qei_index_prev = false;
        self.qei_filt = [0; 3];
        self.qei_filt_cnt = [0; 3];
        self.qei_filt_out = [false; 3];
        self.qei_frac = 0;
        self.qei_timer_prev = false;
        self.qei_ext_psc = 0;
        self.reset_m3();
        self.pads_dirty = true;
        self.exception_pending = None;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        // The per-instruction trace contract (see `SimulationObserver`): all
        // three notifications, in order, gated on anyone observing.
        let observed = !observers.is_empty();

        // Exception entry happens between instructions: vector first, then run
        // the handler's first instruction in this same retired step. The entry
        // itself retires nothing; it only costs cycles.
        let mut used = 0u32;
        if self.take_interrupt()? {
            used += EXCEPTION_ENTRY_CYCLES;
        }

        let pc = self.pc;
        let word = self.fetch()?;
        if observed {
            for obs in observers {
                obs.on_step_start(pc, word);
            }
        }

        used += self.execute(word)?;
        self.cycles += u64::from(used);
        crate::advance_batch_cycle(bus, u64::from(used));
        self.advance_timers(used);
        // M2: run the PWM time base (and sample FLTA1) before publishing the
        // pad drive, then sample the QEI inputs with the input capture units.
        self.advance_mcpwm(used, bus);
        self.advance_adc(used);
        self.advance_spi(used);
        self.advance_i2c(used);
        self.sync_pads(bus);
        self.sample_capture(bus, used);

        if observed {
            // W0-W15, then the standard trailer: SP (W15), then PC.
            let mut registers = [0u32; 18];
            for (slot, value) in registers.iter_mut().take(16).zip(self.w.iter()) {
                *slot = u32::from(*value);
            }
            registers[16] = u32::from(self.w[15]);
            registers[17] = self.pc;

            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode: word },
            );
            for obs in observers {
                obs.on_step_end(used, &registers);
            }
        }
        Ok(())
    }

    fn clock_cycles(&self) -> u64 {
        self.cycles
    }

    /// Longest retired step: exception entry (2) plus RETFIE (3).
    fn max_step_cycles(&self) -> u32 {
        EXCEPTION_ENTRY_CYCLES + RETFIE_CYCLES
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.w[15] = val as u16;
    }

    fn set_exception_pending(&mut self, exception_num: u32) {
        self.exception_pending = Some(exception_num);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=15 => u32::from(self.w[id as usize]),
            16 => self.pc,
            17 => u32::from(self.sr),
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=15 => self.w[id as usize] = val as u16,
            16 => self.pc = val,
            17 => self.sr = val as u16,
            _ => {}
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..16).map(|i| format!("W{i}")).collect();
        names.push("PC".to_string());
        names.push("SR".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let n = name.trim().to_ascii_uppercase();
        if let Some(rest) = n.strip_prefix('W') {
            if let Ok(i) = rest.parse::<u8>() {
                if i < 16 {
                    return Some(i);
                }
            }
        }
        match n.as_str() {
            "PC" => Some(16),
            "SR" => Some(17),
            _ => None,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Pic24(Box::new(Pic24CpuSnapshot {
            registers: self.w.to_vec(),
            pc: self.pc,
            sr: self.sr,
            cycles: self.cycles,
            u1mode: self.u1mode,
            u1sta: self.u1sta,
            t1con: self.t1con,
            tmr1: self.tmr1,
            pr1: self.pr1,
            t1_frac: self.t1_frac,
            t2con: self.t2con,
            tmr2: self.tmr2,
            pr2: self.pr2,
            t2_frac: self.t2_frac,
            t3con: self.t3con,
            tmr3: self.tmr3,
            pr3: self.pr3,
            t3_frac: self.t3_frac,
            ifs0: self.ifs0,
            iec0: self.iec0,
            ipc0: self.ipc0,
            ipc1: self.ipc1,
            ipc2: self.ipc2,
            intcon1: self.intcon1,
            intcon2: self.intcon2,
            corcon: self.corcon,
            acc_a: self.acc_a,
            acc_b: self.acc_b,
            tris: self.tris,
            lat: self.lat,
            odc: self.odc,
            pad_in: self.pad_in,
            rpinr7: self.rpinr7,
            rpor: self.rpor.to_vec(),
            ic_con: [self.ic[0].con, self.ic[1].con],
            ic_fifo: [
                self.ic[0].fifo.iter().copied().collect(),
                self.ic[1].fifo.iter().copied().collect(),
            ],
            ic_prescale: [self.ic[0].prescale, self.ic[1].prescale],
            ic_ici_count: [self.ic[0].ici_count, self.ic[1].ici_count],
            oc_con: [self.oc[0].con, self.oc[1].con],
            oc_r: [self.oc[0].r, self.oc[1].r],
            oc_rs: [self.oc[0].rs, self.oc[1].rs],
            oc_level: [self.oc[0].level, self.oc[1].level],
            oc_halted: [self.oc[0].halted, self.oc[1].halted],
            p1tcon: self.p1tcon,
            p1tmr: self.p1tmr,
            p1tper: self.p1tper,
            p1tper_active: self.p1tper_active,
            p1secmp: self.p1secmp,
            pwm1con1: self.pwm1con1,
            pwm1con2: self.pwm1con2,
            p1dtcon1: self.p1dtcon1,
            p1dtcon2: self.p1dtcon2,
            p1fltacon: self.p1fltacon,
            p1ovdcon: self.p1ovdcon,
            p1dc: self.p1dc,
            p1dc_active: self.p1dc_active,
            p1_frac: self.p1_frac,
            p1_post: self.p1_post,
            p1_gen: self.p1_gen,
            p1_dt_hi: self.p1_dt_hi,
            p1_dt_lo: self.p1_dt_lo,
            p1_fault_force: self.p1_fault_force,
            p1_fault_latched: self.p1_fault_latched,
            p1_fault_release: self.p1_fault_release,
            flta_prev: self.flta_prev,
            rpinr12: self.rpinr12,
            rpinr14: self.rpinr14,
            rpinr15: self.rpinr15,
            ifs3: self.ifs3,
            iec3: self.iec3,
            ipc14: self.ipc14,
            ipc15: self.ipc15,
            qei1con: self.qei1con,
            dflt1con: self.dflt1con,
            pos1cnt: self.pos1cnt,
            max1cnt: self.max1cnt,
            qei_prev_ab: self.qei_prev_ab,
            qei_index_prev: self.qei_index_prev,
            qei_filt: self.qei_filt,
            qei_filt_cnt: self.qei_filt_cnt,
            qei_filt_out: self.qei_filt_out,
            qei_frac: self.qei_frac,
            qei_timer_prev: self.qei_timer_prev,
            qei_ext_psc: self.qei_ext_psc,
            ad1con1: self.ad1con1,
            ad1con2: self.ad1con2,
            ad1con3: self.ad1con3,
            ad1chs123: self.ad1chs123,
            ad1chs0: self.ad1chs0,
            ad1pcfgl: self.ad1pcfgl,
            ad1cssl: self.ad1cssl,
            adc_buf: self.adc_buf,
            adc_analog: self.adc_analog,
            adc_state: match self.adc_state {
                AdcState::Off => 0,
                AdcState::Sampling => 1,
                AdcState::Converting => 2,
            },
            adc_conv_remaining: self.adc_conv_remaining,
            adc_sample_remaining: self.adc_sample_remaining,
            adc_conv_count: self.adc_conv_count,
            adc_seq_pos: self.adc_seq_pos,
            adc_buf_pos: self.adc_buf_pos,
            adc_half: self.adc_half,
            adc_alt: self.adc_alt,
            adc_scan_next: self.adc_scan_next,
            adc_t3_trigger: self.adc_t3_trigger,
            adc_pwm_trigger: self.adc_pwm_trigger,
            spi1stat: self.spi1stat,
            spi1con1: self.spi1con1,
            spi1con2: self.spi1con2,
            spi1_rx: self.spi1_rx,
            spi1_tx: self.spi1_tx,
            spi1_tx_pending: self.spi1_tx_pending,
            spi1_busy: self.spi1_busy,
            spi1_shifting: self.spi1_shifting,
            i2c1con: self.i2c1con,
            i2c1stat: self.i2c1stat,
            i2c1add: self.i2c1add,
            i2c1msk: self.i2c1msk,
            i2c1brg: self.i2c1brg,
            i2c1rcv: self.i2c1rcv,
            i2c1trn: self.i2c1trn,
            i2c_target: self.i2c_target,
            i2c_reading: self.i2c_reading,
            i2c_addressed: self.i2c_addressed,
            i2c_10bit_prefix: self.i2c_10bit_prefix,
            i2c_busy: self.i2c_busy,
            i2c_op: self.i2c_op.map(|op| match op {
                I2cOp::Start => 0,
                I2cOp::RepeatedStart => 1,
                I2cOp::Transmit => 2,
                I2cOp::Receive => 3,
                I2cOp::Ack => 4,
                I2cOp::Stop => 5,
            }),
            ifs1: self.ifs1,
            iec1: self.iec1,
            ipc3: self.ipc3,
            ipc4: self.ipc4,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Pic24(s) = snapshot {
            if s.registers.len() == 16 {
                self.w.copy_from_slice(&s.registers);
            }
            self.pc = s.pc;
            self.sr = s.sr;
            self.cycles = s.cycles;
            self.u1mode = s.u1mode;
            self.u1sta = s.u1sta;
            self.t1con = s.t1con;
            self.tmr1 = s.tmr1;
            self.pr1 = s.pr1;
            self.t1_frac = s.t1_frac;
            self.t2con = s.t2con;
            self.tmr2 = s.tmr2;
            self.pr2 = s.pr2;
            self.t2_frac = s.t2_frac;
            self.t3con = s.t3con;
            self.tmr3 = s.tmr3;
            self.pr3 = s.pr3;
            self.t3_frac = s.t3_frac;
            self.ifs0 = s.ifs0;
            self.iec0 = s.iec0;
            self.ipc0 = s.ipc0;
            self.ipc1 = s.ipc1;
            self.ipc2 = s.ipc2;
            self.intcon1 = s.intcon1;
            self.intcon2 = s.intcon2;
            self.corcon = s.corcon;
            self.acc_a = s.acc_a;
            self.acc_b = s.acc_b;
            self.tris = s.tris;
            self.lat = s.lat;
            self.odc = s.odc;
            self.pad_in = s.pad_in;
            self.rpinr7 = s.rpinr7;
            for (i, v) in s.rpor.iter().take(RPOR_COUNT).enumerate() {
                self.rpor[i] = *v;
            }
            for ch in 0..2 {
                self.ic[ch].con = s.ic_con[ch];
                self.ic[ch].fifo = s.ic_fifo[ch].iter().copied().collect();
                self.ic[ch].prescale = s.ic_prescale[ch];
                self.ic[ch].ici_count = s.ic_ici_count[ch];
                self.oc[ch].con = s.oc_con[ch];
                self.oc[ch].r = s.oc_r[ch];
                self.oc[ch].rs = s.oc_rs[ch];
                self.oc[ch].level = s.oc_level[ch];
                self.oc[ch].halted = s.oc_halted[ch];
            }
            self.p1tcon = s.p1tcon;
            self.p1tmr = s.p1tmr;
            self.p1tper = s.p1tper;
            self.p1tper_active = s.p1tper_active;
            self.p1secmp = s.p1secmp;
            self.pwm1con1 = s.pwm1con1;
            self.pwm1con2 = s.pwm1con2;
            self.p1dtcon1 = s.p1dtcon1;
            self.p1dtcon2 = s.p1dtcon2;
            self.p1fltacon = s.p1fltacon;
            self.p1ovdcon = s.p1ovdcon;
            self.p1dc = s.p1dc;
            self.p1dc_active = s.p1dc_active;
            self.p1_frac = s.p1_frac;
            self.p1_post = s.p1_post;
            self.p1_gen = s.p1_gen;
            self.p1_dt_hi = s.p1_dt_hi;
            self.p1_dt_lo = s.p1_dt_lo;
            self.p1_fault_force = s.p1_fault_force;
            self.p1_fault_latched = s.p1_fault_latched;
            self.p1_fault_release = s.p1_fault_release;
            self.flta_prev = s.flta_prev;
            self.rpinr12 = s.rpinr12;
            self.rpinr14 = s.rpinr14;
            self.rpinr15 = s.rpinr15;
            self.ifs3 = s.ifs3;
            self.iec3 = s.iec3;
            self.ipc14 = s.ipc14;
            self.ipc15 = s.ipc15;
            self.qei1con = s.qei1con;
            self.dflt1con = s.dflt1con;
            self.pos1cnt = s.pos1cnt;
            self.max1cnt = s.max1cnt;
            self.qei_prev_ab = s.qei_prev_ab;
            self.qei_index_prev = s.qei_index_prev;
            self.qei_filt = s.qei_filt;
            self.qei_filt_cnt = s.qei_filt_cnt;
            self.qei_filt_out = s.qei_filt_out;
            self.qei_frac = s.qei_frac;
            self.qei_timer_prev = s.qei_timer_prev;
            self.qei_ext_psc = s.qei_ext_psc;
            self.ad1con1 = s.ad1con1;
            self.ad1con2 = s.ad1con2;
            self.ad1con3 = s.ad1con3;
            self.ad1chs123 = s.ad1chs123;
            self.ad1chs0 = s.ad1chs0;
            self.ad1pcfgl = s.ad1pcfgl;
            self.ad1cssl = s.ad1cssl;
            self.adc_buf = s.adc_buf;
            self.adc_analog = s.adc_analog;
            self.adc_state = match s.adc_state {
                1 => AdcState::Sampling,
                2 => AdcState::Converting,
                _ => AdcState::Off,
            };
            self.adc_conv_remaining = s.adc_conv_remaining;
            self.adc_sample_remaining = s.adc_sample_remaining;
            self.adc_conv_count = s.adc_conv_count;
            self.adc_seq_pos = s.adc_seq_pos;
            self.adc_buf_pos = s.adc_buf_pos;
            self.adc_half = s.adc_half;
            self.adc_alt = s.adc_alt;
            self.adc_scan_next = s.adc_scan_next;
            self.adc_t3_trigger = s.adc_t3_trigger;
            self.adc_pwm_trigger = s.adc_pwm_trigger;
            self.spi1stat = s.spi1stat;
            self.spi1con1 = s.spi1con1;
            self.spi1con2 = s.spi1con2;
            self.spi1_rx = s.spi1_rx;
            self.spi1_tx = s.spi1_tx;
            self.spi1_tx_pending = s.spi1_tx_pending;
            self.spi1_busy = s.spi1_busy;
            self.spi1_shifting = s.spi1_shifting;
            self.i2c1con = s.i2c1con;
            self.i2c1stat = s.i2c1stat;
            self.i2c1add = s.i2c1add;
            self.i2c1msk = s.i2c1msk;
            self.i2c1brg = s.i2c1brg;
            self.i2c1rcv = s.i2c1rcv;
            self.i2c1trn = s.i2c1trn;
            self.i2c_target = s.i2c_target;
            self.i2c_reading = s.i2c_reading;
            self.i2c_addressed = s.i2c_addressed;
            self.i2c_10bit_prefix = s.i2c_10bit_prefix;
            self.i2c_busy = s.i2c_busy;
            self.i2c_op = s.i2c_op.map(|op| match op {
                0 => I2cOp::Start,
                1 => I2cOp::RepeatedStart,
                2 => I2cOp::Transmit,
                3 => I2cOp::Receive,
                4 => I2cOp::Ack,
                _ => I2cOp::Stop,
            });
            self.ifs1 = s.ifs1;
            self.iec1 = s.iec1;
            self.ipc3 = s.ipc3;
            self.ipc4 = s.ipc4;
            // The pad models take the restored state on the next sync.
            self.pads_dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    fn cpu_with(words: &[u32]) -> Pic24 {
        let mut cpu = Pic24::new();
        cpu.load_words(words);
        cpu
    }

    fn mov_lit_w0(val: u16) -> u32 {
        0x0020_0000 | (u32::from(val) << 4)
    }

    fn mov_w0_to_f(f: u16) -> u32 {
        debug_assert!(f % 2 == 0);
        0b1_0001 << 19 | (u32::from(f >> 1) << 4)
    }

    /// A signed 40-bit accumulator value from its ACCxU/H/L bytes.
    fn acc40(u: i64, h: i64, l: i64) -> i64 {
        wrap40((u << 32) | (h << 16) | l)
    }

    fn step<B: Bus>(cpu: &mut Pic24, bus: &mut B) {
        cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
    }

    #[test]
    fn base_sub_inc_clr_and_setm_wreg_forms() {
        // xc16-as words: sub.w w1,w0,[w15] = 0x508F80,
        // inc.w w9,w9 = 0xE80489, clr.w w0 = 0xEB0000, setm.w w12 = 0xEB8600.
        let mut cpu = cpu_with(&[0x0050_8F80]);
        cpu.w[1] = 5;
        cpu.w[0] = 3;
        cpu.w[15] = 0x0A00;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(0x0A00).unwrap(), 2, "W1 - W0");
        assert_eq!(cpu.sr & SR_C, SR_C, "no borrow");

        let mut cpu = cpu_with(&[0x00E8_0489]);
        cpu.w[9] = 0x7FFF;
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[9], 0x8000);
        assert_ne!(cpu.sr & SR_N, 0);
        assert_ne!(cpu.sr & SR_OV, 0);
        assert_eq!(cpu.sr & SR_C, 0);

        // INC 0xFFFF -> 0x0000 sets C and DC; the 32-bit addc idiom depends
        // on that carry.
        let mut cpu = cpu_with(&[0x00E8_0489]);
        cpu.w[9] = 0xFFFF;
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[9], 0);
        assert_ne!(cpu.sr & SR_C, 0, "INC carry");
        assert_ne!(cpu.sr & SR_DC, 0, "INC digit carry");
        assert_ne!(cpu.sr & SR_Z, 0);

        // INC2 W9,W9 adds two, not one (DS70157D §5, 0x00E8_8489).
        let mut cpu = cpu_with(&[0x00E8_8489]);
        cpu.w[9] = 1;
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[9], 3, "INC2 adds 2");
        assert_eq!(cpu.sr & SR_C, 0);

        let mut cpu = cpu_with(&[0x00E8_8489]);
        cpu.w[9] = 0xFFFE;
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[9], 0, "INC2 wraps");
        assert_ne!(cpu.sr & SR_C, 0, "INC2 carry");

        // CLR/SETM affect no status flags (DS70283H Table 22-2 #15/#69).
        let mut cpu = cpu_with(&[0x00EB_0000]);
        cpu.w[0] = 0x1234;
        cpu.sr = SR_C | SR_N | SR_Z | SR_OV | SR_DC;
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[0], 0);
        assert_eq!(cpu.sr, SR_C | SR_N | SR_Z | SR_OV | SR_DC);

        let mut cpu = cpu_with(&[0x00EB_8600]);
        cpu.w[12] = 0x1234;
        cpu.sr = SR_C | SR_Z;
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[12], 0xFFFF);
        assert_eq!(cpu.sr, SR_C | SR_Z);
    }

    #[test]
    fn full_program_flash_loads_through_the_last_instruction() {
        // 11264 instructions, each in its own 4-byte XC16 slot: every
        // instruction up to 0x57FE.
        let slots: Vec<u8> = (0..INSTR_SLOTS)
            .flat_map(|_| mov_lit_w0(0x48).to_le_bytes().to_vec())
            .collect();
        let mut image = crate::memory::ProgramImage::new(0, crate::Arch::Pic24);
        image.add_segment(0, slots);
        let mut cpu = Pic24::new();
        cpu.load_program_image(&image);
        let mut bus = SystemBus::new();
        cpu.set_pc(PROGRAM_PC_END - PC_UNITS_PER_INSTRUCTION);
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.w[0], 0x48,
            "the last instruction slot was loaded, not truncated"
        );
        assert!(
            cpu.fetch().is_err(),
            "nothing past the end of program flash"
        );
    }

    #[test]
    fn odd_program_addresses_are_rejected() {
        assert!(check_xc16_pc_address("entry point", 0x201).is_err());
        assert!(check_xc16_pc_address("entry point", 0x200).is_ok());
    }

    #[test]
    fn entry_points_outside_program_flash_are_rejected() {
        let end = u64::from(PROGRAM_PC_END);
        assert!(check_xc16_entry(end - 2).is_ok());
        assert!(check_xc16_entry(end).is_err());
        assert!(check_xc16_entry(end + 0x100).is_err());
        assert!(check_xc16_entry(0x201).is_err());
    }

    /// A minimal little-endian ELF32 for EM_DSPIC30F (118) with one
    /// executable section per entry, in the XC16 4-byte slot form. Returns
    /// the bytes and each section header's file offset, so tests can corrupt
    /// individual fields.
    fn pic24_elf(entry: u32, sections: &[(u32, Vec<u8>)]) -> (Vec<u8>, Vec<usize>) {
        let shstr = b"\x00.text\x00.shstrtab\x00";
        let n = sections.len() as u16;
        let mut data_off = 52u32;
        let mut sec_off = Vec::new();
        for (_, bytes) in sections {
            sec_off.push(data_off);
            data_off += bytes.len() as u32;
        }
        let shstr_off = data_off;
        data_off += shstr.len() as u32;
        let shoff = data_off;

        let mut out = vec![0x7f, b'E', b'L', b'F', 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        out.extend_from_slice(&118u16.to_le_bytes()); // EM_DSPIC30F
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&entry.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // e_phoff
        out.extend_from_slice(&shoff.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        for half in [52u16, 0, 0, 40, n + 2, n + 1] {
            out.extend_from_slice(&half.to_le_bytes());
        }
        for (_, bytes) in sections {
            out.extend_from_slice(bytes);
        }
        out.extend_from_slice(shstr);
        let mut shdr_off = Vec::new();
        let mut sh =
            |out: &mut Vec<u8>, name: u32, typ: u32, flags: u32, addr: u32, off: u32, size: u32| {
                shdr_off.push(out.len());
                for w in [name, typ, flags, addr, off, size, 0u32, 0, 2, 0] {
                    out.extend_from_slice(&w.to_le_bytes());
                }
            };
        sh(&mut out, 0, 0, 0, 0, 0, 0);
        for (i, (addr, bytes)) in sections.iter().enumerate() {
            sh(&mut out, 1, 1, 0x6, *addr, sec_off[i], bytes.len() as u32);
        }
        sh(&mut out, 8, 3, 0, 0, shstr_off, shstr.len() as u32);
        (out, shdr_off)
    }

    #[test]
    fn sections_past_program_flash_are_rejected() {
        let one = [0u8; 4];
        let end = u64::from(PROGRAM_PC_END);
        assert!(
            check_xc16_section(end - 2, &one).is_ok(),
            "the last instruction fits"
        );
        assert!(
            check_xc16_section(end, &one).is_err(),
            "starts at the end of flash"
        );
        assert!(
            check_xc16_section(end + 0x100, &one).is_err(),
            "starts beyond flash"
        );
        let two = [0u8; 8];
        assert!(
            check_xc16_section(end - 2, &two).is_err(),
            "runs one instruction past the end"
        );
        assert!(
            check_xc16_section(0x200, &[0u8; 6]).is_err(),
            "not a whole number of slots"
        );
    }

    #[test]
    fn parse_elf_image_rejects_malformed_xc16_elves() {
        use crate::system::node::parse_elf_image;
        let code = vec![0u8; 8]; // two 4-byte slots
        assert!(parse_elf_image(&pic24_elf(0x200, &[(0x200, code.clone())]).0).is_ok());

        let odd_entry = parse_elf_image(&pic24_elf(0x201, &[(0x200, code.clone())]).0).unwrap_err();
        assert!(odd_entry.to_string().contains("entry point"), "{odd_entry}");
        let odd_section =
            parse_elf_image(&pic24_elf(0x200, &[(0x201, code.clone())]).0).unwrap_err();
        assert!(odd_section.to_string().contains("is odd"), "{odd_section}");
        for entry in [PROGRAM_PC_END, PROGRAM_PC_END + 0x100] {
            let far = parse_elf_image(&pic24_elf(entry, &[(0x200, code.clone())]).0).unwrap_err();
            assert!(far.to_string().contains("entry point"), "{far}");
        }
        // A section whose bytes lie outside the file is range-checked before
        // anything is loaded, and is an error rather than a silent skip.
        let (mut elf, shdrs) = pic24_elf(0x200, &[(0x200, code.clone())]);
        elf[shdrs[1] + 16..shdrs[1] + 20].copy_from_slice(&0x10_0000u32.to_le_bytes()); // sh_offset
        let err = parse_elf_image(&elf).unwrap_err().to_string();
        assert!(err.contains("runs past the end of the ELF file"), "{err}");
        // Every section is range-checked, not only executable ones.
        let (mut data, shdrs) = pic24_elf(0x200, &[(0x200, code.clone())]);
        data[shdrs[1] + 8..shdrs[1] + 12].copy_from_slice(&4u32.to_le_bytes()); // SHF_ALLOC only
        data[shdrs[1] + 16..shdrs[1] + 20].copy_from_slice(&0x10_0000u32.to_le_bytes());
        let err = parse_elf_image(&data).unwrap_err().to_string();
        assert!(err.contains("runs past the end of the ELF file"), "{err}");
        // A section that is not a whole number of 4-byte slots is rejected.
        let bad = parse_elf_image(&pic24_elf(0x200, &[(0x200, vec![0u8; 6])]).0).unwrap_err();
        assert!(bad.to_string().contains("not a whole number"), "{bad}");
        // A section that runs off program flash is rejected.
        let past = parse_elf_image(&pic24_elf(0x200, &[(PROGRAM_PC_END - 2, code)]).0).unwrap_err();
        assert!(
            past.to_string().contains("does not fit program flash"),
            "{past}"
        );
    }

    #[test]
    fn reset_starts_at_word_zero() {
        let mut cpu = Pic24::new();
        let mut bus = SystemBus::new();
        cpu.reset(&mut bus).unwrap();
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.clock_cycles(), 0);
    }

    #[test]
    fn call_and_return_use_the_upward_stack() {
        // 0x000: CALL 0x008 (2 words, return to 0x004)
        // 0x008: MOV #0x11, W0 ; 0x00A: RETURN
        let mut cpu = cpu_with(&[
            0x0002_0008,
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            mov_lit_w0(0x11),
            0x0006_0000,
        ]);
        cpu.set_sp(0x0800);
        let mut bus = SystemBus::new();

        step(&mut cpu, &mut bus); // CALL
        assert_eq!(cpu.get_pc(), 8, "CALL jumps to its 23-bit target");
        assert_eq!(cpu.w[15], 0x0804, "CALL pushes two words, growing up");
        assert_eq!(cpu.data_read(0x0800).unwrap(), 0x0004, "PC+4 low word");
        assert_eq!(cpu.data_read(0x0802).unwrap(), 0x0000, "PC+4 high word");

        step(&mut cpu, &mut bus); // MOV #0x11, W0
        assert_eq!(cpu.w[0], 0x11);

        step(&mut cpu, &mut bus); // RETURN
        assert_eq!(cpu.get_pc(), 4, "RETURN pops the stacked PC");
        assert_eq!(cpu.w[15], 0x0800, "stack unwound");
    }

    #[test]
    fn lnk_and_ulnk_allocate_a_frame() {
        let mut cpu = cpu_with(&[0x00FA_0004, 0x00FA_8000]);
        cpu.set_sp(0x0800);
        cpu.w[14] = 0x1234;
        let mut bus = SystemBus::new();

        step(&mut cpu, &mut bus); // LNK #4
        assert_eq!(cpu.w[14], 0x0802, "W14 points at the saved frame pointer");
        assert_eq!(cpu.w[15], 0x0806, "frame of 4 bytes allocated");
        assert_eq!(cpu.data_read(0x0800).unwrap(), 0x1234, "W14 saved");

        step(&mut cpu, &mut bus); // ULNK
        assert_eq!(cpu.w[15], 0x0800, "W15 restored from W14");
        assert_eq!(cpu.w[14], 0x1234, "frame pointer popped");
    }

    #[test]
    fn goto_is_a_two_word_jump() {
        let mut cpu = cpu_with(&[0x0004_0200, 0x0000_0000]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 0x200, "GOTO loads the 23-bit target");
    }

    #[test]
    fn byte_moves_and_sign_extend() {
        // 0x000: MOV.B #0x80, W0 ; 0x002: MOV.B W0, [W1] ; 0x004: SE [W1], W2
        let mut cpu = cpu_with(&[0x00B3_C800, 0x0078_4880, 0x00FB_0111]);
        cpu.w[1] = 0x0900;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read_byte(0x0900).unwrap(), 0x80);
        assert_eq!(cpu.w[2], 0xFF80, "SE sign-extends the byte");

        // Byte addressing keeps the neighbouring byte intact.
        cpu.data_write_byte(0x0901, 0x5A).unwrap();
        assert_eq!(cpu.data_read(0x0900).unwrap(), 0x5A80);
    }

    #[test]
    fn mov_literal_loads_w0() {
        let mut cpu = cpu_with(&[mov_lit_w0(0x48)]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[0], 0x48);
        assert_eq!(cpu.get_pc(), 2, "PC advances by 2 per instruction");
        assert_eq!(cpu.clock_cycles(), 1);
    }

    #[test]
    fn mov_w0_to_u1txreg_reaches_sink() {
        let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let mut cpu = cpu_with(&[mov_lit_w0(0x48), mov_w0_to_f(SFR_U1TXREG)]);
        cpu.set_serial_sink(sink.clone());
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert_eq!(*sink.lock().unwrap(), vec![b'H']);
        assert_eq!(cpu.get_pc(), 4);
    }

    #[test]
    fn nop_advances_pc() {
        let mut cpu = cpu_with(&[0x0000_0000]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 2);
    }

    #[test]
    fn bra_minus_one_stays_put() {
        // BRA $-1 → PC = PC + 2 + 2*(-1) = PC.
        let mut cpu = cpu_with(&[0x0037_FFFF]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.clock_cycles(), 2);
    }

    #[test]
    fn u1sta_read_reports_transmit_empty() {
        let mut cpu = Pic24::new();
        assert_eq!(cpu.data_read(SFR_U1STA).unwrap() & U1STA_TRMT, U1STA_TRMT);
        assert_eq!(cpu.data_read(SFR_U1STA).unwrap() & U1STA_UTXBF, 0);
    }

    #[test]
    fn ram_round_trip() {
        let mut cpu = Pic24::new();
        cpu.data_write(RAM_BASE as u16, 0xBEEF).unwrap();
        assert_eq!(cpu.data_read(RAM_BASE as u16).unwrap(), 0xBEEF);
    }

    #[test]
    fn unsupported_opcode_is_a_decode_error() {
        let mut cpu = cpu_with(&[0x00FF_FFFF]);
        let mut bus = SystemBus::new();
        let err = cpu.step(&mut bus, &[], &SimulationConfig::default());
        assert!(matches!(err, Err(SimulationError::DecodeError(_))));
    }

    // ── M1a: timers, interrupts, accumulators ───────────────────────────────

    fn enc(word: u32) -> [u8; INSTR_SLOT_BYTES] {
        word.to_le_bytes()
    }

    fn write_sfr(cpu: &mut Pic24, addr: u16, val: u16) {
        cpu.data_write(addr, val).expect("SFR write");
    }

    #[test]
    fn timer1_counts_to_period_and_raises_flag() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        write_sfr(&mut cpu, SFR_PR1, 2); // period = PR + 1 = 3 ticks
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        let mut bus = SystemBus::new();
        for _ in 0..3 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(cpu.data_read(SFR_TMR1).unwrap(), 0, "wraps on match");
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0),
            0,
            "T1IF set on match"
        );
    }

    #[test]
    fn timer1_prescaler_1_to_8() {
        let mut cpu = cpu_with(&[0x0000_0000; 10]);
        write_sfr(&mut cpu, SFR_PR1, 0);
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON | (1 << TCON_TCKPS_SHIFT)); // 1:8
        let mut bus = SystemBus::new();
        for _ in 0..7 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(
            cpu.data_read(SFR_IFS0).unwrap(),
            0,
            "no tick before the 8th instruction cycle"
        );
        step(&mut cpu, &mut bus);
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0),
            0,
            "ticked and matched on the 8th"
        );
    }

    #[test]
    fn timer23_pair_counts_32_bit_and_flags_timer3() {
        let mut cpu = cpu_with(&[0x0000_0000; 5]);
        write_sfr(&mut cpu, SFR_PR2, 2); // 32-bit period = 3 ticks
        write_sfr(&mut cpu, SFR_PR3, 0);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON | T2CON_T32);
        let mut bus = SystemBus::new();
        for _ in 0..2 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(cpu.data_read(SFR_IFS0).unwrap(), 0, "not yet at period");
        step(&mut cpu, &mut bus);
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T3.0),
            0,
            "32-bit match reports as Timer3 (DS70283H §12)"
        );
        assert_eq!(cpu.data_read(SFR_TMR2).unwrap(), 0);
        assert_eq!(cpu.data_read(SFR_TMR3).unwrap(), 0);
    }

    #[test]
    fn timer1_interrupt_vectors_frames_and_returns() {
        let mut cpu = cpu_with(&[0x0037_FFFF]); // slot 0: BRA $-1
                                                // Handler at PC 0x100:
                                                //   MOV #0, W0 ; MOV W0, T1CON ; MOV W0, IFS0 ; RETFIE
                                                // The timer is stopped first: with PR1 = 0 a tick re-sets the flag in
                                                // the very same step that clears it — correct silicon behaviour, but
                                                // not what this test wants to observe.
        let mut handler = Vec::new();
        handler.extend(enc(mov_lit_w0(0)));
        handler.extend(enc(mov_w0_to_f(SFR_T1CON)));
        handler.extend(enc(mov_w0_to_f(SFR_IFS0)));
        handler.extend(enc(INSN_RETFIE));
        cpu.load_flash(0x100, &handler);
        // The T1 vector (number 11) lives at PC 0x1A and holds the 24-bit ISR
        // address as data (DS70283H §7.1).
        cpu.load_flash(0x1A, &enc(0x000100));

        // The stack grows toward higher addresses (DS70283H Figure 4-4), so
        // it starts at the bottom of the 2 KB SRAM.
        cpu.set_sp(0x0800);
        write_sfr(&mut cpu, SFR_PR1, 0); // flag after every tick
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        write_sfr(&mut cpu, SFR_IEC0, 1 << IRQ_T1.0);
        write_sfr(&mut cpu, SFR_IPC0, 1 << 12); // T1IP = 1

        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus); // BRA; the timer ticks -> T1IF
        assert_ne!(cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0), 0);

        step(&mut cpu, &mut bus); // vector fetch + first handler instruction
        assert_eq!(
            cpu.get_pc(),
            0x102,
            "handler began at the vector's ISR address"
        );
        assert_eq!(
            (cpu.data_read(SFR_SR).unwrap() >> 5) & 0x7,
            1,
            "IPL raised to the source priority"
        );

        // DS70283H frame, stack growing up (Figure 4-4): [SP] = PC<15:0>,
        // [SP+2] = SR<7:0>|IPL3|PC<22:16>.
        let sp = cpu.w[15];
        assert_eq!(cpu.data_read(sp - 2).unwrap(), 0x0000, "packed frame word");
        assert_eq!(cpu.data_read(sp - 4).unwrap(), 0x0000, "saved PC (was 0)");

        step(&mut cpu, &mut bus); // MOV W0, T1CON stops the timer
        assert_eq!(
            cpu.data_read(SFR_T1CON).unwrap() & TCON_TON,
            0,
            "ISR stopped Timer1"
        );
        step(&mut cpu, &mut bus); // MOV W0, IFS0 clears the flag
        assert_eq!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0),
            0,
            "ISR cleared T1IF"
        );
        step(&mut cpu, &mut bus); // RETFIE
        assert_eq!(cpu.get_pc(), 0, "returned to the interrupted loop");
        assert_eq!(cpu.w[15], 0x0800, "stack fully unwound");
        assert_eq!(
            (cpu.data_read(SFR_SR).unwrap() >> 5) & 0x7,
            0,
            "IPL restored"
        );
    }

    #[test]
    fn disabled_source_does_not_vector() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        write_sfr(&mut cpu, SFR_PR1, 0);
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        write_sfr(&mut cpu, SFR_IPC0, 1 << 12); // priority set, IEC0 clear
        cpu.set_sp(0x1000);
        let mut bus = SystemBus::new();
        for _ in 0..4 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(cpu.get_pc(), 8, "kept executing linearly (4 × 2 units)");
        assert_eq!(cpu.w[15], 0x1000);
    }

    #[test]
    fn accumulator_sfr_words_round_trip() {
        let mut cpu = Pic24::new();
        write_sfr(&mut cpu, SFR_ACCAL, 0x1234);
        write_sfr(&mut cpu, SFR_ACCAH, 0x5678);
        write_sfr(&mut cpu, SFR_ACCAU, 0x9A);
        assert_eq!(cpu.data_read(SFR_ACCAL).unwrap(), 0x1234);
        assert_eq!(cpu.data_read(SFR_ACCAH).unwrap(), 0x5678);
        assert_eq!(cpu.data_read(SFR_ACCAU).unwrap(), 0x9A);
        write_sfr(&mut cpu, SFR_ACCBL, 0xBEEF);
        assert_eq!(cpu.data_read(SFR_ACCBL).unwrap(), 0xBEEF);
        assert_eq!(
            cpu.data_read(SFR_ACCAU).unwrap(),
            0x9A,
            "B write left A alone"
        );
    }

    // ── M1b: ports, PPS, capture/compare ────────────────────────────────────

    /// A bus carrying the three real pad models behind the CPU's mirror
    /// window (the routing a chip yaml gives them), plus flat storage for
    /// anything else.
    struct MirrorBus {
        ports: [crate::peripherals::pic24_gpio::Pic24GpioPort; 3],
        mem: std::collections::HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl MirrorBus {
        fn new() -> Self {
            Self {
                ports: [
                    crate::peripherals::pic24_gpio::Pic24GpioPort::new(PORT_A_MASK),
                    crate::peripherals::pic24_gpio::Pic24GpioPort::new(PORT_B_MASK),
                    crate::peripherals::pic24_gpio::Pic24GpioPort::new(PORT_C_MASK),
                ],
                mem: std::collections::HashMap::new(),
                config: SimulationConfig::default(),
            }
        }

        /// `(port, offset)` when `addr` lands in a port mirror window.
        fn route(addr: u64) -> Option<(usize, u64)> {
            for port in 0..3 {
                let base = PIC24_IO_MIRROR_BASE + u64::from(Pic24::port_base(port));
                if (base..base + crate::peripherals::pic24_gpio::WINDOW_BYTES).contains(&addr) {
                    return Some((port, addr - base));
                }
            }
            None
        }

        /// Drive a pad from the outside world (a button, a waveform source).
        fn set_input(&mut self, port: usize, pin: u8, level: bool) {
            use crate::pins::{External, PinPort};
            self.ports[port].set_external(pin, External::Level(level));
        }

        /// The live pad level, as the firmware's `PORTx` read sees it.
        fn pad(&self, port: usize, pin: u8) -> bool {
            use crate::pins::PinPort;
            self.ports[port].input(pin).unwrap_or(false)
        }
    }

    impl Bus for MirrorBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            use crate::Peripheral;
            if let Some((port, off)) = Self::route(addr) {
                return self.ports[port].read(off);
            }
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            use crate::Peripheral;
            if let Some((port, off)) = Self::route(addr) {
                return self.ports[port].write(off, value);
            }
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    #[test]
    fn gpio_output_drives_the_pad_model() {
        use crate::pins::{PadDriver, PinPort};
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_TRISB, 0x0000);
        write_sfr(&mut cpu, SFR_LATB, 0x0001);
        step(&mut cpu, &mut bus);
        assert!(bus.pad(1, 0), "RB0 drives high through the mirror");
        assert!(!bus.pad(1, 1), "RB1 drives its low latch");
        assert_eq!(
            bus.ports[1].driver(0),
            Some(PadDriver::drive(true)),
            "the probe sees the GPIO stage, not an external echo"
        );
    }

    #[test]
    fn port_read_sees_external_levels_and_own_latch() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        let mut bus = MirrorBus::new();
        // RB0 output driving high; RB1 input held high by the outside.
        write_sfr(&mut cpu, SFR_TRISB, !0x0001u16);
        write_sfr(&mut cpu, SFR_LATB, 0x0001);
        bus.set_input(1, 1, true);
        step(&mut cpu, &mut bus);
        let portb = cpu.data_read(SFR_PORTB).unwrap();
        assert_eq!(
            portb & 0b11,
            0b11,
            "PORTB shows the latch on the output and the pin on the input"
        );
    }

    #[test]
    fn oc1_active_low_one_shot_pulses_rp0() {
        let mut cpu = cpu_with(&[0x0000_0000; 8]);
        let mut bus = MirrorBus::new();
        // OC1 → RP0 (RB0); Timer2 free-running with a long period.
        write_sfr(&mut cpu, SFR_RPOR0, PPS_OUT_OC1);
        write_sfr(&mut cpu, SFR_PR2, 100);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON);
        write_sfr(&mut cpu, SFR_OC1R, 3);
        write_sfr(&mut cpu, SFR_OC1CON, 0b001);
        step(&mut cpu, &mut bus); // TMR2 = 1: pin initialised low
        assert!(!bus.pad(1, 0), "one-shot starts low on RP0");
        step(&mut cpu, &mut bus); // TMR2 = 2
        assert!(!bus.pad(1, 0), "still before the match");
        step(&mut cpu, &mut bus); // TMR2 = 3: compare match
        assert!(bus.pad(1, 0), "match drives the pin high");
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_OC1.0),
            0,
            "OC1IF on the rising edge"
        );
        // A finished one-shot waits for the next OCxCON write.
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert!(bus.pad(1, 0), "halted after the single pulse");
    }

    #[test]
    fn oc1_toggle_flips_on_every_match() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_RPOR0, PPS_OUT_OC1);
        write_sfr(&mut cpu, SFR_PR2, 20);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON);
        write_sfr(&mut cpu, SFR_OC1R, 1);
        write_sfr(&mut cpu, SFR_OC1CON, 0b011);
        step(&mut cpu, &mut bus); // TMR2 = 1 → first toggle, high
        assert!(bus.pad(1, 0), "toggled high at the first match");
        step(&mut cpu, &mut bus); // TMR2 = 2
                                  // TMR2 runs 3..20, wraps to 0, then reaches 1 again at step 22.
        for _ in 0..20 {
            step(&mut cpu, &mut bus);
        }
        assert!(!bus.pad(1, 0), "second match flipped it back low");
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_OC1.0),
            0,
            "further matches keep setting OC1IF"
        );
    }

    #[test]
    fn oc1_pwm_reloads_ocxrs_on_rollover() {
        let mut cpu = cpu_with(&[0x0000_0000; 16]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_RPOR0, PPS_OUT_OC1);
        write_sfr(&mut cpu, SFR_PR2, 4);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON);
        write_sfr(&mut cpu, SFR_OC1RS, 2);
        write_sfr(&mut cpu, SFR_OC1CON, 0b110);
        step(&mut cpu, &mut bus); // TMR2 = 1
        assert!(
            !bus.pad(1, 0),
            "starts low while OCxR is still 0 (DS70209A §13.3.2.8)"
        );
        for _ in 0..4 {
            step(&mut cpu, &mut bus); // 2, 3, 4, then rollover → reload
        }
        assert!(bus.pad(1, 0), "OCxRS loaded into OCxR on rollover");
        // OCxR is read-only while a PWM mode runs.
        write_sfr(&mut cpu, SFR_OC1R, 3);
        assert_eq!(
            cpu.data_read(SFR_OC1R).unwrap(),
            2,
            "a PWM-mode OCxR write is ignored"
        );
        step(&mut cpu, &mut bus); // TMR2 = 1
        step(&mut cpu, &mut bus); // TMR2 = 2 → match drives low
        assert!(!bus.pad(1, 0), "match with the reloaded OCxR drives low");
    }

    #[test]
    fn ic1_captures_tmr2_on_a_rising_edge_of_rp0() {
        let mut cpu = cpu_with(&[0x0000_0000; 12]);
        let mut bus = MirrorBus::new();
        // IC1 from RP0 (RPINR7 resets to 0x1F1F = tied to VSS).
        write_sfr(&mut cpu, SFR_RPINR7, 0);
        write_sfr(&mut cpu, SFR_PR2, 0xFFFF);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON);
        write_sfr(&mut cpu, SFR_IC1CON, ICCON_ICTMR | 0b011);
        step(&mut cpu, &mut bus); // TMR2 = 1, pad low
        step(&mut cpu, &mut bus); // TMR2 = 2
        bus.set_input(1, 0, true);
        step(&mut cpu, &mut bus); // TMR2 = 3 and capture
        assert_ne!(
            cpu.data_read(SFR_IC1CON).unwrap() & ICCON_ICBNE,
            0,
            "buffer not empty after the capture"
        );
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_IC1.0),
            0,
            "IC1IF with ICI = 00 fires on every capture"
        );
        // Reading ICxBUF pops the FIFO entry and clears ICBNE.
        assert_eq!(cpu.data_read(SFR_IC1BUF).unwrap(), 3, "captured TMR2");
        assert_eq!(
            cpu.data_read(SFR_IC1CON).unwrap() & ICCON_ICBNE,
            0,
            "draining the FIFO clears ICBNE"
        );
    }

    #[test]
    fn ic_interrupt_divider_and_overflow_rules() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_RPINR7, 0);
        write_sfr(&mut cpu, SFR_PR2, 0xFFFF);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON);
        // Rising edges; interrupt on every second capture (ICI = 01).
        write_sfr(
            &mut cpu,
            SFR_IC1CON,
            ICCON_ICTMR | (1 << ICCON_ICI_SHIFT) | 0b011,
        );
        step(&mut cpu, &mut bus);
        for _ in 0..5 {
            bus.set_input(1, 0, false);
            step(&mut cpu, &mut bus);
            bus.set_input(1, 0, true);
            step(&mut cpu, &mut bus);
        }
        // Four captures: the second and fourth interrupted, buffer full.
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_IC1.0),
            0,
            "4th capture interrupted (ICI = 01)"
        );
        // The FIFO is full; a fifth capture sets ICOV and is inhibited.
        write_sfr(&mut cpu, SFR_IFS0, 0);
        bus.set_input(1, 0, false);
        step(&mut cpu, &mut bus);
        bus.set_input(1, 0, true);
        step(&mut cpu, &mut bus);
        let con = cpu.data_read(SFR_IC1CON).unwrap();
        assert_ne!(con & ICCON_ICOV, 0, "overflow sets ICOV");
        assert_eq!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_IC1.0),
            0,
            "a nonzero ICI overflow inhibits the interrupt (DS70198D §12.4.2)"
        );
        // Draining the four entries clears ICBNE and ICOV alike.
        for _ in 0..4 {
            let _ = cpu.data_read(SFR_IC1BUF).unwrap();
        }
        let con = cpu.data_read(SFR_IC1CON).unwrap();
        assert_eq!(con & (ICCON_ICBNE | ICCON_ICOV), 0, "drained clean");
    }

    // ── M1c: DSP MAC class (DS70000157G §4.15, §5.4) ───────────────────────

    /// `MAC/MPY/MPY.N/MSC Wm*Wn, Acc {,AWB}` word (fields per §5.4).
    #[allow(clippy::too_many_arguments)]
    fn mac_word(
        mmm: u32,
        acc_b: bool,
        negate: bool,
        store: bool,
        xx: u32,
        yy: u32,
        xop: u32,
        yop: u32,
        aa: u32,
    ) -> u32 {
        let mut w = 0x00C0_0000
            | ((mmm & 7) << 16)
            | ((xx & 3) << 12)
            | ((yy & 3) << 10)
            | ((xop & 0xF) << 6)
            | ((yop & 0xF) << 2)
            | (aa & 3);
        if acc_b {
            w |= 1 << 15;
        }
        if negate {
            w |= 1 << 14;
        }
        if store {
            w |= 0b11;
        }
        w
    }

    /// Square `MAC/MPY Wm*Wm, Acc {,prefetch}` word.
    fn square_word(mm: u32, acc_b: bool, store: bool, xx: u32, yy: u32, xop: u32, yop: u32) -> u32 {
        let mut w = 0x00F0_0000
            | ((mm & 3) << 16)
            | ((xx & 3) << 12)
            | ((yy & 3) << 10)
            | ((xop & 0xF) << 6)
            | ((yop & 0xF) << 2);
        if acc_b {
            w |= 1 << 15;
        }
        if store {
            w |= 1;
        }
        w
    }

    fn clr_word(acc_b: bool, xx: u32, yy: u32, xop: u32, yop: u32, aa: u32) -> u32 {
        0x00C3_0000
            | ((acc_b as u32) << 15)
            | ((xx & 3) << 12)
            | ((yy & 3) << 10)
            | ((xop & 0xF) << 6)
            | ((yop & 0xF) << 2)
            | (aa & 3)
    }

    fn movsac_word(acc_b: bool, xx: u32, yy: u32, xop: u32, yop: u32, aa: u32) -> u32 {
        0x00C7_0000
            | ((acc_b as u32) << 15)
            | ((xx & 3) << 12)
            | ((yy & 3) << 10)
            | ((xop & 0xF) << 6)
            | ((yop & 0xF) << 2)
            | (aa & 3)
    }

    fn lac_word(acc_b: bool, wb: u32, slit: i32, mode: u32, ws: u32) -> u32 {
        0x00CA_0000
            | ((acc_b as u32) << 15)
            | ((wb & 0xF) << 11)
            | (((slit & 0xF) as u32) << 7)
            | ((mode & 7) << 4)
            | (ws & 0xF)
    }

    fn sac_word(rounded: bool, acc_b: bool, wb: u32, slit: i32, mode: u32, wd: u32) -> u32 {
        (if rounded { 0x00CD_0000 } else { 0x00CC_0000 })
            | ((acc_b as u32) << 15)
            | ((wb & 0xF) << 11)
            | (((slit & 0xF) as u32) << 7)
            | ((mode & 7) << 4)
            | (wd & 0xF)
    }

    /// Every builder is checked against xc16-as output for the same
    /// mnemonic — the encodings are not transcribed by hand.
    #[test]
    fn dsp_words_match_xc16_as() {
        assert_eq!(
            mac_word(0, false, false, false, 0, 1, 3, 1, 2),
            0x00C0_04C6,
            "mac w4*w5,a,[w8]+=6,w4,[w10]+=2,w5"
        );
        assert_eq!(
            square_word(3, false, false, 0, 3, 4, 15),
            0x00F3_0D3C,
            "mac w7*w7,a,[w11]-=2,w7"
        );
        assert_eq!(
            mac_word(0, false, false, true, 2, 3, 1, 7, 3),
            0x00C0_2C5F,
            "mpy w4*w5,a,[w8]+=2,w6,[w10]-=2,w7"
        );
        assert_eq!(
            mac_word(0, false, true, true, 0, 1, 1, 1, 3),
            0x00C0_4447,
            "mpy.n w4*w5,a,[w8]+=2,w4,[w10]+=2,w5"
        );
        assert_eq!(
            mac_word(6, false, true, false, 2, 3, 6, 6, 2),
            0x00C6_6D9A,
            "msc w6*w7,a,[w8]-=4,w6,[w10]-=4,w7"
        );
        assert_eq!(
            mac_word(0, true, true, false, 0, 1, 4, 12, 0),
            0x00C0_C530,
            "msc w4*w5,b,[w11+w12],w5,w13"
        );
        assert_eq!(
            square_word(0, true, true, 0, 1, 12, 1),
            0x00F0_8705,
            "mpy w4*w4,b,[w9+w12],w4,[w10]+=2,w5"
        );
        assert_eq!(
            clr_word(false, 0, 0, 1, 4, 0),
            0x00C3_0050,
            "clr a,[w8]+=2,w4,w13"
        );
        assert_eq!(
            movsac_word(true, 2, 3, 8, 10, 0),
            0x00C7_AE28,
            "movsac b,[w9],w6,[w11]+=4,w7,w13"
        );
        assert_eq!(lac_word(true, 0, -3, 3, 4), 0x00CA_86B4, "lac [w4++],#-3,b");
        assert_eq!(lac_word(false, 0, 7, 4, 2), 0x00CA_03C2, "lac [--w2],#7,a");
        assert_eq!(
            sac_word(false, false, 0, 4, 0, 5),
            0x00CC_0205,
            "sac a,#4,w5"
        );
        assert_eq!(
            sac_word(false, true, 0, -4, 3, 5),
            0x00CC_8635,
            "sac b,#-4,[w5++]"
        );
        assert_eq!(
            sac_word(true, false, 0, 4, 0, 5),
            0x00CD_0205,
            "sac.r a,#4,w5"
        );
        assert_eq!(
            sac_word(true, true, 0, -4, 3, 5),
            0x00CD_8635,
            "sac.r b,#-4,[w5++]"
        );
        assert_eq!(
            square_word(0, false, true, 0, 0, 4, 4),
            0x00F0_0111,
            "mpy w4*w4,a"
        );
        assert_eq!(
            mac_word(0, false, false, false, 0, 0, 4, 4, 2),
            0x00C0_0112,
            "mac w4*w5,a"
        );
        assert_eq!(
            movsac_word(false, 0, 1, 1, 5, 1),
            0x00C7_0455,
            "movsac a,[w8]+=2,w4,[w10]-=6,w5,[w13]+=2"
        );
        assert_eq!(
            mac_word(6, true, false, false, 0, 0, 4, 4, 2),
            0x00C6_8112,
            "mac w6*w7,b"
        );
        assert_eq!(
            mac_word(0, true, true, false, 0, 0, 4, 4, 0),
            0x00C0_C110,
            "msc w4*w5,b,w13"
        );
    }

    #[test]
    fn dsp_mac_fractional_with_prefetches_matches_prm_example_1() {
        // DS70000157G MAC example 1: CORCON = 0x00C0 (fractional, normal
        // saturation), ACCA = 0x0012000000, W4=0xA022, W5=0xB900.
        let mut cpu = cpu_with(&[0x00C0_04C6]);
        cpu.corcon = 0x00C0;
        cpu.w[4] = 0xA022;
        cpu.w[5] = 0xB900;
        cpu.w[8] = 0x0A00;
        cpu.w[10] = 0x0C00;
        cpu.acc_a = 0x1200_0000;
        cpu.data_write(0x0A00, 0x2567).unwrap();
        cpu.data_write(0x0C00, 0x909C).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, 0x472D_2400, "ACCA = ACCA + W4*W5 in 1.31");
        assert_eq!(cpu.w[4], 0x2567, "X prefetch destination");
        assert_eq!(cpu.w[5], 0x909C, "Y prefetch destination");
        assert_eq!(cpu.w[8], 0x0A06, "X post-increment by 6");
        assert_eq!(cpu.w[10], 0x0C02, "Y post-increment by 2");
        assert_eq!(
            cpu.data_read(SFR_SR).unwrap() & (SR_OA | SR_OB | SR_SA | SR_SB),
            0,
            "no DSP flags"
        );
    }

    #[test]
    fn dsp_mac_square_super_saturation_matches_prm_example_2() {
        // DS70000157G square MAC example 2: CORCON = 0x00D0 (fractional,
        // super saturation), ACCA = 0xFE98344500, W7 = 0x76AE.
        let mut cpu = cpu_with(&[0x00F3_0D3C]);
        cpu.corcon = 0x00D0;
        cpu.w[7] = 0x76AE;
        cpu.w[11] = 0x0C00;
        cpu.acc_a = acc40(0xFE, 0x9834, 0x4500);
        cpu.data_write(0x0C00, 0x23FF).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.acc_a,
            acc40(0xFF, 0x063E, 0x0188),
            "9.31 result keeps the guard bits"
        );
        assert_eq!(cpu.w[7], 0x23FF, "Y prefetch");
        assert_eq!(cpu.w[11], 0x0BFE, "Y post-decrement by 2");
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_ne!(sr & SR_OA, 0, "guard-bit overflow sets OA");
        assert_eq!(sr & SR_OAB, SR_OAB, "OAB follows");
        assert_eq!(sr & (SR_SA | SR_SB | SR_SAB), 0, "not saturated");
    }

    #[test]
    fn dsp_mpy_fractional_matches_prm_example_1() {
        // DS70000157G MPY example 1: CORCON = 0 (fractional, no saturation);
        // the product replaces the accumulator.
        let mut cpu = cpu_with(&[0x00C0_2C5F]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0xC000;
        cpu.w[5] = 0x9000;
        cpu.w[8] = 0x0A00;
        cpu.w[10] = 0x0C00;
        cpu.acc_a = acc40(0xFF, 0xF780, 0x2087);
        cpu.data_write(0x0A00, 0x671F).unwrap();
        cpu.data_write(0x0C00, 0xE3DC).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, 0x3800_0000, "MPY stores the product");
        assert_eq!(cpu.w[6], 0x671F, "X prefetch destination W6");
        assert_eq!(cpu.w[7], 0xE3DC, "Y prefetch destination W7");
        assert_eq!(cpu.w[8], 0x0A02);
        assert_eq!(cpu.w[10], 0x0BFE);
        assert_eq!(cpu.data_read(SFR_SR).unwrap() & 0xF800, 0);
    }

    #[test]
    fn dsp_mpyn_integer_and_fractional_match_prm_examples() {
        // Integer mode (CORCON = 0x0001): the negated product replaces ACCA.
        let mut cpu = cpu_with(&[0x00C0_4447]);
        cpu.corcon = 0x0001;
        cpu.w[4] = 0x3023;
        cpu.w[5] = 0x1290;
        cpu.w[8] = 0x0A00;
        cpu.w[10] = 0x0C00;
        cpu.acc_a = 0x0000_2387;
        cpu.data_write(0x0A00, 0x0054).unwrap();
        cpu.data_write(0x0C00, 0x660A).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, acc40(0xFF, 0xFC82, 0x7650), "integer example 1");
        assert_eq!(cpu.w[4], 0x0054);
        assert_eq!(cpu.w[5], 0x660A);
        assert_eq!(cpu.w[8], 0x0A02);
        assert_eq!(cpu.w[10], 0x0C02);
        assert_eq!(cpu.data_read(SFR_SR).unwrap() & 0xF800, 0);

        // Fractional mode (CORCON = 0): the same word, left-shifted product.
        let mut cpu = cpu_with(&[0x00C0_4447]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0x3023;
        cpu.w[5] = 0x1290;
        cpu.acc_a = 0x0000_2387;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.acc_a,
            acc40(0xFF, 0xF904, 0xECA0),
            "fractional example 2"
        );
    }

    #[test]
    fn dsp_msc_integer_matches_prm_example_1() {
        // DS70000157G MSC example 1: CORCON = 0x0001 (integer).
        let mut cpu = cpu_with(&[0x00C6_6D9A]);
        cpu.corcon = 0x0001;
        cpu.w[6] = 0x9051;
        cpu.w[7] = 0x7230;
        cpu.w[8] = 0x0A00;
        cpu.w[10] = 0x0C00;
        cpu.acc_a = 0x0567_8000;
        cpu.data_write(0x0A00, 0xD309).unwrap();
        cpu.data_write(0x0C00, 0x100B).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, 0x3738_5ED0, "ACCA = ACCA - W6*W7");
        assert_eq!(cpu.w[6], 0xD309);
        assert_eq!(cpu.w[7], 0x100B);
        assert_eq!(cpu.w[8], 0x09FC, "post-decrement by 4");
        assert_eq!(cpu.w[10], 0x0BFC);
        assert_eq!(cpu.data_read(SFR_SR).unwrap() & 0xF800, 0);
    }

    #[test]
    fn dsp_msc_fractional_with_awb_matches_prm_example_2() {
        // DS70000157G MSC example 2: CORCON = 0 (fractional), [W11+W12]
        // prefetch and ACCA write-back to W13.
        let mut cpu = cpu_with(&[0x00C0_C530]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0x0500;
        cpu.w[5] = 0x2000;
        cpu.w[11] = 0x0B00;
        cpu.w[12] = 0x0100;
        cpu.w[13] = 0x6233;
        cpu.acc_a = 0x3738_5ED0;
        cpu.acc_b = 0x1000_0000;
        cpu.data_write(0x0C00, 0x3579).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_b, 0x0EC0_0000, "ACCB = ACCB - W4*W5");
        assert_eq!(cpu.w[5], 0x3579, "Y prefetch through W12 offset");
        assert_eq!(
            cpu.w[11], 0x0B00,
            "register-offset mode does not update W11"
        );
        assert_eq!(cpu.w[12], 0x0100, "nor W12");
        assert_eq!(cpu.w[13], 0x3738, "rounded ACCA written back to W13");
        assert_eq!(cpu.acc_a, 0x3738_5ED0, "write-back leaves ACCA alone");
    }

    #[test]
    fn dsp_mpy_square_with_register_offset_matches_prm_example_2() {
        // DS70000157G MPY square example 2: [W9+W12] with W12 = 0xFF00
        // (signed -256).
        let mut cpu = cpu_with(&[0x00F0_8705]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0xE228;
        cpu.w[9] = 0x0C00;
        cpu.w[12] = 0xFF00;
        cpu.w[10] = 0x0A00;
        cpu.acc_b = 0x9834_4500;
        cpu.data_write(0x0B00, 0x8911).unwrap();
        cpu.data_write(0x0A00, 0xF678).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_b, 0x06F5_4C80);
        assert_eq!(cpu.w[4], 0x8911, "X prefetch at W9 + (W12 as i16)");
        assert_eq!(cpu.w[5], 0xF678, "Y prefetch");
        assert_eq!(cpu.w[9], 0x0C00);
        assert_eq!(cpu.w[10], 0x0A02);
    }

    #[test]
    fn dsp_clr_matches_prm_example_1_and_clears_its_flags() {
        let mut cpu = cpu_with(&[0x00C3_0050]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0xF001;
        cpu.w[8] = 0x0A00;
        cpu.w[13] = 0xC623;
        cpu.acc_a = 0x0067_2345;
        cpu.acc_b = 0x5420_3BDD;
        cpu.sr |= SR_OA | SR_SA;
        cpu.refresh_dsp_status();
        cpu.data_write(0x0A00, 0x1221).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, 0, "ACCA cleared");
        assert_eq!(cpu.acc_b, 0x5420_3BDD, "ACCB untouched");
        assert_eq!(cpu.w[4], 0x1221, "X prefetch");
        assert_eq!(cpu.w[8], 0x0A02);
        assert_eq!(cpu.w[13], 0x5420, "ACCB rounded write-back");
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_eq!(
            sr & (SR_OA | SR_SA | SR_OAB | SR_SAB),
            0,
            "ACCA flags cleared"
        );
    }

    #[test]
    fn dsp_movsac_matches_prm_example_1() {
        let mut cpu = cpu_with(&[0x00C7_AE28]);
        cpu.corcon = 0x0000;
        cpu.w[6] = 0xA022;
        cpu.w[7] = 0xB200;
        cpu.w[9] = 0x0A00;
        cpu.w[11] = 0x0C00;
        cpu.w[13] = 0x0020;
        cpu.acc_a = 0x3290_5968;
        cpu.data_write(0x0A00, 0x7811).unwrap();
        cpu.data_write(0x0C00, 0xB2AF).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[6], 0x7811, "X prefetch");
        assert_eq!(cpu.w[7], 0xB2AF, "Y prefetch");
        assert_eq!(cpu.w[9], 0x0A00, "no update");
        assert_eq!(cpu.w[11], 0x0C04, "post-increment by 4");
        assert_eq!(cpu.w[13], 0x3290, "rounded ACCA write-back");
        assert_eq!(cpu.acc_a, 0x3290_5968, "MOVSAC does not change ACCA");
        assert_eq!(cpu.data_read(SFR_SR).unwrap(), 0, "status not affected");
    }

    #[test]
    fn dsp_lac_left_shift_matches_prm_example_1() {
        // DS70000157G LAC example 1: SATB = 0; the 32-bit shifter loses the
        // sign bit and the guard byte becomes the sign extension.
        let mut cpu = cpu_with(&[0x00CA_86B4]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0x0A00;
        cpu.acc_b = 0x5125_ABCD;
        cpu.data_write(0x0A00, 0x1221).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[4], 0x0A02, "post-increment");
        assert_eq!(cpu.acc_b, acc40(0xFF, 0x9108, 0x0000));
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_eq!(sr, 0x4800, "OB and OAB set (0x4000 | 0x0800)");
    }

    #[test]
    fn dsp_lac_right_shift_matches_prm_example_2() {
        let mut cpu = cpu_with(&[0x00CA_03C2]);
        cpu.corcon = 0x0000;
        cpu.w[2] = 0x0C02;
        cpu.acc_a = 0x5125_ABCD;
        cpu.data_write(0x0C00, 0x9108).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[2], 0x0C00, "pre-decrement");
        assert_eq!(cpu.acc_a, acc40(0xFF, 0xFF22, 0x1000));
        assert_eq!(
            cpu.data_read(SFR_SR).unwrap(),
            0,
            "a right shift never flags"
        );
    }

    #[test]
    fn dsp_sac_and_sacr_match_prm_examples() {
        // SAC example 1: SATDW = 1, truncated store. (The PRM example
        // writes CORCON = 0x0010, but DS70283H Register 3-2 puts SATDW at
        // bit 5 — 0x0010 is ACCSAT; the datasheet wins here.)
        let mut cpu = cpu_with(&[0x00CC_0205]);
        cpu.corcon = CORCON_SATDW;
        cpu.acc_a = 0x120F_FF00;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[5], 0x0120, "SAC stores ACCxH after >> 4");

        // SAC example 2: the shifted 40-bit value needs the guard bits, so
        // SATDW clamps the store to 0x8000.
        let mut cpu = cpu_with(&[0x00CC_8635]);
        cpu.corcon = CORCON_SATDW;
        cpu.acc_b = acc40(0xFF, 0xC891, 0x8F4C);
        cpu.w[5] = 0x0A00;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(0x0A00).unwrap(), 0x8000, "SATDW saturates");
        assert_eq!(cpu.w[5], 0x0A02);
        assert_eq!(
            cpu.acc_b,
            acc40(0xFF, 0xC891, 0x8F4C),
            "SAC never changes the accumulator"
        );

        // SAC.R example 1: rounding adds one to the high word.
        let mut cpu = cpu_with(&[0x00CD_0205]);
        cpu.corcon = CORCON_SATDW;
        cpu.acc_a = 0x120F_FF00;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[5], 0x0121, "SAC.R rounds");

        // SAC.R example 2: this value fits the signed 32-bit range, so it is
        // not saturated; the low word rounds 0xF4C0 up.
        let mut cpu = cpu_with(&[0x00CD_8635]);
        cpu.corcon = CORCON_SATDW;
        cpu.acc_b = acc40(0xFF, 0xF891, 0x8F4C);
        cpu.w[5] = 0x0C00;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(0x0C00).unwrap(), 0x8919);
        assert_eq!(cpu.w[5], 0x0C02);
    }

    #[test]
    fn dsp_saturation_modes_follow_corcon() {
        // MPY W4*W4, A with W4 = 0x8000 (-1.0): the fractional product is
        // exactly +2^31, outside the 1.31 range.
        let cases = [
            (
                0x0000u16,
                0x8000_0000i64,
                true,
                false,
                "disabled: guard bits",
            ),
            (
                CORCON_SATA,
                0x7FFF_FFFF,
                false,
                true,
                "normal: clamp to 1.31",
            ),
            (
                CORCON_SATA | CORCON_ACCSAT,
                0x8000_0000,
                true,
                false,
                "super: 9.31",
            ),
        ];
        for (corcon, expected, oa, sa, name) in cases {
            let mut cpu = cpu_with(&[0xF00111]);
            cpu.corcon = corcon;
            cpu.w[4] = 0x8000;
            let mut bus = SystemBus::new();
            step(&mut cpu, &mut bus);
            assert_eq!(cpu.acc_a, expected, "{name}");
            let sr = cpu.data_read(SFR_SR).unwrap();
            assert_eq!(sr & SR_OA != 0, oa, "{name}: OA");
            assert_eq!(sr & SR_SA != 0, sa, "{name}: SA");
            assert_eq!(
                sr & SR_OAB != 0,
                sr & (SR_OA | SR_OB) != 0,
                "{name}: OAB derived"
            );
        }
    }

    #[test]
    fn dsp_sa_sb_are_sticky_and_clearable_through_sr() {
        let mut cpu = cpu_with(&[0xF00111, 0xC00112]);
        cpu.corcon = CORCON_SATA;
        cpu.w[4] = 0x8000;
        cpu.w[5] = 0;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus); // saturates -> SA
        assert_ne!(cpu.data_read(SFR_SR).unwrap() & SR_SA, 0);
        step(&mut cpu, &mut bus); // non-saturating MAC
        assert_ne!(cpu.data_read(SFR_SR).unwrap() & SR_SA, 0, "SA is sticky");
        // Clearing SAB clears SA and SB.
        let sr = cpu.data_read(SFR_SR).unwrap();
        cpu.data_write(SFR_SR, sr & !SR_SAB).unwrap();
        assert_eq!(
            cpu.data_read(SFR_SR).unwrap() & (SR_SA | SR_SB | SR_SAB),
            0,
            "SAB clear clears the sticky bits"
        );
        // Writing SAB = 1 cannot set SA/SB (read-only).
        cpu.data_write(SFR_SR, SR_SAB).unwrap();
        assert_eq!(cpu.data_read(SFR_SR).unwrap() & (SR_SA | SR_SB | SR_SAB), 0);
    }

    #[test]
    fn dsp_individual_status_bits_clear_independently() {
        // Saturate both accumulators: MPY W4*W4, A and MPY W4*W4, B with
        // W4 = 0x8000 produce +2^31 (the 1.31 clamp).
        let mut cpu = cpu_with(&[0xF00111, 0xF08111]); // MPY sq A, MPY sq B
        cpu.corcon = CORCON_SATA | CORCON_SATB;
        cpu.w[4] = 0x8000;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_ne!(sr & SR_SA, 0, "ACCA saturated");
        assert_ne!(sr & SR_SB, 0, "ACCB saturated");

        // A read-modify-write that clears SA alone: the SAB bit reads back
        // as 1, and the write must still clear SA while keeping SB
        // (DS70000157G §2.16.4 Note 2).
        let sr = cpu.data_read(SFR_SR).unwrap();
        cpu.data_write(SFR_SR, (sr & !SR_SA) | SR_SB | SR_SAB)
            .unwrap();
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_eq!(sr & SR_SA, 0, "SA cleared on its own");
        assert_ne!(sr & SR_SB, 0, "SB kept");
        assert_ne!(sr & SR_SAB, 0, "SAB follows SB");

        // Clearing SB alone works the same way; a written 1 to SA cannot
        // set a flag that is 0.
        let sr = cpu.data_read(SFR_SR).unwrap();
        cpu.data_write(SFR_SR, (sr & !SR_SB) | SR_SA | SR_SAB)
            .unwrap();
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_eq!(sr & (SR_SA | SR_SB), 0);
        assert_eq!(sr & SR_SAB, 0);
    }

    #[test]
    fn dsp_clr_b_clears_only_accb_and_its_flags() {
        // CLR B, [W8]+=2, W4, W13 (xc16-as 0xC38050).
        let mut cpu = cpu_with(&[0x00C3_8050]);
        cpu.corcon = 0x0000;
        cpu.w[8] = 0x0A00;
        cpu.w[13] = 0x0020;
        cpu.acc_a = 0x3290_5968;
        cpu.acc_b = 0x1234_5678;
        cpu.sr |= SR_OA | SR_SA | SR_OB | SR_SB;
        cpu.refresh_dsp_status();
        cpu.data_write(0x0A00, 0x4321).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_b, 0, "ACCB cleared");
        assert_eq!(cpu.acc_a, 0x3290_5968, "ACCA untouched");
        assert_eq!(cpu.w[4], 0x4321, "X prefetch");
        assert_eq!(cpu.w[8], 0x0A02);
        assert_eq!(cpu.w[13], 0x3290, "rounded ACCA write-back");
        let sr = cpu.data_read(SFR_SR).unwrap();
        assert_eq!(sr & (SR_OB | SR_SB), 0, "ACCB flags cleared");
        assert_ne!(sr & (SR_OA | SR_SA), 0, "ACCA flags untouched");
        assert_ne!(sr & SR_OAB, 0, "OAB still follows OA");
        assert_ne!(sr & SR_SAB, 0, "SAB still follows SA");
    }

    #[test]
    fn dsp_lac_and_sac_register_offset_mode() {
        // lac [W4+W5], A (xc16-as 0xCA2864): EA = 0x0B00.
        let mut cpu = cpu_with(&[0x00CA_2864]);
        cpu.corcon = 0x0000;
        cpu.w[4] = 0x0A00;
        cpu.w[5] = 0x0100;
        cpu.data_write(0x0B00, 0x1234).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, 0x1234_0000);
        assert_eq!(cpu.w[4], 0x0A00, "register-offset mode does not update Ws");
        assert_eq!(cpu.w[5], 0x0100, "nor Wb");

        // sac A, [W5+W6] (xc16-as 0xCC3065): EA = 0x0D00.
        let mut cpu = cpu_with(&[0x00CC_3065]);
        cpu.corcon = 0x0000;
        cpu.acc_a = 0x1234_0000;
        cpu.w[5] = 0x0C00;
        cpu.w[6] = 0x0100;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(0x0D00).unwrap(), 0x1234);
        assert_eq!(cpu.w[5], 0x0C00, "register-offset mode does not update Wd");
        assert_eq!(cpu.w[6], 0x0100);
    }

    #[test]
    fn dsp_rounding_modes_differ_only_on_a_tie() {
        // ACCA = 0x0000_1000_8000: the low word is exactly 0x8000 and the
        // high word is even.
        for (rnd, expected, name) in [
            (0u16, 0x1000u16, "convergent rounds a tie to even"),
            (CORCON_RND, 0x1001, "biased rounds a tie up"),
        ] {
            let mut cpu = cpu_with(&[0x00CD_0005]); // SAC.R A, #0, W5
            cpu.corcon = rnd;
            cpu.acc_a = 0x1000_8000;
            let mut bus = SystemBus::new();
            step(&mut cpu, &mut bus);
            assert_eq!(cpu.w[5], expected, "{name}");
        }
    }

    #[test]
    fn dsp_unsigned_and_integer_modes_follow_corcon() {
        // MPY W4*W4, A: US = 1, integer mode -> 0xFFFF * 0xFFFF.
        let mut cpu = cpu_with(&[0xF00111]);
        cpu.corcon = CORCON_US | CORCON_IF;
        cpu.w[4] = 0xFFFF;
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.acc_a, 0xFFFE_0001, "unsigned 16x16 product");
        assert_ne!(
            cpu.data_read(SFR_SR).unwrap() & SR_OA,
            0,
            "the product needs the guard bits"
        );
    }

    #[test]
    fn dsp_awb_indirect_post_increment() {
        // MOVSAC A, [W8]+=2, W4, [W10]-=6, W5, [W13]+=2 stores the other
        // accumulator (B) through the pointer in W13.
        let mut cpu = cpu_with(&[0x00C7_0455]);
        cpu.corcon = 0x0000;
        cpu.w[8] = 0x0A00;
        cpu.w[10] = 0x0B00;
        cpu.w[13] = 0x0900;
        cpu.acc_b = 0x2000_0000;
        cpu.data_write(0x0A00, 0x1111).unwrap();
        cpu.data_write(0x0B00, 0x2222).unwrap();
        cpu.data_write(0x0900, 0xFFFF).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(0x0900).unwrap(), 0x2000, "ACCB written back");
        assert_eq!(cpu.w[13], 0x0902, "pointer post-incremented");
        assert_eq!(cpu.w[4], 0x1111);
        assert_eq!(cpu.w[5], 0x2222);
        assert_eq!(cpu.w[10], 0x0AFA, "post-decrement by 6");
    }

    #[test]
    fn dsp_prefetch_covers_every_x_and_y_operation() {
        // (xop, base register, expected delta)
        let cases = [
            (0u32, 8usize, 0i32),
            (1, 8, 2),
            (2, 8, 4),
            (3, 8, 6),
            (5, 8, -6),
            (6, 8, -4),
            (7, 8, -2),
            (8, 9, 0),
            (9, 9, 2),
            (10, 9, 4),
            (11, 9, 6),
            (13, 9, -6),
            (14, 9, -4),
            (15, 9, -2),
        ];
        for (xop, base, delta) in cases {
            let word = movsac_word(false, 0, 0, xop, 0b0100, 2);
            let mut cpu = cpu_with(&[word]);
            let start = 0x0A00u16;
            cpu.w[base] = start;
            cpu.data_write(start, 0xBEEF).unwrap();
            let mut bus = SystemBus::new();
            step(&mut cpu, &mut bus);
            assert_eq!(cpu.w[4], 0xBEEF, "xop {xop:#x} reads [W{base}]");
            assert_eq!(
                cpu.w[base],
                (i32::from(start) + delta) as u16,
                "xop {xop:#x} updates W{base}"
            );
        }
        // Y operations use W10/W11.
        let y_cases = [
            (0u32, 10usize, 0i32),
            (1, 10, 2),
            (2, 10, 4),
            (3, 10, 6),
            (5, 10, -6),
            (6, 10, -4),
            (7, 10, -2),
            (8, 11, 0),
            (9, 11, 2),
            (10, 11, 4),
            (11, 11, 6),
            (13, 11, -6),
            (14, 11, -4),
            (15, 11, -2),
        ];
        for (yop, base, delta) in y_cases {
            let word = movsac_word(false, 0, 0, 0b0100, yop, 2);
            let mut cpu = cpu_with(&[word]);
            let start = 0x0C00u16;
            cpu.w[base] = start;
            cpu.data_write(start, 0xCAFE).unwrap();
            let mut bus = SystemBus::new();
            step(&mut cpu, &mut bus);
            assert_eq!(cpu.w[4], 0xCAFE, "yop {yop:#x} reads [W{base}]");
            assert_eq!(
                cpu.w[base],
                (i32::from(start) + delta) as u16,
                "yop {yop:#x} updates W{base}"
            );
        }
        // The W12 register-offset mode uses a signed 16-bit offset and does
        // not update either register.
        let mut cpu = cpu_with(&[movsac_word(false, 0, 0, 0b1100, 0b0100, 2)]);
        cpu.w[9] = 0x0A00;
        cpu.w[12] = 0xFF00;
        cpu.data_write(0x0900, 0x1234).unwrap();
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[4], 0x1234);
        assert_eq!(cpu.w[9], 0x0A00);
        assert_eq!(cpu.w[12], 0xFF00);
    }

    // ── M2: Motor Control PWM + QEI (DS70187E / DS70208C) ──────────────────

    /// Configure PWM1 for an edge-aligned complementary pair 1 (PEN1H|PEN1L)
    /// with the given period and duty, then enable the time base at 1:1.
    fn pwm_edge_setup(cpu: &mut Pic24, tper: u16, dc: u16) {
        write_sfr(cpu, SFR_P1TPER, tper);
        write_sfr(cpu, SFR_P1DC1, dc);
        write_sfr(cpu, SFR_PWM1CON1, 0x0011);
        write_sfr(cpu, SFR_P1TCON, PTCON_PTEN);
    }

    #[test]
    fn pwm_edge_aligned_complementary_duty_and_dead_time() {
        let mut cpu = cpu_with(&[0x0000_0000; 16]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_P1DTCON1, 2); // DTA = 2 TCY, prescale 1:1
        write_sfr(&mut cpu, SFR_P1DTCON2, 0); // both edges use unit A
        pwm_edge_setup(&mut cpu, 7, 3);
        // PTEN drives the edge-aligned generator active; the 2-cycle dead
        // time gates the high-side pin.
        step(&mut cpu, &mut bus); // count 1, DT 1
        assert!(!bus.pad(1, 14) && !bus.pad(1, 15), "both off during DT");
        step(&mut cpu, &mut bus); // count 2, DT 0
        assert!(bus.pad(1, 14), "PWM1H1 active after the dead time");
        assert!(!bus.pad(1, 15), "complementary low side stays off");
        // Duty match at TMR = 3: the high side goes inactive immediately,
        // the low side waits its dead time.
        step(&mut cpu, &mut bus); // count 3
        assert!(!bus.pad(1, 14) && !bus.pad(1, 15), "dead time on the fall");
        step(&mut cpu, &mut bus); // count 4, DT 1
        step(&mut cpu, &mut bus); // count 5, DT 0
        assert!(!bus.pad(1, 14), "high side off after the match");
        assert!(bus.pad(1, 15), "low side active after the dead time");
        // 6, 7, then the period match resets TMR and starts a new pulse.
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus); // reset to 0, DT re-armed
        assert_ne!(
            cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_PWM1.0),
            0,
            "PWM1IF on the period match"
        );
        assert!(!bus.pad(1, 14) && !bus.pad(1, 15));
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert!(bus.pad(1, 14), "next period's high side");
        assert_eq!(cpu.data_read(SFR_P1TMR).unwrap() & 0x7FFF, 2);
        assert_eq!(
            cpu.data_read(SFR_P1TMR).unwrap() & 0x8000,
            0,
            "PTDIR clear while counting up"
        );
    }

    #[test]
    fn pwm_output_postscaler_divides_the_interrupt_rate() {
        let mut cpu = cpu_with(&[0x0000_0000; 16]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_P1TPER, 1);
        write_sfr(&mut cpu, SFR_PWM1CON1, 0x0011);
        // PTEN | PTOPS = 1 → 1:2 postscale.
        write_sfr(&mut cpu, SFR_P1TCON, PTCON_PTEN | 0x0010);
        step(&mut cpu, &mut bus); // count 1
        step(&mut cpu, &mut bus); // reset, event 1/2
        assert_eq!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_PWM1.0), 0);
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus); // reset, event 2/2 → flag
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_PWM1.0), 0);
    }

    #[test]
    fn pwm_center_aligned_follows_the_compare_rule() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_P1TPER, 3);
        write_sfr(&mut cpu, SFR_P1DC1, 1);
        write_sfr(&mut cpu, SFR_PWM1CON1, 0x0011);
        write_sfr(&mut cpu, SFR_P1TCON, PTCON_PTEN | 2); // up/down
                                                         // 0→1 (up compare at 0), 1→2 (match at 1 → inactive).
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert!(!bus.pad(1, 14), "inactive after the up-count match");
        // 2→3, 3 (direction flip), 3→2, 2→1, then the down-count match at
        // 1 drives the output active.
        for _ in 0..5 {
            step(&mut cpu, &mut bus);
        }
        assert!(bus.pad(1, 14), "active on the down-count match");
        // Ticks 8 (zero, flag), 9 (up at 0) stay active; the next up match
        // at 1 turns it off.
        step(&mut cpu, &mut bus); // zero boundary + PWM1IF
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_PWM1.0), 0);
        step(&mut cpu, &mut bus);
        assert!(bus.pad(1, 14));
        step(&mut cpu, &mut bus);
        assert!(!bus.pad(1, 14), "inactive again at the up match");
    }

    #[test]
    fn pwm_override_and_fault_states() {
        let mut cpu = cpu_with(&[0x0000_0000; 16]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_RPINR12, 0); // FLTA1 = RP0/RB0
        pwm_edge_setup(&mut cpu, 7, 3);
        step(&mut cpu, &mut bus);
        assert!(bus.pad(1, 14), "normal PWM first");
        // Override pair 1: high follows POUT1H = 0 (inactive), low follows
        // POUT1L = 1 (active) because both POVD bits are cleared.
        write_sfr(&mut cpu, SFR_P1OVDCON, 0x3C01);
        step(&mut cpu, &mut bus);
        assert!(!bus.pad(1, 14), "override drives the high pin inactive");
        assert!(bus.pad(1, 15), "override drives the low pin active");
        // Fault A (active low), cycle-by-cycle, both pins inactive. Give the
        // pin a high (inactive) level first so the fault is a real edge.
        write_sfr(&mut cpu, SFR_P1OVDCON, 0x3F00);
        bus.set_input(1, 0, true);
        step(&mut cpu, &mut bus);
        write_sfr(&mut cpu, SFR_P1FLTACON, 0x0081); // FLTAM = 1, FAEN1
        bus.set_input(1, 0, false);
        step(&mut cpu, &mut bus);
        assert!(!bus.pad(1, 14) && !bus.pad(1, 15), "fault state");
        assert_ne!(
            cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_FLTA1.0),
            0,
            "FLTA1IF"
        );
        // Release: the outputs stay in the fault state until the period
        // boundary, then the generator resumes.
        bus.set_input(1, 0, true);
        step(&mut cpu, &mut bus);
        assert!(
            !bus.pad(1, 14) && !bus.pad(1, 15),
            "held until the boundary"
        );
        for _ in 0..8 {
            step(&mut cpu, &mut bus);
            if bus.pad(1, 14) {
                break;
            }
        }
        assert!(bus.pad(1, 14), "restored at the next period");
    }

    #[test]
    fn pwm_single_event_stops_at_the_period_match() {
        let mut cpu = cpu_with(&[0x0000_0000; 16]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_P1TPER, 3);
        write_sfr(&mut cpu, SFR_P1DC1, 1);
        write_sfr(&mut cpu, SFR_PWM1CON1, 0x0011);
        write_sfr(&mut cpu, SFR_P1TCON, PTCON_PTEN | 1); // single event
        for _ in 0..4 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(
            cpu.data_read(SFR_P1TCON).unwrap() & PTCON_PTEN,
            0,
            "the period match clears PTEN"
        );
        assert!(!bus.pad(1, 14) && !bus.pad(1, 15), "outputs stop inactive");
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_PWM1.0), 0);
    }

    #[test]
    fn m2_interrupt_vectors_and_ipc_mapping() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        // PWM1: IRQ 57 → IPC14 field 1 (bits 6-4), vector 65.
        write_sfr(&mut cpu, SFR_IPC14, 6 << 4);
        write_sfr(&mut cpu, SFR_IEC3, 1 << IRQ_PWM1.0);
        write_sfr(&mut cpu, SFR_IFS3, 1 << IRQ_PWM1.0);
        assert_eq!(cpu.pending_interrupt(), Some((65, 6)));
        // QEI: IRQ 58 → IPC14 field 2 (bits 10-8), vector 66.
        write_sfr(&mut cpu, SFR_IFS3, 1 << IRQ_QEI.0);
        write_sfr(&mut cpu, SFR_IEC3, 1 << IRQ_QEI.0);
        write_sfr(&mut cpu, SFR_IPC14, 7 << 8);
        assert_eq!(cpu.pending_interrupt(), Some((66, 7)));
        // FLTA1: IRQ 63 → IPC15 field 3 (bits 14-12), vector 71.
        write_sfr(&mut cpu, SFR_IFS3, 1 << IRQ_FLTA1.0);
        write_sfr(&mut cpu, SFR_IEC3, 1 << IRQ_FLTA1.0);
        write_sfr(&mut cpu, SFR_IPC15, 5 << 12);
        assert_eq!(cpu.pending_interrupt(), Some((71, 5)));
        // Equal priority: the lower vector number wins.
        write_sfr(&mut cpu, SFR_IFS3, (1 << IRQ_QEI.0) | (1 << IRQ_FLTA1.0));
        write_sfr(&mut cpu, SFR_IEC3, (1 << IRQ_QEI.0) | (1 << IRQ_FLTA1.0));
        write_sfr(&mut cpu, SFR_IPC14, 5 << 8);
        assert_eq!(cpu.pending_interrupt(), Some((66, 5)));
        // IFS0 sources keep their datasheet vectors — the M1a table was off
        // by one for IC2/OC2/T2/T3 (DS70283H Table 7-1).
        for (bit, ipc_reg, shift, vector) in [
            (IRQ_IC2.0, SFR_IPC1, 4u16, 13u32),
            (IRQ_OC2.0, SFR_IPC1, 8, 14),
            (IRQ_T2.0, SFR_IPC1, 12, 15),
            (IRQ_T3.0, SFR_IPC2, 0, 16),
        ] {
            let mut c = cpu_with(&[0x0000_0000; 2]);
            write_sfr(&mut c, ipc_reg, 3 << shift);
            write_sfr(&mut c, SFR_IEC0, 1 << bit);
            write_sfr(&mut c, SFR_IFS0, 1 << bit);
            assert_eq!(c.pending_interrupt(), Some((vector, 3)));
        }
    }

    /// QEI: QEA on RP0/RB0, QEB on RP1/RB1, INDX on RP2/RB2.
    fn qei_setup(cpu: &mut Pic24, mode: u16, max: u16) {
        write_sfr(cpu, SFR_RPINR14, 0x0100);
        write_sfr(cpu, SFR_RPINR15, 2);
        write_sfr(cpu, SFR_MAX1CNT, max);
        write_sfr(cpu, SFR_QEI1CON, (mode << 8) | QEICON_POSRES);
    }

    fn qei_edge(cpu: &mut Pic24, bus: &mut MirrorBus, a: bool, b: bool) {
        bus.set_input(1, 0, a);
        bus.set_input(1, 1, b);
        step(cpu, bus);
    }

    #[test]
    fn qei_x4_counts_forward_and_reverse() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 6, 100); // x4, reset on index
        for (a, b) in [(true, false), (true, true), (false, true), (false, false)] {
            qei_edge(&mut cpu, &mut bus, a, b);
        }
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 4);
        assert_ne!(cpu.data_read(SFR_QEI1CON).unwrap() & QEICON_UPDN, 0);
        for (a, b) in [(false, true), (true, true)] {
            qei_edge(&mut cpu, &mut bus, a, b);
        }
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 2);
        assert_eq!(cpu.data_read(SFR_QEI1CON).unwrap() & QEICON_UPDN, 0);
    }

    #[test]
    fn qei_x2_counts_only_on_phase_a_edges() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 4, 100); // x2
        qei_edge(&mut cpu, &mut bus, true, false); // A edge → +1
        qei_edge(&mut cpu, &mut bus, true, true); // B edge → no count
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 1);
        qei_edge(&mut cpu, &mut bus, false, true); // A edge → +1
        qei_edge(&mut cpu, &mut bus, false, false); // B edge → no count
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 2);
    }

    #[test]
    fn qei_match_reset_rolls_over_and_under() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 7, 3); // x4, reset on match, MAXCNT = 3
        for (a, b) in [(true, false), (true, true), (false, true)] {
            qei_edge(&mut cpu, &mut bus, a, b);
        }
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 3);
        qei_edge(&mut cpu, &mut bus, false, false); // POSCNT == MAX → 0
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 0);
        assert_ne!(
            cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0),
            0,
            "rollover interrupt"
        );
        write_sfr(&mut cpu, SFR_IFS3, 0);
        qei_edge(&mut cpu, &mut bus, false, true); // POSCNT == 0 → MAX
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 3);
        assert_ne!(
            cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0),
            0,
            "underflow interrupt"
        );
    }

    #[test]
    fn qei_index_reset_uses_the_imv_and_direction() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 6, 100); // x4, reset on index
        write_sfr(&mut cpu, SFR_DFLT1CON, 2 << 9); // IMV: A = 0, B = 1
                                                   // Forward to state 01 (A = 0, B = 1), POSCNT = 3.
        for (a, b) in [(true, false), (true, true), (false, true)] {
            qei_edge(&mut cpu, &mut bus, a, b);
        }
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 3);
        bus.set_input(1, 2, true); // INDX rising at the matching state
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 0, "forward → 0");
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
        // Reverse: state 01 → 11 (A rises), POSCNT 10 → 9, UPDN clears.
        write_sfr(&mut cpu, SFR_IFS3, 0);
        write_sfr(&mut cpu, SFR_POS1CNT, 10);
        qei_edge(&mut cpu, &mut bus, true, true);
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 9);
        assert_eq!(cpu.data_read(SFR_QEI1CON).unwrap() & QEICON_UPDN, 0);
        // An index pulse at state 11 does not match IMV = B1/A0: no reset and
        // no interrupt.
        bus.set_input(1, 2, false);
        step(&mut cpu, &mut bus);
        bus.set_input(1, 2, true);
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.data_read(SFR_POS1CNT).unwrap(),
            9,
            "index at a non-matching state is ignored"
        );
        assert_eq!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
        // Reverse to state 00: 11 → 10 → 00, POSCNT 8 → 7.
        qei_edge(&mut cpu, &mut bus, true, false);
        qei_edge(&mut cpu, &mut bus, false, false);
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 7);
        // IMV = 00, reverse index: POSCNT loads MAXCNT.
        write_sfr(&mut cpu, SFR_DFLT1CON, 0);
        bus.set_input(1, 2, false);
        step(&mut cpu, &mut bus);
        bus.set_input(1, 2, true);
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.data_read(SFR_POS1CNT).unwrap(),
            100,
            "reverse index loads MAXCNT"
        );
    }

    #[test]
    fn qei_count_error_and_ceid() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 4, 3); // x2, index mode, MAXCNT = 3
                                   // Four Phase A edges: POSCNT reaches MAXCNT + 1 = 4.
        for (a, b) in [
            (true, false),
            (true, true),
            (false, true),
            (false, false),
            (true, false),
            (true, true),
            (false, true),
            (false, false),
        ] {
            qei_edge(&mut cpu, &mut bus, a, b);
        }
        let con = cpu.data_read(SFR_QEI1CON).unwrap();
        assert_ne!(con & QEICON_CNTERR, 0, "CNTERR at MAXCNT + 1");
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
        // Further errors do not re-interrupt until CNTERR is cleared.
        write_sfr(&mut cpu, SFR_IFS3, 0);
        qei_edge(&mut cpu, &mut bus, true, false);
        assert_eq!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
        // CEID still sets CNTERR but suppresses the interrupt.
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 4, 3);
        write_sfr(&mut cpu, SFR_DFLT1CON, DFLT_CEID);
        for (a, b) in [
            (true, false),
            (true, true),
            (false, true),
            (false, false),
            (true, false),
            (true, true),
            (false, true),
            (false, false),
        ] {
            qei_edge(&mut cpu, &mut bus, a, b);
        }
        assert_ne!(cpu.data_read(SFR_QEI1CON).unwrap() & QEICON_CNTERR, 0);
        assert_eq!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
    }

    #[test]
    fn qei_timer_mode_up_and_down() {
        let mut cpu = cpu_with(&[0x0000_0000; 16]);
        let mut bus = MirrorBus::new();
        write_sfr(&mut cpu, SFR_MAX1CNT, 2);
        // QEIM = 001, internal clock, UPDN = 1 (up).
        write_sfr(&mut cpu, SFR_QEI1CON, 0x0100 | QEICON_UPDN);
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 2);
        step(&mut cpu, &mut bus); // 3 > MAXCNT → 0 + QEIIF
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 0);
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
        // Down-count: start at 0xFFFF, MAXCNT = 0xFFFE → one tick matches.
        write_sfr(&mut cpu, SFR_IFS3, 0);
        write_sfr(&mut cpu, SFR_POS1CNT, 0xFFFF);
        write_sfr(&mut cpu, SFR_MAX1CNT, 0xFFFE);
        write_sfr(&mut cpu, SFR_QEI1CON, 0x0100); // UPDN = 0 (down)
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.data_read(SFR_POS1CNT).unwrap(), 0);
        assert_ne!(cpu.data_read(SFR_IFS3).unwrap() & (1 << IRQ_QEI.0), 0);
    }

    #[test]
    fn qei_digital_filter_rejects_short_pulses() {
        let mut cpu = cpu_with(&[0x0000_0000; 32]);
        let mut bus = MirrorBus::new();
        qei_setup(&mut cpu, 6, 100);
        write_sfr(&mut cpu, SFR_DFLT1CON, DFLT_QEOUT); // 1:1 filter clock
                                                       // A one-cycle high pulse is rejected (needs three equal samples).
        bus.set_input(1, 2, true);
        step(&mut cpu, &mut bus);
        assert_eq!(
            cpu.data_read(SFR_QEI1CON).unwrap() & QEICON_INDEX,
            0,
            "single-sample pulse rejected"
        );
        bus.set_input(1, 2, false);
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        // Three consecutive highs pass the filter.
        for _ in 0..3 {
            bus.set_input(1, 2, true);
            step(&mut cpu, &mut bus);
        }
        assert_ne!(
            cpu.data_read(SFR_QEI1CON).unwrap() & QEICON_INDEX,
            0,
            "three equal samples are accepted"
        );
    }

    // ── M3: ADC1 ────────────────────────────────────────────────────────────

    /// Attach AN4 (RB2) and return a CPU with an idle NOP program.
    fn adc_cpu(form: u16, twelve: bool) -> Pic24 {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.ad1con1 = (if twelve { AD1CON1_AD12B } else { 0 }) | (form << 8);
        cpu.ad1con2 = 0;
        cpu.ad1con3 = 1; // TAD = 2 TCY
        cpu.ad1chs0 = 4; // CH0SA = AN4
        cpu.ad1pcfgl = 0x01FF & !(1 << 4);
        cpu.set_adc_analog(4, 0x2AA);
        cpu
    }

    fn adc_convert(cpu: &mut Pic24) {
        // ADON 0 -> 1 is the hardware "start sampling" edge; then software
        // clears SAMP (SSRC = 000) to start the conversion.
        cpu.ad1con1 &= !AD1CON1_ADON;
        let on = cpu.ad1con1 | AD1CON1_ADON;
        cpu.data_write(SFR_AD1CON1, on).unwrap();
        // ASAM = 0: software sets SAMP to begin sampling.
        cpu.data_write(SFR_AD1CON1, cpu.ad1con1 | AD1CON1_SAMP)
            .unwrap();
        cpu.data_write(SFR_AD1CON1, cpu.ad1con1 & !AD1CON1_SAMP)
            .unwrap();
        // 10-bit: 12 TAD; 12-bit: 14 TAD; TAD = 2 cycles. A CHPS sequence
        // converts each selected channel back-to-back, one conversion at a
        // time, so advance once per channel.
        let tconv = if cpu.ad1con1 & AD1CON1_AD12B != 0 {
            14
        } else {
            12
        };
        for _ in 0..cpu.adc_channels_per_sample() {
            cpu.advance_adc(tconv * 2);
        }
    }

    #[test]
    fn adc_manual_conversion_formats_all_four_modes() {
        let expected = [0x02AAu16, 0x00AA, 0xAA80, 0x2A80];
        for form in 0..4u16 {
            let mut cpu = adc_cpu(form, false);
            adc_convert(&mut cpu);
            assert_eq!(cpu.ad1con1 & AD1CON1_DONE, AD1CON1_DONE);
            assert_eq!(
                cpu.data_read(SFR_ADC1BUF0).unwrap(),
                expected[form as usize],
                "FORM {form}"
            );
            assert_ne!(cpu.ifs0 & (1 << IRQ_AD1.0), 0, "AD1IF (SMPI = 0)");
        }
    }

    #[test]
    fn adc_twelve_bit_and_signed_formats() {
        let mut cpu = adc_cpu(3, true); // signed fractional
        cpu.set_adc_analog(4, 0x800);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x0000);

        let mut cpu = adc_cpu(2, true); // unsigned fractional
        cpu.set_adc_analog(4, 0xFFF);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0xFFF0);

        let mut cpu = adc_cpu(1, true); // signed integer
        cpu.set_adc_analog(4, 0xFFF);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x07FF);
    }

    #[test]
    fn adc_auto_sample_internal_counter_and_asam() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con1 |= 0b111 << 5 | 0x0004; // SSRC = 111, ASAM = 1
        cpu.ad1con3 = 1 | (2 << 8); // TAD = 2, SAMC = 2 (4 cycles)
        cpu.ad1con1 |= AD1CON1_ADON;
        cpu.adc_start_sampling();
        assert_eq!(cpu.adc_state, AdcState::Sampling);
        cpu.advance_adc(4);
        assert_eq!(cpu.adc_state, AdcState::Converting);
        cpu.advance_adc(12 * 2);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x02AA);
        assert_ne!(cpu.ad1con1 & AD1CON1_SAMP, 0, "ASAM restarts sampling");
        assert_eq!(cpu.adc_state, AdcState::Sampling);
        // Second sequence lands in the next buffer word.
        cpu.advance_adc(4);
        cpu.advance_adc(12 * 2);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 2).unwrap(), 0x02AA);
    }

    #[test]
    fn adc_scan_list_and_smpi_postscaler() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con2 = AD1CON2_CSCNA | (2 << 2); // scan, SMPI = 2 (every 3)
        cpu.ad1cssl = (1 << 0) | (1 << 2) | (1 << 4);
        cpu.ad1pcfgl = 0x01FF & !((1 << 0) | (1 << 2) | (1 << 4));
        cpu.set_adc_analog(0, 0x100);
        cpu.set_adc_analog(2, 0x200);
        cpu.set_adc_analog(4, 0x300);
        for _ in 0..3 {
            adc_convert(&mut cpu);
        }
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x0100);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 2).unwrap(), 0x0200);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 4).unwrap(), 0x0300);
        assert_ne!(cpu.ifs0 & (1 << IRQ_AD1.0), 0, "interrupt after 3");
        assert_eq!(cpu.adc_scan_channel(), 0, "scan wraps to AN0");
    }

    #[test]
    fn adc_split_buffer_mode_toggles_halves() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con2 = AD1CON2_BUFM | (7 << 2); // interrupt after 8
        for _ in 0..8 {
            adc_convert(&mut cpu);
        }
        for i in 0..8 {
            assert_eq!(
                cpu.data_read(SFR_ADC1BUF0 + 2 * i).unwrap(),
                0x02AA,
                "lower half {i}"
            );
        }
        assert_ne!(cpu.ifs0 & (1 << IRQ_AD1.0), 0);
        assert_eq!(
            cpu.data_read(SFR_AD1CON2).unwrap() & AD1CON2_BUFS,
            AD1CON2_BUFS,
            "BUFS reports the upper half"
        );
        adc_convert(&mut cpu);
        assert_eq!(
            cpu.data_read(SFR_ADC1BUF0 + 16).unwrap(),
            0x02AA,
            "first result of the upper half"
        );
    }

    #[test]
    fn adc_chps_selects_an3_and_subtracts_the_selected_negative_input() {
        // CHPS = 01 (two channels), CH123SA = 1 -> CH1 = AN3.
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con2 = 1 << 8;
        cpu.ad1chs0 = 0; // CH0 = AN0
        cpu.ad1chs123 = 0b1; // CH123SA = 1 (bit 0)
        cpu.ad1pcfgl = 0x01FF & !((1 << 0) | (1 << 3) | (1 << 6));
        cpu.set_adc_analog(0, 0x100);
        cpu.set_adc_analog(3, 0x200);
        cpu.set_adc_analog(6, 0x080);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x0100);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 2).unwrap(), 0x0200);

        // CH123NA = 10 -> CH1..3 negative input is AN6/7/8 (10-bit mode).
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con2 = 1 << 8;
        cpu.ad1chs0 = 0;
        cpu.ad1chs123 = 0b1 | (0b10 << 1);
        cpu.ad1pcfgl = 0x01FF & !((1 << 0) | (1 << 3) | (1 << 6));
        cpu.set_adc_analog(0, 0x100);
        cpu.set_adc_analog(3, 0x200);
        cpu.set_adc_analog(6, 0x080);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x0100);
        assert_eq!(
            cpu.data_read(SFR_ADC1BUF0 + 2).unwrap(),
            0x0180,
            "AN3 - AN6"
        );
    }

    #[test]
    fn adc_chps_four_channels_converts_ch0_through_ch3() {
        // CHPS = 1x means CH0-CH3 (DS70183D Table 16-6), not three channels.
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con2 = 2 << 8;
        cpu.ad1chs0 = 4; // CH0 = AN4
        cpu.ad1chs123 = 0; // CH1/2/3 = AN0/1/2
        cpu.ad1pcfgl = 0x01FF & !((1 << 4) | (1 << 0) | (1 << 1) | (1 << 2));
        cpu.set_adc_analog(4, 0x2AA);
        cpu.set_adc_analog(0, 0x100);
        cpu.set_adc_analog(1, 0x200);
        cpu.set_adc_analog(2, 0x300);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x02AA);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 2).unwrap(), 0x0100);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 4).unwrap(), 0x0200);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 6).unwrap(), 0x0300);
    }

    #[test]
    fn adc_alts_uses_mux_a_first_and_then_mux_b() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con2 = AD1CON2_ALTS;
        cpu.ad1chs0 = 2 << 8; // CH0SA = AN0, CH0SB = AN2
        cpu.ad1pcfgl = 0x01FF & !((1 << 0) | (1 << 2));
        cpu.set_adc_analog(0, 0x111);
        cpu.set_adc_analog(2, 0x222);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x0111);
        adc_convert(&mut cpu);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0 + 2).unwrap(), 0x0222);
    }

    #[test]
    fn adc_manual_mode_needs_a_software_sample_and_drops_stale_triggers() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con1 &= !AD1CON1_ADON;
        cpu.data_write(SFR_AD1CON1, cpu.ad1con1 | AD1CON1_ADON)
            .unwrap();
        assert_eq!(cpu.ad1con1 & AD1CON1_SAMP, 0, "ASAM = 0: no auto-sample");
        assert_eq!(cpu.adc_state, AdcState::Off);

        // A TMR3 match while the ADC was off must not fire on the next start.
        cpu.adc_t3_trigger = true;
        cpu.data_write(SFR_AD1CON1, cpu.ad1con1 | AD1CON1_SAMP)
            .unwrap();
        assert_eq!(cpu.adc_state, AdcState::Sampling);
        assert!(!cpu.adc_t3_trigger);
    }

    #[test]
    fn adc_pwm_trigger_starts_the_conversion() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con1 |= 0b011 << 5; // SSRC = 011 (PWM1 special event)
        cpu.ad1con1 |= AD1CON1_ADON;
        cpu.adc_start_sampling();
        cpu.adc_pwm_trigger = true;
        cpu.advance_adc(1);
        assert_eq!(cpu.adc_state, AdcState::Converting);
        cpu.advance_adc(12 * 2);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x02AA);
    }

    #[test]
    fn adc_timer3_trigger_starts_the_conversion() {
        let mut cpu = adc_cpu(0, false);
        cpu.ad1con1 |= 0b010 << 5; // SSRC = 010 (TMR3 compare)
        cpu.ad1con1 |= AD1CON1_ADON;
        cpu.adc_start_sampling();
        cpu.pr3 = 1;
        cpu.t3con = TCON_TON;
        cpu.advance_timers(4);
        assert!(cpu.adc_t3_trigger);
        cpu.advance_adc(1);
        assert_eq!(cpu.adc_state, AdcState::Converting);
        cpu.advance_adc(12 * 2);
        assert_eq!(cpu.data_read(SFR_ADC1BUF0).unwrap(), 0x02AA);
    }

    #[test]
    fn adc_interrupt_is_vector_21_on_ipc3() {
        let mut cpu = adc_cpu(0, false);
        cpu.ifs0 |= 1 << IRQ_AD1.0;
        cpu.iec0 |= 1 << IRQ_AD1.0;
        cpu.ipc3 = 7 << 4; // AD1IP = 111 (bits 6:4)
        assert_eq!(cpu.pending_interrupt(), Some((21, 7)));
        cpu.ipc3 = 5 << 4;
        assert_eq!(cpu.pending_interrupt(), Some((21, 5)));
        cpu.iec0 = 0;
        assert_eq!(cpu.pending_interrupt(), None);
    }

    // ── M3: SPI1 ────────────────────────────────────────────────────────────

    struct SpiLoopback;

    impl crate::peripherals::device::SpiDevice for SpiLoopback {
        fn transfer(&mut self, mosi: u8) -> u8 {
            mosi
        }
        fn cs_pin(&self) -> &str {
            "loopback"
        }
    }

    fn spi_cpu() -> Pic24 {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.data_write(SFR_SPI1CON1, SPI1CON1_MSTEN | 0x1C | 0x03)
            .unwrap();
        cpu.data_write(SFR_SPI1STAT, SPI1STAT_SPIEN).unwrap();
        cpu.attach_spi_device(Box::new(SpiLoopback));
        cpu
    }

    #[test]
    fn spi_master_byte_transfer_and_loopback() {
        let mut cpu = spi_cpu();
        cpu.data_write(SFR_SPI1BUF, 0x00A5).unwrap();
        // TXB moved into the shift register: it is no longer full.
        assert_eq!(cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPITBF, 0);
        cpu.advance_spi(7);
        assert_eq!(cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPIRBF, 0);
        cpu.advance_spi(1); // 8 bits * 1 TCY at 1:1 (DS70206C Eq. 18-1)
        let stat = cpu.data_read(SFR_SPI1STAT).unwrap();
        assert_ne!(stat & SPI1STAT_SPIRBF, 0);
        assert_eq!(stat & SPI1STAT_SPITBF, 0);
        assert_ne!(cpu.ifs0 & (1 << IRQ_SPI1.0), 0, "SPI1IF");
        assert_eq!(cpu.data_read(SFR_SPI1BUF).unwrap(), 0x00A5);
        assert_eq!(cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPIRBF, 0);
    }

    #[test]
    fn spi_tx_holding_slot_reports_full_and_chains() {
        let mut cpu = spi_cpu();
        cpu.data_write(SFR_SPI1BUF, 0x0011).unwrap();
        assert_eq!(cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPITBF, 0);
        cpu.data_write(SFR_SPI1BUF, 0x0022).unwrap();
        assert_ne!(
            cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPITBF,
            0,
            "the second write fills TXB"
        );
        cpu.advance_spi(8);
        assert_eq!(cpu.data_read(SFR_SPI1BUF).unwrap(), 0x0011);
        assert_eq!(
            cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPITBF,
            0,
            "TXB moved into the shift register for the next word"
        );
        cpu.advance_spi(8);
        assert_eq!(cpu.data_read(SFR_SPI1BUF).unwrap(), 0x0022);
    }

    #[test]
    fn spi_word_mode_and_prescaler_timing() {
        let mut cpu = spi_cpu();
        // MODE16, SPRE = 000 (8:1), PPRE = 00 (64:1): 16 bits * 2 * 512.
        cpu.data_write(SFR_SPI1CON1, SPI1CON1_MSTEN | SPI1CON1_MODE16)
            .unwrap();
        cpu.data_write(SFR_SPI1BUF, 0x1234).unwrap();
        cpu.advance_spi(16 * 512 - 1);
        assert_eq!(cpu.data_read(SFR_SPI1STAT).unwrap() & SPI1STAT_SPIRBF, 0);
        cpu.advance_spi(1);
        assert_eq!(cpu.data_read(SFR_SPI1BUF).unwrap(), 0x1234);
    }

    #[test]
    fn spi_receive_overrun_sets_spirov_and_the_error_vector() {
        let mut cpu = spi_cpu();
        cpu.data_write(SFR_SPI1BUF, 0x0011).unwrap();
        cpu.advance_spi(8);
        // Do not read SPI1BUF; start another transfer.
        cpu.data_write(SFR_SPI1BUF, 0x0022).unwrap();
        cpu.advance_spi(8);
        let stat = cpu.data_read(SFR_SPI1STAT).unwrap();
        assert_ne!(stat & SPI1STAT_SPIROV, 0, "SPIROV");
        assert_ne!(cpu.ifs0 & (1 << IRQ_SPI1E.0), 0, "SPI1EIF");
        assert_eq!(cpu.data_read(SFR_SPI1BUF).unwrap(), 0x0011, "old RX kept");
    }

    #[test]
    fn spi_slave_clock_exchanges_the_loaded_word() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.data_write(SFR_SPI1STAT, SPI1STAT_SPIEN).unwrap();
        cpu.data_write(SFR_SPI1BUF, 0x005A).unwrap(); // MSTEN = 0: load TXB
        assert_eq!(cpu.spi_slave_clock(0x00C3), 0x005A);
        assert_eq!(cpu.data_read(SFR_SPI1BUF).unwrap(), 0x00C3);
        assert_ne!(cpu.ifs0 & (1 << IRQ_SPI1.0), 0);
    }

    #[test]
    fn spi_interrupt_vectors_match_the_table() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.ifs0 |= 1 << IRQ_SPI1.0;
        cpu.iec0 |= 1 << IRQ_SPI1.0;
        cpu.ipc2 = 3 << 8; // SPI1IP = 011 (IRQ 10, bits 10:8)
        assert_eq!(cpu.pending_interrupt(), Some((18, 3)));
        cpu.ifs0 = 1 << IRQ_SPI1E.0;
        cpu.iec0 = 1 << IRQ_SPI1E.0;
        cpu.ipc2 = 4 << 4; // SPI1EIP = 100 (IRQ 9, bits 6:4)
        assert_eq!(cpu.pending_interrupt(), Some((17, 4)));
    }

    // ── M3: I2C1 ────────────────────────────────────────────────────────────

    fn i2c_cpu_with_tmp102() -> Pic24 {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.attach_i2c_device(Box::new(crate::peripherals::esp32s3::tmp102::Tmp102::new()));
        cpu.data_write(SFR_I2C1BRG, 1).unwrap(); // 2 TCY per SCL
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN).unwrap();
        cpu
    }

    /// Finish the current I2C operation and clear MI2C1IF.
    fn i2c_finish(cpu: &mut Pic24) {
        cpu.advance_i2c(u32::MAX / 2);
        cpu.ifs1 &= !(1 << IRQ_MI2C1.0);
    }

    #[test]
    fn i2c_master_reads_tmp102_temperature() {
        let mut cpu = i2c_cpu_with_tmp102();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_SEN)
            .unwrap();
        i2c_finish(&mut cpu);
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_S, 0);

        cpu.data_write(SFR_I2C1TRN, 0x90).unwrap(); // 0x48 write
        i2c_finish(&mut cpu);
        assert_eq!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_ACKSTAT, 0);

        cpu.data_write(SFR_I2C1TRN, 0x00).unwrap(); // pointer = temperature
        i2c_finish(&mut cpu);

        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_RSEN)
            .unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1TRN, 0x91).unwrap(); // 0x48 read
        i2c_finish(&mut cpu);
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_RW, 0);

        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_RCEN)
            .unwrap();
        i2c_finish(&mut cpu);
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_RBF, 0);
        let msb = cpu.data_read(SFR_I2C1RCV).unwrap();
        assert_eq!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_RBF, 0);

        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_ACKEN)
            .unwrap(); // ACKDT = 0 -> ACK
        i2c_finish(&mut cpu);

        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_RCEN)
            .unwrap();
        i2c_finish(&mut cpu);
        let lsb = cpu.data_read(SFR_I2C1RCV).unwrap();

        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_ACKDT | I2C1CON_ACKEN)
            .unwrap(); // NACK
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_PEN)
            .unwrap();
        i2c_finish(&mut cpu);

        assert_eq!((msb << 8) | lsb, 0x1900, "TMP102 25.0 C");
        let stat = cpu.data_read(SFR_I2C1STAT).unwrap();
        assert_ne!(stat & I2C1STAT_P, 0);
        assert_eq!(stat & I2C1STAT_S, 0);
    }

    #[test]
    fn i2c_missing_slave_nacks_the_address() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.data_write(SFR_I2C1BRG, 1).unwrap();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN).unwrap();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_SEN)
            .unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1TRN, 0x90).unwrap();
        cpu.advance_i2c(u32::MAX / 2);
        let stat = cpu.data_read(SFR_I2C1STAT).unwrap();
        assert_ne!(stat & I2C1STAT_ACKSTAT, 0, "NACK");
        assert_ne!(cpu.ifs1 & (1 << IRQ_MI2C1.0), 0, "MI2C1IF");
    }

    #[test]
    fn i2c_receive_overrun_sets_i2cov() {
        let mut cpu = i2c_cpu_with_tmp102();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_SEN)
            .unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1TRN, 0x91).unwrap(); // read address
        i2c_finish(&mut cpu);
        // Receive one byte and leave RBF set, then receive again: the
        // second word is discarded and I2COV reports the overrun.
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_RCEN)
            .unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_RCEN)
            .unwrap();
        i2c_finish(&mut cpu);
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_I2COV, 0);
        // The overrun word is discarded: RCV still holds the first byte.
        assert_eq!(cpu.data_read(SFR_I2C1RCV).unwrap(), 0x19);
    }

    #[test]
    fn i2c_slave_address_match_and_echo() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.data_write(SFR_I2C1ADD, 0x48).unwrap();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN).unwrap();
        assert!(cpu.i2c_slave_address(0x48, false));
        assert!(!cpu.i2c_slave_address(0x49, false));
        let stat = cpu.data_read(SFR_I2C1STAT).unwrap();
        assert_ne!(stat & I2C1STAT_S, 0);
        assert_ne!(cpu.ifs1 & (1 << IRQ_SI2C1.0), 0);

        cpu.i2c_slave_write(0x5A);
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_RBF, 0);
        assert_eq!(cpu.data_read(SFR_I2C1RCV).unwrap(), 0x5A);
        assert_eq!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_RBF, 0);

        assert!(cpu.i2c_slave_address(0x48, true));
        cpu.i2c1trn = 0x00C3;
        assert_eq!(cpu.i2c_slave_read(), 0xC3);
        cpu.i2c_slave_stop();
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_P, 0);
    }

    struct StopSpy {
        stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl I2cDevice for StopSpy {
        fn address(&self) -> u8 {
            0x48
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _data: u8) {}
        fn stop(&mut self) {
            self.stopped
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn i2c_ten_bit_address_is_nacked_and_not_truncated() {
        let mut cpu = i2c_cpu_with_tmp102();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_SEN)
            .unwrap();
        i2c_finish(&mut cpu);
        // 11110 A9 A8 0, then the low address byte.
        cpu.data_write(SFR_I2C1TRN, 0x00F0).unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1TRN, 0x0048).unwrap();
        i2c_finish(&mut cpu);
        let stat = cpu.data_read(SFR_I2C1STAT).unwrap();
        assert_ne!(stat & I2C1STAT_ACKSTAT, 0, "10-bit address must NACK");
        // ADD10 reports a matched 10-bit address (Register 18-2); the
        // 7-bit I2cDevice contract has no 10-bit match, so it stays clear.
        assert_eq!(stat & I2C1STAT_ADD10, 0);
    }

    #[test]
    fn i2c_slave_ten_bit_match_sets_add10_and_stop_clears_it() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.data_write(SFR_I2C1ADD, 0x0123).unwrap();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_A10M)
            .unwrap();
        assert!(cpu.i2c_slave_address(0x0123, false));
        assert_ne!(
            cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_ADD10,
            0,
            "a matched 10-bit address sets ADD10"
        );
        cpu.i2c_slave_stop();
        assert_eq!(
            cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_ADD10,
            0,
            "Stop clears ADD10 (Register 18-2)"
        );
    }

    #[test]
    fn i2c_transmit_collision_sets_iwcol() {
        let mut cpu = i2c_cpu_with_tmp102();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_SEN)
            .unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1TRN, 0x0090).unwrap(); // transmit pending
        cpu.data_write(SFR_I2C1TRN, 0x0091).unwrap(); // must not overwrite
        assert_ne!(cpu.data_read(SFR_I2C1STAT).unwrap() & I2C1STAT_IWCOL, 0);
        i2c_finish(&mut cpu);
        let stat = cpu.data_read(SFR_I2C1STAT).unwrap();
        assert_eq!(stat & I2C1STAT_ACKSTAT, 0, "the first address was sent");
    }

    #[test]
    fn i2c_disable_stops_the_selected_slave() {
        let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.attach_i2c_device(Box::new(StopSpy {
            stopped: stopped.clone(),
        }));
        cpu.data_write(SFR_I2C1BRG, 1).unwrap();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN).unwrap();
        cpu.data_write(SFR_I2C1CON, I2C1CON_I2CEN | I2C1CON_SEN)
            .unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1TRN, 0x0090).unwrap();
        i2c_finish(&mut cpu);
        cpu.data_write(SFR_I2C1CON, 0).unwrap();
        assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn m3_state_round_trips_through_a_snapshot() {
        let mut cpu = adc_cpu(0, false);
        cpu.set_adc_analog(4, 0x2AA);
        cpu.adc_t3_trigger = true;
        cpu.adc_state = AdcState::Sampling;
        cpu.spi1con1 = 0x0137;
        cpu.i2c1brg = 0x016B;
        cpu.ifs1 = 0x0002;
        let snap = cpu.snapshot();
        let mut back = Pic24::new();
        back.apply_snapshot(&snap);
        assert_eq!(back.adc_analog[4], 0x2AA, "injected analog survives");
        assert!(back.adc_t3_trigger, "pending trigger survives");
        assert_eq!(back.adc_state, AdcState::Sampling);
        assert_eq!(back.spi1con1, 0x0137);
        assert_eq!(back.i2c1brg, 0x016B);
        assert_eq!(back.ifs1, 0x0002);
    }

    #[test]
    fn i2c_interrupt_vectors_match_the_table() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        cpu.ifs1 = (1 << IRQ_SI2C1.0) | (1 << IRQ_MI2C1.0);
        cpu.iec1 = (1 << IRQ_SI2C1.0) | (1 << IRQ_MI2C1.0);
        cpu.ipc4 = 5 | (3 << 4); // SI2C1IP = 101, MI2C1IP = 011
                                 // Highest priority wins: SI2C1 (vector 24, priority 5).
        assert_eq!(cpu.pending_interrupt(), Some((24, 5)));
        cpu.iec1 = 1 << IRQ_MI2C1.0;
        assert_eq!(cpu.pending_interrupt(), Some((25, 3)));
        cpu.ipc4 = 3 | (3 << 4);
        cpu.iec1 = (1 << IRQ_SI2C1.0) | (1 << IRQ_MI2C1.0);
        // Equal priority: the lower vector wins.
        assert_eq!(cpu.pending_interrupt(), Some((24, 3)));
    }
}
