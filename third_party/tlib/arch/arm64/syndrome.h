/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>

#include "bit_helper.h"

//  D17.2.37
#define SYN_A64_COND 14  //  b1110
#define SYN_A64_CV   1

#define SYN_EC_SHIFT 26
#define SYN_EC_WIDTH 6
#define SYN_IL       1 << 25

#define SYN_DATA_ABORT_ISV   1 << 24
#define SYN_DATA_ABORT_S1PTW 1 << 7

//  Based on EC field description in D17.2.37-39.
typedef enum {
    SYN_EC_UNKNOWN_REASON = 0x00,
    SYN_EC_TRAPPED_WF = 0x01,
    SYN_EC_AA32_TRAPPED_MCR_MCR_CP15 = 0x03,
    SYN_EC_AA32_TRAPPED_MCRR_MRRC_CP15 = 0x04,
    SYN_EC_AA32_TRAPPED_MCR_MRC_CP14 = 0x05,
    SYN_EC_AA32_TRAPPED_LDC_STC = 0x06,
    SYN_EC_TRAPPED_SME_SVE_SIMD_FP = 0x07,
    SYN_EC_TRAPPED_VMRS = 0x08,  //  AKA FPID access trap
    SYN_EC_TRAPPED_PAUTH_USE = 0x09,
    SYN_EC_TRAPPED_LD64B_OR_ST64B = 0x0A,
    SYN_EC_AA32_TRAPPED_MRRC_CP14 = 0x0C,
    SYN_EC_BRANCH_TARGET = 0x0D,
    SYN_EC_ILLEGAL_EXECUTION_STATE = 0x0E,
    SYN_EC_AA32_SVC = 0x11,
    SYN_EC_AA32_HVC = 0x12,
    SYN_EC_AA32_SMC = 0x13,
    SYN_EC_AA64_SVC = 0x15,
    SYN_EC_AA64_HVC = 0x16,
    SYN_EC_AA64_SMC = 0x17,
    SYN_EC_TRAPPED_MSR_MRS_SYSTEM_INST = 0x18,
    SYN_EC_TRAPPED_SVE = 0x19,
    SYN_EC_TRAPPED_ERET_ERETAA_ERETAB = 0x1A,
    SYN_EC_TME_TSTART = 0x1B,
    SYN_EC_POINTER_AUTHENTICATION = 0x1C,
    SYN_EC_TRAPPED_SME = 0x1D,
    SYN_EC_GRANULE_PROTECTION_CHECK = 0x1E,
    SYN_EC_IMPLEMENTATION_DEFINED_EL3 = 0x1F,
    SYN_EC_INSTRUCTION_ABORT_LOWER_EL = 0x20,
    SYN_EC_INSTRUCTION_ABORT_SAME_EL = 0x21,
    SYN_EC_PC_ALIGNMENT_FAULT = 0x22,
    SYN_EC_DATA_ABORT_LOWER_EL = 0x24,
    SYN_EC_DATA_ABORT_SAME_EL = 0x25,
    SYN_EC_SP_ALIGNMENT_FAULT = 0x26,
    SYN_EC_MEMORY_OPERATION = 0x27,
    SYN_EC_AA32_TRAPPED_FLOATING_POINT = 0x28,
    SYN_EC_AA64_TRAPPED_FLOATING_POINT = 0x2C,
    SYN_EC_SERROR = 0x2F,
    SYN_EC_BREAKPOINT_LOWER_EL = 0x30,
    SYN_EC_BREAKPOINT_SAME_EL = 0x31,
    SYN_EC_SOFTWARESTEP_LOWER_EL = 0x32,
    SYN_EC_SOFTWARESTEP_SAME_EL = 0x33,
    SYN_EC_WATCHPOINT_LOWER_EL = 0x34,
    SYN_EC_WATCHPOINT_SAME_EL = 0x35,
    SYN_EC_AA32_BKPT = 0x38,
    SYN_EC_AA32_VECTOR_CATCH = 0x3A,
    SYN_EC_AA64_BKPT = 0x3C,
} SyndromeExceptionClass;

//  Based on descriptions of the DFSC and IFSC fields in "ISS encoding for an exception from a Data Abort"
//  and "ISS encoding for an exception from an Instruction Abort" (D17.2.37-39).
//
//  Codes for level=1..3 aren't listed since those are always 'SYN_*_LEVEL_0 + level'.
typedef enum {
    SYN_FAULT_ADDRESS_SIZE_LEVEL_0 = 0x00,
    SYN_FAULT_TRANSLATION_LEVEL_0 = 0x04,
    SYN_FAULT_ACCESS_FLAG_LEVEL_0 = 0x08,
    SYN_FAULT_PERMISSION_LEVEL_0 = 0x0C,
    SYN_FAULT_EXTERNAL_NO_LEVEL = 0x10,  //  "not on translation table walk or hardware update of translation table"
    SYN_FAULT_SYNC_TAG_CHECK = 0x11,
    SYN_FAULT_EXTERNAL_LEVEL_0 = 0x14,
    SYN_FAULT_ECC_PARITY_NO_LEVEL = 0x18,  //  "not on translation table walk or hardware update of translation table"
    SYN_FAULT_ECC_PARITY_LEVEL_0 = 0x1C,
    SYN_FAULT_ALIGNMENT = 0x21,
    SYN_FAULT_DEBUG_EXCEPTION = 0x22,
    SYN_FAULT_TLB_CONFLICT = 0x30,
    SYN_FAULT_UNSUPPORTED_ATOMIC_HW_UPDATE = 0x31,
    SYN_FAULT_IMPLEMENTATION_DEFINED_0x34 = 0x34,  //  "(Lockdown)"
    SYN_FAULT_IMPLEMENTATION_DEFINED_0x35 = 0x35,  //  "(Unsupported Exclusive or Atomic access)"
} ISSFaultStatusCode;

static inline uint64_t syndrome64_create(uint64_t instruction_specific_syndrome2, SyndromeExceptionClass exception_class,
                                         uint32_t instruction_length, uint32_t instruction_specific_syndrome)
{
    tlib_assert(instruction_specific_syndrome2 < (1 << 5));
    tlib_assert(exception_class < (1 << 6));
    tlib_assert(instruction_length < (1 << 1));
    tlib_assert(instruction_specific_syndrome < (1 << 25));

    //  D17-5658: Some of the exceptions should always have IL bit set to 1.
    switch(exception_class) {
        case SYN_EC_SERROR:
        case SYN_EC_INSTRUCTION_ABORT_LOWER_EL:
        case SYN_EC_INSTRUCTION_ABORT_SAME_EL:
        case SYN_EC_PC_ALIGNMENT_FAULT:
        case SYN_EC_SP_ALIGNMENT_FAULT:
        case SYN_EC_ILLEGAL_EXECUTION_STATE:
        case SYN_EC_SOFTWARESTEP_LOWER_EL:
        case SYN_EC_SOFTWARESTEP_SAME_EL:
        case SYN_EC_AA64_BKPT:
        case SYN_EC_UNKNOWN_REASON:
            tlib_assert(instruction_length);
            break;
        case SYN_EC_DATA_ABORT_LOWER_EL:
        case SYN_EC_DATA_ABORT_SAME_EL:
            //  "A Data Abort exception for which the value of the ISV bit is 0."
            if(!(instruction_specific_syndrome & SYN_DATA_ABORT_ISV)) {
                tlib_assert(instruction_length);
            }
            break;
        default:
            break;
    }
    return instruction_specific_syndrome2 << 32 | exception_class << SYN_EC_SHIFT | (instruction_length ? SYN_IL : 0) |
           instruction_specific_syndrome;
}

static inline uint32_t syndrome32_create(SyndromeExceptionClass exception_class, uint32_t instruction_length,
                                         uint32_t instruction_specific_syndrome)
{
    return (uint32_t)syndrome64_create(0, exception_class, instruction_length, instruction_specific_syndrome);
}

static inline uint32_t syn_aa64_sysregtrap(uint32_t op0, uint32_t op1, uint32_t op2, uint32_t crn, uint32_t crm, uint32_t rt,
                                           bool isread)
{
    //  D17.2.37
    uint32_t iss = SYN_A64_CV << 24 | SYN_A64_COND << 20 | op2 << 17 | op1 << 14 | crn << 10 | rt << 5 | crm << 1 | isread;

    //  TODO: iss2 if FEAT_LS64 is implemented
    return syndrome32_create(SYN_EC_TRAPPED_MSR_MRS_SYSTEM_INST, 0, iss);
}

static inline uint32_t syn_uncategorized()
{
    //  D17.2.37, D17-5659
    return syndrome32_create(SYN_EC_UNKNOWN_REASON, 1, 0);
}

static inline uint32_t syn_data_abort_with_iss(bool same_el, uint32_t access_size, bool sign_extend, uint32_t insn_rt,
                                               bool is_64bit_gpr_ldst, bool acquire_or_release, uint32_t set, bool cm, bool s1ptw,
                                               bool wnr, ISSFaultStatusCode dfsc, bool is_16bit)
{
    SyndromeExceptionClass ec = same_el ? SYN_EC_DATA_ABORT_SAME_EL : SYN_EC_DATA_ABORT_LOWER_EL;

    //  IL bit is 0 for 16-bit and 1 for 32-bit instruction trapped.
    bool il = !is_16bit;

    //  When adding support for external aborts, make sure Synchronous Error Type is provided through the 'set' argument.
    //  The field is used as LST for FEAT_LS64, a currently unsupported ARMv8.7 extension.
    //
    //  Also check if 'fnv' and 'ea_type' fields are set correctly.
    tlib_assert(dfsc != SYN_FAULT_EXTERNAL_NO_LEVEL);

    //  "FAR not valid" which can only be set for 'SYN_FAULT_EXTERNAL_NO_LEVEL' fault.
    //  Let's assume FAR is always valid for such an abort so FnV will always be 0.
    bool fnv = 0;

    //  An implementation-defined classification of External aborts.
    bool ea_type = 0;

    //  It's RES0 if FEAT_NV2, a currently unsupported ARMv8.4 extension, is unimplemented.
    bool vncr = 0;

    uint32_t iss = SYN_DATA_ABORT_ISV | access_size << 22 | sign_extend << 21 | insn_rt << 16 | is_64bit_gpr_ldst << 15 |
                   acquire_or_release << 14 | vncr << 13 | set << 11 | fnv << 10 | ea_type << 9 | cm << 8 | s1ptw << 7 |
                   wnr << 6 | dfsc;

    return syndrome32_create(ec, il, iss);
}

//  "No ISS" in the name only applies to ISS[23:14] bits (ISV=0 case).
static inline uint32_t syn_data_abort_no_iss(bool same_el, bool fnv, bool ea, bool cm, bool s1ptw, bool wnr,
                                             ISSFaultStatusCode dfsc)
{
    SyndromeExceptionClass ec = same_el ? SYN_EC_DATA_ABORT_SAME_EL : SYN_EC_DATA_ABORT_LOWER_EL;

    //  D17-5658: IL bit is 1 for "A Data Abort exception for which the value of the ISV bit is 0".
    bool il = 1;

    //  Notice no ISV and instruction-specific bits (11:23).
    uint32_t iss = fnv << 10 | ea << 9 | cm << 8 | s1ptw << 7 | wnr << 6 | dfsc;

    return syndrome32_create(ec, il, iss);
}

static inline uint32_t syn_fp_access_trap(uint32_t cv, uint32_t cond, bool is_16bit, uint32_t coproc)
{
    return syndrome32_create(SYN_EC_TRAPPED_SME_SVE_SIMD_FP, !is_16bit, (cv << 24) | (cond << 20) | coproc);
}

static inline uint32_t syn_instruction_abort(bool same_el, bool s1ptw, ISSFaultStatusCode ifsc)
{
    SyndromeExceptionClass ec = same_el ? SYN_EC_INSTRUCTION_ABORT_SAME_EL : SYN_EC_INSTRUCTION_ABORT_LOWER_EL;
    uint32_t iss = s1ptw << 7 | ifsc;
    return syndrome32_create(ec, 1, iss);
}

static inline SyndromeExceptionClass syn_get_ec(uint64_t syndrome)
{
    return extract64(syndrome, SYN_EC_SHIFT, SYN_EC_WIDTH);
}

static inline uint32_t syn_wfx(int cv, int cond, int ti, bool is_16bit)
{
    uint32_t iss = cv << 24 | cond << 20 | ti;

    return syndrome32_create(SYN_EC_TRAPPED_WF, is_16bit, iss);
}

static inline uint32_t syn_aa64_hvc(uint32_t imm16)
{
    return syndrome32_create(SYN_EC_AA64_HVC, 1, imm16);
}

static inline uint32_t syn_aa64_smc(uint32_t imm16)
{
    return syndrome32_create(SYN_EC_AA64_SMC, 1, imm16);
}

static inline uint32_t syn_aa64_svc(uint32_t imm16)
{
    return syndrome32_create(SYN_EC_AA64_SVC, 1, imm16);
}

static inline uint32_t syn_aa64_bkpt(uint32_t comment)
{
    return syndrome32_create(SYN_EC_AA64_BKPT, 1, comment);
}

static inline uint32_t syn_btitrap(uint32_t btype)
{
    return syndrome32_create(SYN_EC_BRANCH_TARGET, 0, btype);
}

static inline uint32_t syn_illegalstate()
{
    return syndrome32_create(SYN_EC_ILLEGAL_EXECUTION_STATE, 1, 0);
}

static inline uint32_t syn_smetrap(uint32_t smtc, bool is_16bit)
{
    return syndrome32_create(SYN_EC_TRAPPED_SME, is_16bit, smtc);
}

static inline uint32_t syn_sve_access_trap()
{
    return syndrome32_create(SYN_EC_TRAPPED_SVE, 1, 0);
}

static inline uint32_t syn_swstep(bool same_el, uint32_t isv, uint32_t ex)
{
    SyndromeExceptionClass ec = same_el ? SYN_EC_SOFTWARESTEP_SAME_EL : SYN_EC_SOFTWARESTEP_LOWER_EL;
    uint32_t iss = isv << 24 | ex << 6 | SYN_FAULT_DEBUG_EXCEPTION;
    return syndrome32_create(ec, 1, iss);
}

static inline uint32_t syn_serror()
{
    return syndrome32_create(SYN_EC_SERROR, 1, 0);
}

static inline uint32_t syn_aa32_svc(uint32_t imm16, bool is_16bit)
{
    return syndrome32_create(SYN_EC_AA32_SVC, !is_16bit, imm16);
}

static inline uint32_t syn_aa32_hvc(uint32_t imm16)
{
    return syndrome32_create(SYN_EC_AA32_HVC, 1, imm16);
}

static inline uint32_t syn_aa32_smc(void)
{
    return syndrome32_create(SYN_EC_AA32_SMC, 1, 0);
}

static inline uint32_t syn_aa32_bkpt(uint32_t imm16, bool is_16bit)
{
    return syndrome32_create(SYN_EC_AA32_BKPT, !is_16bit, imm16);
}

static inline uint32_t syn_rt_trap(SyndromeExceptionClass ec, bool cond_valid, uint32_t cond, uint32_t opc1, uint32_t opc2,
                                   uint32_t crn, uint32_t crm, uint32_t rt, bool is_read, bool is_16bit)
{
    uint32_t iss = cond_valid << 24 | cond << 20 | opc2 << 17 | opc1 << 14 | crn << 10 | rt << 5 | crm << 1 | is_read;
    return syndrome32_create(ec, !is_16bit, iss);
}

static inline uint32_t syn_rrt_trap(SyndromeExceptionClass ec, bool cond_valid, uint32_t cond, uint32_t opc1, uint32_t crm,
                                    uint32_t rt, uint32_t rt2, bool is_read, bool is_16bit)
{
    uint32_t iss = cond_valid << 24 | cond << 20 | opc1 << 16 | /*res*/ 0 << 15 | rt2 << 10 | rt << 5 | crm << 1 | is_read;
    return syndrome32_create(ec, !is_16bit, iss);
}

static inline uint32_t syn_cp15_rrt_trap(bool cond_valid, uint32_t cond, uint32_t opc1, uint32_t crm, uint32_t rt, uint32_t rt2,
                                         bool is_read, bool is_16bit)
{
    return syn_rrt_trap(SYN_EC_AA32_TRAPPED_MCRR_MRRC_CP15, cond_valid, cond, opc1, crm, rt, rt2, is_read, is_16bit);
}

static inline uint32_t syn_cp14_rrt_trap(bool cond_valid, uint32_t cond, uint32_t opc1, uint32_t crm, uint32_t rt, uint32_t rt2,
                                         bool is_read, bool is_16bit)
{
    return syn_rrt_trap(SYN_EC_AA32_TRAPPED_MRRC_CP14, cond_valid, cond, opc1, crm, rt, rt2, is_read, is_16bit);
}

static inline uint32_t syn_cp15_rt_trap(bool cond_valid, uint32_t cond, uint32_t opc1, uint32_t opc2, uint32_t crn, uint32_t crm,
                                        uint32_t rt, bool is_read, bool is_16bit)
{
    return syn_rt_trap(SYN_EC_AA32_TRAPPED_MCR_MCR_CP15, cond_valid, cond, opc1, opc2, crn, crm, rt, is_read, is_16bit);
}

static inline uint32_t syn_cp14_rt_trap(bool cond_valid, uint32_t cond, uint32_t opc1, uint32_t opc2, uint32_t crn, uint32_t crm,
                                        uint32_t rt, bool is_read, bool is_16bit)
{
    return syn_rt_trap(SYN_EC_AA32_TRAPPED_MCR_MRC_CP14, cond_valid, cond, opc1, opc2, crn, crm, rt, is_read, is_16bit);
}
