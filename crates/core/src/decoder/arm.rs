// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Instruction {
    Nop,
    /// Wait For Interrupt (0xBF30). Suspends the core until a wake-up event
    /// (a sufficiently-prioritised pending exception) arrives; see the
    /// `CortexM` executor and idle fast-forward. WFE/YIELD/SEV stay `Nop` —
    /// no event register is modelled.
    Wfi,
    MovImm {
        rd: u8,
        imm: u8,
    }, // MOV Rd, #imm8
    Branch {
        offset: i32,
    }, // B <label>
    BranchCond {
        cond: u8,
        offset: i32,
    }, // Bcc <label>

    // Arithmetic & Logic
    AddReg {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // ADD Rd, Rn, Rm
    AddImm3 {
        rd: u8,
        rn: u8,
        imm: u8,
    }, // ADD Rd, Rn, #imm3
    AddImm8 {
        rd: u8,
        imm: u8,
    }, // ADD Rd, #imm8

    SubReg {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // SUB Rd, Rn, Rm
    SubImm3 {
        rd: u8,
        rn: u8,
        imm: u8,
    }, // SUB Rd, Rn, #imm3
    SubImm8 {
        rd: u8,
        imm: u8,
    }, // SUB Rd, #imm8

    CmpImm {
        rn: u8,
        imm: u8,
    }, // CMP Rn, #imm8
    CmpReg {
        rn: u8,
        rm: u8,
    }, // CMP Rn, Rm
    Cmn {
        rn: u8,
        rm: u8,
    }, // CMN Rn, Rm
    Tst {
        rn: u8,
        rm: u8,
    }, // TST Rn, Rm
    MovReg {
        rd: u8,
        rm: u8,
    }, // MOV Rd, Rm (High registers)
    Movw {
        rd: u8,
        imm: u16,
    }, // MOVW Rd, #imm16
    Movt {
        rd: u8,
        imm: u16,
    }, // MOVT Rd, #imm16

    AddSp {
        imm: u16,
    }, // ADD SP, SP, #imm
    SubSp {
        imm: u16,
    }, // SUB SP, SP, #imm
    AddRegHigh {
        rd: u8,
        rm: u8,
    }, // ADD Rd, Rm (at least one high register)
    Cpsie {
        primask: bool,
        faultmask: bool,
    }, // CPSIE i/f
    Cpsid {
        primask: bool,
        faultmask: bool,
    }, // CPSID i/f

    And {
        rd: u8,
        rm: u8,
    }, // AND Rd, Rm
    Bic {
        rd: u8,
        rm: u8,
    }, // BIC Rd, Rm
    Orr {
        rd: u8,
        rm: u8,
    }, // ORR Rd, Rm
    Eor {
        rd: u8,
        rm: u8,
    }, // EOR Rd, Rm
    Mvn {
        rd: u8,
        rm: u8,
    }, // MVN Rd, Rm

    // Shifts
    Lsl {
        rd: u8,
        rm: u8,
        imm: u8,
    }, // LSL Rd, Rm, #imm5
    Lsr {
        rd: u8,
        rm: u8,
        imm: u8,
    }, // LSR Rd, Rm, #imm5
    Asr {
        rd: u8,
        rm: u8,
        imm: u8,
    }, // ASR Rd, Rm, #imm5
    LslReg {
        rd: u8,
        rm: u8,
    }, // LSL Rd, Rs
    LsrReg {
        rd: u8,
        rm: u8,
    }, // LSR Rd, Rs
    Adc {
        rd: u8,
        rm: u8,
    }, // ADC Rd, Rm
    Sbc {
        rd: u8,
        rm: u8,
    }, // SBC Rd, Rm
    Ror {
        rd: u8,
        rm: u8,
    }, // ROR Rd, Rm

    // Memory
    LdrImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // LDR Rt, [Rn, #imm] (imm is *4)
    StrImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // STR Rt, [Rn, #imm] (imm is *4)
    StrReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // STR Rt, [Rn, Rm]
    LdrLit {
        rt: u8,
        imm: u16,
    }, // LDR Rt, [PC, #imm]
    LdrImm32 {
        rt: u8,
        rn: u8,
        imm12: u16,
    },
    StrImm32 {
        rt: u8,
        rn: u8,
        imm12: u16,
    },
    /// LDR (immediate) T4: indexed forms with optional base writeback.
    /// `pre_index` true → offset applied before the load (`[Rn, #±imm]{!}`);
    /// false → post-index (`[Rn], #±imm`, writeback implied). `add` selects
    /// add vs subtract. Models the `ldr.w pc, [sp], #4` function-return idiom
    /// (post-index, writeback) that the T3 form cannot express.
    LdrImm32Idx {
        rt: u8,
        rn: u8,
        imm8: u8,
        pre_index: bool,
        add: bool,
        writeback: bool,
    },
    /// STR (immediate) T4: indexed forms with optional base writeback.
    /// Mirror of [`Instruction::LdrImm32Idx`] for stores (`str.w rt,[rn],#imm`
    /// and `str.w rt,[rn,#imm]!`), e.g. clang's pre-decrement stack pushes.
    StrImm32Idx {
        rt: u8,
        rn: u8,
        imm8: u8,
        pre_index: bool,
        add: bool,
        writeback: bool,
    },
    LdrbImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // LDRB Rt, [Rn, #imm]
    LdrbReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDRB Rt, [Rn, Rm]
    StrbImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // STRB Rt, [Rn, #imm]
    StrbReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // STRB Rt, [Rn, Rm]
    LdrhImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // LDRH Rt, [Rn, #imm] (imm is *2)
    StrhImm {
        rt: u8,
        rn: u8,
        imm: u8,
    }, // STRH Rt, [Rn, #imm] (imm is *2)
    StrhReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // STRH Rt, [Rn, Rm]
    LdrsbReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDRSB Rt, [Rn, Rm]
    LdrhReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDRH Rt, [Rn, Rm]
    LdrshReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDRSH Rt, [Rn, Rm]

    // Stack
    Push {
        registers: u8,
        m: bool,
    }, // PUSH {Rlist, LR?}
    Pop {
        registers: u8,
        p: bool,
    }, // POP {Rlist, PC?}
    Ldm {
        rn: u8,
        registers: u8,
    }, // LDM Rn, {Rlist}
    Stm {
        rn: u8,
        registers: u8,
    }, // STM Rn, {Rlist}

    // Control Flow
    Cbz {
        rn: u8,
        imm: u8,
    }, // CBZ Rn, <label>
    Cbnz {
        rn: u8,
        imm: u8,
    }, // CBNZ Rn, <label>
    Bl {
        offset: i32,
    }, // BL <label> (32-bit T1+T2)
    Bx {
        rm: u8,
    }, // BX Rm
    BlxReg {
        rm: u8,
    }, // BLX Rm (T1) — branch with link to register address
    Mul {
        rd: u8,
        rn: u8,
    }, // MUL Rd, Rn (Rd = Rn * Rd)

    // SP-Relative
    LdrSp {
        rt: u8,
        imm: u16,
    }, // LDR Rt, [SP, #imm]
    StrSp {
        rt: u8,
        imm: u16,
    }, // STR Rt, [SP, #imm]
    AddSpReg {
        rd: u8,
        imm: u16,
    }, // ADD Rd, SP, #imm (ADR-like for SP)

    // Other ALU
    Mul32 {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // MUL.W Rd, Rn, Rm (T2)
    Uxtb {
        rd: u8,
        rm: u8,
    }, // UXTB Rd, Rm
    /// SXTH Rd, Rm — sign-extend bottom 16 bits of Rm into Rd.
    Sxth {
        rd: u8,
        rm: u8,
    },
    /// SXTB Rd, Rm — sign-extend bottom 8 bits of Rm into Rd.
    Sxtb {
        rd: u8,
        rm: u8,
    },
    /// UXTH Rd, Rm — zero-extend bottom 16 bits of Rm into Rd.
    Uxth {
        rd: u8,
        rm: u8,
    },
    /// Wide register-extend (T2): {S,U}XT{B,H}.W and the extend-and-add
    /// {S,U}XTA{B,H}.W. `Rd = (Rn==0xF ? 0 : Rn) + extend(ROR(Rm, rotate))`.
    /// `rotate` is 0/8/16/24; `rn`==0xF means the plain extend (no add).
    ExtendW {
        rd: u8,
        rn: u8,
        rm: u8,
        rotate: u8,
        /// 0=SXTH, 1=UXTH, 4=SXTB, 5=UXTB (ARM op field h1[6:4]).
        op: u8,
    },
    Adr {
        rd: u8,
        imm: u16,
    }, // ADR Rd, <label>

    /// ADDW (Thumb-2 T4 plain 12-bit immediate): `Rd = Rn + zero_extend(imm12)`.
    /// Distinct from DataProcImm32/ADD because the immediate is NOT
    /// run through ThumbExpandImm — it's plain zero-extended.
    AddwImm {
        rd: u8,
        rn: u8,
        imm: u16, // 12-bit value, zero-extended at execute time
    },
    /// SUBW (Thumb-2 T4): `Rd = Rn - zero_extend(imm12)`. Same encoding
    /// family as ADDW with op = 0xA in h1[7:4].
    SubwImm {
        rd: u8,
        rn: u8,
        imm: u16,
    },
    AsrReg {
        rd: u8,
        rm: u8,
    }, // ASR Rd, Rm
    LdrReg {
        rt: u8,
        rn: u8,
        rm: u8,
    }, // LDR Rt, [Rn, Rm]
    Rsbs {
        rd: u8,
        rn: u8,
    }, // RSBS Rd, Rn, #0

    // Bit Field Instructions (Thumb-2)
    Bfi {
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    }, // BFI Rd, Rn, #lsb, #width
    Bfc {
        rd: u8,
        lsb: u8,
        width: u8,
    }, // BFC Rd, #lsb, #width
    Sbfx {
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    }, // SBFX Rd, Rn, #lsb, #width
    Ubfx {
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    }, // UBFX Rd, Rn, #lsb, #width

    // Misc Thumb-2 Instructions
    Clz {
        rd: u8,
        rm: u8,
    }, // CLZ Rd, Rm
    Rbit {
        rd: u8,
        rm: u8,
    }, // RBIT Rd, Rm
    Rev {
        rd: u8,
        rm: u8,
    }, // REV Rd, Rm
    Rev16 {
        rd: u8,
        rm: u8,
    }, // REV16 Rd, Rm
    RevSh {
        rd: u8,
        rm: u8,
    }, // REVSH Rd, Rm
    // SIMD byte add/subtract (set APSR.GE) + SEL. Cortex-M4 DSP extension.
    // newlib's optimised strlen/strcmp/memchr emit UADD8+SEL, so omitting
    // these silently corrupts every C-string length computation (found via
    // an IO-Link ISDU vendor-name read returning garbage).
    SimdAddSub8 {
        rd: u8,
        rn: u8,
        rm: u8,
        op: u8, // 0=SADD8 1=UADD8 2=SSUB8 3=USUB8
    },
    /// Parallel HALFWORD add/subtract (ARMv7-M A5.3.9, Cortex-M4/M7 DSP).
    ///
    /// The sibling of [`Instruction::SimdAddSub8`], and omitted for the same
    /// reason it once was: nothing looked like it needed them. `UQADD16` is what
    /// LLVM emits for Rust's `u16::saturating_add` on `thumbv7em`, so a plain
    /// `x.saturating_add(w - 1)` in ordinary firmware decoded to nothing,
    /// skipped 4 bytes, and left Rd holding a stale operand — the addition just
    /// did not happen. It surfaced as an ILI9341 window whose end coordinate was
    /// the rectangle's WIDTH instead of its right edge, i.e. a display that
    /// painted one row of a fourteen-row band. No fault, no diagnostic: the
    /// arithmetic was simply wrong.
    ///
    /// `op` is the h2[7:4] variant selector, shared by both groups:
    /// 0=S 1=Q (signed saturating) 2=SH (signed halving) 4=U 5=UQ (unsigned
    /// saturating) 6=UH (unsigned halving). `sub` picks SUB16 over ADD16.
    SimdAddSub16 {
        rd: u8,
        rn: u8,
        rm: u8,
        op: u8,
        sub: bool,
    },
    Sel {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // SEL Rd, Rn, Rm (per-byte select on APSR.GE)
    Udiv {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // UDIV Rd, Rn, Rm
    Sdiv {
        rd: u8,
        rn: u8,
        rm: u8,
    }, // SDIV Rd, Rn, Rm

    DataProc32 {
        op: u8,
        rn: u8,
        rd: u8,
        rm: u8,
        imm5: u8,
        shift_type: u8,
        set_flags: bool,
    },
    DataProcImm32 {
        op: u8,
        rn: u8,
        rd: u8,
        imm12: u32,
        set_flags: bool,
    },
    ShiftReg32 {
        rd: u8,
        rn: u8,
        rm: u8,
        shift_type: u8,
    }, // LSL/LSR/ASR/ROR (register)

    It {
        cond: u8,
        mask: u8,
    }, // IT <cond> <mask...ish>

    /// LDMIA.W Rn(!), {registers} — 32-bit load multiple increment-after.
    /// reg_list: bit n = register n (0-14=LR, 15=PC).
    LdmiaW {
        rn: u8,
        reg_list: u16,
        writeback: bool,
    },
    /// STMDB Rn(!), {registers} — 32-bit store multiple decrement-before.
    /// reg_list: bit n = register n (0-14=LR).
    StmdbW {
        rn: u8,
        reg_list: u16,
        writeback: bool,
    },
    /// STMIA.W Rn(!), {registers} — 32-bit store multiple increment-after.
    /// The wide form of the compiler's struct-copy / block-store idiom; the
    /// addressing mode is the U bit of the encoding, not implied by it being a
    /// store. reg_list: bit n = register n (0-14=LR).
    StmiaW {
        rn: u8,
        reg_list: u16,
        writeback: bool,
    },
    /// LDMDB.W Rn(!), {registers} — 32-bit load multiple decrement-before.
    /// reg_list: bit n = register n (0-14=LR, 15=PC).
    LdmdbW {
        rn: u8,
        reg_list: u16,
        writeback: bool,
    },
    Ldrd {
        rt: u8,
        rt2: u8,
        rn: u8,
        imm8: u32,
        add_imm: bool,
        index: bool,
        writeback: bool,
    },
    Strd {
        rt: u8,
        rt2: u8,
        rn: u8,
        imm8: u32,
        add_imm: bool,
        index: bool,
        writeback: bool,
    },
    Tbb {
        rn: u8,
        rm: u8,
    },
    Tbh {
        rn: u8,
        rm: u8,
    },

    Bkpt {
        imm8: u8,
    },

    Svc {
        imm8: u8,
    },

    // --- Thumb-2 additions (ARMv7-M data-processing bits that common
    //     firmware compilers emit, but which were missing from main).
    /// DMB / DSB / ISB — all modelled as architectural no-ops on our
    /// single-threaded simulator. Decoding them separately (vs raising
    /// DecodeError) is important because startup code and HAL inline-asm
    /// emit them routinely.
    Barrier,

    /// MRS Rd, <sysm> — read a system register into a GP register.
    /// Only PRIMASK (sysm = 0x10) is modelled; other sysm values are
    /// accepted and return 0 from the executor.
    Mrs {
        rd: u8,
        sysm: u8,
    },

    /// MSR <sysm>, Rn — write a GP register to a system register.
    Msr {
        sysm: u8,
        rn: u8,
    },

    /// 32×32 → 64-bit signed multiply (ARMv7-M). rd_lo, rd_hi receive
    /// the low/high halves of the 64-bit result.
    Smull {
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    },
    /// 32×32 → 64-bit unsigned multiply.
    Umull {
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    },
    /// Signed multiply-accumulate (64-bit).
    /// result = (rd_hi:rd_lo) + (Rn * Rm), signed.
    Smlal {
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    },
    /// Unsigned multiply-accumulate (64-bit).
    Umlal {
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    },
    /// Unsigned multiply-accumulate-accumulate-long (64-bit).
    /// (rd_hi:rd_lo) = Rn * Rm + rd_lo + rd_hi.
    Umaal {
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    },

    /// 32-bit MLA: Rd = Ra + (Rn * Rm).
    Mla {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    /// 32-bit MLS: Rd = Ra - (Rn * Rm).
    Mls {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    /// SMLABB/BT/TB/TT and SMULBB/BT/TB/TT — ARMv7-M A7.7.166/A7.7.171.
    ///
    /// A 16x16 signed multiply picking a HALF of each operand register, with an
    /// optional 32-bit accumulate. Part of the DSP extension, which the
    /// Cortex-M33 in EFR32MG26 has and which GCC reaches for on ordinary code:
    /// it compiles `base + 48 * index` in Arduino's `digitalWrite` to SMLABB
    /// because it can prove both operands fit in 16 bits.
    ///
    /// `accumulate` is false for the SMUL forms (Ra == 0b1111), where there is
    /// no addend and Ra is not a register at all.
    SmlaXy {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
        /// Take Rn's TOP half rather than its bottom.
        n_top: bool,
        /// Take Rm's TOP half rather than its bottom.
        m_top: bool,
        accumulate: bool,
    },

    // -------- VFPv4 single-precision (FPU) --------
    //
    // Implementation note: the executor reads/writes float bits via
    // `f32::from_bits` / `f32::to_bits` against `CortexM::fpu_s[Sd]`.
    // Only single-precision (size = 1010) is modelled; double-precision
    // encodings fall through to Unknown32. firmware compiled for
    // `-mfpu=fpv4-sp-d16` (typical Cortex-M4F target) only emits single.
    /// VLDR.F32 Sd, [Rn, #±imm8*4] (T1).
    Vldr {
        sd: u8,
        rn: u8,
        imm: u16, // already byte-shifted (imm8 << 2)
        add: bool,
    },
    /// VSTR.F32 Sd, [Rn, #±imm8*4] (T1).
    Vstr {
        sd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    },
    /// VMUL.F32 Sd, Sn, Sm.
    VmulF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VADD.F32 Sd, Sn, Sm.
    VaddF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VSUB.F32 Sd, Sn, Sm.
    VsubF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VDIV.F32 Sd, Sn, Sm.
    VdivF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VFMA.F32 Sd, Sn, Sm — fused multiply-accumulate: Sd = fused(Sn*Sm) + Sd.
    VfmaF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VFMS.F32 Sd, Sn, Sm — fused negated-multiply-accumulate:
    /// Sd = fused(-Sn*Sm) + Sd.
    VfmsF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VFNMA.F32 Sd, Sn, Sm — fused multiply-subtract:
    /// Sd = fused(Sn*Sm) - Sd.
    VfnmaF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VFNMS.F32 Sd, Sn, Sm — fused negated-multiply-subtract:
    /// Sd = fused(-Sn*Sm) - Sd.
    VfnmsF32 {
        sd: u8,
        sn: u8,
        sm: u8,
    },
    /// VMOV Sn, Rt — single-precision FPU register from GP register.
    VmovSnRt {
        sn: u8,
        rt: u8,
    },
    /// VMOV Rt, Sn — GP register from single-precision FPU register.
    VmovRtSn {
        rt: u8,
        sn: u8,
    },
    /// VMOV.F32 Sd, Sm — register-to-register float move.
    VmovF32Reg {
        sd: u8,
        sm: u8,
    },
    /// VMOV.F32 Sd, #imm — VFP expanded 8-bit immediate (T2).
    VmovF32Imm {
        sd: u8,
        imm_bits: u32, // already expanded to IEEE-754 single bits
    },
    /// VCVT.F32.S32 / VCVT.F32.U32 — integer bits in Sm → float in Sd.
    /// `signed` selects S32 vs U32 source; `fbits` is the fixed-point
    /// fractional bit count (0 = pure integer, 1..=32 from the `#fbits` form).
    VcvtF32FromInt {
        sd: u8,
        sm: u8,
        signed: bool,
        fbits: u8,
    },
    /// VCVT.S32.F32 / VCVT.U32.F32 — float in Sm → integer bits in Sd
    /// (to-zero / truncate, matching the default FPSCR.RMode after reset).
    VcvtIntFromF32 {
        sd: u8,
        sm: u8,
        signed: bool,
        fbits: u8,
    },

    // -------- VFP load/store multiple + double-precision (Cortex-M7 FPv5-D16) --------
    // The register file is `fpu_s: [u32; 32]` (S0..S31); a double Dn occupies the
    // consecutive pair (fpu_s[2n], fpu_s[2n+1]), so both single and double transfers
    // reduce to moving `count` consecutive 32-bit words starting at S-index `s_first`.
    /// VSTM / VPUSH — store `count` 32-bit FP words. `add` = increment (IA) vs
    /// decrement-before (DB); `wback` writes the updated base back to `rn`.
    VfpStoreMultiple {
        rn: u8,
        s_first: u8,
        count: u8,
        add: bool,
        wback: bool,
    },
    /// VLDM / VPOP — load `count` 32-bit FP words (see `VfpStoreMultiple`).
    VfpLoadMultiple {
        rn: u8,
        s_first: u8,
        count: u8,
        add: bool,
        wback: bool,
    },
    /// VLDR.F64 Dd, [Rn, #±imm8*4] — `dd` is the low S-index (2*Dd).
    Vldr64 {
        dd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    },
    /// VSTR.F64 Dd, [Rn, #±imm8*4].
    Vstr64 {
        dd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    },
    /// VMOV.F64 Dd, Dm — register-to-register double move (`dd`/`dm` low S-index).
    VmovF64Reg {
        dd: u8,
        dm: u8,
    },
    /// VMOV Dm, Rt, Rt2 — pack two GP registers into a double.
    VmovDRtRt2 {
        dm: u8,
        rt: u8,
        rt2: u8,
    },
    /// VMOV Rt, Rt2, Dm — unpack a double into two GP registers.
    VmovRtRt2D {
        rt: u8,
        rt2: u8,
        dm: u8,
    },
    /// VADD.F64 Dd, Dn, Dm (S-indices of the low words).
    VaddF64 {
        dd: u8,
        dn: u8,
        dm: u8,
    },
    /// VSUB.F64 Dd, Dn, Dm.
    VsubF64 {
        dd: u8,
        dn: u8,
        dm: u8,
    },
    /// VMUL.F64 Dd, Dn, Dm.
    VmulF64 {
        dd: u8,
        dn: u8,
        dm: u8,
    },
    /// VDIV.F64 Dd, Dn, Dm.
    VdivF64 {
        dd: u8,
        dn: u8,
        dm: u8,
    },

    Unknown(u16),
    Unknown32(u16, u16),
}

/// Low S-register indices (2*Dreg) of the three operands of a double-precision
/// VFP data-processing encoding: `1110 111o oDoo nnnn dddd 1011 NoMo mmmm`.
fn vfp_dp_regs(h1: u16, h2: u16) -> (u8, u8, u8) {
    let d = (h1 >> 6) & 1;
    let n = (h2 >> 7) & 1;
    let m = (h2 >> 5) & 1;
    let vn = (h1 & 0xF) as u8;
    let vd = ((h2 >> 12) & 0xF) as u8;
    let vm = (h2 & 0xF) as u8;
    let dd = (((d as u8) << 4) | vd) << 1;
    let dn = (((n as u8) << 4) | vn) << 1;
    let dm = (((m as u8) << 4) | vm) << 1;
    (dd, dn, dm)
}

/// Expand a VFP modified-immediate 8-bit constant to IEEE-754 single bits.
///
/// ARM ARM (VFP "modified immediate"): bits `abcdefgh` of the imm8 form
/// `a B bbbbb c defgh 000…0` where `B = !b` — i.e.
/// `bits[31]=a`, `bits[30]=!b`, `bits[29:25]=bbbbb`, `bits[24:19]=cdefgh`.
fn vfp_expand_imm8(imm8: u8) -> u32 {
    let a = (imm8 >> 7) & 1;
    let b = (imm8 >> 6) & 1;
    let cdefgh = imm8 & 0x3F;
    let b_inv = b ^ 1;
    ((a as u32) << 31)
        | ((b_inv as u32) << 30)
        | (((b as u32) * 0x1F) << 25)
        | ((cdefgh as u32) << 19)
}

/// Decodes a 16-bit Thumb instruction
pub fn decode_thumb_16(opcode: u16) -> Instruction {
    // 0. Shift (immediate), add, subtract, move, and compare
    // 0.1 Shift (immediate) (T1): 000xx ...
    if (opcode & 0xE000) == 0x0000 {
        let op = (opcode >> 11) & 0x3;
        let imm5 = ((opcode >> 6) & 0x1F) as u8;
        let rm = ((opcode >> 3) & 0x7) as u8;
        let rd = (opcode & 0x7) as u8;

        match op {
            0 => return Instruction::Lsl { rd, rm, imm: imm5 },
            1 => return Instruction::Lsr { rd, rm, imm: imm5 },
            2 => return Instruction::Asr { rd, rm, imm: imm5 },
            _ => {} // Possibly Add/Sub (Register/Imm3) handled later
        }
    }

    // 1. Move Immediate (T1): 0010 0ddd iiii iiii
    if (opcode & 0xE000) == 0x2000 {
        let op = (opcode >> 11) & 0x3;
        let rd = ((opcode >> 8) & 0x7) as u8;
        let imm = (opcode & 0xFF) as u8;

        return match op {
            0 => Instruction::MovImm { rd, imm },     // 00100 = MOV
            1 => Instruction::CmpImm { rn: rd, imm }, // 00101 = CMP
            2 => Instruction::AddImm8 { rd, imm },    // 00110 = ADD
            3 => Instruction::SubImm8 { rd, imm },    // 00111 = SUB
            _ => Instruction::Unknown(opcode),
        };
    }

    // 2. Add/Sub (Register/Imm3) (T1): 0001 1xx ...
    if (opcode & 0xF800) == 0x1800 {
        let op_sub = (opcode >> 9) & 0x3;
        let rm_imm = ((opcode >> 6) & 0x7) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rd = (opcode & 0x7) as u8;

        return match op_sub {
            0 => Instruction::AddReg { rd, rn, rm: rm_imm },
            1 => Instruction::SubReg { rd, rn, rm: rm_imm },
            2 => Instruction::AddImm3 {
                rd,
                rn,
                imm: rm_imm,
            },
            3 => Instruction::SubImm3 {
                rd,
                rn,
                imm: rm_imm,
            },
            _ => unreachable!(),
        };
    }

    // 3. ALU Operations (T1): 0100 00xx ...
    if (opcode & 0xFC00) == 0x4000 {
        let op_alu = (opcode >> 6) & 0xF;
        let rm = ((opcode >> 3) & 0x7) as u8;
        let rd = (opcode & 0x7) as u8;

        return match op_alu {
            0x0 => Instruction::And { rd, rm },        // AND
            0x1 => Instruction::Eor { rd, rm },        // EOR
            0x2 => Instruction::LslReg { rd, rm },     // LSL (register)
            0x3 => Instruction::LsrReg { rd, rm },     // LSR (register)
            0x4 => Instruction::AsrReg { rd, rm },     // ASR (register)
            0x5 => Instruction::Adc { rd, rm },        // ADC
            0x6 => Instruction::Sbc { rd, rm },        // SBC
            0x7 => Instruction::Ror { rd, rm },        // ROR
            0x8 => Instruction::Tst { rn: rd, rm },    // TST
            0x9 => Instruction::Rsbs { rd, rn: rm },   // RSBS Rd, Rn, #0
            0xA => Instruction::CmpReg { rn: rd, rm }, // CMP (register) T1
            0xB => Instruction::Cmn { rn: rd, rm },    // CMN
            0xC => Instruction::Orr { rd, rm },        // ORR
            0xD => Instruction::Mul { rd, rn: rm },    // MUL
            0xE => Instruction::Bic { rd, rm },        // BIC
            0xF => Instruction::Mvn { rd, rm },        // MVN
            _ => Instruction::Unknown(opcode),
        };
    }

    // 3.1 Special Data / Branch Exchange (T1): 0100 01xx ...
    if (opcode & 0xFC00) == 0x4400 {
        let op = (opcode >> 8) & 0x3;
        match op {
            0 => {
                // ADD (register) T2 (High registers)
                let rd = (((opcode >> 4) & 0x8) | (opcode & 0x7)) as u8;
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::AddRegHigh { rd, rm };
            }
            1 => {
                // CMP (register) T2 (High registers)
                let n = ((opcode >> 7) & 0x1) << 3;
                let rn = (n | (opcode & 0x7)) as u8;
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::CmpReg { rn, rm };
            }
            2 => {
                // MOV (register) T1
                let d = ((opcode >> 7) & 0x1) << 3;
                let rd = (d | (opcode & 0x7)) as u8;
                let rm = ((opcode >> 3) & 0xF) as u8;
                return Instruction::MovReg { rd, rm };
            }
            3 => {
                // BX T1 (bit7=0) / BLX T1 (bit7=1)
                let rm = ((opcode >> 3) & 0xF) as u8;
                if (opcode & 0x0080) != 0 {
                    return Instruction::BlxReg { rm };
                } else {
                    return Instruction::Bx { rm };
                }
            }
            _ => return Instruction::Unknown(opcode),
        }
    }

    // 4. Load/Store (Imm5) (T1): 0110 0... -> STR, 0110 1... -> LDR
    // Format: 0110 Liii iinn nttt
    if (opcode & 0xF000) == 0x6000 {
        let is_load = (opcode & 0x0800) != 0;
        let imm5 = ((opcode >> 6) & 0x1F) as u8;
        // The immediate is scaled by 4 for word access
        let imm = imm5 << 2;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;

        if is_load {
            return Instruction::LdrImm { rt, rn, imm }; // 0x68xx
        } else {
            return Instruction::StrImm { rt, rn, imm }; // 0x60xx
        }
    }

    // 4.3 Load/Store Byte (Imm5) (T1): 0111 Liii iinn nttt
    if (opcode & 0xF000) == 0x7000 {
        let is_load = (opcode & 0x0800) != 0;
        let imm = ((opcode >> 6) & 0x1F) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;

        if is_load {
            return Instruction::LdrbImm { rt, rn, imm }; // 0x78xx
        } else {
            return Instruction::StrbImm { rt, rn, imm }; // 0x70xx
        }
    }

    // 4.5 Load/Store Halfword (Imm5) (T1): 1000 Liii iinn nttt
    if (opcode & 0xF000) == 0x8000 {
        let is_load = (opcode & 0x0800) != 0;
        let imm5 = ((opcode >> 6) & 0x1F) as u8;
        // The immediate is scaled by 2 for halfword access
        let imm = imm5 << 1;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;

        if is_load {
            return Instruction::LdrhImm { rt, rn, imm }; // 0x88xx
        } else {
            return Instruction::StrhImm { rt, rn, imm }; // 0x80xx
        }
    }

    // 4.1 LDR Literal (T1): 0100 1ttt iiii iiii
    if (opcode & 0xF800) == 0x4800 {
        let rt = ((opcode >> 8) & 0x7) as u8;
        let imm8 = opcode & 0xFF;
        return Instruction::LdrLit { rt, imm: imm8 << 2 };
    }

    // 4.2 Load/Store Register Offset (T1): 0101 op2 op1 op0 Rm Rn Rt
    // bits[15:9] = 0101 op[2:0]; all 8 ops share the 0101 prefix (bits[15:12]).
    // 0101 000 ... STR   (op=0)
    // 0101 001 ... STRH  (op=1)
    // 0101 010 ... STRB  (op=2)
    // 0101 011 ... LDRSB (op=3)
    // 0101 100 ... LDR   (op=4)
    // 0101 101 ... LDRH  (op=5)
    // 0101 110 ... LDRB  (op=6)
    // 0101 111 ... LDRSH (op=7)
    // The former mask 0xF200 incorrectly required bit 9 (op[0]) to be 0,
    // making the four odd-op forms (STRH/LDRSB/LDRH/LDRSH) fall through to
    // Unknown.  The correct mask is 0xF000, matching only bits[15:12]=0101.
    if (opcode & 0xF000) == 0x5000 {
        let op = (opcode >> 9) & 0x7;
        let rm = ((opcode >> 6) & 0x7) as u8;
        let rn = ((opcode >> 3) & 0x7) as u8;
        let rt = (opcode & 0x7) as u8;
        return match op {
            0 => Instruction::StrReg { rt, rn, rm },
            1 => Instruction::StrhReg { rt, rn, rm },
            2 => Instruction::StrbReg { rt, rn, rm },
            3 => Instruction::LdrsbReg { rt, rn, rm },
            4 => Instruction::LdrReg { rt, rn, rm },
            5 => Instruction::LdrhReg { rt, rn, rm },
            6 => Instruction::LdrbReg { rt, rn, rm },
            7 => Instruction::LdrshReg { rt, rn, rm },
            _ => unreachable!(),
        };
    }

    // 4.2 PUSH/POP
    // PUSH: 1011 010M rrrr rrrr (0xB400)
    if (opcode & 0xFE00) == 0xB400 {
        let m = (opcode & 0x0100) != 0; // LR saved?
        let registers = (opcode & 0xFF) as u8;
        return Instruction::Push { registers, m };
    }
    // POP: 1011 110P rrrr rrrr (0xBC00)
    if (opcode & 0xFE00) == 0xBC00 {
        let p = (opcode & 0x0100) != 0; // PC restored?
        let registers = (opcode & 0xFF) as u8;
        return Instruction::Pop { registers, p };
    }

    // 6. SP-relative Load/Store (T1): 1001 Lttt iiii iiii (0x9000 mask 0xF000)
    // STR: 1001 0... (0x90xx)
    // LDR: 1001 1... (0x98xx)
    if (opcode & 0xF000) == 0x9000 {
        let rt = ((opcode >> 8) & 0x7) as u8;
        let imm8 = opcode & 0xFF;
        // Immediate is scaled by 4
        let imm = imm8 << 2;

        if (opcode & 0x0800) != 0 {
            return Instruction::LdrSp { rt, imm };
        } else {
            return Instruction::StrSp { rt, imm };
        }
    }

    // 6.5 Load/Store Multiple (T1): 1100 Lnnn rrrr rrrr
    if (opcode & 0xF000) == 0xC000 {
        let is_load = (opcode & 0x0800) != 0;
        let rn = ((opcode >> 8) & 0x7) as u8;
        let registers = (opcode & 0xFF) as u8;

        if is_load {
            return Instruction::Ldm { rn, registers }; // 0xC8xx
        } else {
            return Instruction::Stm { rn, registers }; // 0xC0xx
        }
    }

    // 7. Conditional Branch (Bcc): 1101 xxxx iiii iiii
    if (opcode & 0xF000) == 0xD000 {
        let cond = ((opcode >> 8) & 0xF) as u8;
        // cond 0xF (1101 1111 ...) is SVC (supervisor call), not a branch.
        if cond == 0xF {
            return Instruction::Svc {
                imm8: (opcode & 0xFF) as u8,
            };
        }
        // cond 0xE (1101 1110 ...) is UDF — PERMANENTLY UNDEFINED (A7.7.194).
        // The B T1 encoding (A7.7.12) excludes both 1110 and 1111 from `cond`;
        // only 1111 was excluded here, so `UDF #imm8` decoded as a branch with
        // cond = "always" and offset = imm8 << 1. `UDF #0` therefore became a
        // 4-byte forward step that looked exactly like ordinary execution.
        //
        // This is the instruction compilers emit to trap on purpose:
        // `__builtin_trap()`, an unreachable arm, a Rust panic in some
        // configurations. Firmware saying "stop, this must never happen" was
        // being simulated as "jump forward and keep going".
        if cond == 0xE {
            return Instruction::Unknown(opcode);
        }
        let mut offset = (opcode & 0xFF) as i32;
        // Sign extend 8-bit to 32-bit
        if (offset & 0x80) != 0 {
            offset |= !0xFF;
        }
        return Instruction::BranchCond {
            cond,
            offset: offset << 1,
        };
    }

    // 7.1 ADR (T1) / ADD (SP) (T1)
    if (opcode & 0xF000) == 0xA000 {
        let is_add_sp = (opcode & 0x0800) != 0;
        let rd = ((opcode >> 8) & 0x7) as u8;
        let imm8 = opcode & 0xFF;
        let imm = imm8 << 2;
        if is_add_sp {
            return Instruction::AddSpReg { rd, imm };
        } else {
            return Instruction::Adr { rd, imm };
        }
    }

    // 8. Branch (T1/T2)
    // Unconditional Branch T2: 1110 0...
    if (opcode & 0xF800) == 0xE000 {
        let mut offset = (opcode & 0x7FF) as i32;
        if (offset & 0x400) != 0 {
            offset |= !0x7FF;
        }
        return Instruction::Branch {
            offset: offset << 1,
        };
    }

    // 8.1 Misc (T1) (0xBxxx)
    if (opcode & 0xF000) == 0xB000 {
        // REV/REV16/REVSH (T1): 1011 1010 op mmm ddd.
        // newlib strlen emits REV after its word-at-a-time zero-byte scan.
        if (opcode & 0xFF00) == 0xBA00 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return match (opcode >> 6) & 0x3 {
                0b00 => Instruction::Rev { rd, rm },
                0b01 => Instruction::Rev16 { rd, rm },
                0b11 => Instruction::RevSh { rd, rm },
                _ => Instruction::Unknown(opcode),
            };
        }

        // SXTH/SXTB/UXTH/UXTB (T1): 1011 0010 [op2] mmm ddd
        //   op2 = 00 -> SXTH (0xB200..0xB23F)
        //   op2 = 01 -> SXTB (0xB240..0xB27F)
        //   op2 = 10 -> UXTH (0xB280..0xB2BF)
        //   op2 = 11 -> UXTB (0xB2C0..0xB2FF)
        // Bits[15:8] = 1011_0010 are the fixed family bits; bits[7:6]
        // select the operation. Mask 0xFF00 fixes the family.
        if (opcode & 0xFF00) == 0xB200 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return match (opcode >> 6) & 0x3 {
                0b00 => Instruction::Sxth { rd, rm },
                0b01 => Instruction::Sxtb { rd, rm },
                0b10 => Instruction::Uxth { rd, rm },
                _ => Instruction::Uxtb { rd, rm },
            };
        }

        // UXTH (T1): 1011 0010 10 mmm ddd -> 0xB280 base
        if (opcode & 0xFFC0) == 0xB280 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return Instruction::Uxth { rd, rm };
        }

        // SXTH (T1): 1011 0010 00 mmm ddd -> 0xB200 base
        if (opcode & 0xFFC0) == 0xB200 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return Instruction::Sxth { rd, rm };
        }

        // SXTB (T1): 1011 0010 01 mmm ddd -> 0xB240 base
        if (opcode & 0xFFC0) == 0xB240 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return Instruction::Sxtb { rd, rm };
        }

        // REV/REV16/REVSH (T1, 16-bit): 1011 1010 op Rm Rd
        //   op=00 → REV  (0xBA00..0xBA3F)
        //   op=01 → REV16 (0xBA40..0xBA7F)
        //   op=11 → REVSH (0xBAC0..0xBAFF)
        // Distinct from the 32-bit FA90 forms already handled in decode_32.
        if (opcode & 0xFF00) == 0xBA00 {
            let rm = ((opcode >> 3) & 0x7) as u8;
            let rd = (opcode & 0x7) as u8;
            return match (opcode >> 6) & 0x3 {
                0b00 => Instruction::Rev { rd, rm },
                0b01 => Instruction::Rev16 { rd, rm },
                _ => Instruction::RevSh { rd, rm },
            };
        }

        // CBZ/CBNZ (T1): 1011 op i 1 imm5 rn
        if (opcode & 0xF500) == 0xB100 {
            let op = (opcode >> 11) & 1;
            let i = (opcode >> 9) & 1;
            let imm5 = ((opcode >> 3) & 0x1F) as u8;
            let rn = (opcode & 0x7) as u8;
            let imm = ((i << 6) as u8) | (imm5 << 1);
            if op == 0 {
                return Instruction::Cbz { rn, imm };
            } else {
                return Instruction::Cbnz { rn, imm };
            }
        }

        // HINT/IT (T1): 1011 1111 ...
        if (opcode & 0xFF00) == 0xBF00 {
            let cond = ((opcode >> 4) & 0xF) as u8;
            let mask = (opcode & 0xF) as u8;
            if mask != 0 {
                return Instruction::It { cond, mask };
            }
            // Hint space (mask == 0): the `cond` nibble selects the hint.
            // 0x3 = WFI, which we model with a real sleep + idle fast-forward.
            // WFE (0x2)/YIELD (0x1)/SEV (0x4) fall through to Nop: no event
            // register is modelled, so they have no observable effect.
            if cond == 0x3 {
                return Instruction::Wfi;
            }
            return Instruction::Nop;
        }
    }

    // 6. 32-bit Instruction Prefix (0xE800-0xFFFF range, excluding B/BL 16-bit range)
    // 32-bit Thumb instructions start with 111, with bits [12:11] != 00
    if (opcode & 0xE000) == 0xE000 && (opcode & 0x1800) != 0 {
        return Instruction::Unknown(opcode);
    }

    // ADD/SUB SP (T1): 1011 0000 x iii iiii
    if (opcode & 0xFF00) == 0xB000 {
        let is_sub = (opcode & 0x0080) != 0;
        let imm7 = opcode & 0x7F;
        let imm = imm7 << 2;
        if is_sub {
            return Instruction::SubSp { imm };
        } else {
            return Instruction::AddSp { imm };
        }
    }

    // CPS (T1): 1011 0110 011 im 0 0 (A) I F (0xB660 mask 0xFFE8).
    // im (bit 4): 0 = CPSIE (enable/clear), 1 = CPSID (disable/set).
    // I (bit 1): affects PRIMASK. F (bit 0): affects FAULTMASK. Both may be set
    // (`cpsid if`). The f-variant (FAULTMASK) was previously undecoded → Unknown.
    if (opcode & 0xFFE8) == 0xB660 {
        let disable = (opcode & 0x0010) != 0;
        let primask = (opcode & 0x0002) != 0;
        let faultmask = (opcode & 0x0001) != 0;
        if disable {
            return Instruction::Cpsid { primask, faultmask };
        } else {
            return Instruction::Cpsie { primask, faultmask };
        }
    }

    // NOP: 1011 1111 0000 0000 -> 0xBF00
    if opcode == 0xBF00 {
        return Instruction::Nop;
    }

    // BKPT: 1011 1110 imm8
    if (opcode & 0xFF00) == 0xBE00 {
        return Instruction::Bkpt {
            imm8: (opcode & 0xFF) as u8,
        };
    }

    Instruction::Unknown(opcode)
}

#[path = "arm_thumb32.rs"]
mod arm_thumb32;

/// Decodes a 32-bit Thumb instruction (requires two 16-bit halfwords)
pub fn decode_thumb_32(h1: u16, h2: u16) -> Instruction {
    // 32-bit Thumb instruction encoding:
    // First Halfword: 1110 1... or 1111 ...
    //
    // ORDER IS LOAD-BEARING. The encoding checks are first-match-wins and the
    // special patterns must be tested before the greedy generic
    // data-processing matchers, which would otherwise claim them and decode
    // nonsense. The per-group functions below are called in exactly the
    // original block order; where a group's blocks were interleaved with
    // another group in the original chain, the group is split into
    // `_early`/`_mid`/`_late` functions rather than reordered.
    if let Some(i) = arm_thumb32::decode_system(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_long_multiply(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_mul_acc_divide_early(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_vfp_single(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_vfp_double(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_modified_imm_early(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_shifted_reg_early(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_modified_imm_late(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_plain_imm_early(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_shifted_reg_mid(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_bitfield_early(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_plain_imm_late(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_ldst_single(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_bitfield_late(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_extend_misc_early(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_simd(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_dp_shifted_reg_late(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_extend_misc_late(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_ldst_dual_exclusive(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_branch(h1, h2) {
        return i;
    }
    if let Some(i) = arm_thumb32::decode_mul_acc_divide_late(h1, h2) {
        return i;
    }

    Instruction::Unknown32(h1, h2)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ... (existing tests)

    #[test]
    fn test_decode_bfi() {
        // BFI R0, R1, #4, #12
        // Encoding: T1
        // h1 = F361 (Rn=1)
        // h2: 0ii0 dddd iiim mmmm
        // i=0, d=0 (Rd=0)
        // imm3=1 (4>>2), imm2=0 (4&3) -> lsb=4
        // msb = 4+12-1 = 15 = 01111
        // h2 = 0000 0000 0001 01111 -> 0x010F ??
        // No.
        // imm3 is bits 14:12. imm2 bits 7:6.
        // imm3=1 -> 001. d=0 -> 0000.
        // h2 top: 0 001 0 0000 -> 0x10.
        // imm2=0 -> 00. msb=15 -> 01111.
        // h2 bot: 00 01111 -> 0x0F.
        // h2 = 0x100F.

        // Wait, decode logic:
        // lsb = ((h2 >> 12) & 0x7) << 2 | ((h2 >> 6) & 0x3);
        // (0x1<<2)|0 = 4. Correct.
        // msb = h2 & 0x1F = 0xF = 15. Correct.

        assert_eq!(
            decode_thumb_32(0xF361, 0x100F),
            Instruction::Bfi {
                rd: 0,
                rn: 1,
                lsb: 4,
                width: 12
            }
        );
    }

    #[test]
    fn test_decode_bfc() {
        // BFC R2, #8, #16
        // Rn=15 (0xF) -> h1 = F36F
        // lsb=8 -> imm3=2 (8>>2), imm2=0.
        // width=16 -> msb = 8+16-1 = 23 (0x17).
        // imm3=2 -> 010. d=2 -> 0010.
        // h2 top: 0 010 0 0010 -> 0x22..
        // imm2=0 -> 00. msb=23 -> 10111.
        // h2 bot: 00 10111 -> 0x17.
        // h2 = 0x2217.

        assert_eq!(
            decode_thumb_32(0xF36F, 0x2217),
            Instruction::Bfc {
                rd: 2,
                lsb: 8,
                width: 16
            }
        );
    }

    #[test]
    fn test_decode_ubfx() {
        // UBFX R3, R4, #2, #5
        // lsb=2 -> imm3=0, imm2=2.
        // width=5 -> widthm1=4.
        // h2 = 0x0384 (bits 7:6 = 10 -> imm2=2)
        assert_eq!(
            decode_thumb_32(0xF3C4, 0x0384),
            Instruction::Ubfx {
                rd: 3,
                rn: 4,
                lsb: 2,
                width: 5
            }
        );
    }

    #[test]
    fn test_decode_basic_arithmetic() {
        // MOVS R0, #10 (T1) -> 200A
        assert_eq!(
            decode_thumb_16(0x200A),
            Instruction::MovImm { rd: 0, imm: 10 }
        );

        // ADDS R1, #5 (T1) -> 3105
        assert_eq!(
            decode_thumb_16(0x3105),
            Instruction::AddImm8 { rd: 1, imm: 5 }
        );

        // SUBS R2, #3 (T1) -> 3A03
        assert_eq!(
            decode_thumb_16(0x3A03),
            Instruction::SubImm8 { rd: 2, imm: 3 }
        );
    }

    #[test]
    fn test_decode_thumb2_conditional_branch() {
        assert_eq!(
            decode_thumb_32(0xF100, 0x809D),
            Instruction::BranchCond {
                cond: 0x4,
                offset: 0x13A,
            }
        );
    }

    #[test]
    fn test_decode_branch() {
        // B <offset> (T1) -> E000 (offset=0) -> B PC+4
        // E7FE -> B -2 (infinite loop)
        // Offset is imm11 << 1.
        // 0x7FE = 2046. Signed 11-bit: -2.
        assert_eq!(
            decode_thumb_16(0xE7FE),
            Instruction::Branch { offset: -4 } // -2 * 2 = -4
        );

        // B (T2) not implemented in 16-bit decoder, handled in T2?
        // Wait, T2 encoding is 32-bit.
        // T1 is 16-bit.
    }

    #[test]
    fn test_decode_memory_ops() {
        // LDR R0, [R1, #4] (T1) -> 6848
        // imm5 = 1 (4>>2). rn=1. rt=0.
        // 0110 1000 0100 1000 -> 6848
        assert_eq!(
            decode_thumb_16(0x6848),
            Instruction::LdrImm {
                rt: 0,
                rn: 1,
                imm: 4
            }
        );

        // STR R2, [R3, #0] (T1) -> 601A
        // imm5=0. rn=3. rt=2.
        // 0110 0000 0001 1010 -> 601A
        assert_eq!(
            decode_thumb_16(0x601A),
            Instruction::StrImm {
                rt: 2,
                rn: 3,
                imm: 0
            }
        );
    }

    #[test]
    fn test_decode_push_pop() {
        // PUSH {R0, LR} (T1) -> B501
        // 1011 0101 0000 0001
        // m=1 (LR), regs=1 (R0)
        assert_eq!(
            decode_thumb_16(0xB501),
            Instruction::Push {
                registers: 1,
                m: true
            }
        );

        // POP {R1, PC} (T1) -> BD02
        // 1011 1101 0000 0010
        // p=1 (PC), regs=2 (R1)
        assert_eq!(
            decode_thumb_16(0xBD02),
            Instruction::Pop {
                registers: 2,
                p: true
            }
        );
    }

    #[test]
    fn test_decode_misc_rev() {
        // REV R0, R2 (using F081 -> Rd=0)
        assert_eq!(
            decode_thumb_32(0xFA92, 0xF081),
            Instruction::Rev { rd: 0, rm: 2 }
        );
    }

    #[test]
    fn test_decode_simd_uadd8_sel() {
        // Real newlib strlen encodings (from arm-none-eabi objdump):
        //   fa82 f24c  uadd8 r2, r2, ip(12)
        //   faa4 f28c  sel   r2, r4, ip(12)
        assert_eq!(
            decode_thumb_32(0xFA82, 0xF24C),
            Instruction::SimdAddSub8 {
                rd: 2,
                rn: 2,
                rm: 12,
                op: 1
            }
        );
        assert_eq!(
            decode_thumb_32(0xFAA4, 0xF28C),
            Instruction::Sel {
                rd: 2,
                rn: 4,
                rm: 12
            }
        );
        // SADD8 r1,r2,r3 = fa82 f103 ; USUB8 r1,r2,r3 = fac2 f143 ; SSUB8 = fac2 f103
        assert_eq!(
            decode_thumb_32(0xFA82, 0xF103),
            Instruction::SimdAddSub8 {
                rd: 1,
                rn: 2,
                rm: 3,
                op: 0
            }
        );
        assert_eq!(
            decode_thumb_32(0xFAC2, 0xF103),
            Instruction::SimdAddSub8 {
                rd: 1,
                rn: 2,
                rm: 3,
                op: 2
            }
        );
        assert_eq!(
            decode_thumb_32(0xFAC2, 0xF143),
            Instruction::SimdAddSub8 {
                rd: 1,
                rn: 2,
                rm: 3,
                op: 3
            }
        );
    }

    #[test]
    fn test_decode_rev_t1_16bit() {
        // REV T1 (16-bit): `rev r3, r3` = 0xBA1B (0xBA00 | (rm=3<<3) | rd=3).
        assert_eq!(decode_thumb_16(0xBA1B), Instruction::Rev { rd: 3, rm: 3 });
        // REV16 T1: `rev16 r5, r5` = 0xBA6D (0xBA40 | (rm=5<<3) | rd=5).
        assert_eq!(decode_thumb_16(0xBA6D), Instruction::Rev16 { rd: 5, rm: 5 });
        // REVSH T1: `revsh r0, r1` = 0xBAC8 (0xBAC0 | (rm=1<<3) | rd=0).
        assert_eq!(decode_thumb_16(0xBAC8), Instruction::RevSh { rd: 0, rm: 1 });
    }

    #[test]
    fn test_decode_ldrd_negative_offset() {
        // e951 0708 → ldrd r0, r7, [r1, #-32]  (U=0, subtract).
        assert_eq!(
            decode_thumb_32(0xE951, 0x0708),
            Instruction::Ldrd {
                rt: 0,
                rt2: 7,
                rn: 1,
                imm8: 8,
                add_imm: false,
                index: true,
                writeback: false
            }
        );
        // e9d1 0708 → ldrd r0, r7, [r1, #32]  (U=1, add).
        assert_eq!(
            decode_thumb_32(0xE9D1, 0x0708),
            Instruction::Ldrd {
                rt: 0,
                rt2: 7,
                rn: 1,
                imm8: 8,
                add_imm: true,
                index: true,
                writeback: false
            }
        );
    }

    #[test]
    fn test_decode_strd_predec_writeback() {
        // e96d ce04 → strd ip, lr, [sp, #-16]!  (P=1, U=0, W=1).
        // This is the libgcc __aeabi_uldivmod prologue used by mbedTLS bignum.
        assert_eq!(
            decode_thumb_32(0xE96D, 0xCE04),
            Instruction::Strd {
                rt: 12,
                rt2: 14,
                rn: 13,
                imm8: 4,
                add_imm: false,
                index: true,
                writeback: true
            }
        );
    }

    #[test]
    fn test_decode_ldrd_postindex_writeback() {
        // e8f1 2304 → ldrd r2, r3, [r1], #16  (P=0, U=1, W=1).
        assert_eq!(
            decode_thumb_32(0xE8F1, 0x2304),
            Instruction::Ldrd {
                rt: 2,
                rt2: 3,
                rn: 1,
                imm8: 4,
                add_imm: true,
                index: false,
                writeback: true
            }
        );
    }

    #[test]
    fn test_decode_extend_t1() {
        // SXTH R1, R0 -> 0xB201
        assert_eq!(decode_thumb_16(0xB201), Instruction::Sxth { rd: 1, rm: 0 });
        // SXTB R3, R2 -> 0xB253 (B240 | (rm << 3) | rd)
        assert_eq!(decode_thumb_16(0xB253), Instruction::Sxtb { rd: 3, rm: 2 });
        // UXTH R1, R0 -> 0xB281
        assert_eq!(decode_thumb_16(0xB281), Instruction::Uxth { rd: 1, rm: 0 });
        // UXTB R5, R4 -> 0xB2E5 (preserves existing behavior)
        assert_eq!(decode_thumb_16(0xB2E5), Instruction::Uxtb { rd: 5, rm: 4 });
    }

    #[test]
    fn test_decode_rev_t1() {
        // REV r2, r2 = 0xBA12, emitted by newlib strlen.
        assert_eq!(decode_thumb_16(0xBA12), Instruction::Rev { rd: 2, rm: 2 });
        assert_eq!(decode_thumb_16(0xBA52), Instruction::Rev16 { rd: 2, rm: 2 });
        assert_eq!(decode_thumb_16(0xBAD2), Instruction::RevSh { rd: 2, rm: 2 });
    }

    #[test]
    fn test_decode_mul_w_t2() {
        // MUL.W R4, R1, LR -> 0xFB01 0xF40E (Rn=1, Rd=4, Rm=14)
        assert_eq!(
            decode_thumb_32(0xFB01, 0xF40E),
            Instruction::Mul32 {
                rd: 4,
                rn: 1,
                rm: 14
            }
        );
    }

    #[test]
    fn test_decode_shift_reg32_lsl() {
        // LSL.W R2, R1, R2
        assert_eq!(
            decode_thumb_32(0xFA01, 0xF202),
            Instruction::ShiftReg32 {
                rd: 2,
                rn: 1,
                rm: 2,
                shift_type: 0
            }
        );
    }

    #[test]
    fn test_decode_umaal() {
        // UMAAL R0, R1, R2, R3 -> 0xFBE2 0x0163
        assert_eq!(
            decode_thumb_32(0xFBE2, 0x0163),
            Instruction::Umaal {
                rd_lo: 0,
                rd_hi: 1,
                rn: 2,
                rm: 3
            }
        );
    }

    #[test]
    fn test_decode_dataproc32_adc_sbc_rsb() {
        // ADCS R3, R4, R5 -> 0xEB54 0x0305
        assert_eq!(
            decode_thumb_32(0xEB54, 0x0305),
            Instruction::DataProc32 {
                op: 0xA,
                rn: 4,
                rd: 3,
                rm: 5,
                imm5: 0,
                shift_type: 0,
                set_flags: true
            }
        );
        // RSB R2, R0, R1 -> 0xEBC0 0x0201
        assert_eq!(
            decode_thumb_32(0xEBC0, 0x0201),
            Instruction::DataProc32 {
                op: 0xE,
                rn: 0,
                rd: 2,
                rm: 1,
                imm5: 0,
                shift_type: 0,
                set_flags: false
            }
        );
    }

    #[test]
    fn test_decode_dataproc32_eb_prefix() {
        // Pattern seen in H563 path.
        assert_eq!(
            decode_thumb_32(0xEB00, 0x1010),
            Instruction::DataProc32 {
                op: 8,
                rn: 0,
                rd: 0,
                rm: 0,
                imm5: 4,
                shift_type: 1,
                set_flags: false
            }
        );
    }

    #[test]
    fn test_decode_mov_cmp_add_sub_imm8() {
        // MOV R0, #42 -> 0x202A
        assert_eq!(
            decode_thumb_16(0x202A),
            Instruction::MovImm { rd: 0, imm: 42 }
        );
        // CMP R1, #10 -> 0x290A (0010 1001 0000 1010)
        assert_eq!(
            decode_thumb_16(0x290A),
            Instruction::CmpImm { rn: 1, imm: 10 }
        );
        // ADD R2, #5 -> 0x3205
        assert_eq!(
            decode_thumb_16(0x3205),
            Instruction::AddImm8 { rd: 2, imm: 5 }
        );
        // SUB R3, #1 -> 0x3B01
        assert_eq!(
            decode_thumb_16(0x3B01),
            Instruction::SubImm8 { rd: 3, imm: 1 }
        );
    }

    #[test]
    fn test_decode_add_sub_reg_imm3() {
        // ADD R0, R1, R2 -> 0x1888 (0001 100 0 10 001 000)
        assert_eq!(
            decode_thumb_16(0x1888),
            Instruction::AddReg {
                rd: 0,
                rn: 1,
                rm: 2
            }
        );
        // SUB R3, R4, R5 -> 0x1B63 (0001 101 1 01 100 011) ?
        // 0001 101 101 100 011 -> 0x1B63
        // Op=1 (SubReg), Rm=5, Rn=4, Rd=3
        assert_eq!(
            decode_thumb_16(0x1B63),
            Instruction::SubReg {
                rd: 3,
                rn: 4,
                rm: 5
            }
        );

        // ADD R1, R2, #7 -> 0x1DD1 (0001 110 111 010 001)
        assert_eq!(
            decode_thumb_16(0x1DD1),
            Instruction::AddImm3 {
                rd: 1,
                rn: 2,
                imm: 7
            }
        );
        // SUB R0, R0, #1 -> 0x1E40 (0001 111 001 000 000)
        assert_eq!(
            decode_thumb_16(0x1E40),
            Instruction::SubImm3 {
                rd: 0,
                rn: 0,
                imm: 1
            }
        );
    }

    #[test]
    fn test_decode_ldr_str() {
        // STR R0, [R1, #4] -> 0x6048
        // 0110 0 00001 001 000
        // L=0, imm5=1 (so imm=4), Rn=1, Rt=0
        assert_eq!(
            decode_thumb_16(0x6048),
            Instruction::StrImm {
                rt: 0,
                rn: 1,
                imm: 4
            }
        );

        // LDR R2, [R3, #0] -> 0x681A
        // 0110 1 00000 011 010
        // L=1, imm5=0, Rn=3, Rt=2
        assert_eq!(
            decode_thumb_16(0x681A),
            Instruction::LdrImm {
                rt: 2,
                rn: 3,
                imm: 0
            }
        );
    }

    #[test]
    fn test_decode_alu() {
        // AND R0, R1 -> 0x4008 (0100 00 0000 001 000)
        assert_eq!(decode_thumb_16(0x4008), Instruction::And { rd: 0, rm: 1 });
        // ORR R2, R3 -> 0x431A (0100 00 1100 011 010)
        assert_eq!(decode_thumb_16(0x431A), Instruction::Orr { rd: 2, rm: 3 });
        // EOR R4, R5 -> 0x406C (0100 00 0001 101 100)
        assert_eq!(decode_thumb_16(0x406C), Instruction::Eor { rd: 4, rm: 5 });
        // BIC R1, R2 -> 0x4391 (0100 00 1110 010 001)
        assert_eq!(decode_thumb_16(0x4391), Instruction::Bic { rd: 1, rm: 2 });
        // MVN R6, R7 -> 0x43FE (0100 00 1111 111 110)
        assert_eq!(decode_thumb_16(0x43FE), Instruction::Mvn { rd: 6, rm: 7 });
    }

    #[test]
    fn test_decode_stack_control() {
        // PUSH {R0, LR} -> 0xB501 (1011 0101 0000 0001)
        // M=1, Regs=0x01
        assert_eq!(
            decode_thumb_16(0xB501),
            Instruction::Push {
                registers: 1,
                m: true
            }
        );

        // POP {R1, PC} -> 0xBD02 (1011 1101 0000 0010)
        // P=1, Regs=0x02
        assert_eq!(
            decode_thumb_16(0xBD02),
            Instruction::Pop {
                registers: 2,
                p: true
            }
        );

        // BX R14 -> 0x4770 (0100 0111 0111 0000)
        // Rm=14 (LR)
        assert_eq!(decode_thumb_16(0x4770), Instruction::Bx { rm: 14 });
    }

    #[test]
    fn test_decode_sp_rel() {
        // STR R0, [SP, #0] -> 0x9000 (1001 0 000 00000000)
        assert_eq!(
            decode_thumb_16(0x9000),
            Instruction::StrSp { rt: 0, imm: 0 }
        );

        // LDR R1, [SP, #4] -> 0x9901 (1001 1 001 00000001)
        // imm8=1, scaled*4 = 4.
        assert_eq!(
            decode_thumb_16(0x9901),
            Instruction::LdrSp { rt: 1, imm: 4 }
        );
    }

    #[test]
    fn test_decode_cond_branch() {
        // BNE +4 (Target PC+4+4)
        // Encoding: 1101 0001 0000 0001 -> 0xD101
        // Cond=1 (NE), imm8=1. Offset = 1<<1 = 2.
        assert_eq!(
            decode_thumb_16(0xD101),
            Instruction::BranchCond { cond: 1, offset: 2 }
        );

        // BEQ -4 (0xFD) -> 0xD0FD
        // Cond=0 (EQ), imm8=FD (-3). Offset = -3<<1 = -6.
        assert_eq!(
            decode_thumb_16(0xD0FD),
            Instruction::BranchCond {
                cond: 0,
                offset: -6
            }
        );
    }

    #[test]
    fn test_decode_svc() {
        // SVC #2 -> 1101 1111 0000 0010 = 0xDF02. The 0xF cond field in the
        // 0xD000 block is the supervisor call, not a conditional branch.
        assert_eq!(decode_thumb_16(0xDF02), Instruction::Svc { imm8: 2 });
        // Full imm8 range.
        assert_eq!(decode_thumb_16(0xDF00), Instruction::Svc { imm8: 0 });
        assert_eq!(decode_thumb_16(0xDFFF), Instruction::Svc { imm8: 255 });
    }

    #[test]
    fn test_decode_wide_cond_branch_t3() {
        // B<cond>.W (T3, 32-bit). The first halfword shares its top bits with the
        // plain-immediate data-processing group (MOVW/MOVT/ADDW/SUBW/SBFX), so the
        // decoder must only pick those when h2[15]==0. With h2[15]==1 these must
        // decode as conditional branches. Regression: BLS.W was mis-decoded as
        // MOVW (h1=0xF240) and silently never branched — see the Nokia paddle bug.

        // BLS.W +452: F240 80E2 (cond=LS=9). Real encoding from the invaders ELF.
        assert_eq!(
            decode_thumb_32(0xF240, 0x80E2),
            Instruction::BranchCond {
                cond: 0x9,
                offset: 452
            }
        );
        // BHI.W (cond=HI=8) shares h1 with ADDW (0xF200).
        assert_eq!(
            decode_thumb_32(0xF200, 0x8000),
            Instruction::BranchCond {
                cond: 0x8,
                offset: 0
            }
        );
        // BLE.W (cond=LE=D) shares h1 with SBFX (0xF340).
        assert_eq!(
            decode_thumb_32(0xF340, 0x8000),
            Instruction::BranchCond {
                cond: 0xD,
                offset: 0
            }
        );
        // The same h1 with h2[15]==0 must still decode as the data-processing op.
        assert_eq!(
            decode_thumb_32(0xF240, 0x0000),
            Instruction::Movw { rd: 0, imm: 0 }
        );
    }

    #[test]
    fn test_decode_nop() {
        assert_eq!(decode_thumb_16(0xBF00), Instruction::Nop);
    }

    #[test]
    fn test_decode_shifts() {
        // LSLS R0, R1, #2 -> 0x0088 (000 00 00010 001 000)
        assert_eq!(
            decode_thumb_16(0x0088),
            Instruction::Lsl {
                rd: 0,
                rm: 1,
                imm: 2
            }
        );
        // LSRS R2, R3, #4 -> 0x091A (000 01 00100 011 010)
        assert_eq!(
            decode_thumb_16(0x091A),
            Instruction::Lsr {
                rd: 2,
                rm: 3,
                imm: 4
            }
        );
        // ASRS R4, R5, #6 -> 0x11AC (000 10 00110 101 100)
        assert_eq!(
            decode_thumb_16(0x11AC),
            Instruction::Asr {
                rd: 4,
                rm: 5,
                imm: 6
            }
        );

        // LSLS R0, R0, #0 (Opcode 0x0000)
        assert_eq!(
            decode_thumb_16(0x0000),
            Instruction::Lsl {
                rd: 0,
                rm: 0,
                imm: 0
            }
        );
    }

    #[test]
    fn test_decode_cmp_reg() {
        // CMP R1, R0 -> 0x4281 (0100 0010 10 000 001)
        assert_eq!(
            decode_thumb_16(0x4281),
            Instruction::CmpReg { rn: 1, rm: 0 }
        );
    }

    #[test]
    fn test_decode_mov_reg() {
        // MOV R7, SP -> 0x466F (0100 0110 0110 1111)
        // Rd=7, Rm=13 (SP)
        assert_eq!(
            decode_thumb_16(0x466F),
            Instruction::MovReg { rd: 7, rm: 13 }
        );
    }

    #[test]
    fn test_decode_ldrb_strb_imm() {
        // STRB R1, [R0, #0] -> 0x7001 (0111 0 00000 000 001)
        assert_eq!(
            decode_thumb_16(0x7001),
            Instruction::StrbImm {
                rt: 1,
                rn: 0,
                imm: 0
            }
        );
        // LDRB R1, [R0, #0] -> 0x7801 (0111 1 00000 000 001)
        assert_eq!(
            decode_thumb_16(0x7801),
            Instruction::LdrbImm {
                rt: 1,
                rn: 0,
                imm: 0
            }
        );
    }

    // --- Thumb-1 load/store register-offset (ARMv7-M A6.2.4) ---
    // Encoding: 0101 op2 op1 op0 Rm[2:0] Rn[2:0] Rt[2:0]
    // All 8 op values; op[0]==1 forms were gated out by the wrong 0xF200 mask.

    #[test]
    fn test_decode_reg_offset_even_ops_work() {
        // Verify the four even ops (op[0]==0) already decode correctly.
        // Using Rt=0, Rn=1, Rm=2 for all:
        // op=000 STR:  0101 000 010 001 000 = 0x5088
        assert_eq!(
            decode_thumb_16(0x5088),
            Instruction::StrReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
        // op=010 STRB: 0101 010 010 001 000 = 0x5488
        assert_eq!(
            decode_thumb_16(0x5488),
            Instruction::StrbReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
        // op=100 LDR:  0101 100 010 001 000 = 0x5888
        assert_eq!(
            decode_thumb_16(0x5888),
            Instruction::LdrReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
        // op=110 LDRB: 0101 110 010 001 000 = 0x5C88
        assert_eq!(
            decode_thumb_16(0x5C88),
            Instruction::LdrbReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
    }

    #[test]
    fn test_decode_strh_reg_offset() {
        // STRH Rt,[Rn,Rm] — op=001
        // Rt=0, Rn=1, Rm=2: 0101 001 010 001 000 = 0x5288
        assert_eq!(
            decode_thumb_16(0x5288),
            Instruction::StrhReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
    }

    #[test]
    fn test_decode_ldrsb_reg_offset() {
        // LDRSB Rt,[Rn,Rm] — op=011
        // Rt=0, Rn=1, Rm=2: 0101 011 010 001 000 = 0x5688
        assert_eq!(
            decode_thumb_16(0x5688),
            Instruction::LdrsbReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
    }

    #[test]
    fn test_decode_ldrh_reg_offset() {
        // LDRH Rt,[Rn,Rm] — op=101
        // Rt=0, Rn=1, Rm=2: 0101 101 010 001 000 = 0x5A88
        assert_eq!(
            decode_thumb_16(0x5A88),
            Instruction::LdrhReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
    }

    #[test]
    fn test_decode_ldrsh_reg_offset() {
        // LDRSH Rt,[Rn,Rm] — op=111
        // Rt=0, Rn=1, Rm=2: 0101 111 010 001 000 = 0x5E88
        assert_eq!(
            decode_thumb_16(0x5E88),
            Instruction::LdrshReg {
                rt: 0,
                rn: 1,
                rm: 2
            }
        );
    }
}
