/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include <limits.h>

#include "cpu.h"
#include "helper.h"
#include "mmu.h"
#include "syndrome.h"
#include "system_registers.h"

uint64_t arm_sctlr(struct CPUState *env, int el)
{
    tlib_assert(el >= 0 && el <= 3);
    if(el == 0) {
        el = arm_is_el2_enabled(env) && are_hcr_e2h_and_tge_set(arm_hcr_el2_eff(env)) ? 2 : 1;
    }
    return env->cp15.sctlr_el[el];
}

void HELPER(exception_bkpt_insn)(CPUARMState *env, uint32_t syndrome)
{
    helper_exception_with_syndrome(env, EXCP_BKPT, syndrome);
}

void HELPER(sysreg_tlb_flush)(CPUARMState *env, void *info_ptr)
{
    //  TODO: Use register info to flush precisely.
    const ARMCPRegInfo *info = info_ptr;
    (void)info;

    tlb_flush(env, 1, true);
}

/* Functions called by arch-independent code. */

void cpu_exec_epilogue(CPUState *env)
{
    //  Intentionally left blank.
}

void cpu_exec_prologue(CPUState *env)
{
    //  Intentionally left blank.
}

void cpu_reset(CPUState *env)
{
    cpu_reset_state(env);
    cpu_reset_vfp(env);
    system_instructions_and_registers_reset(env);

    //  TODO? 64-bit ARMv8 can start with AArch32 based on the AA64nAA32 configuration signal.
    if(arm_feature(env, ARM_FEATURE_AARCH64)) {
        cpu_reset_v8_a64(env);
    } else {
        cpu_reset_v8_a32(env);
    }
    arm_rebuild_hflags(env);
}

void cpu_after_load(CPUState *env)
{
    //  Trigger tlib_on_execution_mode_changed callback to allow subscribed cores
    //  on the managed part update their Exception Level and Security State.
    //  Otherwise cores with two security states like ARMv8A
    //  will incorrectly update their state to the default after reset,
    //  while the correct state is visible only after loading the state.
    //  To circumvent it, we explicitly trigger callback after loading the state.
    tlib_on_execution_mode_changed(arm_current_el(env), arm_is_secure(env));
}

void do_interrupt(CPUState *env)
{
    if(env->interrupt_begin_callback_enabled) {
        tlib_on_interrupt_begin(env->exception_index);
    }

    if(arm_el_is_aa64(env, env->exception.target_el)) {
        do_interrupt_a64(env);
    } else {
        do_interrupt_a32(env);
    }
}

int process_interrupt(int interrupt_request, CPUState *env)
{
    //  CPU_INTERRUPT_EXITTB is handled in arch-independent code.
    if(interrupt_request & CPU_INTERRUPT_EXITTB || tlib_is_in_debug_mode()) {
        return 0;
    }

    return process_interrupt_v8a(interrupt_request, env);
}

void tlib_arch_dispose()
{
    ttable_remove(cpu->cp_regs);
}

/* CPU initialization and reset. */

#include "cpu_names.h"

/* Pre-v8 ARM */

void cpu_init_a9(CPUState *env, uint32_t id)
{
    assert(id == ARM_CPUID_CORTEXA9);

    //  The features and initialization are based on cpu_reset_model_id in arm/helper.c
    set_feature(env, ARM_FEATURE_V4T);
    set_feature(env, ARM_FEATURE_V5);
    set_feature(env, ARM_FEATURE_V6);
    set_feature(env, ARM_FEATURE_V6K);
    set_feature(env, ARM_FEATURE_V7);
    set_feature(env, ARM_FEATURE_VBAR);
    set_feature(env, ARM_FEATURE_AUXCR);
    set_feature(env, ARM_FEATURE_THUMB2);
    set_feature(env, ARM_FEATURE_NEON);
    set_feature(env, ARM_FEATURE_THUMB2EE);
    /* Note that A9 supports the MP extensions even for
     * A9UP and single-core A9MP (which are both different
     * and valid configurations; we don't model A9UP).
     */
    set_feature(env, ARM_FEATURE_V7MP);

    //  Extra flags/features added when porting to arm64 tlib
    set_feature(env, ARM_FEATURE_A);
    set_feature(env, ARM_FEATURE_THUMB_DSP);
    set_feature(env, ARM_FEATURE_CBAR_RO);
    set_feature(env, ARM_FEATURE_PMU);
    set_feature(env, ARM_FEATURE_MVFR);

    //  Initial register values, adapted from arm/helper.c and extended
    //  Reference manuals:
    //  CA9     R0P1: DDI0388B
    //  CA9MP   R0P1: DDI0407B
    //  CA9 FPU R0P0: DDI0408B

    //  From CA9 3.1.1
    env->arm_core_config.reset_sctlr = 0x00C50078;
    env->arm_core_config.mpidr =
        1 << 31;  //  High bit always set (3.3.5), rest handled in system_registers.c depending on MP index
    env->arm_core_config.isar.id_pfr0 = 0x00001231;
    env->arm_core_config.isar.id_pfr1 = 0x00000011;
    env->arm_core_config.isar.id_dfr0 = 0x00010444;
    env->arm_core_config.isar.id_mmfr0 = 0x00100103;
    env->arm_core_config.isar.id_mmfr1 = 0x20000000;
    env->arm_core_config.isar.id_mmfr2 = 0x01230000;
    env->arm_core_config.isar.id_mmfr3 = 0x00002111;
    env->arm_core_config.isar.id_isar0 = 0x00101111;
    env->arm_core_config.isar.id_isar1 = 0x13112111;
    env->arm_core_config.isar.id_isar2 = 0x21232041;
    env->arm_core_config.isar.id_isar3 = 0x11112131;
    env->arm_core_config.isar.id_isar4 = 0x00011142;
    env->arm_core_config.ctr = 0x80038003;
    env->arm_core_config.clidr = 0x09000003;

    //  From arm/helper.c
    env->arm_core_config.ccsidr[0] = 0xe00fe015; /* 16k L1 dcache. */
    env->arm_core_config.ccsidr[1] = 0x200fe015; /* 16k L1 icache. */

    //  From arm/helper.c
    env->arm_core_config.reset_fpsid = 0x41034000;  //  CA9 FPU 2.4: 0x41033090
    env->arm_core_config.isar.mvfr0 = 0x1111022;    //  CA9 FPU 2.4: 0x10110221
    env->arm_core_config.isar.mvfr1 = 0x01111111;   //  CA9 FPU 2.4: 0x01000011
}

/* ARMv8 */

void cpu_init_v8_2(CPUState *env, uint32_t id)
{
    assert(id == ARM_CPUID_CORTEXA55 || id == ARM_CPUID_CORTEXA75 || id == ARM_CPUID_CORTEXA76 || id == ARM_CPUID_CORTEXA78);

    set_feature(env, ARM_FEATURE_A);
    set_feature(env, ARM_FEATURE_AARCH64);
    set_feature(env, ARM_FEATURE_V8);
    set_feature(env, ARM_FEATURE_V7VE);
    set_feature(env, ARM_FEATURE_V7MP);
    set_feature(env, ARM_FEATURE_V7);
    set_feature(env, ARM_FEATURE_V6K);
    set_feature(env, ARM_FEATURE_V6);
    set_feature(env, ARM_FEATURE_V5);
    set_feature(env, ARM_FEATURE_V4T);

    set_feature(env, ARM_FEATURE_NEON);
    set_feature(env, ARM_FEATURE_GENERIC_TIMER);
    set_feature(env, ARM_FEATURE_CBAR_RO);
    set_feature(env, ARM_FEATURE_PMU);
    set_feature(env, ARM_FEATURE_THUMB2);
    set_feature(env, ARM_FEATURE_THUMB_DSP);  //  Mandatory in A-profile, see description of ISAR3_EL1.SIMD

    set_feature(env, ARM_FEATURE_EL2);
    set_feature(env, ARM_FEATURE_EL3);

    //  From B2.4 AArch64 registers
    env->arm_core_config.clidr = 0x82000023;  //  No L3 cache version.
    env->arm_core_config.ctr = 0x8444C004;
    env->arm_core_config.dcz_blocksize = 4;
    env->arm_core_config.id_aa64afr0 = 0;
    env->arm_core_config.id_aa64afr1 = 0;
    env->arm_core_config.isar.id_aa64dfr0 = 0x0000000010305408ull;
    env->arm_core_config.isar.id_aa64isar0 = 0x0000100010211120ull;  //  Version with Cryptographic Extension.
    env->arm_core_config.isar.id_aa64isar1 = 0x0000000000100001ull;
    env->arm_core_config.isar.id_aa64mmfr0 = 0x0000000000101122ull;
    env->arm_core_config.isar.id_aa64mmfr1 = 0x0000000010212122ull;
    env->arm_core_config.isar.id_aa64mmfr2 = 0x0000000000001011ull;
    env->arm_core_config.isar.id_aa64pfr0 = id == ARM_CPUID_CORTEXA75 ? 0x1100000010112222ull : 0x1100000010111112ull;
    env->arm_core_config.isar.id_aa64pfr1 = 0x0000000000000010ull;
    env->arm_core_config.id_afr0 = 0x00000000;
    env->arm_core_config.isar.id_dfr0 = 0x04010088;
    env->arm_core_config.isar.id_isar0 = 0x02101110;
    env->arm_core_config.isar.id_isar1 = 0x13112111;
    env->arm_core_config.isar.id_isar2 = 0x21232042;
    env->arm_core_config.isar.id_isar3 = 0x01112131;
    env->arm_core_config.isar.id_isar4 = 0x00010142;
    env->arm_core_config.isar.id_isar5 = 0x01011121;  //  Version with Cryptographic Extension.
    env->arm_core_config.isar.id_isar6 = 0x00000010;
    env->arm_core_config.isar.id_mmfr0 = 0x10201105;
    env->arm_core_config.isar.id_mmfr1 = 0x40000000;
    env->arm_core_config.isar.id_mmfr2 = 0x01260000;
    env->arm_core_config.isar.id_mmfr3 = 0x02122211;
    env->arm_core_config.isar.id_mmfr4 = 0x00021110;
    env->arm_core_config.isar.id_pfr0 = 0x10010131;
    env->arm_core_config.isar.id_pfr1 = 0x00010000;
    env->arm_core_config.isar.id_pfr2 = 0x00000011;

    //  MPIDR will be modified in runtime, by calling `tlib_get_mp_index`
    //  which passes CPUID, CLUSTERIDAFF2 and CLUSTERIDAFF3 configuration signals.
    env->arm_core_config.mpidr = (1u << 31) /* RES1 */ | (0u << 30) /* U */ | (1u << 24) /* MT */;
    env->arm_core_config.revidr = 0;

    //  From A75's B5.4. It's only accessible from AArch32 EL1-3 which aren't supported in A76/A78.
    env->arm_core_config.reset_fpsid = id == ARM_CPUID_CORTEXA75 ? 0x410340a3 : 0;

    //  From B2.23
    env->arm_core_config.ccsidr[0] = 0x701fe01a;
    env->arm_core_config.ccsidr[1] = 0x201fe01a;
    env->arm_core_config.ccsidr[2] = 0x707fe03a;

    //  From B2.97
    env->arm_core_config.reset_sctlr = 0x30c50838;

    //  From B4.23
    env->arm_core_config.gic_num_lrs = 4;
    env->arm_core_config.gic_vpribits = 5;
    env->arm_core_config.gic_vprebits = 5;
    //  From B4.7
    env->arm_core_config.gic_pribits = 5;

    //  From B5.1
    env->arm_core_config.isar.mvfr0 = 0x10110222;
    env->arm_core_config.isar.mvfr1 = 0x13211111;
    env->arm_core_config.isar.mvfr2 = 0x00000043;

    //  From D5.1
    env->arm_core_config.pmceid0 = 0x7FFF0F3F;
    env->arm_core_config.pmceid1 = 0x00F2AE7F;

    //  From D5.4
    env->arm_core_config.isar.reset_pmcr_el0 = 0x410b3000;

    //  TODO: Add missing ones? reset_cbar, reset_auxcr, reset_hivecs
    //  reset_cbar should be based on GIC PERIPHBASE signal.
}

void cpu_init_a53(CPUState *env, uint32_t id)
{
    assert(id == ARM_CPUID_CORTEXA53);

    set_feature(env, ARM_FEATURE_A);
    set_feature(env, ARM_FEATURE_AARCH64);
    set_feature(env, ARM_FEATURE_V8);
    set_feature(env, ARM_FEATURE_V7VE);
    set_feature(env, ARM_FEATURE_V7MP);
    set_feature(env, ARM_FEATURE_V7);
    set_feature(env, ARM_FEATURE_V6K);
    set_feature(env, ARM_FEATURE_V6);
    set_feature(env, ARM_FEATURE_V5);
    set_feature(env, ARM_FEATURE_V4T);

    set_feature(env, ARM_FEATURE_NEON);
    set_feature(env, ARM_FEATURE_GENERIC_TIMER);
    set_feature(env, ARM_FEATURE_CBAR_RO);
    set_feature(env, ARM_FEATURE_PMU);
    set_feature(env, ARM_FEATURE_THUMB2);
    set_feature(env, ARM_FEATURE_THUMB_DSP);  //  Mandatory in A-profile, see description of ISAR3_EL1.SIMD

    set_feature(env, ARM_FEATURE_EL2);
    set_feature(env, ARM_FEATURE_EL3);

    env->arm_core_config.clidr = 0x0A200023;
    env->arm_core_config.ctr = 0x84448004;
    env->arm_core_config.dcz_blocksize = 4;
    env->arm_core_config.id_aa64afr0 = 0;
    env->arm_core_config.id_aa64afr1 = 0;
    env->arm_core_config.isar.id_aa64dfr0 = 0x10305106ull;
    env->arm_core_config.isar.id_aa64isar0 = 0x00011120ull;
    env->arm_core_config.isar.id_aa64isar1 = 0x00000000ull;
    env->arm_core_config.isar.id_aa64mmfr0 = 0x00001122ull;
    env->arm_core_config.isar.id_aa64mmfr1 = 0x00000000ull;
    env->arm_core_config.isar.id_aa64pfr0 = 0x00002222ull;
    env->arm_core_config.isar.id_aa64pfr1 = 0x00000000ull;
    env->arm_core_config.id_afr0 = 0x00000000;
    env->arm_core_config.isar.id_dfr0 = 0x03010066;
    env->arm_core_config.isar.id_isar0 = 0x02101110;
    env->arm_core_config.isar.id_isar1 = 0x13112111;
    env->arm_core_config.isar.id_isar2 = 0x21232042;
    env->arm_core_config.isar.id_isar3 = 0x01112131;
    env->arm_core_config.isar.id_isar4 = 0x00011142;
    env->arm_core_config.isar.id_isar5 = 0x00011121;
    env->arm_core_config.isar.id_mmfr0 = 0x10201105;
    env->arm_core_config.isar.id_mmfr1 = 0x40000000;
    env->arm_core_config.isar.id_mmfr2 = 0x01260000;
    env->arm_core_config.isar.id_mmfr3 = 0x02102211;
    env->arm_core_config.isar.id_pfr0 = 0x00000131;
    env->arm_core_config.isar.id_pfr1 = 0x00011011;

    //  MPIDR will be modified in runtime, by calling `tlib_get_mp_index`
    //  which passes CPUID, CLUSTERIDAFF2 and CLUSTERIDAFF3 configuration signals.
    env->arm_core_config.mpidr = (1u << 31) /* RES1 */ | (0u << 30) /* U */ | (0u << 24) /* MT */;
    env->arm_core_config.revidr = 0;

    env->arm_core_config.reset_fpsid = 0x41034034;

    env->arm_core_config.ccsidr[0] = 0x700fe01a;
    env->arm_core_config.ccsidr[1] = 0x201fe01a;
    env->arm_core_config.ccsidr[2] = 0x707fe07a;

    env->arm_core_config.reset_sctlr = 0x00C50838;

    env->arm_core_config.gic_num_lrs = 4;
    env->arm_core_config.gic_vpribits = 5;
    env->arm_core_config.gic_vprebits = 5;
    env->arm_core_config.gic_pribits = 5;

    env->arm_core_config.isar.mvfr0 = 0x10110222;
    env->arm_core_config.isar.mvfr1 = 0x13211111;
    env->arm_core_config.isar.mvfr2 = 0x00000043;

    env->arm_core_config.pmceid0 = 0x7FFF0F3F;
    env->arm_core_config.pmceid1 = 0x00F2AE7F;

    env->arm_core_config.isar.reset_pmcr_el0 = 0x41033000;

    env->arm_core_config.midr = 0x410FD034;
}

void cpu_init_r52(CPUState *env, uint32_t id)
{
    //  Comments point to sections from
    //  the Arm Cortex-R52 Processor Technical Reference Manual (version: r1p3)

    set_feature(env, ARM_FEATURE_R);
    set_feature(env, ARM_FEATURE_V8);
    set_feature(env, ARM_FEATURE_V7VE);  //  enables ERET
    set_feature(env, ARM_FEATURE_V7MP);
    set_feature(env, ARM_FEATURE_V7);
    set_feature(env, ARM_FEATURE_V6K);
    set_feature(env, ARM_FEATURE_V6);
    set_feature(env, ARM_FEATURE_V5);
    set_feature(env, ARM_FEATURE_V4T);  //  enables BX

    set_feature(env, ARM_FEATURE_NEON);           //  from 1.2.2
    set_feature(env, ARM_FEATURE_GENERIC_TIMER);  //  from 1.2
    set_feature(env, ARM_FEATURE_PMSA);           //  from 1.1.
    set_feature(env, ARM_FEATURE_PMU);            //  from 1.1.1
    set_feature(env, ARM_FEATURE_THUMB2);         //  from 3.3.83
    set_feature(env, ARM_FEATURE_MVFR);           //  from 15.5
    set_feature(env, ARM_FEATURE_THUMB_DSP);

    set_feature(env, ARM_FEATURE_EL2);  //  EL2 virtualization, from 1.2

    env->arm_core_config.isar.id_isar0 = 0x02101110;  //  from 3.2.1
    env->arm_core_config.isar.id_isar1 = 0x13112111;  //  from 3.2.1
    env->arm_core_config.isar.id_isar2 = 0x21232142;  //  from 3.2.1
    env->arm_core_config.isar.id_isar3 = 0x01112131;  //  from 3.2.1
    env->arm_core_config.isar.id_isar4 = 0x00010142;  //  from 3.2.1
    env->arm_core_config.isar.id_isar5 = 0x00010001;  //  from 3.2.1
    env->arm_core_config.isar.id_mmfr0 = 0x00211040;  //  from 3.2.1
    env->arm_core_config.isar.id_mmfr1 = 0x40000000;  //  from 3.2.1
    env->arm_core_config.isar.id_mmfr2 = 0x01200000;  //  from 3.2.1
    env->arm_core_config.isar.id_mmfr3 = 0xF0102211;  //  from 3.2.1
    env->arm_core_config.isar.id_mmfr4 = 0x00000010;  //  from 3.2.1
    env->arm_core_config.isar.id_pfr0 = 0x00000131;   //  from 3.2.1
    env->arm_core_config.isar.id_pfr1 = 0x10111001;   //  from 3.2.1
    env->arm_core_config.isar.mvfr0 = 0x10110222;     //  full advanced SIMD, 0x10110021 for SP-only, from 15.5
    env->arm_core_config.isar.mvfr1 = 0x12111111;     //  full advanced SIMD, 0x11000011 for SP-only, from 15.5
    env->arm_core_config.isar.mvfr2 = 0x00000043;     //  full advanced SIMD, 0x00000040 for SP-only, from 15.5

    env->arm_core_config.isar.id_dfr0 =  //  32bit, from 3.3.24
        (0x0 << 28) |                    //  RES0
        (0x3 << 24) |                    //  PerfMon
        (0x0 << 20) |                    //  MProfDbg
        (0x1 << 16) |                    //  MMapTrc
        (0x0 << 12) |                    //  CopTrc
        (0x0 << 8) |                     //  MMapDbg, RES0
        (0x0 << 4) |                     //  CopSDbg, RES0
        (0x6 << 0);                      //  CopDbg

    env->arm_core_config.isar.dbgdidr =  //  32bit, from 11.4.1
        (0x7 << 28) |                    //  WRPs
        (0x7 << 24) |                    //  BRPs
        (0x1 << 23) |                    //  CTX_CMPs
        (0x6 << 16) |                    //  Version
        (0x1 << 15) |                    //  RES1
        (0x0 << 14) |                    //  nSUHD_imp
        (0x0 << 13) |                    //  RES0
        (0x0 << 12) |                    //  SE_imp
        (0x0 << 0);                      //  RES0

    env->arm_core_config.isar.dbgdevid =  //  32bit, from 11.4.2
        (0x0 << 28) |                     //  CIDMask
        (0x0 << 24) |                     //  AuxRegs
        (0x1 << 20) |                     //  DoubleLock
        (0x1 << 16) |                     //  VirExtns
        (0x0 << 12) |                     //  VectorCatch
        (0xF << 8) |                      //  BPAddrMask
        (0x1 << 4) |                      //  WPAddrMask
        (0x3 << 0);                       //  PCsample

    env->arm_core_config.isar.dbgdevid1 =  //  32bit, from 11.4.3
        (0x0 << 4) |                       //  RES0
        (0x2 << 0);                        //  PCSROffset

    env->arm_core_config.revidr = 0x00000000;       //  from 3.2.1
    env->arm_core_config.reset_fpsid = 0x41034023;  //  from 15.5
    env->arm_core_config.ctr = 0x8144c004;          //  from 3.2.1

    env->arm_core_config.reset_sctlr =  //  32bit, from 3.3.92
        (0x0 << 31) |                   //  RES0
        (0x0 << 30) |                   //  TE, here exceptions taken in A32 state
        (0x3 << 28) |                   //  RES1
        (0x0 << 26) |                   //  RES0
        (0x0 << 25) |                   //  EE, here little endianness exception, 0 in CPSR.E
        (0x0 << 24) |                   //  RES0
        (0x3 << 22) |                   //  RES1
        (0x0 << 21) |                   //  FI
        (0x0 << 20) |                   //  UWXN
        (0x0 << 19) |                   //  WXN
        (0x1 << 18) |                   //  nTWE
        (0x0 << 17) |                   //  BR
        (0x1 << 16) |                   //  nTWI
        (0x0 << 13) |                   //  RES0
        (0x0 << 12) |                   //  I
        (0x1 << 11) |                   //  RES1
        (0x0 << 9) |                    //  RES0
        (0x0 << 8) |                    //  SED
        (0x0 << 7) |                    //  ITD
        (0x0 << 6) |                    //  RES0
        (0x1 << 5) |                    //  CP15BEN
        (0x3 << 3) |                    //  RES1
        (0x0 << 2) |                    //  C
        (0x0 << 1) |                    //  A
        (0x0 << 0);                     //  M

    env->arm_core_config.pmceid0 = 0x6E1FFFDB;  //  3.2.11
    env->arm_core_config.pmceid1 = 0x0000001E;  //  3.2.11
    env->arm_core_config.id_afr0 = 0x00000000;  //  3.2.19

    env->arm_core_config.clidr =  //  32bit, from 3.3.13
        (0x0 << 30) |             //  ICB
        (0x1 << 27) |             //  LoUU, set if either cache is implemented
        (0x1 << 24) |             //  LoC, set if either cache is implemented
        (0x0 << 21) |             //  LoUIS
        (0x0 << 18) |             //  Ctype7
        (0x0 << 15) |             //  Ctype6
        (0x0 << 12) |             //  Ctype5
        (0x0 << 9) |              //  Ctype4
        (0x0 << 6) |              //  Ctype3
        (0x0 << 3) |              //  Ctype2
        (0x3 << 0);               //  Ctype1, separate instructions and data caches

    //  MPIDR will be modified in runtime, by calling `tlib_get_mp_index`
    //  which passes CPUID, CLUSTERIDAFF2 and CLUSTERIDAFF3 configuration signals.
    env->arm_core_config.mpidr =  //  32bit, from 3.3.78
        (0x1u << 31) |            //  M, RES1
        (0x0 << 30) |             //  U, core is part of cluster (no single core)
        (0x0 << 25) |             //  RES0
        (0x0 << 24);              //  MT

    env->arm_core_config.ccsidr[0] =  //  32bit, 3.3.20
        (0x0 << 31) |                 //  WT, here no Write-Through
        (0x1 << 30) |                 //  WB, here support Write-Back
        (0x1 << 29) |                 //  RA, here support Read-Allocation
        (0x1 << 28) |                 //  WA, here support Write-Allocation
        (0x7F << 13) |                //  NumSets, config for 32KB
        (0x3 << 3) |                  //  Associativity
        (0x2 << 0);                   //  LineSize

    env->arm_core_config.ccsidr[1] =  //  32bit, 3.3.20
        (0x0 << 31) |                 //  WT, here support Write-Through
        (0x0 << 30) |                 //  WB, here no Write-Back
        (0x1 << 29) |                 //  RA, here support Read-Allocation
        (0x0 << 28) |                 //  WA, here no Write-Allocation
        (0x7F << 13) |                //  NumSets, config for 32KB
        (0x3 << 3) |                  //  Associativity
        (0x2 << 0);                   //  LineSize

    env->arm_core_config.gic_num_lrs = 4;   //  from 3.2.14
    env->arm_core_config.gic_vpribits = 5;  //  from 9.3.3
    env->arm_core_config.gic_vprebits = 5;  //  from 9.3.3
    env->arm_core_config.gic_pribits = 5;   //  from 9.3.4

    env->arm_core_config.gt_cntfrq_hz = 0;  //  from 3.2.16

    env->arm_core_config.mpuir =              //  from 3.3.76
        (16 << 8);                            //  DREGION, here 16 EL1-controlled MPU regions
    env->arm_core_config.hmpuir = (16 << 0);  //  REGION, here 16 EL2-controlled MPU regions
    //  Main ID register from Arm Cortex-R52 Processor Technical Reference Manual rev. r1p3 (100026_0103_00_en)
    env->arm_core_config.midr = 0x411FD132;

    //  TODO: Add missing ones: reset_cbar, reset_auxcr, reset_hivecs
}

static void cpu_init_core_config(CPUState *env, uint32_t id)
{
    //  Main ID Register.
    env->arm_core_config.midr = id;

    switch(id) {
        case ARM_CPUID_CORTEXA9:
            cpu_init_a9(env, id);
            break;
        case ARM_CPUID_CORTEXA53:
            cpu_init_a53(env, id);
            break;
        case ARM_CPUID_CORTEXA55:
        case ARM_CPUID_CORTEXA75:
        case ARM_CPUID_CORTEXA76:
        case ARM_CPUID_CORTEXA78:
            cpu_init_v8_2(env, id);
            break;
        case ARM_CPUID_CORTEXR52:
            cpu_init_r52(env, id);
            break;
        default:
            cpu_abort(env, "Bad CPU ID: %x\n", id);
            break;
    }
}

void cpu_init_v8(CPUState *env, uint32_t id)
{
    cpu_init_core_config(env, id);
    system_instructions_and_registers_init(env, id);
}

void cpu_reset_state(CPUState *env)
{
    //  Let's preserve arm_core_config, features and CPU ID.
    ARMCoreConfig config = env->arm_core_config;
    uint64_t features = env->features;
    uint32_t id = env->cp15.c0_cpuid;

    memset(env, 0, RESET_OFFSET);

    arm_clear_exclusive(env);

    //  Restore preserved fields.
    env->arm_core_config = config;
    env->features = features;
    env->cp15.c0_cpuid = id;
}

void cpu_reset_v8_a64(CPUState *env)
{
    tlib_assert(arm_feature(env, ARM_FEATURE_AARCH64));

    uint32_t pstate;

    env->aarch64 = 1;

    //  Reset values of some registers are defined per CPU model.
    env->cp15.sctlr_el[1] = env->arm_core_config.reset_sctlr | (1u << 20);  //  Bit20 is RES1 without FEAT_CSV2_2/_1p2.
    env->cp15.sctlr_el[2] = env->arm_core_config.reset_sctlr;
    env->cp15.sctlr_el[3] = env->arm_core_config.reset_sctlr;
    env->cp15.vmpidr_el2 = env->arm_core_config.mpidr;
    env->cp15.vpidr_el2 = env->arm_core_config.midr;
    env->cp15.c9_pmcr = env->arm_core_config.isar.reset_pmcr_el0;

    //  The default reset state for AArch64 is the highest available ELx (handler=true: use SP_ELx).
    pstate = aarch64_pstate_mode(arm_highest_el(env), true);

    //  Reset value for each of the Interrupt Mask Bits (DAIF) is 1.
    pstate |= PSTATE_DAIF;

    //  Zero flag should be unset after reset.
    //  It's interpreted as set if PSTATE_Z bit is zero.
    pstate |= PSTATE_Z;

    pstate_write(env, pstate);

    env->emulate_smc_calls = false;
    env->emulate_hvc_calls = false;
}

void cpu_reset_v8_a32(CPUState *env)
{
    env->aarch64 = false;

    env->cp15.sctlr_ns = env->arm_core_config.reset_sctlr;
    env->cp15.hsctlr = env->arm_core_config.reset_sctlr;
    env->cp15.sctlr_s = env->arm_core_config.reset_sctlr;

    env->cp15.vmpidr_el2 = env->arm_core_config.mpidr;
    env->cp15.vpidr_el2 = env->arm_core_config.midr;

    env->cp15.rvbar = env->arm_core_config.rvbar_prop;

    uint32_t cpsr = arm_get_highest_cpu_mode(env);
    cpsr |= CPSR_AIF | CPSR_Z;
    cpsr_write(env, cpsr, 0xFFFFFFFF, CPSRWriteRaw);

    env->regs[15] = env->cp15.rvbar;

    /* v7 performance monitor control register: same implementor
     * field as main ID register, and we implement no event counters.
     */
    env->cp15.c9_pmcr = (env->cp15.c0_cpuid & 0xff000000);

    env->emulate_smc_calls = false;
    env->emulate_hvc_calls = false;
}

void do_interrupt_a64(CPUState *env)
{
    uint32_t current_el = arm_current_el(env);
    uint32_t target_el = env->exception.target_el;

    //  The function is only valid if target EL uses AArch64.
    tlib_assert(arm_el_is_aa64(env, target_el));

    if(current_el > target_el) {
        tlib_abortf("do_interrupt: exception level can never go down by taking an exception");
    }
    if(target_el == 0) {
        tlib_abortf("do_interrupt: exceptions cannot be taken to EL0");
    }

    //  ARMv8-A manual's rule RDPLSC
    if(current_el == 0 && arm_el_is_aa64(env, 2)) {
        bool hcr_tge_set = arm_hcr_el2_eff(env) & HCR_TGE;
        bool mdcr_tde_set = env->cp15.mdcr_el2 & MDCR_TDE;
        switch(syn_get_ec(env->exception.syndrome)) {
            case SYN_EC_DATA_ABORT_LOWER_EL:
            case SYN_EC_INSTRUCTION_ABORT_LOWER_EL:
                //  The rule only applies to Stage 1 Data/Instruction aborts.
                if(env->exception.syndrome & SYN_DATA_ABORT_S1PTW) {
                    target_el = hcr_tge_set ? 2 : 1;
                }
                break;
            case SYN_EC_PC_ALIGNMENT_FAULT:
            case SYN_EC_SP_ALIGNMENT_FAULT:
            case SYN_EC_BRANCH_TARGET:
            case SYN_EC_ILLEGAL_EXECUTION_STATE:
            case SYN_EC_AA32_TRAPPED_FLOATING_POINT:
            case SYN_EC_AA64_TRAPPED_FLOATING_POINT:
            case SYN_EC_AA32_SVC:
            case SYN_EC_AA64_SVC:
            //  TODO: case for Undefined Instruction Exception
            case SYN_EC_TRAPPED_SVE:
            case SYN_EC_POINTER_AUTHENTICATION:
            case SYN_EC_TRAPPED_WF:
            case SYN_EC_TRAPPED_SME_SVE_SIMD_FP:
                //  TODO: case for Synchronous External Aborts
                //  TODO: case for Memory Copy and Memory Set Exceptions
                target_el = hcr_tge_set ? 2 : 1;
                break;
            case SYN_EC_AA32_VECTOR_CATCH:
                tlib_assert(hcr_tge_set || mdcr_tde_set);
                /* fall through */
            case SYN_EC_BREAKPOINT_LOWER_EL:
            case SYN_EC_AA32_BKPT:
            case SYN_EC_AA64_BKPT:
            case SYN_EC_SOFTWARESTEP_LOWER_EL:
            case SYN_EC_WATCHPOINT_LOWER_EL:
                target_el = hcr_tge_set || mdcr_tde_set ? 2 : 1;
                break;
            default:
                break;
        }
    }

    //  new pstate mode according to the ARMv8-A manual's rule WTXBY
    //  set new exception level and 'PSTATE.SP' field
    uint32_t new_pstate = aarch64_pstate_mode(target_el, true);
    //  set DAIF bits
    new_pstate |= PSTATE_DAIF;  //  TODO: Set also TCO bit after adding support for ARMv8.5-MTE
    //  set PSTATE.SSBS to value of SCTLR.DSSBS
    new_pstate |= (!!(arm_sctlr(env, target_el) & SCTLR_DSSBS_64) << 12);

    //  TODO: set PSTATE.SS according to the rules in Chapter D2 AArch64 Self-hosted Debug
    if(current_el == 0 && target_el == 2 && (arm_hcr_el2_eff(env) & HCR_TGE) && (arm_hcr_el2_eff(env) & HCR_E2H) &&
       !((arm_sctlr(env, target_el) & SCTLR_SPAN))) {
        new_pstate |= PSTATE_PAN;
        //  TODO: set PSTATE_PAN also when PSTATE.ALLINT is set to the inverse value of SCTLR_ELx.SPINTMASK
    }

    //  current pstate mode
    uint32_t old_pstate = is_a64(env) ? pstate_read(env) : cpsr_read_to_spsr_elx(env);
    //  exception vector table, base adress for target el
    target_ulong addr = env->cp15.vbar_el[target_el];
    //  save current pstate in SPSR_ELn
    env->banked_spsr[aarch64_banked_spsr_index(target_el)] = old_pstate;

    if(current_el == target_el) {
        if(old_pstate & PSTATE_SP) {
            addr += 0x200;
        }
    } else {
        if(is_a64(env)) {
            //  Lower EL using AArch64
            addr += 0x400;
        } else {
            //  Lower EL using AArch32
            addr += 0x600;
        }
    }

    switch(env->exception_index) {
        case EXCP_DATA_ABORT:
        case EXCP_PREFETCH_ABORT:
            //  Fault Address Register, holds the faulting virtual address
            env->cp15.far_el[target_el] = env->exception.vaddress;
            break;
        case EXCP_IRQ:
        case EXCP_VIRQ:
            addr += 0x80;
            break;
        case EXCP_FIQ:
        case EXCP_VFIQ:
            addr += 0x100;
            break;
        case EXCP_SERR:
            env->cp15.far_el[target_el] = env->exception.vaddress;
            addr += 0x180;
            break;
        case EXCP_VSERR:
            tlib_abortf("do_interrupt: unsupported SError exception");
            break;
        case EXCP_BKPT:
            tlib_printf(LOG_LEVEL_DEBUG, "Handling BKPT exception");
            break;
        case EXCP_HVC:
            tlib_printf(LOG_LEVEL_DEBUG, "Handling HVC exception");
            break;
        case EXCP_SMC:
            tlib_printf(LOG_LEVEL_DEBUG, "Handling SMC exception");
            break;
        case EXCP_SWI_SVC:
            //  The ARMv8-A manual states it was previously called SWI (see: F5.1.250 "SVC").
            tlib_printf(LOG_LEVEL_DEBUG, "Handling SVC exception");
            break;
        case EXCP_UDEF:
            switch(syn_get_ec(env->exception.syndrome)) {
                case SYN_EC_BRANCH_TARGET:
                case SYN_EC_TRAPPED_MSR_MRS_SYSTEM_INST:
                case SYN_EC_TRAPPED_SME:
                case SYN_EC_TRAPPED_SME_SVE_SIMD_FP:
                case SYN_EC_TRAPPED_SVE:
                case SYN_EC_TRAPPED_WF:
                    break;
                case SYN_EC_ILLEGAL_EXECUTION_STATE:
                    tlib_printf(LOG_LEVEL_WARNING, "Handling illegal execution state exception; PSTATE=0x%" PRIx32, env->pstate);
                    break;
                case SYN_EC_UNKNOWN_REASON:
                    tlib_printf(LOG_LEVEL_DEBUG, "Undefined instruction at PC=0x%" PRIx64 ": %" PRIx32, CPU_PC(env),
                                ldl_code(CPU_PC(env)));
                    break;
                default:
                    //  All the syndromes used with EXCP_UDEF have explicit cases.
                    tlib_assert_not_reached();
            }
            break;
        default:
            cpu_abort(env, "Unhandled exception 0x%x\n", env->exception_index);
            __builtin_unreachable();
    }
    env->cp15.esr_el[target_el] = env->exception.syndrome;

    //  save current PC to ELR_ELn
    env->elr_el[target_el] = CPU_PC(env);
    if(!env->aarch64) {
        aarch64_sync_32_to_64(env);
        env->condexec_bits = 0;
    } else {
        aarch64_save_sp(env);
    }
    env->aarch64 = true;
    pstate_write(env, new_pstate);
    aarch64_restore_sp(env);
    helper_rebuild_hflags_a64(env, target_el);

    tlib_printf(LOG_LEVEL_DEBUG, "%s: excp=%d, addr=0x%" PRIx64 ", target_el=%d, syndrome=0x%x, pc=0x%" PRIx64 ", far=0x%" PRIx64,
                __func__, env->exception_index, addr, target_el, env->exception.syndrome, CPU_PC(env), env->exception.vaddress);

    //  execute exception handler
    env->pc = addr;

    //  Reset the exception structure.
    memset(&env->exception, 0, sizeof(env->exception));

    set_interrupt_pending(env, CPU_INTERRUPT_EXITTB);
    if(unlikely(env->guest_profiler_enabled)) {
        tlib_announce_stack_change(CPU_PC(env), STACK_FRAME_ADD);
    }
}

bool check_scr_el3_mask(uint64_t scr_el3, int ns, int aw, int fw, int ea, int irq, int fiq)
{
    bool result = 1;
    if(ns != -1) {
        result &= (!!(scr_el3 & SCR_NS) == ns);
    }
    if(aw != -1) {
        result &= (!!(scr_el3 & SCR_AW) == aw);
    }
    if(fw != -1) {
        result &= (!!(scr_el3 & SCR_FW) == fw);
    }
    if(ea != -1) {
        result &= (!!(scr_el3 & SCR_EA) == ea);
    }
    if(irq != -1) {
        result &= (!!(scr_el3 & SCR_IRQ) == irq);
    }
    if(fiq != -1) {
        result &= (!!(scr_el3 & SCR_FIQ) == fiq);
    }
    return result;
}

//  Pass '-1' if the given field should have no influence on the result.
bool check_scr_el3(uint64_t scr_el3, int ns, int eel2, int ea, int irq, int fiq, int rw)
{
    bool result = 1;
    if(ns != -1) {
        result &= (!!(scr_el3 & SCR_NS) == ns);
    }
    if(eel2 != -1) {
        result &= (!!(scr_el3 & SCR_EEL2) == eel2);
    }
    if(ea != -1) {
        result &= (!!(scr_el3 & SCR_EA) == ea);
    }
    if(irq != -1) {
        result &= (!!(scr_el3 & SCR_IRQ) == irq);
    }
    if(fiq != -1) {
        result &= (!!(scr_el3 & SCR_FIQ) == fiq);
    }
    if(rw != -1) {
        result &= (!!(scr_el3 & SCR_RW) == rw);
    }
    return result;
}

//  Pass '-1' if the given field should have no influence on the result.
bool check_hcr_el2(uint64_t hcr_el2, int tge, int amo, int imo, int fmo, int e2h, int rw)
{
    bool result = 1;
    if(tge != -1) {
        result &= (!!(hcr_el2 & HCR_TGE) == tge);
    }
    if(amo != -1) {
        result &= (!!(hcr_el2 & HCR_AMO) == amo);
    }
    if(imo != -1) {
        result &= (!!(hcr_el2 & HCR_IMO) == imo);
    }
    if(fmo != -1) {
        result &= (!!(hcr_el2 & HCR_FMO) == fmo);
    }
    if(e2h != -1) {
        result &= (!!(hcr_el2 & HCR_E2H) == e2h);
    }
    if(rw != -1) {
        result &= (!!(hcr_el2 & HCR_RW) == rw);
    }
    return result;
}

bool interrupt_masked(bool pstate_mask_bit, bool sctlr_nmi, bool allintmask, bool superpriority)
{
    bool masked;
    if(pstate_mask_bit) {
        masked = !sctlr_nmi || allintmask || !superpriority;
    } else {
        masked = sctlr_nmi && allintmask;
    }
    return masked;
}

bool irq_masked(CPUState *env, uint32_t target_el, bool superpriority, bool ignore_pstate_aif)
{
    uint32_t pstate = pstate_read(env);
    uint64_t sctlr = arm_sctlr(env, target_el);

    bool pstate_i = pstate & PSTATE_I;
    if(ignore_pstate_aif) {
        pstate_i = false;
    }
    bool sctlr_nmi = sctlr & SCTLR_NMI;
    bool allintmask = pstate & PSTATE_ALLINT || (pstate & PSTATE_SP && sctlr & SCTLR_SPINTMASK);
    return interrupt_masked(pstate_i, sctlr_nmi, allintmask, superpriority);
}

bool fiq_masked(CPUState *env, uint32_t target_el, bool superpriority, bool ignore_pstate_aif)
{
    uint32_t pstate = pstate_read(env);
    uint64_t sctlr = arm_sctlr(env, target_el);

    bool pstate_f = pstate & PSTATE_F;
    if(ignore_pstate_aif) {
        pstate_f = false;
    }
    bool sctlr_nmi = sctlr & SCTLR_NMI;
    bool allintmask = pstate & PSTATE_ALLINT || (pstate & PSTATE_SP && sctlr & SCTLR_SPINTMASK);
    return interrupt_masked(pstate_f, sctlr_nmi, allintmask, superpriority);
}

uint32_t aarch32_interrupt_masked(CPUState *env, uint64_t scr_el3, uint64_t hcr_el2, uint32_t current_el, int exception_index)
{
    bool el3_enabled = arm_feature(env, ARM_FEATURE_EL3);
    bool el2_enabled = arm_feature(env, ARM_FEATURE_EL2);
    bool scr, hcr, cpsr;
    int result;

    switch(exception_index) {
        case EXCP_VIRQ:
            return current_el > 1 || !(hcr_el2 & HCR_IMO) || (env->daif & CPSR_I);
        case EXCP_VFIQ:
            return current_el > 1 || !(hcr_el2 & HCR_FMO) || (env->daif & CPSR_F);
        case EXCP_IRQ:
            hcr = el2_enabled && (hcr_el2 & HCR_IMO);
            scr = el3_enabled && (scr_el3 & SCR_IRQ);
            cpsr = env->daif & CPSR_I;
            break;
        case EXCP_FIQ:
            scr = el3_enabled && (scr_el3 & SCR_FIQ);
            hcr = (el2_enabled && (hcr_el2 & HCR_FMO)) || (scr && scr_el3 & SCR_FW);
            cpsr = env->daif & CPSR_F;
            break;
        default:
            tlib_abortf("Unexpected exception index: %x", exception_index);
            __builtin_unreachable();
    }

    if(el3_enabled && !(scr_el3 & SCR_NS)) {
        result = 0xB;
    } else {
        hcr = hcr || (el2_enabled && (hcr_el2 & HCR_TGE));
        //  Minimum current EL where interrupts can be masked
        uint32_t min_el = scr ? 3 : 2;
        if(hcr) {
            result = current_el >= min_el ? 0xB : 0xA;
        } else {
            result = 0xB;
        }
    }

    switch(result) {
        case 0xA:
            return false;
        case 0xB:
            return cpsr;
        default:
            tlib_abortf("Entered unreachable code");
            __builtin_unreachable();
    }
}

uint32_t get_aarch32_interrupt_target_el(CPUState *env, uint64_t scr_el3, uint64_t hcr_el2, uint32_t current_el,
                                         uint32_t exception_index)
{
    bool el3_enabled = arm_feature(env, ARM_FEATURE_EL3);
    bool el2_enabled = arm_feature(env, ARM_FEATURE_EL2);
    bool hcr, scr;

    switch(exception_index) {
        case EXCP_VIRQ:
        case EXCP_VFIQ:
            return 1;
        case EXCP_IRQ:
            scr = el3_enabled && (scr_el3 & SCR_IRQ);
            hcr = el2_enabled && (hcr_el2 & HCR_IMO);
            break;
        case EXCP_FIQ:
            scr = el3_enabled && (scr_el3 & SCR_FIQ);
            hcr = el2_enabled && (hcr_el2 & HCR_FMO);
            break;
        default:
            tlib_abortf("Unexpected exception index: %x", exception_index);
            __builtin_unreachable();
    }

    if(el3_enabled && !(scr_el3 & SCR_NS)) {
        return 1;
    }
    if(scr) {
        return 3;
    }
    if(current_el == 3) {
        return 1;
    }
    if(current_el == 2) {
        return 2;
    }

    return hcr ? 2 : 1;
}

int process_interrupt_v8a_aarch32(int interrupt_request, CPUState *env, uint64_t scr_el3, uint64_t hcr_el2)
{
    int exception_index = -1;
    if(interrupt_request & CPU_INTERRUPT_HARD) {
        exception_index = EXCP_IRQ;
    } else if(interrupt_request & CPU_INTERRUPT_FIQ) {
        exception_index = EXCP_FIQ;
    } else if(interrupt_request & CPU_INTERRUPT_VIRQ) {
        exception_index = EXCP_VIRQ;
    } else if(interrupt_request & CPU_INTERRUPT_VFIQ) {
        exception_index = EXCP_VFIQ;
    } else {
        tlib_abortf("Unexpected interrupt request 0x%x", interrupt_request);
        return 1;
    }

    uint32_t current_el = arm_current_el(env);
    uint32_t target_el = get_aarch32_interrupt_target_el(env, scr_el3, hcr_el2, current_el, exception_index);

    tlib_assert(current_el <= 3);
    if(target_el == 0) {
        tlib_abortf("process_interrupt: invalid target_el!");
    }

    if(aarch32_interrupt_masked(env, scr_el3, hcr_el2, current_el, exception_index)) {
        return 0;
    }

    env->exception.target_el = target_el;
    env->exception_index = exception_index;
    do_interrupt(env);
    return 1;
}

#define IRQ_IGNORED UINT32_MAX
uint32_t establish_interrupts_target_el(uint32_t current_el, uint64_t scr_el3, uint64_t hcr_el2)
{
    tlib_assert(current_el <= 3);

    //  Establishing the target Exception level of an asynchronous exception (ARMv8-A manual's rule NMMXK).
    //
    //  The 'check_scr_el3' and 'check_hcr_el2' will return true only if the state of the bits passed matches
    //  their current state in SCR_EL3 and HCR_EL2 (respectively). The bit is ignored if '-1' is passed.
    if(check_scr_el3(scr_el3, 0, 0, 0, 0, 0, 0)) {
        switch(current_el) {
            case 0:
            case 1:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the FIQ/IRQ/Abort mode (unless "
                                             "masked). AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 2:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") state for an EL2 interrupt", scr_el3);
                break;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 0, 0, 0, 0, 0, 1)) {
        switch(current_el) {
            case 0:
            case 1:
                return 1;
            case 2:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") for an EL2 interrupt", scr_el3);
                break;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
        //  TODO: does all EA, IRQ, FIQ needs to be set at single time
        //  or only one of them, depending on irq type needs to be set?
    } else if(check_scr_el3(scr_el3, 0, 0, 1, 1, 1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
            case 3:
                return 3;
            case 2:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") for an EL2 interrupt", scr_el3);
                break;
        }
    } else if(check_scr_el3(scr_el3, 0, 1, 0, 0, 0, -1) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, 0, 0)) {
        switch(current_el) {
            case 0:
            case 1:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the FIQ/IRQ/Abort mode (unless "
                                             "masked). AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 2:
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 0, 1, 0, 0, 0, -1) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, 0, 1)) {
        switch(current_el) {
            case 0:
            case 1:
                return 1;
            case 2:
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 0, 1, 0, 0, 0, -1) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, 1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
                return 1;
            case 2:
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
        //  TODO: does all AMO, IMO, FMO needs to be set at single time
        //  or only one of them?
    } else if(check_scr_el3(scr_el3, 0, 1, 0, 0, 0, -1) && check_hcr_el2(hcr_el2, 0, 1, 1, 1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
            case 2:
                return 2;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 0, 1, 0, 0, 0, -1) && check_hcr_el2(hcr_el2, 1, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 2:
                return 2;
            case 1:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") and HCR_EL2 (0x%" PRIx64 ") for an EL1 interrupt", scr_el3, hcr_el2);
                break;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
        //  TODO: does all EA, IRQ, FIQ needs to be set at single time
        //  or only one of them, depending on irq type needs to be set?
    } else if(check_scr_el3(scr_el3, 0, 1, 1, 1, 1, -1) && check_hcr_el2(hcr_el2, 0, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
            case 2:
            case 3:
                return 3;
        }
    } else if(check_scr_el3(scr_el3, 0, 1, 1, 1, 1, -1) && check_hcr_el2(hcr_el2, 1, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 2:
            case 3:
                return 3;
            case 1:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") and HCR_EL2 (0x%" PRIx64 ") for an EL1 interrupt", scr_el3, hcr_el2);
                break;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 0) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, -1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the FIQ/IRQ/Abort mode (unless "
                                             "masked). AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 2:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the HYP mode (unless masked). "
                                             "AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 0) && check_hcr_el2(hcr_el2, 0, 1, 1, 1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
            case 2:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the HYP mode (unless masked). "
                                             "AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 0) && check_hcr_el2(hcr_el2, 1, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 2:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the HYP mode (unless masked). "
                                             "AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 1:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") and HCR_EL2 (0x%" PRIx64 ") for an EL1 interrupt", scr_el3, hcr_el2);
                break;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 1) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, 0, 0)) {
        switch(current_el) {
            case 0:
            case 1:
                //  TODO: Implement AArch32 exception handling or at least implement AArch32 exception masking and abort if
                //  unmasked.
                tlib_printf(LOG_LEVEL_DEBUG, "Ignoring IRQ request that should be handled at the FIQ mode (unless masked). "
                                             "AArch32 exceptions aren't currently supported.");
                return IRQ_IGNORED;
            case 2:
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 1) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, 0, 1)) {
        switch(current_el) {
            case 0:
            case 1:
                return 1;
            case 2:
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 1) && check_hcr_el2(hcr_el2, 0, 0, 0, 0, 1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
                return 1;
            case 2:
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 1) && check_hcr_el2(hcr_el2, 0, 1, 1, 1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
            case 2:
                return 2;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 0, 0, 0, 1) && check_hcr_el2(hcr_el2, 1, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 2:
                return 2;
            case 1:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") and HCR_EL2 (0x%" PRIx64 ") for an EL1 interrupt", scr_el3, hcr_el2);
                break;
            case 3:
                //  interrupt not taken and ignored, just return
                return IRQ_IGNORED;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 1, 1, 1, -1) && check_hcr_el2(hcr_el2, 0, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 1:
            case 2:
            case 3:
                return 3;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 1, 1, 1, -1) && check_hcr_el2(hcr_el2, 1, -1, -1, -1, -1, -1)) {
        switch(current_el) {
            case 0:
            case 2:
            case 3:
                return 3;
            case 1:
                //  Not applicable
                tlib_abortf("Invalid SCR_EL3 (0x%" PRIx64 ") and HCR_EL2 (0x%" PRIx64 ") for an EL1 interrupt", scr_el3, hcr_el2);
                break;
        }
    } else if(check_scr_el3(scr_el3, 1, -1, 1, 0, 0, -1)) {
        tlib_printf(
            LOG_LEVEL_DEBUG,
            "Setting different interrupt targets for FIQ, IRQ and SError is not currently supported. Ignoring interrupt.");
        return IRQ_IGNORED;
    } else {
        tlib_abortf("Unexpected register state in process_interrupt!");
    }
    tlib_assert_not_reached();
}

int process_interrupt_v8a_aarch64(int interrupt_request, CPUState *env, uint64_t scr_el3, uint64_t hcr_el2)
{
    tlib_assert(is_a64(env));

    uint32_t current_el = arm_current_el(env);
    uint32_t target_el = 1;

    target_el = establish_interrupts_target_el(current_el, scr_el3, hcr_el2);

    if(target_el == IRQ_IGNORED) {
        return 0;
    }

    //  ARMv8-A manual's rule LMWZH
    if(target_el < current_el) {
        //  mask interrupt
        return 0;
    }

    if(interrupt_request & (CPU_INTERRUPT_FIQ | CPU_INTERRUPT_HARD)) {
        bool ignore_pstate_aif = false;
        if(target_el > current_el) {
            //  ARMv8-A manual's rule RXBYXL
            if(target_el == 3) {
                ignore_pstate_aif = true;
            } else if(target_el == 2) {
                if(!are_hcr_e2h_and_tge_set(hcr_el2)) {
                    ignore_pstate_aif = true;
                }
            }
        }

        if(interrupt_request & CPU_INTERRUPT_FIQ) {
            //  TODO: when physical fiq have superpriority?
            //  ARMv8-A manual's rule (RPBKNX) says it is 'IMPLEMENTATION DEFINED'
            if(fiq_masked(env, target_el, false, ignore_pstate_aif)) {
                return 0;
            }

            env->exception_index = EXCP_FIQ;
        } else if(interrupt_request & CPU_INTERRUPT_HARD) {
            //  TODO: when physical irq have superpriority?
            //  ARMv8-A manual's rule (RPBKNX) says it is 'IMPLEMENTATION DEFINED'
            if(irq_masked(env, target_el, false, ignore_pstate_aif)) {
                return 0;
            }

            env->exception_index = EXCP_IRQ;
        }
    } else if(interrupt_request & CPU_INTERRUPT_VFIQ) {
        if(current_el > 1) {
            //  ARMv8-A manual's rule GYGBD
            return 0;
        }
        if(target_el != 1) {
            //  ARMv8-A manual's rule GYGBD
            tlib_abortf("Wrong current_el or target_el while handling vfiq!");
        }
        if(target_el == current_el) {
            if(fiq_masked(env, target_el, env->cp15.hcrx_el2 & HCRX_VFNMI, false)) {
                return 0;
            }
        }
        env->exception_index = EXCP_VFIQ;
    } else if(interrupt_request & CPU_INTERRUPT_VIRQ) {
        if(current_el > 1) {
            //  ARMv8-A manual's rule GYGBD
            return 0;
        }
        if(target_el != 1) {
            //  ARMv8-A manual's rule GYGBD
            tlib_abortf("Wrong current_el or target_el while handling virq!");
        }
        if(target_el == current_el) {
            if(irq_masked(env, target_el, env->cp15.hcrx_el2 & HCRX_VINMI, false)) {
                return 0;
            }
        }
        env->exception_index = EXCP_VIRQ;
    } else if(interrupt_request & CPU_INTERRUPT_VSERR) {
        if(target_el == current_el) {
            if(!(scr_el3 & SCR_NMEA) && !(pstate_read(env) & PSTATE_A)) {
                return 0;
            }
        } else if(target_el > current_el) {
            bool ignore_pstate_aif = false;
            if(target_el == 3) {
                ignore_pstate_aif = true;
            } else if(target_el == 2) {
                if(!are_hcr_e2h_and_tge_set(hcr_el2)) {
                    ignore_pstate_aif = true;
                }
            }
            //  TODO: when physical irq have superpriority?
            //  ARMv8-A manual's rule (RPBKNX) says it is 'IMPLEMENTATION DEFINED'
            if(irq_masked(env, target_el, false, ignore_pstate_aif)) {
                return 0;
            }
        }
        env->exception_index = EXCP_VSERR;
    } else {
        tlib_printf(LOG_LEVEL_ERROR, "process_interrupt: interrupt not masked and didn't throw exception!");
        return 0;
    }
    env->exception.target_el = target_el;
    do_interrupt(env);
    return 1;
}

int process_interrupt_v8a(int interrupt_request, CPUState *env)
{
    //  ARMv8-A manual's rule QZPXL
    //  If EL3 is not enabled, the effective values of SCR_EL3 fields are:
    //  * 1 for EEL2 and RW
    //  * 0 for FIQ/IRQ/EA
    //  If EL2 is not enabled, the effective values of HCR (AArch32 naming) or HCR_EL2 fields are:
    //  * SCR_EL3.RW for RW
    //  * 0 for FMO/IMO/AMO, TGE and E2H
    uint64_t scr_el3 = arm_is_el3_enabled(env) ? env->cp15.scr_el3 : (SCR_EEL2 | SCR_RW);
    uint64_t hcr_el2;
    if(arm_is_el2_enabled(env)) {
        hcr_el2 = arm_hcr_el2_eff(env);
    } else {
        hcr_el2 = (scr_el3 & SCR_RW) ? HCR_RW : 0x0;
    }

    if(env->aarch64) {
        return process_interrupt_v8a_aarch64(interrupt_request, env, scr_el3, hcr_el2);
    } else {
        return process_interrupt_v8a_aarch32(interrupt_request, env, scr_el3, hcr_el2);
    }
}

void HELPER(rebuild_hflags_a32)(CPUState *env, int el)
{
    //  AARCH64_STATE - whether we execute on arm64
    DP_TBFLAG_ANY(env->hflags, AARCH64_STATE, 0);

    //  SS_ACTIVE - software step active
    DP_TBFLAG_ANY(env->hflags, SS_ACTIVE, 0);

    //  BE - big endian data
    DP_TBFLAG_ANY(env->hflags, BE_DATA, arm_cpu_data_is_big_endian(env));

    ARMMMUIdx mmuidx;
    switch(el) {
        case 3:
            mmuidx = ARMMMUIdx_SE3;
            break;
        case 2:
            mmuidx = ARMMMUIdx_SE2;
            break;
        case 1:
            mmuidx = ARMMMUIdx_SE10_1;
            break;
        case 0:
            mmuidx = ARMMMUIdx_SE10_0;
            break;
        default:
            tlib_abortf("Invalid el: %d", el);
            __builtin_unreachable();
    };
    DP_TBFLAG_ANY(env->hflags, MMUIDX, arm_to_core_mmu_idx(mmuidx));

    //  FPEXC_EL - Target Exception Level for handling Floating-Point-Disabled Exception
    DP_TBFLAG_ANY(env->hflags, FPEXC_EL, get_fp_exc_el(env, el));

    //  ALIGN_MEM - Aligment check enable, SCTLR_ELx.A
    DP_TBFLAG_ANY(env->hflags, ALIGN_MEM, (arm_sctlr(env, el) & SCTLR_A) == SCTLR_A);

    //  PSTATE__IL - Illegal execution state SPSR.IL
    DP_TBFLAG_ANY(env->hflags, PSTATE__IL, (env->pstate & PSTATE_IL) == PSTATE_IL);

    /* A-Profile flags */

    //  VFP enable (ARM floating-point extension enabled)
    bool fpen;
    if(arm_el_is_aa64(env, 1)) {
        fpen = get_fp_exc_el(env, el) == 0;
    } else {
        fpen = FIELD_EX32(env->vfp.xregs[ARM_VFP_FPEXC], FPEXC, EN);
    }
    DP_TBFLAG_A32(env->hflags, VFPEN, fpen);

    //  VFP state
    DP_TBFLAG_A32(env->hflags, VECLEN, env->vfp.vec_len);
    DP_TBFLAG_A32(env->hflags, VECSTRIDE, env->vfp.vec_stride);

    //  Legacy support for alternative big-endian memory model (BE-32)
    DP_TBFLAG_A32(env->hflags, SCTLR__B, arm_sctlr_b(env));

    //  HSTR_ACTIVE - Hyp System Trap register
    //  TODO: Disable for now. Enable when adding virtualization extension
    DP_TBFLAG_A32(env->hflags, HSTR_ACTIVE, 0);

    //  Indicates whether cp register reads and writes by guest code should access
    //  the secure or nonsecure bank of banked registers
    DP_TBFLAG_A32(env->hflags, NS, !access_secure_reg(env));

    //  Indicates that SME Streaming mode is active, and SMCR_ELx.FA64 is not.
    //  This requires an SME trap from AArch32 mode when using NEON.
    //  TODO: Disable for now. Enable when adding scalable matrix extension
    DP_TBFLAG_A32(env->hflags, SME_TRAP_NONSTREAMING, 0);
}

void aarch64_sync_64_to_32(CPUARMState *env)
{
    for(int i = 0; i < 15; i++) {
        env->regs[i] = env->xregs[i];
    }

    env->regs[15] = env->pc;

    env->banked_r14[r14_bank_number(ARM_CPU_MODE_IRQ)] = env->xregs[16];
    env->banked_r13[bank_number(ARM_CPU_MODE_IRQ)] = env->xregs[17];

    env->banked_r14[r14_bank_number(ARM_CPU_MODE_SVC)] = env->xregs[18];
    env->banked_r13[bank_number(ARM_CPU_MODE_SVC)] = env->xregs[19];

    env->banked_r14[r14_bank_number(ARM_CPU_MODE_ABT)] = env->xregs[20];
    env->banked_r13[bank_number(ARM_CPU_MODE_ABT)] = env->xregs[21];

    env->banked_r14[r14_bank_number(ARM_CPU_MODE_UND)] = env->xregs[22];
    env->banked_r13[bank_number(ARM_CPU_MODE_UND)] = env->xregs[23];

    for(int i = 24; i < 29; ++i) {
        env->fiq_regs[i - 24] = env->xregs[i];
    }

    env->banked_r13[bank_number(ARM_CPU_MODE_FIQ)] = env->xregs[29];
    env->banked_r14[r14_bank_number(ARM_CPU_MODE_FIQ)] = env->xregs[30];
}

void aarch64_sync_32_to_64(CPUARMState *env)
{
    for(int i = 0; i < 15; i++) {
        env->xregs[i] = env->regs[i];
    }

    env->pc = env->regs[15];

    env->xregs[16] = env->banked_r14[r14_bank_number(ARM_CPU_MODE_IRQ)];
    env->xregs[17] = env->banked_r13[bank_number(ARM_CPU_MODE_IRQ)];

    env->xregs[18] = env->banked_r14[r14_bank_number(ARM_CPU_MODE_SVC)];
    env->xregs[19] = env->banked_r13[bank_number(ARM_CPU_MODE_SVC)];

    env->xregs[20] = env->banked_r14[r14_bank_number(ARM_CPU_MODE_ABT)];
    env->xregs[21] = env->banked_r13[bank_number(ARM_CPU_MODE_ABT)];

    env->xregs[22] = env->banked_r14[r14_bank_number(ARM_CPU_MODE_UND)];
    env->xregs[23] = env->banked_r13[bank_number(ARM_CPU_MODE_UND)];

    for(int i = 24; i < 29; ++i) {
        env->xregs[i] = env->fiq_regs[i - 24];
    }

    env->xregs[29] = env->banked_r13[bank_number(ARM_CPU_MODE_FIQ)];
    env->xregs[30] = env->banked_r14[r14_bank_number(ARM_CPU_MODE_FIQ)];
}

void TLIB_NORETURN arch_raise_external_abort(CPUState *env, target_ulong address, int access_type, void *retaddr)
{
    if(is_a64(env)) {
        //  Based on: Arm Architecture Reference Manual for A-profile architecture DDI0487
        //  J1.1.2.18 AArch64.TakePhysicalSErrorException
        env->exception.vaddress = address;
        raise_exception(env, EXCP_SERR, syn_serror(), exception_target_el(env));
    } else {
        if(unlikely(arm_feature(env, ARM_FEATURE_M))) {
            tlib_abortf("Signaling external aborts is not implemented for Cortex-M cores of the arm64 guest");
            tlib_assert_not_reached();
        }

        //  Based on: Arm Architecture Reference Manual Supplement, Armv8, for the Armv8-R AArch32 architecture profile
        //  DDI0568A.c, H1.2.2 aarch32/exceptions/asynch/AArch32.TakePhysicalSErrorException
        int target_el = 1;
        int current_el = arm_current_el(env);
        if(arm_feature(env, ARM_FEATURE_EL2) && !arm_is_secure(env) &&
           check_hcr_el2(arm_hcr_el2_eff(env), /*tge=*/1, /*amo=*/1, -1, -1, -1, -1)) {
            target_el = 2;
        }
        set_mmu_fault_registers(ACCESS_DATA_LOAD, address, SERROR_FAULT);
        env->exception.target_el = target_el;
        if(target_el == 2 && current_el != 2) {
            env->exception_index = EXCP_HYP_TRAP;
        }
        arch_raise_mmu_fault_exception(env, TRANSLATE_FAIL, access_type, address, retaddr);
    }
    tlib_assert_not_reached();
}

#define STUB_MVE_HELPER(name)                                               \
    void HELPER(mve_##name)(uint64_t a, uint64_t b, uint64_t c, uint32_t d) \
    {                                                                       \
        (void)a;                                                            \
        (void)b;                                                            \
        (void)c;                                                            \
        (void)d;                                                            \
        tlib_abortf("Stub encountered: %s", #name);                         \
    }

STUB_MVE_HELPER(sqrshrl48)
STUB_MVE_HELPER(sqrshrl)
STUB_MVE_HELPER(sqrshr)
STUB_MVE_HELPER(sshrl)
STUB_MVE_HELPER(uqrshll48)
STUB_MVE_HELPER(uqrshll)
STUB_MVE_HELPER(uqrshl)
STUB_MVE_HELPER(ushll)
