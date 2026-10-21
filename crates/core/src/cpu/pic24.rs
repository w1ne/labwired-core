// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC24 / dsPIC33F CPU — **M0 + M1a + M1b subset** for the
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
//!
//! Not yet modelled — unsupported opcodes return
//! [`SimulationError::DecodeError`] rather than guessing: PWM/QEI (M2), ADC/
//! SPI/I2C (M3), DSP MAC (field tables pending, M1c), PSV/EDS windows.
//! Status-flag arithmetic (SR N/Z/C/OV) is not evaluated yet. Change
//! notification (CNENx/CNPUx) and the remaining PPS functions (UART/SPI/
//! INT/QEI routing) are not modelled: the PPS registers store values, but
//! only the IC1/IC2 inputs and OC1/OC2 outputs are consumed.
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
//! Conventions (XC16 ELF layout verified against DS50002071F):
//! * `pc` is a **PC-unit address**: two units per instruction, so
//!   instructions sit at even addresses and the PC advances by 2. This is
//!   the datasheet's addressing (`PC<0>` always 0; interrupt vectors at
//!   `0x4 + 2*N`) and the XC16 guide's "Program Counter Unit".
//! * Internally each instruction occupies one 4-byte little-endian slot
//!   (`INSTR_SLOT_BYTES`, slot index = `pc / 2`). XC16 executable sections
//!   pack instructions as **3 bytes each** ("2 program counter units are
//!   equivalent to 3 bytes", DS50002071F); the ELF parsers expand them into
//!   slots with [`xc16_executable_slots`]. Phantom bytes are a HEX-file
//!   concept, not an ELF one.
//! * Non-executable PIC24 program-memory segments (read-only data reached
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
//! | Instruction | Encoding source |
//! | :--- | :--- |
//! | `NOP` | DS70000157G |
//! | `MOV #lit16, Wnd` | DS70000157G |
//! | `MOV Wns, f` | DS70000157G |
//! | `MOV f, Wnd` | DS70000157G |
//! | `BRA Expr` (slit16) | DS70000157G |
//! | `RETFIE` | DS70000157G |
//!
//! UART1 registers (DS70283H): `U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224`.

use crate::snapshot::{CpuSnapshot, Pic24CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Flash size of the dsPIC33FJ32MC204 (DS70283H): 32 KB.
pub const FLASH_BYTES: usize = 32 * 1024;
/// Internal storage per instruction: 24-bit word in a 4-byte little-endian
/// slot. **Not** the XC16 ELF layout — executable sections pack 3 bytes per
/// instruction and the parsers expand them ([`xc16_executable_slots`]).
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
/// Internal flash storage: one 4-byte slot per instruction. Larger than
/// [`FLASH_BYTES`] because each 3-byte instruction occupies a 4-byte slot.
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

/// Reject an XC16 executable segment that is odd, starts past program flash,
/// or runs off its end (0x0057FE, DS70283H Figure 4-1). `slots` is the
/// slot-expanded segment from [`xc16_executable_slots`]. Loading would
/// otherwise truncate it silently.
pub fn check_xc16_segment(addr: u64, slots: &[u8]) -> Result<(), String> {
    check_xc16_span(addr, (slots.len() / INSTR_SLOT_BYTES) as u64)
}

/// [`check_xc16_segment`] from the instruction count alone, so a segment can
/// be checked from its program header before its bytes are read.
pub fn check_xc16_span(addr: u64, instructions: u64) -> Result<(), String> {
    check_xc16_pc_address("executable segment", addr)?;
    let end = addr.saturating_add(instructions * u64::from(PC_UNITS_PER_INSTRUCTION));
    if addr >= u64::from(PROGRAM_PC_END) || end > u64::from(PROGRAM_PC_END) {
        return Err(format!(
            "PIC24 executable segment {addr:#x}..{end:#x} does not fit program flash \
             0x0..{PROGRAM_PC_END:#x} (DS70283H Figure 4-1)"
        ));
    }
    Ok(())
}

/// Expand an XC16 executable section into this CPU's internal slots.
///
/// XC16 addresses executable code in PC units (2 per instruction) while
/// storing **3 bytes per instruction** in the ELF file ("In an executable
/// section, 2 program counter units are equivalent to 3 bytes",
/// DS50002071F "Program Counter Unit"). Each 24-bit little-endian
/// instruction becomes one 4-byte slot with a zero pad byte. `None` when
/// `data` is not a whole number of instructions — callers reject such a
/// segment rather than guess (the 4-byte "phantom" layout belongs to HEX
/// files, not ELF).
pub fn xc16_executable_slots(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || data.len() % 3 != 0 {
        return None;
    }
    let mut slots = Vec::with_capacity(data.len() / 3 * INSTR_SLOT_BYTES);
    for insn in data.chunks_exact(3) {
        slots.extend_from_slice(insn);
        slots.push(0);
    }
    Some(slots)
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

/// IFS0 flag bit and vector number for each modelled source (DS70283H §7.3.1).
pub const IRQ_INT0: (u16, u32) = (0, 8);
pub const IRQ_IC1: (u16, u32) = (1, 9);
pub const IRQ_OC1: (u16, u32) = (2, 10);
pub const IRQ_T1: (u16, u32) = (3, 11);
pub const IRQ_IC2: (u16, u32) = (5, 12);
pub const IRQ_OC2: (u16, u32) = (6, 13);
pub const IRQ_T2: (u16, u32) = (7, 14);
pub const IRQ_T3: (u16, u32) = (8, 15);

/// INTCON1.NSTDIS — nesting disable (bit 15).
pub const INTCON1_NSTDIS: u16 = 0x8000;
/// INTCON2.DISI — disable interrupts 1-6 (bit 14).
pub const INTCON2_DISI: u16 = 0x4000;

/// Exception entry costs two extra instruction cycles (DS70283H §7).
pub const EXCEPTION_ENTRY_CYCLES: u32 = 2;
/// RETFIE takes three instruction cycles (DS70000157G).
pub const RETFIE_CYCLES: u32 = 3;

/// RETFIE — 0000 0110 0100 0000 0000 0000 (DS70000157G).
pub const INSN_RETFIE: u32 = 0x0064_0000;

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

/// dsPIC33F CPU core (M0 + M1a + M1b subset).
#[derive(Debug, Clone)]
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
    iec0: u16,
    ipc0: u16,
    ipc1: u16,
    ipc2: u16,
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
            iec0: 0,
            ipc0: 0,
            ipc1: 0,
            ipc2: 0,
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
    /// the parsers hand over slot-expanded executable segments; place each
    /// slot at `addr / 2`, then start at the ELF entry point.
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
        self.pc = image.entry_point as u32;
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
    fn sample_capture(&mut self, bus: &dyn Bus) {
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
    fn push_word(&mut self, val: u16) -> SimResult<()> {
        self.w[15] = self.w[15].wrapping_sub(2);
        let a = self.w[15];
        let end = RAM_BASE as u16 + RAM_BYTES as u16;
        if a >= RAM_BASE as u16 && a + 2 <= end {
            let o = a as usize - RAM_BASE as usize;
            self.ram[o..o + 2].copy_from_slice(&val.to_le_bytes());
            Ok(())
        } else {
            Err(SimulationError::MemoryViolation(u64::from(a)))
        }
    }

    fn pop_word(&mut self) -> SimResult<u16> {
        let a = self.w[15];
        let end = RAM_BASE as u16 + RAM_BYTES as u16;
        if a < RAM_BASE as u16 || a + 2 > end {
            return Err(SimulationError::MemoryViolation(u64::from(a)));
        }
        let o = a as usize - RAM_BASE as usize;
        let val = u16::from_le_bytes([self.ram[o], self.ram[o + 1]]);
        self.w[15] = self.w[15].wrapping_add(2);
        Ok(val)
    }

    /// Current CPU priority level: IPL<2:0> (SR<7:5>) plus IPL<3> (CORCON<3>).
    fn current_ipl(&self) -> u16 {
        ((self.sr >> 5) & 0x7) | (((self.corcon >> 3) & 1) << 3)
    }

    /// The highest-priority pending, enabled interrupt: (vector, priority).
    ///
    /// Natural order = IFS0 bit order (DS70283H §7.3.1). Simplified gating:
    /// DISI blocks everything we model (levels 1-6), NSTDIS blocks nesting.
    fn pending_interrupt(&self) -> Option<(u32, u16)> {
        if self.intcon2 & INTCON2_DISI != 0 {
            return None;
        }
        let ipl = self.current_ipl();
        if self.intcon1 & INTCON1_NSTDIS != 0 && ipl > 0 {
            return None;
        }
        let sources: [(u16, u32, u8); 8] = [
            (IRQ_INT0.0, IRQ_INT0.1, 0),
            (IRQ_IC1.0, IRQ_IC1.1, 4),
            (IRQ_OC1.0, IRQ_OC1.1, 8),
            (IRQ_T1.0, IRQ_T1.1, 12),
            (IRQ_IC2.0, IRQ_IC2.1, 4),
            (IRQ_OC2.0, IRQ_OC2.1, 8),
            (IRQ_T2.0, IRQ_T2.1, 12),
            (IRQ_T3.0, IRQ_T3.1, 12),
        ];
        for (i, (bit, vector, shift)) in sources.iter().enumerate() {
            if self.ifs0 & (1 << bit) == 0 || self.iec0 & (1 << bit) == 0 {
                continue;
            }
            let ipc = match i {
                0..=3 => self.ipc0,
                4..=6 => self.ipc1,
                _ => self.ipc2,
            };
            let prio = (ipc >> shift) & 0x7;
            if prio != 0 && prio > ipl {
                return Some((*vector, prio));
            }
        }
        None
    }

    /// Take the pending interrupt, if any: push the DS70283H frame
    /// (PC<15:0>, then SR<7:0>|IPL3|PC<22:16>), raise IPL, jump the vector.
    fn take_interrupt(&mut self) -> SimResult<bool> {
        let Some((vector, prio)) = self.pending_interrupt() else {
            return Ok(false);
        };
        self.push_word(self.pc as u16)?;
        let packed = ((self.sr & 0x00FF) << 8)
            | (((self.corcon >> 3) & 1) << 7)
            | ((self.pc >> 16) as u16 & 0x007F);
        self.push_word(packed)?;
        self.sr = (self.sr & !0x00E0) | ((prio & 0x7) << 5);
        self.pc = 0x4 + 2 * vector;
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
                a if (SFR_RPOR0..SFR_RPOR0 + 2 * RPOR_COUNT as u16).contains(&a) => {
                    Ok(self.rpor[((a - SFR_RPOR0) / 2) as usize])
                }
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
                    SFR_SR => self.sr = val,
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

    /// Execute one decoded word. Returns the instruction cycles consumed.
    fn execute(&mut self, word: u32) -> SimResult<u32> {
        // NOP — DS70000157G: 0000 0000 xxxx xxxx xxxx xxxx.
        if word & 0x00FFFF == 0 && word >> 16 == 0 {
            self.pc = self.pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV #lit16, Wnd — 0010 kkkk kkkk kkkk kkkk dddd.
        if word & 0x00F0_0000 == 0x0020_0000 {
            let k = ((word >> 4) & 0xFFFF) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = k;
            self.pc = self.pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV Wns, f — 1000 1fff ffff ffff ffff ssss (f is a 16-bit even address:
        // 15 stored bits, LSB implied 0 — DS70000157G).
        if word >> 19 == 0b1_0001 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let s = (word & 0xF) as usize;
            self.data_write(f, self.w[s])?;
            self.pc = self.pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // MOV f, Wnd — 1000 0fff ffff ffff ffff dddd.
        if word >> 19 == 0b1_0000 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let d = (word & 0xF) as usize;
            let v = self.data_read(f)?;
            self.w[d] = v;
            self.pc = self.pc.wrapping_add(PC_UNITS_PER_INSTRUCTION);
            return Ok(1);
        }

        // BRA Expr — 0011 0111 nnnn nnnn nnnn nnnn (slit16 words: PC = PC + 2 + 2*offset).
        if word >> 16 == 0x0037 {
            let off = (word & 0xFFFF) as u16 as i16;
            let next = self.pc as i64 + 2 + 2 * i64::from(off);
            if next < 0 {
                return Err(SimulationError::DecodeError(self.pc as u64));
            }
            self.pc = next as u32;
            return Ok(2);
        }

        // RETFIE — 0000 0110 0100 0000 0000 0000 (DS70000157G).
        if word == INSN_RETFIE {
            let low = self.pop_word()?;
            let packed = self.pop_word()?;
            self.sr = (self.sr & 0xFF00) | ((packed >> 8) & 0x00FF);
            self.corcon = (self.corcon & !0x0008) | (((packed >> 7) & 1) << 3);
            self.pc = (u32::from(packed & 0x007F) << 16) | u32::from(low);
            return Ok(RETFIE_CYCLES);
        }

        // Everything else is deliberately unimplemented (see module scope).
        Err(SimulationError::DecodeError(self.pc as u64))
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
        self.iec0 = 0;
        self.ipc0 = 0;
        self.ipc1 = 0;
        self.ipc2 = 0;
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
        // M1b: publish the port/OC drive to the pad models, then sample the
        // pads for input capture (order: what this step drives is what it
        // samples).
        self.sync_pads(bus);
        self.sample_capture(bus);

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
        CpuSnapshot::Pic24(Pic24CpuSnapshot {
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
        })
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

    fn step<B: Bus>(cpu: &mut Pic24, bus: &mut B) {
        cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
    }

    #[test]
    fn full_program_flash_loads_through_the_last_instruction() {
        // 11264 instructions (33,792 bytes of 3-byte packed XC16 code): every
        // instruction up to 0x57FE.
        let packed: Vec<u8> = (0..INSTR_SLOTS)
            .flat_map(|_| mov_lit_w0(0x48).to_le_bytes()[..3].to_vec())
            .collect();
        let mut image = crate::memory::ProgramImage::new(0, crate::Arch::Pic24);
        image.add_segment(0, xc16_executable_slots(&packed).unwrap());
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
    /// executable PT_LOAD per segment, XC16 3-byte packed.
    fn pic24_elf(entry: u32, segments: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let phoff = 52u32;
        let mut data_off = phoff + 32 * segments.len() as u32;
        let mut out = vec![0x7f, b'E', b'L', b'F', 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        out.extend_from_slice(&118u16.to_le_bytes()); // EM_DSPIC30F
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&entry.to_le_bytes());
        out.extend_from_slice(&phoff.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // e_shoff
        out.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        for half in [52u16, 32, segments.len() as u16, 40, 0, 0] {
            out.extend_from_slice(&half.to_le_bytes());
        }
        for (paddr, bytes) in segments {
            let len = bytes.len() as u32;
            // p_type PT_LOAD, offset, vaddr, paddr, filesz, memsz, flags R|X, align
            for word in [1u32, data_off, *paddr, *paddr, len, len, 5, 2] {
                out.extend_from_slice(&word.to_le_bytes());
            }
            data_off += len;
        }
        for (_, bytes) in segments {
            out.extend_from_slice(bytes);
        }
        out
    }

    #[test]
    fn segments_past_program_flash_are_rejected() {
        let one = xc16_executable_slots(&[0, 0, 0]).unwrap();
        let end = u64::from(PROGRAM_PC_END);
        assert!(
            check_xc16_segment(end - 2, &one).is_ok(),
            "the last instruction fits"
        );
        assert!(
            check_xc16_segment(end, &one).is_err(),
            "starts at the end of flash"
        );
        assert!(
            check_xc16_segment(end + 0x100, &one).is_err(),
            "starts beyond flash"
        );
        let two = xc16_executable_slots(&[0; 6]).unwrap();
        assert!(
            check_xc16_segment(end - 2, &two).is_err(),
            "runs one instruction past the end"
        );
    }

    #[test]
    fn parse_elf_image_rejects_malformed_xc16_elves() {
        use crate::system::node::parse_elf_image;
        let code = vec![0u8; 6];
        assert!(parse_elf_image(&pic24_elf(0x200, &[(0x200, code.clone())])).is_ok());
        let odd_entry = parse_elf_image(&pic24_elf(0x201, &[(0x200, code.clone())])).unwrap_err();
        assert!(odd_entry.to_string().contains("entry point"), "{odd_entry}");
        let odd_segment = parse_elf_image(&pic24_elf(0x200, &[(0x201, code.clone())])).unwrap_err();
        assert!(odd_segment.to_string().contains("is odd"), "{odd_segment}");
        for entry in [PROGRAM_PC_END, PROGRAM_PC_END + 0x100] {
            let far = parse_elf_image(&pic24_elf(entry, &[(0x200, code.clone())])).unwrap_err();
            assert!(far.to_string().contains("entry point"), "{far}");
        }
        // A segment whose bytes lie outside the file is still range-checked,
        // and is an error rather than a silent skip.
        let out_of_file = |paddr: u32| {
            let mut elf = pic24_elf(0x200, &[(paddr, code.clone())]);
            elf[52 + 4..52 + 8].copy_from_slice(&0x10_0000u32.to_le_bytes()); // p_offset
            parse_elf_image(&elf).unwrap_err().to_string()
        };
        let far = out_of_file(PROGRAM_PC_END + 0x100);
        assert!(far.contains("does not fit program flash"), "{far}");
        let truncated = out_of_file(0x200);
        assert!(
            truncated.contains("past the end of the file"),
            "{truncated}"
        );
        let past = parse_elf_image(&pic24_elf(0x200, &[(PROGRAM_PC_END - 2, code)])).unwrap_err();
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
                                                // Handler at the T1 vector (number 11 -> address 0x1A):
                                                //   MOV #0, W0 ; MOV W0, T1CON ; MOV W0, IFS0 ; RETFIE
                                                // The timer is stopped first: with PR1 = 0 a tick re-sets the flag in
                                                // the very same step that clears it — correct silicon behaviour, but
                                                // not what this test wants to observe.
        let mut handler = Vec::new();
        handler.extend(enc(mov_lit_w0(0)));
        handler.extend(enc(mov_w0_to_f(SFR_T1CON)));
        handler.extend(enc(mov_w0_to_f(SFR_IFS0)));
        handler.extend(enc(INSN_RETFIE));
        cpu.load_flash(0x1A, &handler);

        cpu.set_sp(0x1000);
        write_sfr(&mut cpu, SFR_PR1, 0); // flag after every tick
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        write_sfr(&mut cpu, SFR_IEC0, 1 << IRQ_T1.0);
        write_sfr(&mut cpu, SFR_IPC0, 1 << 12); // T1IP = 1

        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus); // BRA; the timer ticks -> T1IF
        assert_ne!(cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0), 0);

        step(&mut cpu, &mut bus); // vector + first handler instruction
        assert_eq!(cpu.get_pc(), 0x1C, "handler began at the T1 vector");
        assert_eq!(
            (cpu.data_read(SFR_SR).unwrap() >> 5) & 0x7,
            1,
            "IPL raised to the source priority"
        );

        // DS70283H frame: [SP] = SR<7:0>|IPL3|PC<22:16>, [SP+2] = PC<15:0>.
        let sp = cpu.w[15];
        assert_eq!(cpu.data_read(sp).unwrap(), 0x0000, "packed frame word");
        assert_eq!(cpu.data_read(sp + 2).unwrap(), 0x0000, "saved PC (was 0)");

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
        assert_eq!(cpu.w[15], 0x1000, "stack fully unwound");
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
}
