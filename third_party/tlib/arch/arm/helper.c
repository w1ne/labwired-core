#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bit_helper.h"
#include "cpu.h"
#include "helper.h"
#include "host-utils.h"
#include "infrastructure.h"
#include "translate_trustzone.h"
#include "arch_callbacks.h"
#include "pmu.h"

#ifdef HOST_WORDS_BIGENDIAN
#warning "FP state preservation will not work correctly on big-endian hosts"
#endif

static uint32_t cortexa15_cp15_c0_c1[8] = { 0x00001131, 0x00011011, 0x02010555, 0x00000000,
                                            0x10201105, 0x20000000, 0x01240000, 0x02102211 };

static uint32_t cortexr5_cp15_c0_c1[8] = { 0x00000131, 0x00000001, 0x00010400, 0x00000000,
                                           0x00110130, 0x00000000, 0x01200000, 0x00000211 };

static uint32_t cortexr8_cp15_c0_c1[8] = { 0x00000131, 0x00000001, 0x00010404, 0x00000000,
                                           0x00210030, 0x00000000, 0x01200000, 0x00002111 };

static uint32_t cortexa15_cp15_c0_c2[8] = { 0x02101110, 0x13112111, 0x21232041, 0x11112131, 0x10011142, 0, 0, 0 };

//  since Cortex-R5, r1p0
static uint32_t cortexr5_cp15_c0_c2[8] = { 0x02101111, 0x13112111, 0x21232141, 0x01112131, 0x00010142, 0, 0, 0 };

static uint32_t cortexr8_cp15_c0_c2[8] = { 0x02101111, 0x13112111, 0x21232141, 0x01112131, 0x00010142, 0, 0, 0 };

static uint32_t cortexa9_cp15_c0_c1[8] = { 0x1031, 0x11, 0x000, 0, 0x00100103, 0x20000000, 0x01230000, 0x00002111 };

static uint32_t cortexa9_cp15_c0_c2[8] = { 0x00101111, 0x13112111, 0x21232041, 0x11112131, 0x00111142, 0, 0, 0 };

static uint32_t cortexa8_cp15_c0_c1[8] = { 0x1031, 0x11, 0x400, 0, 0x31100003, 0x20000000, 0x01202000, 0x11 };

static uint32_t cortexa8_cp15_c0_c2[8] = { 0x00101111, 0x12112111, 0x21232031, 0x11112131, 0x00111142, 0, 0, 0 };

static uint32_t mpcore_cp15_c0_c1[8] = { 0x111, 0x1, 0, 0x2, 0x01100103, 0x10020302, 0x01222000, 0 };

static uint32_t mpcore_cp15_c0_c2[8] = { 0x00100011, 0x12002111, 0x11221011, 0x01102131, 0x141, 0, 0, 0 };

static uint32_t arm1136_cp15_c0_c1[8] = { 0x111, 0x1, 0x2, 0x3, 0x01130003, 0x10030302, 0x01222110, 0 };

static uint32_t arm1136_cp15_c0_c2[8] = { 0x00140011, 0x12002111, 0x11231111, 0x01102131, 0x141, 0, 0, 0 };

static uint32_t arm1176_cp15_c0_c1[8] = { 0x111, 0x11, 0x33, 0, 0x01130003, 0x10030302, 0x01222100, 0 };

static uint32_t arm1176_cp15_c0_c2[8] = { 0x0140011, 0x12002111, 0x11231121, 0x01102131, 0x01141, 0, 0, 0 };

static uint32_t cortexa7_cp15_c0_c1[8] = {
    0x00001131, 0x00011011, 0x02010555, 0, 0x10101105, 0x40000000, 0x01240000, 0x02102211
};

static uint32_t cortexa7_cp15_c0_c2[8] = { 0x01101110, 0x13112111, 0x21232041, 0x11112131, 0x10011142, 0, 0, 0 };

static uint32_t cpu_arm_find_by_name(const char *name);

static inline void set_feature(CPUState *env, int feature)
{
    env->features |= 1u << feature;
}

static void cpu_reset_model_id(CPUState *env, uint32_t id)
{
    env->cp15.c0_cpuid = id;
    switch(id) {
        case ARM_CPUID_ARM7TDMI:
            set_feature(env, ARM_FEATURE_V4T);
            break;
        case ARM_CPUID_ARM926EJ_S:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_VFP);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x41011090;
            env->cp15.c0_cachetype = 0x1dd20d2;
            env->cp15.c1_sys = 0x00090078;
            break;
        case ARM_CPUID_ARM946E_S:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_MPU);
            env->cp15.c0_cachetype = 0x0f004006;
            env->cp15.c1_sys = 0x00000078;
            break;
        case ARM_CPUID_ARM1026EJ_S:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_AUXCR);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x410110a0;
            env->cp15.c0_cachetype = 0x1dd20d2;
            env->cp15.c1_sys = 0x00090078;
            break;
        case ARM_CPUID_ARM1136JF_S:
            /* This is the 1136 r1, which is a v6K core */
            set_feature(env, ARM_FEATURE_V6K);
        /* Fall through */
        case ARM_CPUID_ARM1136JF_S_R2:
            /* What qemu calls "arm1136_r2" is actually the 1136 r0p2, ie an
             * older core than plain "arm1136". In particular this does not
             * have the v6K features.
             */
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_AUXCR);
            /* These ID register values are correct for 1136 but may be wrong
             * for 1136_r2 (in particular r0p2 does not actually implement most
             * of the ID registers).
             */
            env->vfp.xregs[ARM_VFP_FPSID] = 0x410120b4;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x11111111;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x00000000;
            memcpy(env->cp15.c0_c1, arm1136_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, arm1136_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x1dd20d2;
            env->cp15.c1_sys = 0x00050078;
            break;
        case ARM_CPUID_ARM1176JZF_S:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_VAPA);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x410120b5;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x11111111;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x00000000;
            memcpy(env->cp15.c0_c1, arm1176_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, arm1176_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x1dd20d2;
            env->cp15.c1_sys = 0x00050078;
            break;
        case ARM_CPUID_ARM11MPCORE:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_VAPA);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x410120b4;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x11111111;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x00000000;
            memcpy(env->cp15.c0_c1, mpcore_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, mpcore_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x1dd20d2;
            break;
        case ARM_CPUID_CORTEXA7:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7SEC);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_NEON);
            set_feature(env, ARM_FEATURE_GENERIC_TIMER);
            set_feature(env, ARM_FEATURE_THUMB2EE);
            set_feature(env, ARM_FEATURE_LPAE);
            set_feature(env, ARM_FEATURE_V7MP);
            set_feature(env, ARM_FEATURE_ARM_DIV);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x41023070;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x10110221;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x11000011;
            memcpy(env->cp15.c0_c1, cortexa7_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexa7_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x84448003;
            env->cp15.c0_clid = (1 << 27) | (2 << 24) | (1 << 21) | 3;
            env->cp15.c0_ccsid[0] = 0x7003E01A; /* 8k L1 dcache */
            env->cp15.c0_ccsid[1] = 0x200FE009; /* 8k L1 icache */
            env->cp15.c0_ccsid[2] = 0x701FE03A; /* 128k L2 cache */
            env->cp15.c1_sys = 0x00C50078;
            break;
        case ARM_CPUID_CORTEXA8:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7SEC);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_NEON);
            set_feature(env, ARM_FEATURE_THUMB2EE);
            set_feature(env, ARM_FEATURE_LPAE);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x410330c0;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x11110222;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x00011100;
            memcpy(env->cp15.c0_c1, cortexa8_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexa8_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x82048004;
            env->cp15.c0_clid = (1 << 27) | (2 << 24) | 3;
            env->cp15.c0_ccsid[0] = 0xe007e01a; /* 16k L1 dcache. */
            env->cp15.c0_ccsid[1] = 0x2007e01a; /* 16k L1 icache. */
            env->cp15.c0_ccsid[2] = 0xf0000000; /* No L2 icache. */
            env->cp15.c1_sys = 0x00c50078;
            break;
        //  treating A5 as A9 is a simplification and should
        //  be improved in the future
        case ARM_CPUID_CORTEXA5:
        case ARM_CPUID_CORTEXA9:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7SEC);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_NEON);
            set_feature(env, ARM_FEATURE_THUMB2EE);
            /* Note that A9 supports the MP extensions even for
             * A9UP and single-core A9MP (which are both different
             * and valid configurations; we don't model A9UP).
             */
            set_feature(env, ARM_FEATURE_V7MP);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x41034000; /* Guess */
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x11110222;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x01111111;
            memcpy(env->cp15.c0_c1, cortexa9_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexa9_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x80038003;
            env->cp15.c0_clid = (1 << 27) | (1 << 24) | 3;
            env->cp15.c0_ccsid[0] = 0xe00fe015; /* 16k L1 dcache. */
            env->cp15.c0_ccsid[1] = 0x200fe015; /* 16k L1 icache. */
            env->cp15.c1_sys = 0x00c50078;
            break;
        case ARM_CPUID_CORTEXA15:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7SEC);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_NEON);
            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_GENERIC_TIMER);
            set_feature(env, ARM_FEATURE_THUMB2EE);
            set_feature(env, ARM_FEATURE_ARM_DIV);
            set_feature(env, ARM_FEATURE_V7MP);
            set_feature(env, ARM_FEATURE_LPAE);
            env->vfp.xregs[ARM_VFP_FPSID] = 0x410430f0;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x10110222;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x11111111;
            memcpy(env->cp15.c0_c1, cortexa15_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexa15_cp15_c0_c2, 8 * sizeof(uint32_t));
            env->cp15.c0_cachetype = 0x8444c004;
            env->cp15.c0_clid = 0x0a200023;
            env->cp15.c0_ccsid[0] = 0x701fe00a; /* 32K L1 dcache */
            env->cp15.c0_ccsid[1] = 0x201fe00a; /* 32K L1 icache */
            env->cp15.c0_ccsid[2] = 0x711fe07a; /* 4096K L2 unified cache */
            env->cp15.c1_sys = 0x00c50078;
            break;
#ifdef TARGET_PROTO_ARM_M
        case ARM_CPUID_CORTEXM85:
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_VFP5);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP);

            set_feature(env, ARM_FEATURE_V8_1M);
            set_feature(env, ARM_FEATURE_V8);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_MVE);
            set_feature(env, ARM_FEATURE_DSP);
            break;
        case ARM_CPUID_CORTEXM7:
            set_feature(env, ARM_FEATURE_VFP5);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP);

            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_DSP);
            break;
        case ARM_CPUID_CORTEXM52:
        case ARM_CPUID_CORTEXM55:
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_VFP5);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP);

            set_feature(env, ARM_FEATURE_V8_1M);
            set_feature(env, ARM_FEATURE_V8);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_MVE);
            set_feature(env, ARM_FEATURE_DSP);
            break;
        case ARM_CPUID_CORTEXM4:
            //  TODO: Remove FPU from non-f variant CPUs
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP);

            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_DSP);
            break;
        case ARM_CPUID_CORTEXM33:
            set_feature(env, ARM_FEATURE_VFP5);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP);

            set_feature(env, ARM_FEATURE_V8);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_DSP);
            break;
        case ARM_CPUID_CORTEXM3:
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_THUMB2);
            break;
        case ARM_CPUID_CORTEXM23:
            set_feature(env, ARM_FEATURE_V8);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_THUMB2);
            break;
        case ARM_CPUID_CORTEXM0:
            //  TODO: Those should not be present on M0 processors,
            //        but some of our samples break without them.
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_THUMB_DIV);
            set_feature(env, ARM_FEATURE_MPU);

            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V4T);

            set_feature(env, ARM_FEATURE_THUMB2);
            break;
#endif
        case ARM_CPUID_ANY: /* For userspace emulation.  */
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP4);
            set_feature(env, ARM_FEATURE_VFP5);
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_NEON);
            set_feature(env, ARM_FEATURE_THUMB2EE);
            set_feature(env, ARM_FEATURE_ARM_DIV);
            set_feature(env, ARM_FEATURE_V7MP);
            break;
        case ARM_CPUID_TI915T:
        case ARM_CPUID_TI925T:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_OMAPCP);
            env->cp15.c0_cpuid = ARM_CPUID_TI925T; /* Depends on wiring.  */
            env->cp15.c0_cachetype = 0x5109149;
            env->cp15.c1_sys = 0x00000070;
            env->cp15.c15_i_max = 0x000;
            env->cp15.c15_i_min = 0xff0;
            break;
        case ARM_CPUID_PXA250:
        case ARM_CPUID_PXA255:
        case ARM_CPUID_PXA260:
        case ARM_CPUID_PXA261:
        case ARM_CPUID_PXA262:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_XSCALE);
            /* JTAG_ID is ((id << 28) | 0x09265013) */
            env->cp15.c0_cachetype = 0xd172172;
            env->cp15.c1_sys = 0x00000078;
            break;
        case ARM_CPUID_PXA270_A0:
        case ARM_CPUID_PXA270_A1:
        case ARM_CPUID_PXA270_B0:
        case ARM_CPUID_PXA270_B1:
        case ARM_CPUID_PXA270_C0:
        case ARM_CPUID_PXA270_C5:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_XSCALE);
            /* JTAG_ID is ((id << 28) | 0x09265013) */
            set_feature(env, ARM_FEATURE_IWMMXT);
            env->iwmmxt.cregs[ARM_IWMMXT_wCID] = 0x69051000 | 'Q';
            env->cp15.c0_cachetype = 0xd172172;
            env->cp15.c1_sys = 0x00000078;
            break;
        case ARM_CPUID_SA1100:
        case ARM_CPUID_SA1110:
            set_feature(env, ARM_FEATURE_STRONGARM);
            env->cp15.c1_sys = 0x00000070;
            break;
        case ARM_CPUID_CORTEXR5F:
            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_NEON);

            env->vfp.xregs[ARM_VFP_FPSID] = 0x41023150;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x10110221;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x00000011;
            /* fallthrough */
        case ARM_CPUID_CORTEXR5:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7MP);
            set_feature(env, ARM_FEATURE_ARM_DIV);  //  not for rp0p0

            set_feature(env, ARM_FEATURE_THUMB2);
            set_feature(env, ARM_FEATURE_THUMB_DIV);

            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_GENERIC_TIMER);
            set_feature(env, ARM_FEATURE_PMSA);

            memcpy(env->cp15.c0_c1, cortexr5_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexr5_cp15_c0_c2, 8 * sizeof(uint32_t));

            env->cp15.c0_cachetype = 0x80030003;  //  CTR
            env->cp15.c0_tcmtype = 0x00010001;    //  TCMTR
            env->cp15.c0_clid = 0x09200003;       //  CLIDR, for all caches implemented
            env->cp15.c0_ccsid[0] = 0xf01fe019;   /* 32K L1 dcache */
            env->cp15.c0_ccsid[1] = 0xf01fe019;   /* 32K L1 icache */

            env->cp15.c1_sys = 0xe50878;                                             //  SCTLR
            env->cp15.c1_coproc |= (1 << 30 /* D32DIS */) | (1 << 31 /* ASEDIS */);  //  CPACR
            break;
        case ARM_CPUID_CORTEXR8:
            set_feature(env, ARM_FEATURE_V4T);
            set_feature(env, ARM_FEATURE_V5);
            set_feature(env, ARM_FEATURE_V6);
            set_feature(env, ARM_FEATURE_V6K);
            set_feature(env, ARM_FEATURE_V7);
            set_feature(env, ARM_FEATURE_V7MP);
            set_feature(env, ARM_FEATURE_ARM_DIV);

            set_feature(env, ARM_FEATURE_VFP);
            set_feature(env, ARM_FEATURE_VFP3);
            set_feature(env, ARM_FEATURE_VFP_FP16);
            set_feature(env, ARM_FEATURE_NEON);

            set_feature(env, ARM_FEATURE_THUMB2);

            set_feature(env, ARM_FEATURE_AUXCR);
            set_feature(env, ARM_FEATURE_CBAR_RO);
            set_feature(env, ARM_FEATURE_GENERIC_TIMER);
            set_feature(env, ARM_FEATURE_PMSA);

            env->vfp.xregs[ARM_VFP_FPSID] = 0x41023180;
            env->vfp.xregs[ARM_VFP_MVFR0] = 0x10110021 | /* if f64 supported */ 0x00000200;
            env->vfp.xregs[ARM_VFP_MVFR1] = 0x01000011;

            memcpy(env->cp15.c0_c1, cortexr8_cp15_c0_c1, 8 * sizeof(uint32_t));
            memcpy(env->cp15.c0_c2, cortexr8_cp15_c0_c2, 8 * sizeof(uint32_t));

            env->cp15.c0_cachetype = 0x8333C003;  //  CTR
            env->cp15.c0_tcmtype = 0x80010001;    //  TCMTR
            env->cp15.c0_clid = 0x09200003;       //  CLIDR, for cache implemented
            env->cp15.c0_ccsid[0] = 0x701fe019;   /* 32K L1 dcache */
            env->cp15.c0_ccsid[1] = 0x201fe019;   /* 32K L1 icache */
            env->cp15.c1_sys = 0xc50078;          //  SCTLR
            env->cp15.c1_coproc = 0xC0000000;     //  CPACR
            break;
        default:
            cpu_abort(env, "Bad CPU ID: %x\n", id);
            break;
    }

    /* Some features automatically imply others: */
    if(arm_feature(env, ARM_FEATURE_V7)) {
        set_feature(env, ARM_FEATURE_VAPA);
    }
    if(arm_feature(env, ARM_FEATURE_ARM_DIV)) {
        set_feature(env, ARM_FEATURE_THUMB_DIV);
    }
    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        set_feature(env, ARM_FEATURE_MPU);
    }
}

void system_instructions_and_registers_reset(CPUState *env);
void system_instructions_and_registers_init(CPUState *env);

void cpu_on_leaving_reset_state(CPUState *env)
{
    configuration_signals_apply(env);
}

void cpu_reset(CPUState *env)
{
    uint32_t id = env->cp15.c0_cpuid;
    uint32_t number_of_mpu_regions = env->number_of_mpu_regions;
#ifdef TARGET_PROTO_ARM_M
    bool was_locked_up = env->v7m.locked_up;
    uint32_t number_of_idau_regions = env->number_of_idau_regions;
    uint32_t number_of_sau_regions = env->number_of_sau_regions;
#endif
    memset(env, 0, RESET_OFFSET);
    if(id) {
        cpu_reset_model_id(env, id);
    }
    env->number_of_mpu_regions = number_of_mpu_regions;
#ifdef TARGET_PROTO_ARM_M
    env->number_of_idau_regions = number_of_idau_regions;
    env->number_of_sau_regions = number_of_sau_regions;
    if(was_locked_up) {
        /* Armv8-M ARM rule RXQSR: a Cold or Warm reset exits Lockup. */
        tlib_on_lockup_state_change(false);
    }
#endif
    /* SVC mode with interrupts disabled.  */
    env->uncached_cpsr = ARM_CPU_MODE_SVC | CPSR_A | CPSR_F | CPSR_I;

#ifdef TARGET_PROTO_ARM_M
    env->v7m.has_trustzone = tlib_has_enabled_trustzone() > 0;
    //  Set initial Security State to Secure if there is TrustZone support
    env->secure = env->v7m.has_trustzone;

    if(!env->v7m.has_trustzone) {
        /* If TrustZone is disabled, set NSACR to value that would allow access to FPU from Non-secure state */
        env->v7m.nsacr = 0xcff;
    }

    fpccr_write(env, (fpccr_read(env, false) & ~ARM_FPCCR_LSPACT_MASK) | ARM_FPCCR_ASPEN_MASK | ARM_FPCCR_LSPEN_MASK, false);
    fpccr_write(env, (fpccr_read(env, true) & ~ARM_FPCCR_LSPACT_MASK) | ARM_FPCCR_ASPEN_MASK | ARM_FPCCR_LSPEN_MASK, true);
#endif

#ifdef TARGET_PROTO_ARM_M
    /* ltpsize field resets to 4 when Low Overhead Branch (LOB) extension is present.
     * LOB is required extension for v8.1M so we gate it behind this check.
     * This field should be RES0 if cpu doesn't implement LOB.
     */
    if(arm_feature(env, ARM_FEATURE_V8_1M)) {
        env->v7m.ltpsize = LTPSIZE_PREDICATION_DISABLED;
        env->v7m.fpdscr[M_REG_NS] = FIELD_DP32(0, VFP_FPSCR, LTPSIZE, LTPSIZE_PREDICATION_DISABLED);
        env->v7m.fpdscr[M_REG_S] = FIELD_DP32(0, VFP_FPSCR, LTPSIZE, LTPSIZE_PREDICATION_DISABLED);
    }
#endif

    env->vfp.xregs[ARM_VFP_FPEXC] = 0;
    env->cp15.c2_base_mask = 0xffffc000u;
    /* v7 performance monitor control register: same implementor
     * field as main ID register, and we implement no event counters.
     */
    env->cp15.c9_pmcr = (env->cp15.c0_cpuid & 0xff000000);

    set_flush_to_zero(1, &env->vfp.standard_fp_status);
    set_flush_inputs_to_zero(1, &env->vfp.standard_fp_status);
    set_default_nan_mode(1, &env->vfp.standard_fp_status);
    set_float_detect_tininess(float_tininess_before_rounding, &env->vfp.fp_status);
    set_float_detect_tininess(float_tininess_before_rounding, &env->vfp.standard_fp_status);

    system_instructions_and_registers_reset(env);

    pmu_init_reset(env);
}

int cpu_init(const char *cpu_model)
{
    uint32_t id = cpu_arm_find_by_name(cpu_model);
    if(id == 0) {
        return -1;
    }
    cpu->cp15.c0_cpuid = id;

    //  We need this to set CPU feature flags, before calling `system_instructions_and_registers_init`
    cpu_reset_model_id(env, id);

    system_instructions_and_registers_init(env);

    cpu_reset(cpu);
    return 0;
}

struct arm_cpu_t {
    uint32_t id;
    const char *name;
};

static const struct arm_cpu_t arm_cpu_names[] = {
    { ARM_CPUID_ARM7TDMI,       "arm7tdmi"       },
    { ARM_CPUID_ARM926EJ_S,     "arm926ej-s"     },
    { ARM_CPUID_ARM946E_S,      "arm946e-s"      },
    { ARM_CPUID_ARM1026EJ_S,    "arm1026ej-s"    },
    { ARM_CPUID_ARM1136JF_S,    "arm1136jf-s"    },
    { ARM_CPUID_ARM1136JF_S_R2, "arm1136jf-s-r2" },
    { ARM_CPUID_ARM1176JZF_S,   "arm1176jzf-s"   },
    { ARM_CPUID_ARM11MPCORE,    "arm11mpcore"    },

    //  TODO: M0+ shouldn't be the same as M3. It doesn't support hardware division.
    { ARM_CPUID_CORTEXM0,       "cortex-m0"      },
    { ARM_CPUID_CORTEXM3,       "cortex-m0+"     },
    { ARM_CPUID_CORTEXM3,       "cortex-m1"      },
    { ARM_CPUID_CORTEXM23,      "cortex-m23"     },
    { ARM_CPUID_CORTEXM3,       "cortex-m3"      },
    { ARM_CPUID_CORTEXM33,      "cortex-m33"     },
    //  TODO: M4F should be separate from M4.
    { ARM_CPUID_CORTEXM4,       "cortex-m4"      },
    { ARM_CPUID_CORTEXM7,       "cortex-m4f"     },
    { ARM_CPUID_CORTEXM52,      "cortex-m52"     },
    { ARM_CPUID_CORTEXM55,      "cortex-m55"     },
    { ARM_CPUID_CORTEXM7,       "cortex-m7"      },
    { ARM_CPUID_CORTEXM85,      "cortex-m85"     },

    { ARM_CPUID_CORTEXR5,       "cortex-r5"      },
    { ARM_CPUID_CORTEXR5F,      "cortex-r5f"     },
    { ARM_CPUID_CORTEXR8,       "cortex-r8"      },

    { ARM_CPUID_CORTEXA5,       "cortex-a5"      },
    { ARM_CPUID_CORTEXA7,       "cortex-a7"      },
    { ARM_CPUID_CORTEXA8,       "cortex-a8"      },
    { ARM_CPUID_CORTEXA9,       "cortex-a9"      },
    { ARM_CPUID_CORTEXA15,      "cortex-a15"     },

    { ARM_CPUID_TI925T,         "ti925t"         },
    { ARM_CPUID_PXA250,         "pxa250"         },
    { ARM_CPUID_SA1100,         "sa1100"         },
    { ARM_CPUID_SA1110,         "sa1110"         },
    { ARM_CPUID_PXA255,         "pxa255"         },
    { ARM_CPUID_PXA260,         "pxa260"         },
    { ARM_CPUID_PXA261,         "pxa261"         },
    { ARM_CPUID_PXA262,         "pxa262"         },
    { ARM_CPUID_PXA270,         "pxa270"         },
    { ARM_CPUID_PXA270_A0,      "pxa270-a0"      },
    { ARM_CPUID_PXA270_A1,      "pxa270-a1"      },
    { ARM_CPUID_PXA270_B0,      "pxa270-b0"      },
    { ARM_CPUID_PXA270_B1,      "pxa270-b1"      },
    { ARM_CPUID_PXA270_C0,      "pxa270-c0"      },
    { ARM_CPUID_PXA270_C5,      "pxa270-c5"      },
    { ARM_CPUID_ANY,            "any"            },
    { 0,                        NULL             }
};

/* return 0 if not found */
static uint32_t cpu_arm_find_by_name(const char *name)
{
    int i;
    uint32_t id;

    id = 0;
    for(i = 0; arm_cpu_names[i].name; i++) {
        if(strcmp(name, arm_cpu_names[i].name) == 0) {
            id = arm_cpu_names[i].id;
            break;
        }
    }
    return id;
}

uint32_t cpsr_read(CPUState *env)
{
    int ZF;
    ZF = (env->ZF == 0);
    return env->uncached_cpsr | (env->NF & 0x80000000) | (ZF << 30) | (env->CF << 29) | ((env->VF & 0x80000000) >> 3) |
           (env->QF << 27) | (env->thumb << 5) | ((env->condexec_bits & 3) << 25) | ((env->condexec_bits & 0xfc) << 8) |
           (env->GE << 16);
}

void cpsr_write(CPUState *env, uint32_t val, uint32_t mask)
{
    if(mask & CPSR_NZCV) {
        env->ZF = (~val) & CPSR_Z;
        env->NF = val;
        env->CF = (val >> 29) & 1;
        env->VF = (val << 3) & 0x80000000;
    }
    if(mask & CPSR_Q) {
        env->QF = ((val & CPSR_Q) != 0);
    }
    if(mask & CPSR_T) {
        env->thumb = ((val & CPSR_T) != 0);
    }
    if(mask & CPSR_IT_0_1) {
        env->condexec_bits &= ~3;
        env->condexec_bits |= (val >> 25) & 3;
    }
    if(mask & CPSR_IT_2_7) {
        env->condexec_bits &= 3;
        env->condexec_bits |= (val >> 8) & 0xfc;
    }
    if(mask & CPSR_GE) {
        env->GE = (val >> 16) & 0xf;
    }

    if((env->uncached_cpsr ^ val) & mask & CPSR_M) {
        switch_mode(env, val & CPSR_M);
    }
    mask &= ~CACHED_CPSR_BITS;
    env->uncached_cpsr = (env->uncached_cpsr & ~mask) | (val & mask);

    refresh_pending_irq();
}

/* Sign/zero extend */
uint32_t HELPER(sxtb16)(uint32_t x)
{
    uint32_t res;
    res = (uint16_t)(int8_t)x;
    res |= (uint32_t)(int8_t)(x >> 16) << 16;
    return res;
}

uint32_t HELPER(uxtb16)(uint32_t x)
{
    uint32_t res;
    res = (uint16_t)(uint8_t)x;
    res |= (uint32_t)(uint8_t)(x >> 16) << 16;
    return res;
}

uint32_t HELPER(clz)(uint32_t x)
{
    return clz32(x);
}

int32_t HELPER(sdiv)(CPUState *env, int32_t num, int32_t den)
{
    if(den == 0) {
#ifdef TARGET_PROTO_ARM_M
        if(env->v7m.ccr[env->secure] & FIELD_MASK(V7M_CCR, DIV_0_TRP)) {
            env->exception_index = EXCP_DIV_0;
            cpu_loop_exit(env);
        }
#endif
        return 0;
    }
    if(num == INT_MIN && den == -1) {
        return INT_MIN;
    }
    return num / den;
}

uint32_t HELPER(udiv)(CPUState *env, uint32_t num, uint32_t den)
{
    if(den == 0) {
#ifdef TARGET_PROTO_ARM_M
        if(env->v7m.ccr[env->secure] & FIELD_MASK(V7M_CCR, DIV_0_TRP)) {
            env->exception_index = EXCP_DIV_0;
            cpu_loop_exit(env);
        }
#endif
        return 0;
    }
    return num / den;
}

uint32_t HELPER(rbit)(uint32_t x)
{
    x = ((x & 0xff000000) >> 24) | ((x & 0x00ff0000) >> 8) | ((x & 0x0000ff00) << 8) | ((x & 0x000000ff) << 24);
    x = ((x & 0xf0f0f0f0) >> 4) | ((x & 0x0f0f0f0f) << 4);
    x = ((x & 0x88888888) >> 3) | ((x & 0x44444444) >> 1) | ((x & 0x22222222) << 1) | ((x & 0x11111111) << 3);
    return x;
}

uint32_t HELPER(abs)(uint32_t x)
{
    return ((int32_t)x < 0) ? -x : x;
}

/* Map CPU modes onto saved register banks.  */
/* 26-bit mode currently affects only the bank number. */
static inline int bank_number(int mode)
{
    switch(mode) {
        case ARM_CPU_MODE_USR:
        case ARM_CPU_MODE_USR26:
        case ARM_CPU_MODE_SYS:
            return 0;
        case ARM_CPU_MODE_SVC:
        case ARM_CPU_MODE_SVC26:
            return 1;
        case ARM_CPU_MODE_ABT:
            return 2;
        case ARM_CPU_MODE_UND:
            return 3;
        case ARM_CPU_MODE_IRQ:
        case ARM_CPU_MODE_IRQ26:
            return 4;
        case ARM_CPU_MODE_FIQ:
        case ARM_CPU_MODE_FIQ26:
            return 5;
    }
    cpu_abort(cpu, "Bad mode %x\n", mode);
    return -1;
}

void switch_mode(CPUState *env, int mode)
{
    int old_mode;
    int i;

    old_mode = env->uncached_cpsr & CPSR_M;
    if(mode == old_mode) {
        return;
    }

    //  PMU only has to be informed about changes between Privilege Levels
    //  but it doesn't care about mode changes within the same PL
    if(unlikely(env->pmu.counters_enabled) && (mode == ARM_CPU_MODE_USR || old_mode == ARM_CPU_MODE_USR)) {
        pmu_switch_mode_user(mode);
    }

    if(old_mode == ARM_CPU_MODE_FIQ) {
        memcpy(env->fiq_regs, env->regs + 8, 5 * sizeof(uint32_t));
        memcpy(env->regs + 8, env->usr_regs, 5 * sizeof(uint32_t));
    } else if(mode == ARM_CPU_MODE_FIQ) {
        memcpy(env->usr_regs, env->regs + 8, 5 * sizeof(uint32_t));
        memcpy(env->regs + 8, env->fiq_regs, 5 * sizeof(uint32_t));
    }

    i = bank_number(old_mode);
    env->banked_r13[i] = env->regs[13];
    env->banked_r14[i] = env->regs[14];
    env->banked_spsr[i] = env->spsr;

    i = bank_number(mode);
    env->regs[13] = env->banked_r13[i];
    env->regs[14] = env->banked_r14[i];
    env->spsr = env->banked_spsr[i];
}

static inline void arm_announce_stack_change()
{
    if(unlikely(env->guest_profiler_enabled)) {
        tlib_announce_stack_change(CPU_PC(env), STACK_FRAME_ADD);
    }
}

#ifdef TARGET_PROTO_ARM_M
static int v7m_exception_number_with_security(CPUState *env, int exception, bool secure)
{
    if(!env->v7m.has_trustzone || exception >= ARMV7M_EXCP_HARDIRQ0) {
        return exception;
    }

    switch(exception) {
        case ARMV7M_EXCP_NMI:
        case ARMV7M_EXCP_BUS:
        case ARMV7M_EXCP_RESET:
        case ARMV7M_EXCP_SECURE:
            return exception;
        case ARMV7M_EXCP_HARD:
            /* HardFault is banked only when AIRCR.BFHFNMINS is one. */
            if(tlib_nvic_interrupt_targets_secure(exception)) {
                return exception;
            }
            break;
        default:
            break;
    }

    return secure ? BANKED_SECURE_EXCP(exception) : exception;
}

static void v7m_enter_lockup(CPUState *env, bool clear_itstate);

static int v7m_push(CPUState *env, uint32_t val)
{
    uint32_t phys_ptr = 0;
    target_ulong page_size = 0;
    int ret, prot = 0;
    uint32_t address = env->regs[13] - 4;

    /* PushStack() allocates the complete frame even if a stacking access
     * faults. Rule RQSSB permits abandoning the remaining memory accesses and
     * we choose to abandon them while still advancing the architectural SP. */
    env->regs[13] = address;
    if(env->v7m.exception_phase_fault != 0) {
        return 1;
    }

    int exception_index = env->exception_index;
    ret = get_phys_addr(env, address, env->secure, ACCESS_DATA_STORE, !in_privileged_mode(env), &phys_ptr, &prot, &page_size,
                        false);
    if(ret == TRANSLATE_SUCCESS) {
        stl_phys(env->regs[13], val);
        if(env->v7m.exception_phase_fault != 0) {
            env->exception_index = exception_index;
            return 1;
        }
    } else if(env->exception_index == EXCP_SECURE) {
        /* ValidateAddress(AccType_STACK) preserves a Security attribution
         * failure as an AUVIOL SecureFault. Do not overwrite it with the
         * MemManage stacking syndrome merely because both failures are
         * reported by get_phys_addr(). */
        if(env->v7m.exception_phase_fault == 0) {
            env->v7m.exception_phase_fault = ARMV7M_EXCP_SECURE;
        }
    } else {
        /* An MPU failure during PushStack() is a MemManage stacking fault.
         * get_phys_addr() has already recorded the fault address and access
         * classification; add the stacking-specific syndrome here. */
        env->v7m.fault_status[env->secure] |= MEM_FAULT_MSTKERR;
        if(env->v7m.exception_phase_fault == 0) {
            env->v7m.exception_phase_fault = v7m_exception_number_with_security(env, ARMV7M_EXCP_MEM, env->secure);
        }
    }
    env->exception_index = exception_index;
    if(ret != TRANSLATE_SUCCESS) {
        return 1;
    }
    return 0;
}

static uint32_t v7m_pop(CPUState *env)
{
    uint32_t val;
    val = ldl_phys(env->regs[13]);
    env->regs[13] += 4;
    return val;
}

static uint32_t v7m_pop_exception_frame(CPUState *env, bool is_user)
{
    uint32_t val = 0;
    tlib_assert(env->v7m.exception_phase == V7M_EXCEPTION_PHASE_UNSTACKING);

    /* PopStack() abandons frame reads after the first fault, but continues
     * advancing SP so the complete frame can be consumed on Lockup. */
    if(env->v7m.exception_phase_fault == 0) {
        uint32_t phys_ptr = 0;
        target_ulong page_size = 0;
        int prot = 0;
        int exception_index = env->exception_index;
        int ret = get_phys_addr(env, env->regs[13], env->secure, ACCESS_DATA_LOAD, is_user, &phys_ptr, &prot, &page_size, false);
        if(ret == TRANSLATE_SUCCESS) {
            val = ldl_phys(phys_ptr);
        } else if(env->exception_index == EXCP_SECURE) {
            /* Stack() uses the destination Security state for unstacking.
             * A Non-secure read of Secure stack memory is therefore an
             * AUVIOL SecureFault, not a MemManage unstacking fault. */
            env->v7m.exception_phase_fault = ARMV7M_EXCP_SECURE;
        } else {
            /* An MPU failure during PopStack() is a MemManage unstacking
             * fault. get_phys_addr() has already recorded the address. */
            env->v7m.fault_status[env->secure] |= MEM_FAULT_MUNSTKERR;
            env->v7m.exception_phase_fault = v7m_exception_number_with_security(env, ARMV7M_EXCP_MEM, env->secure);
        }
        env->exception_index = exception_index;
    }
    env->regs[13] += 4;
    return val;
}

static void v7m_pop_core_register(CPUState *env, unsigned int reg, bool is_user)
{
    uint32_t value = v7m_pop_exception_frame(env, is_user);
    if(env->v7m.exception_phase_fault == 0) {
        env->regs[reg] = value;
    }
}

static inline int fp_get_reservation_size(CPUState *env)
{
    const int reg_size = sizeof(env->vfp.regs[0]);
    return reg_size * 8 + sizeof(vfp_get_fpscr(env)) + 4 /* Reserved for MVE */
           + (env->secure && ((env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) > 0) ? (reg_size * 8) : 0);
}

static inline void swap_u32(uint32_t *a, uint32_t *b)
{
    uint32_t tmp;
    tmp = *a;
    *a = *b;
    *b = tmp;
}

/* Switch to v7-M main or process stack pointer.  */
void switch_v7m_sp(CPUState *env, bool process)
{
    if(env->v7m.process_sp != process) {
        swap_u32(&env->v7m.other_sp, &env->regs[13]);
        env->v7m.process_sp = process;
    }
}

static void invalidate_vfp_regs(CPUState *env, bool should_clear, bool do_callee)
{
    if(!should_clear) {
        /* Registers are UNKNOWN if should_clear is false, we decide to just not clear them */
        return;
    }

    const int value = 0;

    memset(env->vfp.regs, value, 16 * sizeof(env->vfp.regs[0]));
    if(do_callee) {
        memset(env->vfp.regs + 16, value, 16 * sizeof(env->vfp.regs[0]));
    }
    vfp_set_fpscr(env, value);
    env->v7m.vpr = value;
}

static void switch_v7m_security_state(CPUState *env, bool secure)
{
    if(secure == env->secure) {
        return;
    }

    /* If we entered this function but have no TZ, means we have a bug somewhere */
    if(unlikely(!env->v7m.has_trustzone)) {
        cpu_abort(env, "Tried to internally switch CPU state to %s but TrustZone is not enabled. This is a translation lib bug",
                  secure ? "Secure" : "Non-secure");
    }

    /* If we were in thread mode before the switch, we need to remember to swap them
     * to make sure that MSP is really the Main Stack Pointer
     */
    uint32_t new_ss_msp = env->v7m.other_ss_msp;
    uint32_t new_ss_psp = env->v7m.other_ss_psp;

    if(env->v7m.process_sp) {
        env->v7m.other_ss_psp = env->regs[13];
        env->v7m.other_ss_msp = env->v7m.other_sp;
    } else {
        env->v7m.other_ss_msp = env->regs[13];
        env->v7m.other_ss_psp = env->v7m.other_sp;
    }

    env->secure = secure;

    if(is_using_process_sp_current(env)) {
        env->regs[13] = new_ss_psp;
        env->v7m.other_sp = new_ss_msp;
        env->v7m.process_sp = true;
    } else {
        env->regs[13] = new_ss_msp;
        env->v7m.other_sp = new_ss_psp;
        env->v7m.process_sp = false;
    }
}

static inline bool tz_v8m_should_pop_additional_registers(uint32_t type)
{
    return (type & ARM_EXC_RETURN_S_MASK) && (!(type & ARM_EXC_RETURN_ES_MASK) || !(type & ARM_EXC_RETURN_DCRS_MASK));
}

static void v7m_consume_exception_stack_frame(CPUState *env, uint32_t type)
{
    bool to_secure = env->v7m.has_trustzone && (type & ARM_EXC_RETURN_S_MASK);
    if(env->v7m.has_trustzone) {
        switch_v7m_security_state(env, to_secure);
    }
    switch_v7m_sp(env, is_using_process_sp(env, (type & ARM_EXC_RETURN_MODE_MASK) == 0, env->secure));

    uint32_t frame_size = tz_v8m_should_pop_additional_registers(type) ? 0x48 : 0x20;
    if(~type & ARM_EXC_RETURN_NFPCA_MASK) {
        frame_size += fp_get_reservation_size(env);
    }
    env->regs[13] += frame_size;
    env->v7m.control[M_REG_COMMON] ^= (env->v7m.control[M_REG_COMMON] ^ (~type >> (ARM_EXC_RETURN_NFPCA - ARM_CONTROL_FPCA))) &
                                      ARM_CONTROL_FPCA_MASK;
}

static void v7m_handle_early_exception_return_fault(CPUState *env, uint32_t type, int exception)
{
    if(tlib_nvic_set_pending_synchronous_fault(exception) != V7M_SYNCHRONOUS_FAULT_PENDING) {
        /* HandleExceptionTransitions() consumes the unstacked frame only
         * when an exception-return fault enters Lockup. The alignment flag is
         * UNKNOWN in this case and is deterministically treated as zero. */
        v7m_consume_exception_stack_frame(env, type);
        env->v7m.exception = (type & ARM_EXC_RETURN_MODE_MASK) ? 0 : ARMV7M_EXCP_HARD;
        v7m_enter_lockup(env, true);
        arm_announce_stack_change();
        return;
    }

    /* Rule RVJKL: the derived exception reuses the unconsumed frame. */
    env->v7m.exception_return_tailchain = true;
    env->v7m.exception_return_type = type;
    env->exception_index = EXCP_IRQ;
    cpu_loop_exit(env);
}

void do_v7m_exception_exit(CPUState *env)
{
    uint32_t type;
    uint32_t xpsr = 0;
    uint32_t pre_unstack_regs[16];
    uint64_t pre_unstack_vfp_regs[32];
    uint32_t pre_unstack_xpsr;
    uint32_t pre_unstack_control;
    uint32_t pre_unstack_fpccr[M_REG_NUM_BANKS];
    uint32_t pre_unstack_fpscr;
    uint32_t pre_unstack_vpr;
    bool fp_unstack_started = false;

    type = env->regs[15];
    bool exception_was_secure = env->v7m.has_trustzone && (type & ARM_EXC_RETURN_ES_MASK);
    int validation_fault = 0;
    if(env->v7m.has_trustzone) {
        /* RNCQN: If the PE was in Non-secure state when EXC_RETURN was loaded into the PC
         * and EXC_RETURN.ES is one, an INVER SecureFault is generated [...] */
        /* ValidateExceptionReturn() also rejects DCRS == 0 for a Non-secure
         * exception. Both integrity failures generate INVER, and a bad ES is
         * treated as zero for all subsequent transitions. */
        if((!env->secure && exception_was_secure) || (!exception_was_secure && !(type & ARM_EXC_RETURN_DCRS_MASK))) {
            type &= ~ARM_EXC_RETURN_ES_MASK;
            exception_was_secure = false;
            env->v7m.secure_fault_status |= SECURE_FAULT_INVER;
            validation_fault = ARMV7M_EXCP_SECURE;
        }
    }

    /* Restore FAULTMASK to 0 only if the interrupt that we are exiting permits it. */
    /* Armv8-M ARM rule RCDCR: without the Security Extension, exception return
     * clears FAULTMASK except on return from NMI. With the Security Extension
     * it clears the bank selected by ES only when RawExecutionPriority() is
     * non-negative. At this point a negative raw priority means the current
     * exception is NMI or HardFault (no configurable exception can preempt
     * either), so both fixed-priority returns retain FAULTMASK. */
    bool clears_faultmask = env->v7m.exception != ARMV7M_EXCP_NMI &&
                            (!env->v7m.has_trustzone || env->v7m.exception != ARMV7M_EXCP_HARD);
    if(clears_faultmask) {
        env->v7m.faultmask[exception_was_secure] = 0;
    }

    if(env->v7m.exception != 0) {
        /* This ensures we properly complete banked secure exceptions */
        int completed_exception = v7m_exception_number_with_security(env, env->v7m.exception, exception_was_secure);
        if(!tlib_nvic_complete_irq(completed_exception) && validation_fault == 0) {
            /* ValidateExceptionReturn(): returning from an exception which
             * is not active in the state selected by ES raises INVPC. */
            env->v7m.fault_status[env->secure] |= USAGE_FAULT_INVPC;
            validation_fault = v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure);
        }
    }

    if(env->interrupt_end_callback_enabled) {
        tlib_on_interrupt_end(env->exception_index);
    }

    env->v7m.control[exception_was_secure] =
        FIELD_DP32(env->v7m.control[exception_was_secure], V7M_CONTROL, SPSEL, type >> ARM_EXC_RETURN_SPSEL);

    if(validation_fault != 0) {
        v7m_handle_early_exception_return_fault(env, type, validation_fault);
        return;
    }

    if((env->v7m.control[M_REG_COMMON] & ARM_CONTROL_FPCA_MASK) &&
       (fpccr_read(env, exception_was_secure) & ARM_FPCCR_CLRONRET_MASK)) {
        if(fpccr_read(env, true) & ARM_FPCCR_LSPACT_MASK) {
            /* Secure LSPACT won't be set if TrustZone is disabled */
            /* Rule RJMGQ: lazy state cannot be deactivated by CLRONRET.
             * This SecureFault is generated after exception deactivation and
             * therefore tail-chains using the existing frame (RVJKL). */
            tlib_assert(env->v7m.has_trustzone);
            env->v7m.secure_fault_status |= SECURE_FAULT_LSERR;
            v7m_handle_early_exception_return_fault(env, type, ARMV7M_EXCP_SECURE);
            return;
        } else {
            for(int i = 0; i < 8; ++i) {
                env->vfp.regs[i] = 0;
            }
            vfp_set_fpscr(env, 0);
            env->v7m.vpr = 0;
        }
    }

    if(env->v7m.has_trustzone) {
        /* Location of return stack type (Secure/Non-secure) */
        switch_v7m_security_state(env, type & ARM_EXC_RETURN_S_MASK);
    }

    /* Switch to the target stack */
    switch_v7m_sp(env, is_using_process_sp(env, (type & ARM_EXC_RETURN_MODE_MASK) == 0, env->secure));
    bool unstack_is_user = (type & ARM_EXC_RETURN_MODE_MASK) && FIELD_EX32(env->v7m.control[env->secure], V7M_CONTROL, nPRIV);

    uint32_t frame_pointer = env->regs[13];
    /*
     * PopStack() only commits restored state after all frame reads and
     * integrity checks succeed. Keep a checkpoint so that a fault which can
     * tail-chain leaves the interrupted frame and the returning handler's
     * register state intact. CLRONRET has already run, so any clearing it
     * performed remains architecturally visible.
     */
    memcpy(pre_unstack_regs, env->regs, sizeof(pre_unstack_regs));
    memcpy(pre_unstack_vfp_regs, env->vfp.regs, sizeof(pre_unstack_vfp_regs));
    memcpy(pre_unstack_fpccr, env->v7m.fpccr, sizeof(pre_unstack_fpccr));
    pre_unstack_xpsr = xpsr_read(env);
    pre_unstack_control = env->v7m.control[M_REG_COMMON];
    pre_unstack_fpscr = vfp_get_fpscr(env);
    pre_unstack_vpr = env->v7m.vpr;
    env->v7m.exception_phase = V7M_EXCEPTION_PHASE_UNSTACKING;
    env->v7m.exception_phase_fault = 0;

    /* Pop registers.  */
    if(cpu->v7m.has_trustzone) {
        /* We need to pop additional state registers, if they were pushed before */
        if(tz_v8m_should_pop_additional_registers(type)) {
            uint32_t integrity = INTEGRITY_SIGN;
            integrity |= (type & ARM_EXC_RETURN_NFPCA_MASK) >> ARM_EXC_RETURN_NFPCA;
            uint32_t signature = v7m_pop_exception_frame(env, unstack_is_user);
            /* PopStack() validates the signature only after a successful read. */
            if(env->v7m.exception_phase_fault == 0 && signature != integrity) {
                tlib_printf(LOG_LEVEL_WARNING,
                            "Integrity signature mismatch on stack, expected 0x%" PRIx32 ", got 0x%" PRIx32 ", type 0x%" PRIx32
                            ". SecureFault!",
                            integrity, signature, type);
                /* On security integrity signature mismatch, report SecureFault */
                /* Armv8-M ARM rules RNZCD and RTCJR: a failed integrity check
                 * on exception return is a derived fault that must be handled
                 * *after* the returning exception's active bit and FAULTMASK
                 * have been cleared and after the stack pointer has been
                 * advanced as if unstacking completed. Set
                 * exception_phase_fault so that the remaining
                 * v7m_pop_exception_frame() calls skip loads but still
                 * advance SP, then fall through to the unstacking fault
                 * handler which sets IPSR from EXC_RETURN.Mode and clears
                 * ITSTATE on lockup. */
                env->v7m.secure_fault_status |= SECURE_FAULT_INVIS;
                env->v7m.secure_fault_address = env->regs[15];
                env->v7m.exception_phase_fault = ARMV7M_EXCP_SECURE;
            }
            /* Reserved */
            v7m_pop_exception_frame(env, unstack_is_user);
            v7m_pop_core_register(env, 4, unstack_is_user);
            v7m_pop_core_register(env, 5, unstack_is_user);
            v7m_pop_core_register(env, 6, unstack_is_user);
            v7m_pop_core_register(env, 7, unstack_is_user);
            v7m_pop_core_register(env, 8, unstack_is_user);
            v7m_pop_core_register(env, 9, unstack_is_user);
            v7m_pop_core_register(env, 10, unstack_is_user);
            v7m_pop_core_register(env, 11, unstack_is_user);
        }
    }

    v7m_pop_core_register(env, 0, unstack_is_user);
    v7m_pop_core_register(env, 1, unstack_is_user);
    v7m_pop_core_register(env, 2, unstack_is_user);
    v7m_pop_core_register(env, 3, unstack_is_user);
    v7m_pop_core_register(env, 12, unstack_is_user);
    v7m_pop_core_register(env, 14, unstack_is_user);
    /* Do not commit values read after an unstacking fault. */
    uint32_t return_address = v7m_pop_exception_frame(env, unstack_is_user);
    if(env->v7m.exception_phase_fault == 0) {
        env->regs[15] = return_address & ~1;
    }
    uint32_t stacked_xpsr = v7m_pop_exception_frame(env, unstack_is_user);
    if(env->v7m.exception_phase_fault == 0) {
        xpsr = stacked_xpsr;
    }
    if(env->v7m.exception_phase_fault == 0) {
        uint32_t stacked_exception = xpsr & 0x1ff;
        bool return_to_handler = (type & ARM_EXC_RETURN_MODE_MASK) == 0;
        if(return_to_handler == (stacked_exception == 0)) {
            /* PopStack() RETPSR integrity check: Handler mode requires a
             * nonzero IPSR, while Thread mode requires IPSR == 0. A mismatch
             * raises UsageFault.INVPC (rules RVJKL and RNZCD). */
            env->v7m.fault_status[env->secure] |= USAGE_FAULT_INVPC;
            env->v7m.exception_phase_fault = v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure);
        }
    }
    if(env->v7m.exception_phase_fault == 0) {
        env->v7m.control[M_REG_COMMON] |= (xpsr & RETPSR_SFPA) ? ARM_CONTROL_SFPA_MASK : 0;
        xpsr_write(env, xpsr, 0xfffffdff);
    }
    /* Pop extended frame  */
    if(~type & ARM_EXC_RETURN_NFPCA_MASK) {
        uint32_t fp_frame_size = fp_get_reservation_size(env);

        if(env->v7m.exception_phase_fault != 0) {
            /* ConsumeExcStackFrame() still includes the complete FP frame if
             * an earlier integrity or core-unstacking fault enters Lockup. */
            env->regs[13] += fp_frame_size;
        } else if(!env->secure && !!(env->v7m.fpccr[M_REG_S] & ARM_FPCCR_LSPACT_MASK)) {
            /* Rule RLMSY: The PE generates an LSERR SecureFault on exception return before unstacking the Floating-point context
             * or Additional floating-point context, when the following conditions are met: EXC_RETURN.FType is 0. Secure lazy
             * floating-point state preservation is active, that is, FPCCR_S.LSPACT is 1. The return is to Non-secure state. */
            env->v7m.secure_fault_status |= SECURE_FAULT_LSERR;
            env->v7m.exception_phase_fault = ARMV7M_EXCP_SECURE;
            env->regs[13] += fp_frame_size;
        } else if(env->v7m.fpccr[env->secure] & ARM_FPCCR_LSPACT_MASK) {
            /* FP state is still valid, pop space from stack  */
            env->v7m.fpccr[env->secure] ^= ARM_FPCCR_LSPACT_MASK;
            env->regs[13] += fp_frame_size;
        } else {
            if(~env->vfp.xregs[ARM_VFP_FPEXC] & ARM_VFP_FPEXC_FPUEN_MASK) {
                /* FPU is disabled, raise Usage Fault without consuming the frame  */
                /* RRXJC: a failed permission check before FP unstacking
                 * generates NOCP and leaves the frame available to the
                 * derived tail-chained exception. */
                env->v7m.fault_status[env->secure] |= USAGE_FAULT_NOCP;
                env->v7m.exception_phase_fault = v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure);
                env->regs[13] += fp_frame_size;
            } else {
                fp_unstack_started = true;
                for(int i = 0; i < 8; ++i) {
                    env->vfp.regs[i] = v7m_pop_exception_frame(env, unstack_is_user);
                    env->vfp.regs[i] |= ((uint64_t)v7m_pop_exception_frame(env, unstack_is_user)) << 32;
                }
                vfp_set_fpscr(env, v7m_pop_exception_frame(env, unstack_is_user));
                env->v7m.vpr = v7m_pop_exception_frame(env, unstack_is_user);

                if(arm_feature(env, ARM_FEATURE_V8)) {
                    /* At this point, the internal state is Secure, so it's OK to just use env->secure here,
                     * instead of `type.S` bit*/
                    if(env->secure && (env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) > 0) {
                        for(int i = 8; i < 16; ++i) {
                            env->vfp.regs[i] = v7m_pop_exception_frame(env, unstack_is_user);
                            env->vfp.regs[i] |= ((uint64_t)v7m_pop_exception_frame(env, unstack_is_user)) << 32;
                        }
                    }
                }
            }
        }
    }
    /* Set CONTROL.FPCA to NOT(type[ARM_EXC_RETURN_NFPCA])  */
    env->v7m.control[M_REG_COMMON] ^= (env->v7m.control[M_REG_COMMON] ^ ~type >> (ARM_EXC_RETURN_NFPCA - ARM_CONTROL_FPCA)) &
                                      ARM_CONTROL_FPCA_MASK;
    /* Undo stack alignment.  */
    if(xpsr & 0x200) {
        env->regs[13] |= 4;
    }

    int unstacking_fault = env->v7m.exception_phase_fault;
    env->v7m.exception_phase = V7M_EXCEPTION_PHASE_NONE;
    env->v7m.exception_phase_fault = 0;
    if(unstacking_fault != 0) {
        /* Armv8-M ARM rules RNZCD and RTCJR: the returning exception and
         * FAULTMASK have already been cleared, and the stack pointer has been
         * advanced as if unstacking completed. If the derived exception still
         * cannot preempt, Lockup records the destination mode in IPSR. */
        if(tlib_nvic_set_pending_synchronous_fault(unstacking_fault) != V7M_SYNCHRONOUS_FAULT_PENDING) {
            env->v7m.exception = (type & ARM_EXC_RETURN_MODE_MASK) ? 0 : ARMV7M_EXCP_HARD;
            v7m_enter_lockup(env, true);
        } else {
            /* PopStack() does not consume a frame when a non-Lockup
             * exception-return fault is pended. HandleExceptionTransitions()
             * tail-chains into the derived exception using that frame (rule
             * RVJKL and pseudocode operation HandleExceptionTransitions). */
            memcpy(env->regs, pre_unstack_regs, sizeof(pre_unstack_regs));
            memcpy(env->vfp.regs, pre_unstack_vfp_regs, sizeof(pre_unstack_vfp_regs));
            memcpy(env->v7m.fpccr, pre_unstack_fpccr, sizeof(pre_unstack_fpccr));
            xpsr_write(env, pre_unstack_xpsr, 0xffffffff);
            env->v7m.control[M_REG_COMMON] = pre_unstack_control;
            vfp_set_fpscr(env, pre_unstack_fpscr);
            env->v7m.vpr = pre_unstack_vpr;
            if(fp_unstack_started) {
                /* Rules RHNNW and RLMNG: when FP unstacking is abandoned for
                 * a tail-chain, the would-be-restored FP state is zero with
                 * the Security Extension and UNKNOWN otherwise. We choose zero
                 * as UNKNOWN, too, for simplicity. */
                int fp_reg_count = env->secure && (pre_unstack_fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) ? 16 : 8;
                memset(env->vfp.regs, 0, fp_reg_count * sizeof(env->vfp.regs[0]));
                vfp_set_fpscr(env, 0);
                env->v7m.vpr = 0;
            }
            env->regs[13] = frame_pointer;
            env->v7m.exception_return_tailchain = true;
            env->v7m.exception_return_type = type;
            env->exception_index = EXCP_IRQ;
            cpu_loop_exit(env);
        }
    }

    env->event_register = 1;
}

void do_v7m_secure_return(CPUState *env)
{
    uint32_t partialRETPSR;

    switch_v7m_security_state(env, true);
    /* Only Thumb mode is supported for this architecture */
    env->thumb = true;
    env->regs[15] = v7m_pop(env) & ~1;

    partialRETPSR = v7m_pop(env);
    env->v7m.control[M_REG_COMMON] |=
        deposit32(env->v7m.control[M_REG_COMMON], ARM_CONTROL_SFPA, 1, partialRETPSR & RETPSR_SFPA ? 1 : 0);
    env->v7m.exception = partialRETPSR & ~RETPSR_SFPA;

    env->event_register = 1;

    tlib_printf(LOG_LEVEL_NOISY, "Secure return to 0x%08" PRIx32 ", xpsr: 0x%08" PRIx32, env->regs[15], xpsr_read(env));
}

bool need_fp_lazy_state_preservation(CPUState *env)
{
    bool is_secure = !!(env->v7m.fpccr[M_REG_S] & ARM_FPCCR_S_MASK);
    return env->v7m.fpccr[is_secure] & ARM_FPCCR_LSPACT_MASK;
}

bool need_fp_context(CPUState *env)
{
    bool need_new_context = false;
    if((env->v7m.fpccr[env->secure] & ARM_FPCCR_ASPEN_MASK) > 0) {
        bool fp_context_active = env->v7m.control[M_REG_COMMON] & ARM_CONTROL_FPCA_MASK;
        bool fp_regs_contain_secure = env->v7m.control[M_REG_COMMON] & ARM_CONTROL_SFPA_MASK;
        need_new_context = (!fp_context_active || (env->secure && !fp_regs_contain_secure));
    }
    return need_new_context;
}

uint32_t ccr_read(CPUState *env, bool is_secure)
{
    uint32_t common = env->v7m.ccr[M_REG_COMMON];
    uint32_t banked = env->v7m.ccr[is_secure];
    uint32_t value = (common & V7M_CCR_NOT_BANKED_FIELDS) | (banked & ~V7M_CCR_NOT_BANKED_FIELDS);

    if(!is_secure) {
        value &= ~FIELD_MASK(V7M_CCR, TRD);
    }

    return value;
}

void ccr_write(CPUState *env, uint32_t value, bool is_secure)
{
    if(arm_feature(env, ARM_FEATURE_V8_1M) && (FIELD_MASK(V7M_CCR, LOB) & value)) {
        tlib_printf(LOG_LEVEL_WARNING, "Trying to enable branch info cache, but it's not supported");
        value &= ~FIELD_MASK(V7M_CCR, LOB);
    }
    if(FIELD_MASK(V7M_CCR, BP) & value) {
        tlib_printf(LOG_LEVEL_WARNING, "Trying to enable branch predication, but it's not supported");
        value &= ~FIELD_MASK(V7M_CCR, BP);
    }
    if(FIELD_MASK(V7M_CCR, IC) & value) {
        tlib_printf(LOG_LEVEL_WARNING, "Trying to enable instruction cache, but it's not supported");
        value &= ~FIELD_MASK(V7M_CCR, IC);
    }
    if(!is_secure) {
        value &= ~FIELD_MASK(V7M_CCR, TRD);
    }

    env->v7m.ccr[M_REG_COMMON] = value & V7M_CCR_NOT_BANKED_FIELDS;
    env->v7m.ccr[is_secure] = value & ~V7M_CCR_NOT_BANKED_FIELDS;
}

static inline bool lsp_store_helper(CPUState *env, uint32_t *address, uint32_t val, bool is_secure)
{
    bool is_user = !!(env->v7m.fpccr[is_secure] & FIELD_MASK(V7M_FPCCR, USER));
    uint32_t secure_fault_status = env->v7m.secure_fault_status;
    int exception_index = env->exception_index;

    /* No address translation in ARM-M, so discard phys_ptr */
    uint32_t phys_ptr = 0;
    target_ulong page_size = 0;
    int prot = 0;

    int ret = get_phys_addr(env, *address, is_secure, ACCESS_DATA_STORE, is_user, &phys_ptr, &prot, &page_size, false);
    if(ret == TRANSLATE_SUCCESS) {
        stl_phys(*address, val);
        if(env->v7m.exception_phase_fault != 0) {
            return false;
        }
        *address += sizeof(val);
        return true;
    }

    if(env->exception_index == EXCP_SECURE) {
        /* A security attribution failure during an AccType_LAZYFP access is
         * reported as SFSR.LSPERR, rather than as the ordinary AUVIOL/INV*
         * syndrome produced by get_phys_addr() (PreserveFPState, ValidateAddress). */
        env->v7m.secure_fault_status = secure_fault_status | SECURE_FAULT_LSPERR | SECURE_FAULT_SFARVALID;
        env->v7m.secure_fault_address = *address;
        env->v7m.exception_phase_fault = ARMV7M_EXCP_SECURE;
    } else {
        /* An MPU failure during lazy preservation reports MMFSR.MLSPERR.
         * MMFARVALID is not set for this access type (ValidateAddress, CheckPermission). */
        env->v7m.fault_status[is_secure] |= MEM_FAULT_MLSPERR;
        env->v7m.exception_phase_fault = v7m_exception_number_with_security(env, ARMV7M_EXCP_MEM, is_secure);
    }
    env->exception_index = exception_index;
    return false;
}

/* FPU Lazy State Preservation logic */
void fp_lsp_save_to_stack(CPUState *env)
{
    const int regSize = sizeof(env->vfp.regs[0]);
    tlib_assert(regSize == 8);

    bool is_secure = !!(env->v7m.fpccr[M_REG_S] & ARM_FPCCR_S_MASK);
    /* Save FP state if FPCCR.LSPACT is set  */
    if(unlikely(need_fp_lazy_state_preservation(env))) {
        /* Bits[0:2] are RES0 (range inclusive) */
        uint32_t address = env->v7m.fpcar[is_secure] & ARM_FPCAR_ADDRESS_MASK;
        /* Remember, we operate with double-precision aliases here
         * so for D7, up to S14, S15 are preserved, and so on
         */
        bool failed = false;
        env->v7m.exception_phase = V7M_EXCEPTION_PHASE_LAZY_FP;
        env->v7m.exception_phase_fault = 0;
        for(int i = 0; i < 8; ++i) {
            if(!lsp_store_helper(env, &address, env->vfp.regs[i], is_secure) ||
               !lsp_store_helper(env, &address, env->vfp.regs[i] >> 32, is_secure)) {
                failed = true;
                break;
            }
        }
        if(!failed) {
            uint32_t fpscr = vfp_get_fpscr(env);
            failed = !lsp_store_helper(env, &address, fpscr, is_secure);
        }

        if(!failed && arm_feature(env, ARM_FEATURE_V8)) {
            if(arm_feature(env, ARM_FEATURE_MVE)) {
                failed = !lsp_store_helper(env, &address, env->v7m.vpr, is_secure);
            } else {
                /* Write UNKNOWN if MVE is not implemented */
                failed = !lsp_store_helper(env, &address, 0xBADCAFEE, is_secure);
            }
            if(!failed && is_secure && (env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) > 0) {
                for(int i = 8; i < 16; ++i) {
                    if(!lsp_store_helper(env, &address, env->vfp.regs[i], is_secure) ||
                       !lsp_store_helper(env, &address, env->vfp.regs[i] >> 32, is_secure)) {
                        failed = true;
                        break;
                    }
                }
            }
        }

        int lazy_fp_fault = env->v7m.exception_phase_fault;
        env->v7m.exception_phase = V7M_EXCEPTION_PHASE_NONE;
        env->v7m.exception_phase_fault = 0;

        if(failed) {
            /* TakePreserveFPException() uses the readiness snapshot saved in
             * FPCCR, rather than re-running ordinary synchronous escalation
             * against the context which triggered preservation. */
            uint32_t saved_fpccr = fpccr_read(env, is_secure) | (env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_COMMON_READY_MASK);
            int result = tlib_nvic_set_pending_lazy_fp_fault(lazy_fp_fault, saved_fpccr);
            if(result == 2) {
                /* Rule RRNKB and TakePreserveFPException(): HFRDY == 0
                 * makes a fault which cannot preempt enter Lockup, without
                 * setting HFSR.FORCED or changing pending/active state. */
                v7m_enter_lockup(env, false);
                cpu_loop_exit(env);
            }
            if(result == 1) {
                /* The derived exception can preempt immediately. Preserve
                 * LSPACT and the FP register contents, then terminate the
                 * instruction as required by PreserveFPState(). */
                env->exception_index = EXCP_IRQ;
                cpu_loop_exit(env);
            }
        }

        bool treat_secure = env->v7m.fpccr[M_REG_COMMON] & FIELD_MASK(V7M_FPCCR, TS);
        invalidate_vfp_regs(env, is_secure && treat_secure, is_secure && treat_secure);

        /* Rule ITWPT: Arm recommends that when performing lazy Floating-point state preservation both the Secure and Non-secure
         * FPCCR.LSPACT flags should be cleared. */
        env->v7m.fpccr[M_REG_S] &= ~ARM_FPCCR_LSPACT_MASK;
        env->v7m.fpccr[M_REG_NS] &= ~ARM_FPCCR_LSPACT_MASK;
    }
}

void fp_lsp_create_context(CPUState *env)
{
    bool is_secure = env->secure;

    /* Update the "S" flag*/
    env->v7m.fpccr[M_REG_COMMON] = FIELD_DP32(env->v7m.fpccr[M_REG_COMMON], V7M_FPCCR, S, is_secure);

    if(unlikely(need_fp_context(env))) {
        env->v7m.control[M_REG_COMMON] |= ARM_CONTROL_FPCA_MASK;
        if(is_secure) {
            env->v7m.control[M_REG_COMMON] |= ARM_CONTROL_SFPA_MASK;
        }
        vfp_set_fpscr(env, env->v7m.fpdscr[is_secure]);
        env->v7m.vpr = 0x0;
    }
}

void HELPER(fp_lsp_no_context)(CPUState *env)
{
    fp_lsp_save_to_stack(env);
}

void HELPER(fp_lsp)(CPUState *env)
{
    fp_lsp_save_to_stack(env);
    fp_lsp_create_context(env);
}

static inline void fpccr_update(CPUState *env, uint32_t frameptr, int original_exception)
{
    bool is_secure = env->secure;
    /* Set address of the FP frame on the stack */
    env->v7m.fpcar[is_secure] = frameptr & ARM_FPCAR_ADDRESS_MASK;

    /* Mark that Lazy Floating Point preservation will happen */
    env->v7m.fpccr[is_secure] |= FIELD_MASK(V7M_FPCCR, LSPACT);

    /* TODO: Check if SP violates the current stack limit and set SPLIMVIOL accordingly */

    /* Update fields to information about the current CPU state */
    env->v7m.fpccr[M_REG_COMMON] = FIELD_DP32(env->v7m.fpccr[M_REG_COMMON], V7M_FPCCR, S, is_secure);
    env->v7m.fpccr[is_secure] = FIELD_DP32(env->v7m.fpccr[is_secure], V7M_FPCCR, USER, in_user_mode(env));
    env->v7m.fpccr[is_secure] = FIELD_DP32(env->v7m.fpccr[is_secure], V7M_FPCCR, THREAD, !in_handler_mode(env));

    /* UpdateFPCCR() snapshots whether every fault class could have preempted
     * the context owning this FP state. The NVIC model acknowledges an exception
     * before stacking, so the callback evaluates the priority with the
     * acknowledged original exception temporarily hidden. */
    uint32_t ready_ns = tlib_nvic_get_fpccr_ready_bits(original_exception, M_REG_NS);
    uint32_t ready_s = tlib_nvic_get_fpccr_ready_bits(original_exception, M_REG_S);
    uint32_t common_ready_mask = FIELD_MASK(V7M_FPCCR, HFRDY) | FIELD_MASK(V7M_FPCCR, BFRDY) | FIELD_MASK(V7M_FPCCR, SFRDY) |
                                 FIELD_MASK(V7M_FPCCR, MONRDY);
    uint32_t banked_ready_mask = FIELD_MASK(V7M_FPCCR, MMRDY) | FIELD_MASK(V7M_FPCCR, UFRDY);

    env->v7m.fpccr[M_REG_COMMON] = (env->v7m.fpccr[M_REG_COMMON] & ~common_ready_mask) | (ready_s & common_ready_mask);
    env->v7m.fpccr[M_REG_NS] = (env->v7m.fpccr[M_REG_NS] & ~banked_ready_mask) | (ready_ns & banked_ready_mask);
    env->v7m.fpccr[M_REG_S] = (env->v7m.fpccr[M_REG_S] & ~banked_ready_mask) | (ready_s & banked_ready_mask);
}

void v7m_set_locked_up(CPUState *env, bool locked_up)
{
    if(env->v7m.locked_up == locked_up) {
        return;
    }

    env->v7m.locked_up = locked_up;
    tlib_on_lockup_state_change(locked_up);
}

static void v7m_enter_lockup(CPUState *env, bool clear_itstate)
{
    if(clear_itstate) {
        env->condexec_bits = 0;
        env->uncached_cpsr &= ~CPSR_IT;
    }

    /* Armv8-M ARM pseudocode operation Lockup and rule RMBTM. */
    env->regs[15] = ARMV7M_LOCKUP_PC;
    v7m_set_locked_up(env, true);
    env->exception_index = EXCP_LOCKUP;
}

void v7m_enter_reset_lockup(CPUState *env)
{
    /* Armv8-M ARM rule RBHVG and pseudocode operation TakeReset: reset-vector
     * BusFault Lockup is in HardFault active state, but IPSR remains zero.
     * We choose zero for the UNKNOWN state. */
    env->regs[13] = 0;
    env->regs[14] = UINT32_MAX;
    env->v7m.exception = 0;
    env->thumb = false;
    v7m_enter_lockup(env, true);
}

static void v7m_raise_synchronous_exception(CPUState *env, int exception)
{
    /* Armv8-M ARM rule RGNVS and pseudocode operations ExceptionDetails and
     * CreateException: NVIC decides whether the exception is taken, escalated,
     * or cannot escalate and instead causes Lockup. */
    if(tlib_nvic_set_pending_synchronous_fault(exception) == V7M_SYNCHRONOUS_FAULT_PENDING) {
        return;
    }

    /* Armv8-M ARM rules RXHMT and RMBTM, corresponding to pseudocode
     * operation Lockup: preserve ITSTATE, set the sentinel PC, and stop. */
    v7m_enter_lockup(env, false);
}

static bool v7m_exception_targets_secure(CPUState *env, uint32_t *exception)
{
    bool secure_target = env->secure;

    if(!env->v7m.has_trustzone) {
        return secure_target;
    }

    /* If we have TrustZone, NVIC_ITNSx determines the security state
     * the hardware IRQ is taken to */
    if(*exception >= ARMV7M_EXCP_HARDIRQ0 && *exception < BANKED_SECURE_EXCP_BIT) {
        secure_target = tlib_nvic_interrupt_targets_secure(*exception);
    } else {
        switch(*exception) {
            case ARMV7M_EXCP_NMI:
            case ARMV7M_EXCP_BUS:
                /* `AIRCR.BFHFNMINS` determines this behavior, but we store its value
                 * within this structure, the same as for hard IRQ */
                secure_target = tlib_nvic_interrupt_targets_secure(*exception);
                break;
            case ARMV7M_EXCP_HARD:
                /* If `AIRCR.BFHFNMINS` is set to 1, HardFault is a regular banked IRQ
                 * so nothing special here - the other HardFault will be handled automatically in the other clause
                 * otherwise, escalate to Secure */
                secure_target = tlib_nvic_interrupt_targets_secure(*exception);
                /* It's negation, since it's a Non-secure target! It's as expected */
                if(!secure_target) {
                    goto banked_exception;
                }
                break;
            /* Reset and Secure Fault are secure only */
            case ARMV7M_EXCP_RESET:
            case ARMV7M_EXCP_SECURE:
                secure_target = true;
                break;
            /* Debug monitor (ARMV7M_EXCP_DEBUG) should be configured with `DEMCR.SDME` but since it's unimplemented
             * we implement it as banked, to minimize side-effects
             * Any other exception is banked too */
            banked_exception:
            default:
                secure_target = (*exception & BANKED_SECURE_EXCP_BIT) > 0;
                /* We need to clear the "SECURE" bit, so everything works correctly */
                *exception &= ~BANKED_SECURE_EXCP_BIT;
                break;
        }
    }

    return secure_target;
}

static void v7m_acknowledge_replacement_exception(CPUState *env, int *active_exception, bool *secure_target)
{
    env->v7m.exception = tlib_nvic_acknowledge_irq();
    *active_exception = env->v7m.exception;
    tlib_assert(env->v7m.exception != 0);
    *secure_target = v7m_exception_targets_secure(env, &env->v7m.exception);
}

static V7MSynchronousFaultResult v7m_resolve_stacking_fault(CPUState *env, int stacking_fault, int acknowledged_exception,
                                                            int *active_exception, bool *secure_target)
{
    V7MSynchronousFaultResult result = tlib_nvic_set_pending_stacking_fault(stacking_fault, acknowledged_exception);
    if(result != V7M_SYNCHRONOUS_FAULT_REPLACED) {
        return result;
    }

    /* DerivedLateArrival() replaces the original exception without
     * allocating another stack frame. */
    v7m_acknowledge_replacement_exception(env, active_exception, secure_target);
    return result;
}

static int v7m_prepare_exception_taken(CPUState *env, uint32_t *lr, bool secure_target, bool do_tailchain)
{
    int stack_status = 0;

    if(env->v7m.has_trustzone) {
        bool previous_exception_was_secure = (*lr & ARM_EXC_RETURN_ES_MASK) != 0;

        /* RSHNX: On taking an exception, excluding tail-chaining that requires a transition from Secure to Non-secure state, the
         * PE hardware saves Additional state context registers. Push additional state context registers when switching from
         * Secure to Non-secure, unless a tail-chain reuses the context already on the stack. */
        if(*lr & ARM_EXC_RETURN_S_MASK) {
            if(secure_target) {
                if(do_tailchain && !previous_exception_was_secure) {
                    /* ExceptionTaken(): a tail-chain from Non-secure to
                     * Secure over a Secure background frame reuses the
                     * Additional state context already on the stack. Clear
                     * DCRS so that a later return consumes it. */
                    *lr &= ~ARM_EXC_RETURN_DCRS_MASK;
                }
            } else {
                bool additional_state_already_present = do_tailchain && !previous_exception_was_secure;
                if((*lr & ARM_EXC_RETURN_DCRS_MASK) && !additional_state_already_present) {
                    tlib_printf(LOG_LEVEL_NOISY, "Pushing additional state context registers on stack");
                    stack_status |= v7m_push(env, env->regs[11]);
                    stack_status |= v7m_push(env, env->regs[10]);
                    stack_status |= v7m_push(env, env->regs[9]);
                    stack_status |= v7m_push(env, env->regs[8]);
                    stack_status |= v7m_push(env, env->regs[7]);
                    stack_status |= v7m_push(env, env->regs[6]);
                    stack_status |= v7m_push(env, env->regs[5]);
                    stack_status |= v7m_push(env, env->regs[4]);
                    /* Marked as reserved in docs */
                    stack_status |= v7m_push(env, 0xDEADBEEF);
                    /* Push integrity signature */
                    uint32_t integrity = INTEGRITY_SIGN;
                    /* Set SFTC bit */
                    integrity |= (*lr & ARM_EXC_RETURN_NFPCA_MASK) >> ARM_EXC_RETURN_NFPCA;
                    stack_status |= v7m_push(env, integrity);

                    /* On transition between security states, let's clear registers (RWBND) */
                    for(int i = 0; i <= 12; ++i) {
                        env->regs[i] = 0;
                    }
                    env->regs[14] = 0;
                }
                /* ExceptionTaken() restores DCRS to its default on every
                 * transition to a Non-secure exception. */
                *lr |= ARM_EXC_RETURN_DCRS_MASK;
            }
        }

        *lr = deposit32(*lr, ARM_EXC_RETURN_ES, 1, secure_target);
    }

    if(arm_feature(env, ARM_FEATURE_V8)) {
        /* We're changing v8-M mode here after we know which security state we're targeting. On v8-M SPSEL is always saved. */
        *lr = deposit32(*lr, ARM_EXC_RETURN_SPSEL, 1, FIELD_EX32(env->v7m.control[secure_target], V7M_CONTROL, SPSEL));
    }
    return stack_status;
}

static void v7m_clear_active_fp_state_on_exception_entry(CPUState *env)
{
    /* ActivateException() clears both unbanked active FP state fields on
     * every exception entry, including tail-chained entries. */
    env->v7m.control[M_REG_COMMON] &= ~(ARM_CONTROL_FPCA_MASK | ARM_CONTROL_SFPA_MASK);
}

static void do_interrupt_v7m(CPUState *env)
{
    uint32_t xpsr = xpsr_read(env);
    uint32_t lr;
    uint32_t addr;
    int nr;
    int stack_status = 0;
    bool secure_target = env->secure;
    int acknowledged_exception = 0;
    int active_exception = 0;
    bool exception_return_tailchain = env->v7m.exception_return_tailchain;
    uint32_t exception_return_type = env->v7m.exception_return_type;
    env->v7m.exception_return_tailchain = false;

    /* For exceptions we just mark as pending on the NVIC, and let that
       handle it. We'll return to this function with exception_index set
       to EXCP_IRQ when exception actually gets processed. */
    switch(env->exception_index) {
        case EXCP_UDEF:
            env->v7m.fault_status[env->secure] |= USAGE_FAULT_UNDEFINSTR;
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure));
            return;
        case EXCP_NOCP:
            env->v7m.fault_status[env->secure] |= USAGE_FAULT_NOCP;
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure));
            return;
        case EXCP_INVSTATE:
            env->v7m.fault_status[env->secure] |= USAGE_FAULT_INVSTATE;
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure));
            return;
        case EXCP_DIV_0:
            env->v7m.fault_status[env->secure] |= USAGE_FAULT_DIVBYZERO;
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_USAGE, env->secure));
            return;
        case EXCP_SWI:
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_SVC, env->secure));
            return;
        case EXCP_PREFETCH_ABORT:
            /* Access violation */
            env->v7m.fault_status[env->secure] |= MEM_FAULT_IACCVIOL;
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_MEM, env->secure));
            return;
        case EXCP_DATA_ABORT:
            /* ACK faulting address and set Data acces violation */
            env->v7m.fault_status[env->secure] |= MEM_FAULT_MMARVALID | MEM_FAULT_DACCVIOL;
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_MEM, env->secure));
            return;
        case EXCP_BKPT:
            nr = lduw_code(env->regs[15]) & 0xff;
            if(nr == 0xab) {
                env->regs[15] += 2;
                env->regs[0] = tlib_do_semihosting();
                return;
            }
            /* Banked DEBUG, but it's not exactly true, see below */
            v7m_raise_synchronous_exception(env, v7m_exception_number_with_security(env, ARMV7M_EXCP_DEBUG, env->secure));
            return;
        case EXCP_SECURE:
            /* Secure Fault address and status bits should be set by respective routines. This only raises the fault to be handled
             * in NVIC */
            tlib_assert(cpu->v7m.has_trustzone);
            v7m_raise_synchronous_exception(env, ARMV7M_EXCP_SECURE);
            return;
        case EXCP_BUS_FAULT:
            v7m_raise_synchronous_exception(env, ARMV7M_EXCP_BUS);
            return;
        case EXCP_IRQ:
            /* Continue the execution after the switch */
            break;
        default:
            cpu_abort(env, "Unhandled exception 0x%x\n", env->exception_index);
            return; /* Never happens.  Keep compiler happy.  */
    }

    if(exception_return_tailchain) {
        /* TailChain() reuses the EXC_RETURN of the frame which the failed
         * return did not consume. */
        lr = exception_return_type;
    } else if(arm_feature(env, ARM_FEATURE_V8)) {
        /* [31:7] PREFIX and RES1.
         *
         * All SecureExtensions bits are set to their disabled state:
         * [6]: 0
         * [5]: 1
         * [0]: 0
         */
        lr = 0xffffffb0;
    } else {
        /* [31:7] PREFIX and RES1.
         *
         * SecureExtension doesn't exist on v7-M but extension's bits are still set to 1. It's the difference between v7-M and
         * v8-M [6]: 1 [5]: 1 [0]: 1
         */
        lr = 0xfffffff1;
        /* We have to set SPSEL for v7-M before we'll change the mode. We're setting it only when leaving Thread mode. */
        if(!in_handler_mode(env)) {
            lr |= FIELD_EX32(env->v7m.control[M_REG_NS], V7M_CONTROL, SPSEL) << ARM_EXC_RETURN_SPSEL;
        }
    }

    /* Set Mode field to the current execution mode
     * 0 - handler mode
     * 1 - thread mode
     */
    if(!exception_return_tailchain && !in_handler_mode(env)) {
        lr |= ARM_EXC_RETURN_MODE_MASK;
    }

    /* v7-M and v8-M share FP stack FP context active fields */
    if(!exception_return_tailchain && (env->v7m.control[M_REG_COMMON] & ARM_CONTROL_FPCA_MASK)) {
        lr ^= ARM_EXC_RETURN_NFPCA_MASK;
    }

    env->v7m.exception = tlib_nvic_acknowledge_irq();
    acknowledged_exception = env->v7m.exception;
    active_exception = acknowledged_exception;
    if(env->v7m.exception == 0) {
        //  We were notified of an IRQ but there isn't one anymore - this can happen if the interrupt was triggered right
        //  before an instruction (eg. a write to BASEPRI) that makes us ignore the interrupt
        tlib_printf(LOG_LEVEL_DEBUG, "Spurious NVIC IRQ ignored");
        return;
    }

    if(env->v7m.has_trustzone) {
        /* There are two most relevant bits here
         * [0] ES (Exception Secure) - "The security domain the exception was taken to"
         * so whether we will be in secure mode after taking this exception
         * [6] S (Secure or Non-secure stack) - "Indicates whether a Secure or Non-secure stack is used to restore stack frame
         * on exception return" so if we will return to secure or non-secure mode, when executing exception return later
         *
         * This will be changed later depending on:
         * - value of NVIC_ITNSx registers for hardware IRQ
         * - value of AIRCR.BFHFNMINS for HardFault, NMI, BusFault
         * - for banked IRQs, the security state the PE was in when the exception was taken. We cheat a little, and use
         * `BANKED_SECURE_EXCP` to reserve extra exception. Look at "EXCP_IRQ" for how this logic works
         */
        if(!exception_return_tailchain) {
            lr |= env->secure << ARM_EXC_RETURN_S;
        }

        secure_target = v7m_exception_targets_secure(env, &env->v7m.exception);
    }

    env->condexec_bits = 0;
    env->v7m.exception_phase = V7M_EXCEPTION_PHASE_STACKING;
    env->v7m.exception_phase_fault = 0;

    if(!exception_return_tailchain) {
        /* Align stack pointer.  */
        /* ??? Should do this if Configuration Control Register
           STACKALIGN bit is set or extended frame is being pushed.  */
        if(env->regs[13] & 4) {
            env->regs[13] -= 4;
            xpsr |= 0x200;
        }
        xpsr |= env->v7m.control[M_REG_COMMON] & ARM_CONTROL_SFPA_MASK ? RETPSR_SFPA : 0;

        /* Push extended frame  */
        if(env->v7m.control[M_REG_COMMON] & ARM_CONTROL_FPCA_MASK) {
            env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_FPCA_MASK;
            env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_SFPA_MASK;
            if(fpccr_read(env, env->secure) & ARM_FPCCR_LSPEN_MASK) {
                /* Set lazy FP state preservation  */
                env->regs[13] -= fp_get_reservation_size(env);
                fpccr_update(env, env->regs[13], acknowledged_exception);
            } else {
                if(~env->vfp.xregs[ARM_VFP_FPEXC] & ARM_VFP_FPEXC_FPUEN_MASK) {
                    /* FPU is disabled, revert SP and raise Usage Fault  */
                    if(xpsr & 0x200) {
                        env->regs[13] |= 4;
                    }
                    env->v7m.exception_phase = V7M_EXCEPTION_PHASE_NONE;
                    env->v7m.exception_phase_fault = 0;
                    env->exception_index = EXCP_UDEF;
                    cpu_loop_exit(env);
                }

                bool push_callee_frame = (env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) > 0;
                if(arm_feature(env, ARM_FEATURE_V8)) {
                    if(push_callee_frame) {
                        for(int i = 15; i >= 8; --i) {
                            v7m_push(env, env->vfp.regs[i] >> 32);
                            v7m_push(env, env->vfp.regs[i]);
                        }
                    }
                }

                if(arm_feature(env, ARM_FEATURE_MVE)) {
                    v7m_push(env, env->v7m.vpr);
                } else {
                    /* Write UNKNOWN if MVE is not implemented */
                    v7m_push(env, 0xBADCAFEE);
                }

                uint32_t fpscr = vfp_get_fpscr(env);
                v7m_push(env, fpscr);

                for(int i = 7; i >= 0; i--) {
                    /* We need to swap low and high register parts, to pop them correctly on state restore.
                     * The state can be restored on excp exit, or by specific load instructions */
                    v7m_push(env, env->vfp.regs[i] >> 32);
                    v7m_push(env, env->vfp.regs[i]);
                }

                invalidate_vfp_regs(env, push_callee_frame, push_callee_frame);
            }
        }
        /* Switch to the handler mode.  */
        stack_status |= v7m_push(env, xpsr);
        stack_status |= v7m_push(env, env->regs[15]);
        stack_status |= v7m_push(env, env->regs[14]);
        stack_status |= v7m_push(env, env->regs[12]);
        stack_status |= v7m_push(env, env->regs[3]);
        stack_status |= v7m_push(env, env->regs[2]);
        stack_status |= v7m_push(env, env->regs[1]);
        stack_status |= v7m_push(env, env->regs[0]);
    }

    int stacking_fault = env->v7m.exception_phase_fault;
    tlib_assert(!stack_status || stacking_fault != 0);
    env->v7m.exception_phase_fault = 0;
    bool ignore_followup_stacking_faults = false;
    bool lockup_after_exception_taken = false;
    bool exception_taken_tailchain = exception_return_tailchain;

    if(stacking_fault != 0) {
        /* PushStack() precedes ExceptionTaken(). Resolve its derived fault
         * first, so Additional state is prepared for the exception that
         * DerivedLateArrival() actually selects, not the original
         * exception. Any later nonterminal stacking fault is ignored by the
         * IgnoreFaults_STACK/ALL mode used for forward progress. */
        V7MSynchronousFaultResult stacking_fault_result =
            v7m_resolve_stacking_fault(env, stacking_fault, acknowledged_exception, &active_exception, &secure_target);
        ignore_followup_stacking_faults = true;
        exception_taken_tailchain = false;
        if(stacking_fault_result == V7M_SYNCHRONOUS_FAULT_LOCKUP) {
            /* Rules RVKTX and RGJJG: enter the original exception with
             * faults ignored before entering Lockup at its priority. */
            lockup_after_exception_taken = true;
        }
    }

    int exception_taken_stack_status = v7m_prepare_exception_taken(env, &lr, secure_target, exception_taken_tailchain);
    int exception_taken_stacking_fault = env->v7m.exception_phase_fault;
    tlib_assert(!exception_taken_stack_status || exception_taken_stacking_fault != 0);
    env->v7m.exception_phase_fault = 0;

    if(exception_taken_stacking_fault != 0 && !ignore_followup_stacking_faults) {
        /* A PushCalleeStack() failure occurs within ExceptionTaken(), so
         * DerivedLateArrival() re-enters ExceptionTaken() as a tail-chain
         * using the LR value already finalized for the original exception. */
        V7MSynchronousFaultResult stacking_fault_result = v7m_resolve_stacking_fault(
            env, exception_taken_stacking_fault, acknowledged_exception, &active_exception, &secure_target);
        if(stacking_fault_result == V7M_SYNCHRONOUS_FAULT_LOCKUP) {
            lockup_after_exception_taken = true;
        }

        v7m_prepare_exception_taken(env, &lr, secure_target, true);
        env->v7m.exception_phase_fault = 0;
    }

    env->v7m.exception_phase = V7M_EXCEPTION_PHASE_NONE;
    if(env->v7m.has_trustzone) {
        tlib_printf(LOG_LEVEL_NOISY, "Loading to LR, while entering exception with TrustZone, value 0x%" PRIx32, lr);
        switch_v7m_security_state(env, secure_target);
    }
    switch_v7m_sp(env, false);
    v7m_clear_active_fp_state_on_exception_entry(env);

    env->uncached_cpsr &= ~CPSR_IT;

    refresh_pending_irq();

    env->regs[14] = lr;

    while(true) {
        env->v7m.exception_phase = V7M_EXCEPTION_PHASE_VECTOR_READ;
        /* TODO: Vector[] uses MemA_with_priv_security(AccType_VECTABLE), so
         * this access should eventually pass through SAU/IDAU validation as
         * well. */
        addr = ldl_phys(env->v7m.vecbase[env->secure] + env->v7m.exception * 4);
        int vector_fault = env->v7m.exception_phase_fault;
        env->v7m.exception_phase = V7M_EXCEPTION_PHASE_NONE;
        env->v7m.exception_phase_fault = 0;

        if(vector_fault == 0) {
            env->regs[15] = addr & 0xfffffffe;
            env->thumb = addr & 1;
            if(lockup_after_exception_taken) {
                v7m_enter_lockup(env, true);
            }
            break;
        }

        /* Rule RCTKP and DerivedLateArrival(): a vector bus error is a
         * terminal VECTTBL HardFault. Rule RDXYK targets it to the Security
         * state of the exception whose vector was read. */
        bool vector_fault_targets_secure = secure_target;
        V7MSynchronousFaultResult vector_fault_result =
            tlib_nvic_set_pending_vector_fault(vector_fault_targets_secure, active_exception, lockup_after_exception_taken);
        if(lockup_after_exception_taken) {
            /* DerivedLateArrival() calls ExceptionTaken(IgnoreFaults_ALL)
             * before Lockup, so the ignored vector error still records
             * HFSR.VECTTBL without creating another derived exception. */
            tlib_assert(vector_fault_result == V7M_SYNCHRONOUS_FAULT_IGNORED);
            v7m_enter_lockup(env, true);
            break;
        }
        if(vector_fault_result == V7M_SYNCHRONOUS_FAULT_LOCKUP) {
            v7m_enter_lockup(env, true);
            break;
        }

        tlib_assert(vector_fault_result == V7M_SYNCHRONOUS_FAULT_REPLACED);
        v7m_acknowledge_replacement_exception(env, &active_exception, &secure_target);
        if(env->v7m.has_trustzone) {
            v7m_prepare_exception_taken(env, &lr, secure_target, true);
            env->v7m.exception_phase_fault = 0;
            switch_v7m_security_state(env, secure_target);
        }
        switch_v7m_sp(env, false);
        v7m_clear_active_fp_state_on_exception_entry(env);
        env->regs[14] = lr;
    }

    arm_announce_stack_change();
}
#endif

#ifndef TARGET_PROTO_ARM_M
/* Handle a CPU exception for non-M architectures.  */
static void do_interrupt_normal(CPUState *env)
{
    uint32_t addr;
    uint32_t mask;
    int new_mode;
    uint32_t offset;

    /* TODO: Vectored interrupt controller.  */
    switch(env->exception_index) {
        case EXCP_UDEF:
            new_mode = ARM_CPU_MODE_UND;
            addr = 0x04;
            mask = CPSR_I;
            if(env->thumb) {
                offset = 2;
            } else {
                offset = 4;
            }
            break;
        case EXCP_SWI:
            /* Check for semihosting interrupt.  */
            if(env->thumb) {
                mask = lduw_code(env->regs[15] - 2) & 0xff;
            } else {
                mask = ldl_code(env->regs[15] - 4) & 0xffffff;
            }
            /* Only intercept calls from privileged modes, to provide some
               semblance of security.  */
            if(((mask == 0x123456 && !env->thumb) || (mask == 0xab && env->thumb)) &&
               (env->uncached_cpsr & CPSR_M) != ARM_CPU_MODE_USR) {
                env->regs[0] = tlib_do_semihosting();
                return;
            }
            new_mode = ARM_CPU_MODE_SVC;
            addr = 0x08;
            mask = CPSR_I;
            /* The PC already points to the next instruction.  */
            offset = 0;
            break;
        case EXCP_BKPT:
            /* See if this is a semihosting syscall.  */
            mask = lduw_code(env->regs[15]) & 0xff;
            if(mask == 0xab && (env->uncached_cpsr & CPSR_M) != ARM_CPU_MODE_USR) {
                env->regs[15] += 2;
                env->regs[0] = tlib_do_semihosting();
                return;
            }
            env->cp15.c5_insn = 2;
            /* Go to prefetch abort.  */
            goto case_EXCP_PREFETCH_ABORT;
        case EXCP_PREFETCH_ABORT:
        case_EXCP_PREFETCH_ABORT:
            new_mode = ARM_CPU_MODE_ABT;
            addr = 0x0c;
            mask = CPSR_A | CPSR_I;
            offset = 4;
            break;
        case EXCP_DATA_ABORT:
            new_mode = ARM_CPU_MODE_ABT;
            addr = 0x10;
            mask = CPSR_A | CPSR_I;
            offset = 8;
            break;
        case EXCP_IRQ:
            new_mode = ARM_CPU_MODE_IRQ;
            addr = 0x18;
            /* Disable IRQ and imprecise data aborts.  */
            mask = CPSR_A | CPSR_I;
            offset = 4;
            break;
        case EXCP_FIQ:
            new_mode = ARM_CPU_MODE_FIQ;
            addr = 0x1c;
            /* Disable FIQ, IRQ and imprecise data aborts.  */
            mask = CPSR_A | CPSR_I | CPSR_F;
            offset = 4;
            break;
        default:
            cpu_abort(env, "Unhandled exception 0x%x\n", env->exception_index);
            return; /* Never happens.  Keep compiler happy.  */
    }
    /* High vectors.  */
    if(env->cp15.c1_sys & (1 << 13)) {
        /* High vectors are not affected by VBAR */
        addr += 0xffff0000;
    } else {
        /* CPUs w/ Security Extensions allow for relocation of the vector table.
         * c12_vbar is initialized to zero so the following maintains compatible
         * with targets that don't have Security Extensions.
         *
         * Even though VBAR can only be set by software for such CPUs, accessors
         * are exported for all pre-v8 A-profile and R-profile CPUs. Therefore
         * it can be set for all such CPUs.
         */
        addr += env->cp15.c12_vbar;
    }
    switch_mode(env, new_mode);
    env->spsr = cpsr_read(env);
    /* Clear IT bits.  */
    env->condexec_bits = 0;
    /* Switch to the new mode, and to the correct instruction set.  */
    env->uncached_cpsr = (env->uncached_cpsr & ~CPSR_M) | new_mode;
    env->uncached_cpsr |= mask;

    /* this is a lie, as the was no c1_sys on V4T/V5, but who cares
     * and we should just guard the thumb mode on V4 */
    if(arm_feature(env, ARM_FEATURE_V4T)) {
        env->thumb = (env->cp15.c1_sys & (1 << 30)) != 0;
    }
    env->regs[14] = env->regs[15] + offset;
    env->regs[15] = addr;
    set_interrupt_pending(env, CPU_INTERRUPT_EXITTB);

    arm_announce_stack_change();
}
#endif

/* Handle a CPU exception.  */
void do_interrupt(CPUState *env)
{
    if(env->interrupt_begin_callback_enabled) {
        tlib_on_interrupt_begin(env->exception_index);
    }

#ifdef TARGET_PROTO_ARM_M
    do_interrupt_v7m(env);
#else
    do_interrupt_normal(env);
#endif
}

/* Check section/page access permissions.
   Returns the page protection flags, or zero if the access is not
   permitted.  */
static inline int check_ap(CPUState *env, int ap, int domain, int access_type, int is_user)
{
    int prot_ro;

    if(domain == 3) {
        return PAGE_READ | PAGE_WRITE;
    }

    if(access_type == ACCESS_DATA_STORE) {
        prot_ro = 0;
    } else {
        prot_ro = PAGE_READ;
    }

    switch(ap) {
        case 0:
            if(access_type == ACCESS_DATA_STORE) {
                return 0;
            }
            switch((env->cp15.c1_sys >> 8) & 3) {
                case 1:
                    return is_user ? 0 : PAGE_READ;
                case 2:
                    return PAGE_READ;
                default:
                    return 0;
            }
        case 1:
            return is_user ? 0 : PAGE_READ | PAGE_WRITE;
        case 2:
            if(is_user) {
                return prot_ro;
            } else {
                return PAGE_READ | PAGE_WRITE;
            }
        case 3:
            return PAGE_READ | PAGE_WRITE;
        case 4: /* Reserved.  */
            return 0;
        case 5:
            return is_user ? 0 : prot_ro;
        case 6:
            return prot_ro;
        case 7:
            if(!arm_feature(env, ARM_FEATURE_V6K)) {
                return 0;
            }
            return prot_ro;
        default:
            abort();
    }
}

static uint32_t get_level1_table_address(CPUState *env, uint32_t address)
{
    uint32_t table;

    if(address & env->cp15.c2_mask) {
        table = env->cp15.c2_base1 & 0xffffc000;
    } else {
        table = env->cp15.c2_base0 & env->cp15.c2_base_mask;
    }

    table |= (address >> 18) & 0x3ffc;
    return table;
}

static int get_phys_addr_v5(CPUState *env, uint32_t address, int access_type, int is_user, uint32_t *phys_ptr, int *prot,
                            target_ulong *page_size)
{
    int code;
    uint32_t table;
    uint32_t desc;
    int type;
    int ap;
    int domain;
    uint32_t phys_addr;

    /* Pagetable walk.  */
    /* Lookup l1 descriptor.  */
    table = get_level1_table_address(env, address);
    desc = ldl_phys(table);
    type = (desc & 3);
    domain = (env->cp15.c3 >> ((desc >> 4) & 0x1e)) & 3;
    if(type == 0) {
        /* Section translation fault.  */
        code = 5;
        goto do_fault;
    }
    if(domain == 0 || domain == 2) {
        if(type == 2) {
            code = 9; /* Section domain fault.  */
        } else {
            code = 11; /* Page domain fault.  */
        }
        goto do_fault;
    }
    if(type == 2) {
        /* 1Mb section.  */
        phys_addr = (desc & 0xfff00000) | (address & 0x000fffff);
        ap = (desc >> 10) & 3;
        code = 13;
        *page_size = 1024 * 1024;
    } else {
        /* Lookup l2 entry.  */
        if(type == 1) {
            /* Coarse pagetable.  */
            table = (desc & 0xfffffc00) | ((address >> 10) & 0x3fc);
        } else {
            /* Fine pagetable.  */
            table = (desc & 0xfffff000) | ((address >> 8) & 0xffc);
        }
        desc = ldl_phys(table);
        switch(desc & 3) {
            case 0: /* Page translation fault.  */
                code = 7;
                goto do_fault;
            case 1: /* 64k page.  */
                phys_addr = (desc & 0xffff0000) | (address & 0xffff);
                ap = (desc >> (4 + ((address >> 13) & 6))) & 3;
                *page_size = 0x10000;
                break;
            case 2: /* 4k page.  */
                phys_addr = (desc & 0xfffff000) | (address & 0xfff);
                ap = (desc >> (4 + ((address >> 13) & 6))) & 3;
                *page_size = 0x1000;
                break;
            case 3: /* 1k page.  */
                if(type == 1) {
                    if(arm_feature(env, ARM_FEATURE_XSCALE)) {
                        phys_addr = (desc & 0xfffff000) | (address & 0xfff);
                    } else {
                        /* Page translation fault.  */
                        code = 7;
                        goto do_fault;
                    }
                } else {
                    phys_addr = (desc & 0xfffffc00) | (address & 0x3ff);
                }
                ap = (desc >> 4) & 3;
                *page_size = 0x400;
                break;
            default:
                /* Never happens, but compiler isn't smart enough to tell.  */
                abort();
        }
        code = 15;
    }
    *prot = check_ap(env, ap, domain, access_type, is_user);
    if(!*prot) {
        /* Access permission fault.  */
        goto do_fault;
    }
    *prot |= PAGE_EXEC;
    *phys_ptr = phys_addr;
    return TRANSLATE_SUCCESS;
do_fault:
    return code | (domain << 4);  //  TRANSLATE_FAIL
}

static int get_phys_addr_lpae(CPUState *env, uint32_t address, int access_type, int is_user, uint32_t *phys_ptr, int *prot,
                              target_ulong *page_size)
{
    //  NOTE: the implementation is limited to u-boot usecase (i.e. identity mapping, no faults)
    uint32_t table;
    uint64_t desc;
    uint32_t index;
    int type;
    uint32_t phys_addr;

    /* page table walk */

    /* LEVEL 1 */
    table = env->cp15.c2_base0_ea;
    index = address >> 30;
    desc = ldq_phys(table + index * 8);
    type = desc & 3;

    switch(type) {
        case 0:
        case 2:
            //  descriptor type: invalid
            goto do_fault;
        case 1:
            //  descriptor type: block
            phys_addr = (desc & 0xC0000000) | (address & 0x3FFFFFFF);
            *page_size = 0x40000000;
            goto success;
    }
    //  type: table
    //  TODO: check page fault etc.
    table = desc & 0xfffff000;  //  level 2 PT address = desc[31:13]

    /* LEVEL 2 */
    index = (address >> 21) & 0b111111111;
    desc = ldq_phys(table + index * 8);
    type = desc & 3;

    switch(type) {
        case 0:
        case 2:
            //  descriptor type: invalid
            goto do_fault;
        case 1:
            //  descriptor type: block
            phys_addr = (desc & 0xFFE00000) | (address & 0x1FFFFF);
            *page_size = 0x200000;
            goto success;
    }
    //  type: table
    //  TODO: check page fault etc.
    table = desc & 0xfffff000;

    /* LEVEL 3 */
    index = (address >> 12) & 0b111111111;
    desc = ldq_phys(table + index * 8);
    //  TODO: check page fault etc.
    phys_addr = (desc & 0xfffff000) | (address & 0xfff);
    *page_size = 0x1000;
success:
    *phys_ptr = phys_addr;
    *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
    return TRANSLATE_SUCCESS;

do_fault:
    return TRANSLATE_FAIL;
}

static int get_phys_addr_v6(CPUState *env, uint32_t address, int access_type, int is_user, uint32_t *phys_ptr, int *prot,
                            target_ulong *page_size)
{
    int code;
    uint32_t table;
    uint32_t desc;
    uint32_t xn;
    int type;
    int ap;
    int domain;
    uint32_t phys_addr;

    /* Pagetable walk.  */
    /* Lookup l1 descriptor.  */
    table = get_level1_table_address(env, address);
    desc = ldl_phys(table);
    type = (desc & 3);
    if(type == 0) {
        /* Section translation fault.  */
        code = 5;
        domain = 0;
        goto do_fault;
    } else if(type == 2 && (desc & (1 << 18))) {
        /* Supersection.  */
        domain = 0;
    } else {
        /* Section or page.  */
        domain = (desc >> 4) & 0x1e;
    }
    domain = (env->cp15.c3 >> domain) & 3;
    if(domain == 0 || domain == 2) {
        if(type == 2) {
            code = 9; /* Section domain fault.  */
        } else {
            code = 11; /* Page domain fault.  */
        }
        goto do_fault;
    }
    if(type == 2) {
        if(desc & (1 << 18)) {
            /* Supersection.  */
            phys_addr = (desc & 0xff000000) | (address & 0x00ffffff);
            *page_size = 0x1000000;
        } else {
            /* Section.  */
            phys_addr = (desc & 0xfff00000) | (address & 0x000fffff);
            *page_size = 0x100000;
        }
        ap = ((desc >> 10) & 3) | ((desc >> 13) & 4);
        xn = desc & (1 << 4);
        code = 13;
    } else {
        /* Lookup l2 entry.  */
        table = (desc & 0xfffffc00) | ((address >> 10) & 0x3fc);
        desc = ldl_phys(table);
        ap = ((desc >> 4) & 3) | ((desc >> 7) & 4);
        switch(desc & 3) {
            case 0: /* Page translation fault.  */
                code = 7;
                goto do_fault;
            case 1: /* 64k page.  */
                phys_addr = (desc & 0xffff0000) | (address & 0xffff);
                xn = desc & (1 << 15);
                *page_size = 0x10000;
                break;
            case 2:  //  4k page.
            case 3:
                phys_addr = (desc & 0xfffff000) | (address & 0xfff);
                xn = desc & 1;
                *page_size = 0x1000;
                break;
            default:
                /* Never happens, but compiler isn't smart enough to tell.  */
                abort();
        }
        code = 15;
    }
    if(domain == 3) {
        *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
    } else {
        if(xn && access_type == ACCESS_INST_FETCH) {
            goto do_fault;
        }

        /* The simplified model uses AP[0] as an access control bit.  */
        if((env->cp15.c1_sys & (1 << 29)) && (ap & 1) == 0) {
            /* Access flag fault.  */
            code = (code == 15) ? 6 : 3;
            goto do_fault;
        }
        *prot = check_ap(env, ap, domain, access_type, is_user);
        if(!*prot) {
            /* Access permission fault.  */
            goto do_fault;
        }
        if(!xn) {
            *prot |= PAGE_EXEC;
        }
    }
    *phys_ptr = phys_addr;
    return TRANSLATE_SUCCESS;
do_fault:
    return code | (domain << 4);  //  TRANSLATE_FAIL
}

static int cortexm_check_default_mapping(uint32_t address, int *prot, int access_type)
{
    switch(address) {
        case 0x00000000 ... 0x1FFEFFFF:
            *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
            break;
        case 0x1FFF0000 ... 0x1FFF77FF:
            *prot = PAGE_READ | PAGE_EXEC;
            break;
        case 0x1FFF7800 ... 0x1FFFFFFF:
        case 0x20000000 ... 0x3FFFFFFF:
        case 0x60000000 ... 0x7FFFFFFF:
        case 0x80000000 ... 0x9FFFFFFF:
            *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
            break;
        case 0x40000000 ... 0x5FFFFFFF:
        case 0xA0000000 ... 0xBFFFFFFF:
        case 0xC0000000 ... 0xDFFFFFFF:
        case 0xE0000000 ... 0xE00FFFFF:
            *prot = PAGE_READ | PAGE_WRITE;
            break;
        case 0xE0100000 ... 0xFFFFFFFF:
        default:
            *prot = 0;
            return PAGE_READ | PAGE_WRITE | PAGE_EXEC;
    }
    return !(*prot & (1 << access_type));
}

static int pmsav7_check_default_mapping(uint32_t address, int *prot, int access_type)
{
    *prot = PAGE_READ | PAGE_WRITE;
    switch(address) {
        case 0xF0000000 ... 0xFFFFFFFF:
            //  executable if high exception vectors are selected
            if(!(env->cp15.c1_sys & (1 << 13))) {
                break;
            }
            /* fallthrough */
        case 0x00000000 ... 0x7FFFFFFF:
            *prot |= PAGE_EXEC;
        default:
            break;
    }
    return (*prot & (1 << access_type)) ? MPU_SUCCESS : MPU_PERMISSION_FAULT;
}

static int get_mpu_subregion_number(uint32_t region_base_address, uint32_t region_size, uint32_t address)
{
    /* Subregion size is 2^(region_size - 3) */
    uint32_t subregion_size = 1 << (region_size - 3);
    return (address - region_base_address) / subregion_size;
}

static bool page_with_address_is_fully_covered_by_consistent_mpu_subregions(uint32_t subregion_disable_mask,
                                                                            uint32_t region_base_address, uint32_t region_size,
                                                                            uint32_t address)
{
    int i, first_subregion_number, last_subregion_number;
    uint32_t first_subregion_state;
    uint32_t page_start = address & TARGET_PAGE_MASK;
    uint32_t page_size = TARGET_PAGE_SIZE;

    if(region_base_address > page_start || region_base_address + region_size < page_start + page_size) {
        //  No need to check particular subregions as a page is not contained within the whole region
        return false;
    }

    first_subregion_number = get_mpu_subregion_number(region_base_address, region_size, page_start);
    last_subregion_number = get_mpu_subregion_number(region_base_address, region_size, page_start + page_size - 1);

    if(first_subregion_number == last_subregion_number) {
        return true;
    }

    first_subregion_state = !(subregion_disable_mask & (1 << first_subregion_number));
    for(i = first_subregion_number + 1; i <= last_subregion_number; i++) {
        if(first_subregion_state != !(subregion_disable_mask & (1 << i))) {
            //  There are mixed disabled and enabled subregions covering a single page
            return false;
        }
    }
    return true;
}

static int get_phys_addr_mpu(CPUState *env, uint32_t address, int access_type, int is_user, uint32_t *phys_ptr, int *prot,
                             target_ulong *page_size)
{
    int n;
    uint32_t base;
    uint32_t size;
    uint32_t mask;
    uint32_t perms;
    bool page_contains_mpu_region = false;

    *phys_ptr = address;
    *prot = 0;

#if DEBUG
    tlib_printf(LOG_LEVEL_DEBUG, "MPU: Trying to access address 0x%X", address);
#endif

    for(n = env->number_of_mpu_regions - 1; n >= 0; n--) {
        if(!(env->cp15.c6_size_and_enable[n] & MPU_REGION_ENABLED_BIT)) {
            continue;
        }
        switch((env->cp15.c6_size_and_enable[n] & MPU_SIZE_FIELD_MASK) >> 1) {
            case 0 ... 3:
                tlib_printf(LOG_LEVEL_WARNING,
                            "Encountered MPU region size smaller than 32bytes, this is an unpredictable setting!");
                continue;
            default:
                size = ((env->cp15.c6_size_and_enable[n] & MPU_SIZE_FIELD_MASK) >> 1) + 1;
                break;
        }

        base = env->cp15.c6_base_address[n];
        mask = (1ull << size) - 1;
        if(base & mask) {
            //  The ARMv7 reference manual specifies that memory regions should be naturally alligned, otherwise the
            //  behavior is `UNPREDICTABLE` However some hardware implementations (e.g. NXP S32K388) ignore the lower bits,
            //  forcing alignment. Since some versions of NXP's SDK rely on this behavior we emulate it here.
            tlib_printf(LOG_LEVEL_DEBUG, "Base address 0x%x is not naturally aligned for size 0x%x, adjusting to 0x%x", base,
                        size, base & (~mask));
            base = base & (~mask);
        }

        if((address & TARGET_PAGE_MASK) == (base & TARGET_PAGE_MASK)) {
            page_contains_mpu_region = true;
        }

        /* Check if the region is enabled */
        if(address >= base && address <= base + mask) {
            page_contains_mpu_region = true;
            /* Check subregions, only if region size is equal to or bigger than 256 bytes (region size = 2^size) */
            if(size >= 8) {
                if(!page_with_address_is_fully_covered_by_consistent_mpu_subregions(env->cp15.c6_subregion_disable[n], base,
                                                                                    1 << size, address)) {
                    /* MPU subregions with the same state (enabled/disabled) don't cover the whole page.
                     * Setting page size != TARGET_PAGE_SIZE effectively makes the tlb page entry one-shot:
                     * Thanks to this every access to this page will be verified against MPU.
                     */
                    *page_size = 0;
                }
                if(env->cp15.c6_subregion_disable[n] & (1 << get_mpu_subregion_number(base, size, address))) {
                    /* Subregion containing this address is disabled, try to match this address to a different region. */
                    continue;
                }
            } else {
                /* The page is not fully covered by a single MPU region */
                *page_size = 0;
            }

            break;
        }
    }

    if(n < 0) {  //  background fault
        int background_result;
#ifdef TARGET_PROTO_ARM_M
        if(is_user && unlikely(env->v7m.ccr[env->secure] & FIELD_MASK(V7M_CCR, USERSETMPEND)) &&
           tlib_nvic_is_stir_address(address)) {
            *prot = PAGE_READ | PAGE_WRITE;
            background_result = !(*prot & (1 << access_type));
            //  Unlike other background faults, this one isn't bigger than a page
            *page_size = 0;
        } else
#endif
            if(arm_feature(env, ARM_FEATURE_PMSA)) {
            if(is_user || !(env->cp15.c1_sys & (1 << 17 /* BR, Background Region */))) {
                background_result = MPU_BACKGROUND_FAULT;
            } else {
                background_result = pmsav7_check_default_mapping(address, prot, access_type);
            }
        } else if(!is_user) {
            background_result = cortexm_check_default_mapping(address, prot, access_type);
        } else {
            background_result = TRANSLATE_FAIL;
        }

        if(background_result == TRANSLATE_SUCCESS && page_contains_mpu_region) {
            /* Background pages cannot be stored in tlb if those pages contain any MPU regions
             * as access checks will not be performed for pages that are present in TLB.
             * Setting page size != TARGET_PAGE_SIZE effectively makes the tlb page entry one-shot:
             * Thanks to this every access to this page will be verified against MPU.
             */
            *page_size = 0;
        }
        return background_result;
    }

    perms = (env->cp15.c6_access_control[n] & MPU_PERMISSION_FIELD_MASK) >> 8;

    switch(perms) {
        case 0:
            return MPU_PERMISSION_FAULT;
        case 1:
            if(is_user) {
                return MPU_PERMISSION_FAULT;
            }
            *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
            break;
        case 2:
            *prot = PAGE_READ | PAGE_EXEC;
            if(!is_user) {
                *prot |= PAGE_WRITE;
            }
            break;
        case 3:
            *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
            break;
        case 5:
            if(is_user) {
                return MPU_PERMISSION_FAULT;
            }
            *prot = PAGE_READ | PAGE_EXEC;
            break;
        case 6:
            *prot = PAGE_READ | PAGE_EXEC;
            break;
#ifdef TARGET_PROTO_ARM_M
        case 7:
            *prot |= PAGE_READ | PAGE_EXEC;
            break;
#endif
        default:
            /* Bad permission.  */
            break;
    }

    /* Check if the region is executable */
    if((env->cp15.c6_access_control[n] & MPU_NEVER_EXECUTE_BIT)) {
        *prot &= ~PAGE_EXEC;
    }

    /* PAGE_READ  = 1 ; ACCESS_TYPE = 0
     * PAGE_WRITE = 2 ; ACCESS_TYPE = 1
     * PAGE_EXEC  = 3 ; ACCESS_TYPE = 2
     */
    if(*prot & (1 << access_type)) {
        return TRANSLATE_SUCCESS;
    }
    return MPU_PERMISSION_FAULT;
}

#ifdef TARGET_PROTO_ARM_M

static int cortexm_check_default_mapping_v8(uint32 address)
{
    switch(address) {
        case 0x00000000 ... 0x7FFFFFFF:
            return PAGE_READ | PAGE_WRITE | PAGE_EXEC;
            break;
        //  Devices
        case 0x80000000 ... 0xFFFFFFFF:
            return PAGE_READ | PAGE_WRITE;
            break;
        default:
            tlib_abortf("Address out of range. This should never happen");
            return 0; /* Never happens.  Keep compiler happy.  */
    }
}

#define PMSA_MPU_REGION_INVALID -1

static inline bool pmsav8_mpu_region_valid(int region_index)
{
    return region_index != PMSA_MPU_REGION_INVALID;
}

/* Helper used in PMSAv8 and IDAU/SAU lookups checking if the region doesn't break the accessed page into parts. */
static inline void update__applies_to_whole_page(uint32_t address, uint32_t region_start, uint32_t region_end,
                                                 bool *applies_to_whole_page)
{
    /* Any region start or end on the page which is not equal to the page start and end means that
     * the function result for the address doesn't necessarily apply to the whole page.
     *
     * This isn't needed if we already know that the whole page can't be treated the same.
     */
    if(applies_to_whole_page != NULL && *applies_to_whole_page) {
        uint32_t page_start = address & TARGET_PAGE_MASK;
        uint32_t page_end = page_start + TARGET_PAGE_SIZE - 1;

        if(((region_start > page_start) && (region_start <= page_end)) ||
           ((region_end >= page_start) && (region_end < page_end))) {
            *applies_to_whole_page = false;
        }
    }
}

/* `applies_to_whole_page` can be passed NULL in which case the function won't be checking if the
 * returned permissions are valid for the whole page. The same applies if it's value is false from
 * the beginning in which case it will always stay false.
 */
static inline bool pmsav8_get_region(CPUState *env, uint32_t address, bool secure, int *region_index, bool *multiple_regions,
                                     bool *applies_to_whole_page)
{
    int n;
    bool hit = false;
    *multiple_regions = false;
    *region_index = PMSA_MPU_REGION_INVALID;

    for(n = env->number_of_mpu_regions - 1; n >= 0; n--) {
        if(!(env->pmsav8[secure].rlar[n] & 0x1)) {
            /* Region disabled */
            continue;
        }

        uint32_t base = pmsav8_idau_sau_get_region_base(env->pmsav8[secure].rbar[n]);
        uint32_t limit = pmsav8_idau_sau_get_region_limit(env->pmsav8[secure].rlar[n]);

        update__applies_to_whole_page(address, base, limit, applies_to_whole_page);

        if(address < base || address > limit) {
            /* Addr not in this region */
            continue;
        }

        /* region matched */
        if(hit) {
            /* multiple regions always return a failure
             * in this case region_index _must not_ be used
             */
            *multiple_regions = true;
            *region_index = PMSA_MPU_REGION_INVALID;

            /* Returning after finding the second region is safe even if `applies_to_whole_page` is
             * still true because `multiple_regions` will fail translation so permissions won't be
             * added to TLB anyway.
             */
            return false;
        }

        hit = true;
        *region_index = n;
    }
    return hit;
}

//  Always check return value first as `found_index` is only valid on success.
//  NULL can be safely passed when the index doesn't matter.
inline bool try_get_impl_def_attr_exemption_region(uint32_t address, uint32_t start_at, uint32_t *found_index,
                                                   bool *applies_to_whole_page)
{
    bool applies_to_whole_page_was_true = applies_to_whole_page != NULL && *applies_to_whole_page;
    bool result = false;

    for(uint32_t index = start_at; index < cpu->impl_def_attr_exemptions.count; index++) {
        uint32_t start = cpu->impl_def_attr_exemptions.start[index];
        uint32_t end = cpu->impl_def_attr_exemptions.end[index];
        if(start <= address && end >= address) {
            if(found_index != NULL) {
                *found_index = index;
            }
            result = true;

            /* This is a special case in all the lookups because multiple matched regions have no
             * special meaning. Therefore the result will be certainly the same for the whole page
             * if the matched region covers the whole page.
             *
             * It only matters if `applies_to_whole_page` was true from the very beginning; in that
             * case we restore it and break the loop after it's updated for this matched region.
             */
            if(applies_to_whole_page_was_true) {
                *applies_to_whole_page = true;
            }
        }
        update__applies_to_whole_page(address, start, end, applies_to_whole_page);

        if(result) {
            break;
        }
    }
    return result;
}

inline bool is_impl_def_exempt_from_attribution(uint32_t address, bool *applies_to_whole_page)
{
    return try_get_impl_def_attr_exemption_region(address, /* start_at: */ 0, /* found_index: */ NULL, applies_to_whole_page);
}

static inline bool pmsav8_is_exempt_from_attribution(uint32_t address, int access_type, bool *applies_to_whole_page)
{
    /* ARMv8-M Manual: Rule LDTN */
    switch(address) {
        case 0xE0000000 ... 0xE0003FFF:  //  ITM, DWT, FPB, PMU
        case 0xE0005000 ... 0xE0005FFF:  //  RAS error record registers
        case 0xE000E000 ... 0xE000EFFF:  //  SCS Secure and Non-secure range
        case 0xE002E000 ... 0xE002EFFF:  //  SCS Non-secure alias range
        case 0xE0040000 ... 0xE0041FFF:  //  TPIU, ETM
        case 0xE00FF000 ... 0xE00FFFFF:  //  ROM table
            return true;
    }

    if(is_impl_def_exempt_from_attribution(address, applies_to_whole_page)) {
        return true;
    }

    if(access_type == ACCESS_INST_FETCH && address >= 0xE0000000 && address <= 0xEFFFFFFF) {
        /* We don't want to cache this special case as we have no guarantee there will be no
         * security fault for data accesses. This case is checked last because of that.
         */
        if(applies_to_whole_page != NULL) {
            *applies_to_whole_page = false;
        }
        return true;
    }
    return false;
}

/* `applies_to_whole_page` can be passed NULL in which case the function won't be checking if the
 * returned attribution is the same for the whole page. The same applies if it's value is false from
 * the beginning in which case it will always stay false.
 */
static inline bool pmsav8_idau_sau_try_get_region(uint32_t address, uint32_t *rbars, uint32_t *rlars, uint32_t regions_count,
                                                  int *region_index, enum security_attribution *attribution,
                                                  bool *applies_to_whole_page)
{
    int n;
    bool hit = false;
    *region_index = -1;
    *attribution = SA_SECURE;

    for(n = 0; n < regions_count; n++) {
        if(!(rlars[n] & IDAU_SAU_RLAR_ENABLE)) {
            /* Region disabled */
            continue;
        }

        uint32_t base = pmsav8_idau_sau_get_region_base(rbars[n]);
        uint32_t limit = pmsav8_idau_sau_get_region_limit(rlars[n]);
        bool nsc = rlars[n] & IDAU_SAU_RLAR_NSC;

        update__applies_to_whole_page(address, base, limit, applies_to_whole_page);

        if(address < base || address > limit) {
            /* Addr not in this region */
            continue;
        }

        /* Another region matched?
         *
         * Note that we don't break the loop after finding the first matching region as we need to
         * make sure it's the only matching region. SAU region isn't valid otherwise and we follow
         * the same rules for IDAU.
         */
        if(hit) {
            /* An address that matches multiple SAU regions is marked as Secure and
             * not Not-secure callable regardless of the attributes specified by the
             * regions that matched the address; ARMv8-M Manual: Rule WGDK.
             */
            *attribution = SA_SECURE;

            /* Returning after finding the second region is safe even if `applies_to_whole_page` is
             * still true and there is yet another region only partially covering the page.
             *
             * The only possible option for that is that both matched regions cover the whole page.
             * In that case a potential third region only partially covering the page won't really
             * matter as the two matched regions covering the whole page will make the the result
             * the same for every access of this page.
             */
            return false;
        }

        hit = true;
        *region_index = n;

        /* Memory is marked as Secure by default. However, if the address matches a region with
         * SAU_REGIONn.ENABLE set to 1 and SAU_REGIONn.NSC set to 0, then memory is marked as
         * Non-secure; ARMv8-M Manual: Rule MPJC.
         *
         * This is somewhat contrary to the SAU_RLAR.NSC bit description which states that 0 means
         * "Region is marked with the Secure attribute and is not Non-secure callable." but the
         * behavior is confirmed by pseudocode for SecurityCheck in the ARMv8-M Manual.
         */
        *attribution = nsc ? SA_SECURE_NSC : SA_NONSECURE;
    }
    return hit;
}

/* The return value is true if SAU is enabled and a single region was matched. */
static inline bool pmsav8_sau_try_get_region(CPUState *env, uint32_t address, int *region_index,
                                             enum security_attribution *attribution, bool *applies_to_whole_page)
{
    if(!(env->sau.ctrl & SAU_CTRL_ENABLE)) {
        *attribution = env->sau.ctrl & SAU_CTRL_ALLNS ? SA_NONSECURE : SA_SECURE;

        /* `applies_to_whole_page` is untouched intentionally. This result definitely applies to the
         * whole page but it's only one of a few lookups so let's keep it false if it was false.
         */
        return false;
    }

    return pmsav8_idau_sau_try_get_region(address, env->sau.rbar, env->sau.rlar, env->number_of_sau_regions, region_index,
                                          attribution, applies_to_whole_page);
}

/* The return value is true if a single IDAU region was matched. */
static inline bool pmsav8_idau_try_get_region(CPUState *env, uint32_t address, int *region_index,
                                              enum security_attribution *attribution, bool *applies_to_whole_page)
{
    //  The function should only be called if IDAU is enabled cause IDAU disabled is indistinguishable from no hit
    tlib_assert(env->idau.enabled);

    return pmsav8_idau_sau_try_get_region(address, env->idau.rbar, env->idau.rlar, env->number_of_idau_regions, region_index,
                                          attribution, applies_to_whole_page);
}

/* `applies_to_whole_page` can be passed NULL in which case the function won't be checking if the
 * returned attribution is the same for the whole page. The same applies if it's value is false from
 * the beginning in which case it will always stay false.
 */
static inline void pmsav8_get_security_attribution(CPUState *env, uint32_t address, bool secure, int access_type,
                                                   int access_width, bool *idau_valid, int *idau_region, bool *sau_valid,
                                                   int *sau_region, enum security_attribution *attribution,
                                                   bool *applies_to_whole_page)
{
    *idau_valid = false;
    *sau_valid = false;

    if(applies_to_whole_page != NULL) {
        *applies_to_whole_page = true;
    }

    tlib_assert(access_width > 0);
    uint32_t access_end_address = address + access_width - 1;

    //  Base address is used for all the checks, granularity is 32B.
    address = pmsav8_idau_sau_get_region_base(address);

    //  Whole 0xF0000000-0xFFFFFFFF is Secure with TrustZone, NS otherwise; ARMv8-M Manual: Rule FGDW.
    if((address & 0xF0000000) == 0xF0000000) {
        *attribution = env->v7m.has_trustzone ? SA_SECURE : SA_NONSECURE;
        return;
    }

    //  Attribution is the same as access security for exemptions; ARMv8-M Manual: Rule LDTN.
    if(pmsav8_is_exempt_from_attribution(address, access_type, applies_to_whole_page)) {
        *attribution = secure ? SA_SECURE : SA_NONSECURE;
        return;
    }

    //  Even if SAU region not valid, the attribution returned can be NS due to `ALLNS` option changing the default.
    *sau_valid = pmsav8_sau_try_get_region(env, address, sau_region, attribution, applies_to_whole_page);

    //  Make sure last byte accessed belongs to the same region.
    tlib_assert(!*sau_valid || access_end_address <= pmsav8_idau_sau_get_region_limit(env->sau.rlar[*sau_region]));

    //  Implementation-defined exemptions have been checked already so we can just skip IDAU lookup if it's disabled.
    if(!env->idau.enabled) {
        return;
    }

    enum security_attribution idau_attribution;
    if(unlikely(env->idau.custom_handler_enabled)) {
        ExternalIDAURequest request = { address, (int32_t)secure, access_type, access_width };
        *idau_valid = tlib_custom_idau_handler(&request, idau_region, &idau_attribution);

        //  Custom IDAU's attribution isn't guaranteed to be the same for the whole page (and all access types).
        if(applies_to_whole_page != NULL) {
            *applies_to_whole_page = false;
        }
    } else if(pmsav8_idau_try_get_region(env, address, idau_region, &idau_attribution, applies_to_whole_page)) {
        //  Make sure last byte accessed belongs to the same region.
        tlib_assert(access_end_address <= pmsav8_idau_sau_get_region_limit(env->idau.rlar[*idau_region]));

        *idau_valid = true;
    } else {
        return;
    }
    //  More restrictive IDAU attribution overrides SAU attribution.
    *attribution = attribution_get_more_secure(idau_attribution, *attribution);
}

#define PMSA_ENABLED(ctrl)    ((ctrl & 0b001))
#define PMSA_PRIVDEFENA(ctrl) ((ctrl & 0b100))
#define PMSA_AP_PRIVONLY(ap)  ((ap & 0b01) == 0)
#define PMSA_AP_READONLY(ap)  ((ap & 0b10) != 0)

//  Returns `TRANSLATE_SUCCESS` in case access is valid and `TRANSLATE_FAILURE` otherwise.
//  MPU region matched is valid in both cases if different than `PMSA_MPU_REGION_INVALID`.
//  `page_size` can be safely passed NULL if it doesn't matter.
static inline int pmsav8_check_access_with_region(CPUState *env, uint32_t address, bool secure, int access_type, int is_user,
                                                  int *prot, target_ulong *page_size, int *resolved_region)
{
    bool applies_to_whole_page = true;
    bool hit;
    bool multiple_regions = false;
    bool mpu_enabled = PMSA_ENABLED(env->pmsav8[secure].ctrl);

    *prot = 0;
    *resolved_region = PMSA_MPU_REGION_INVALID;

    if(!mpu_enabled) {
        hit = false;
    } else {
        hit = pmsav8_get_region(env, address, secure, resolved_region, &multiple_regions, &applies_to_whole_page);

        /* Overlapping regions generate MemManage Fault
         * R_LLLP in Arm® v8-M Architecture Reference Manual DDI0553B.l ID30062020 */
        if(unlikely(multiple_regions)) {
            tlib_assert(!pmsav8_mpu_region_valid(*resolved_region));
            goto do_fault;
        }
    }
    tlib_assert(hit || !pmsav8_mpu_region_valid(*resolved_region));

    if(hit) {
        uint32_t rbar = env->pmsav8[secure].rbar[*resolved_region];
        uint32_t rlar = env->pmsav8[secure].rlar[*resolved_region];
        int xn = extract32(rbar, 0, 1);
        int ap = extract32(rbar, 1, 2);
        int pxn = arm_feature(env, ARM_FEATURE_V8_1M) && extract32(rlar, 4, 1);

        if(!PMSA_AP_PRIVONLY(ap) || !is_user) {
            *prot |= PAGE_READ;
            if(!PMSA_AP_READONLY(ap)) {
                *prot |= PAGE_WRITE;
            }
        }

        if(!xn && (is_user || !pxn)) {
            *prot |= PAGE_EXEC;
        }
    } else {
        /* No region hit, use background region if:
         * - MPU disabled: for all accesses
         * - MPU enabled: for privileged accesses if default memory map is enabled (PRIVDEFENA)
         */
        if(!mpu_enabled || (!is_user && PMSA_PRIVDEFENA(env->pmsav8[secure].ctrl))) {
            *prot = cortexm_check_default_mapping_v8(address);
        } else {
            goto do_fault;
        }
    }

    //  XN is enforced in 0xE0000000-0xFFFFFFFF space; ARMv8-M Manual: Rules VCTC and KDJG.
    if(address >= 0xE0000000) {
        *prot &= ~PAGE_EXEC;
    }

    if(is_page_access_valid(*prot, access_type)) {
        //  Page size might not be needed, e.g., for the TT(A)(T) instruction helper.
        if(page_size == NULL) {
            return TRANSLATE_SUCCESS;
        }

        /* Otherwise, making sure the returned permissions are valid for the whole page is crucial
         * when returning success cause those will be cached per page. Precise size isn't strictly
         * necessary as one-shot TLB entries are currently always added if `page_size` is smaller
         * than `TARGET_PAGE_SIZE` (see `tlb_set_page`). It can't be lower than PMSAv8 granularity
         * though so let's use that value if page isn't uniform MPU-wise.
         *
         * `TARGET_PAGE_SIZE` is always returned for valid accesses if MPU is disabled cause background
         * regions are much bigger than page size.
         */
        *page_size = applies_to_whole_page || !mpu_enabled ? TARGET_PAGE_SIZE : PMSAV8_IDAU_SAU_REGION_GRANULARITY_B;
        return TRANSLATE_SUCCESS;
    }

do_fault:
    return TRANSLATE_FAIL;
}

static inline int pmsav8_check_access(CPUState *env, uint32_t address, bool secure, int access_type, int is_user, int *prot,
                                      target_ulong *page_size)
{
    int region;  //  ignored
    return pmsav8_check_access_with_region(env, address, secure, access_type, is_user, prot, page_size, &region);
}

/* Must be in sync with CortexM.StateBits.
Semihosting bit is not set by tlib.
It is automatically added by Renode when
semihosting access is performed on behalf of the CPU.
*/
union cpu_transaction_state {
    uint64_t value;
    struct {
#if __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
        unsigned char padding[7];
        unsigned char : 4;
        /*  3 */ bool semihosting : 1;
        /*  2 */ bool bus_secure : 1;
        /*  1 */ bool secure : 1;
        /*  0 */ bool privileged : 1;
#else
        /*  0 */ bool privileged : 1;
        /*  1 */ bool secure : 1;
        /*  2 */ bool bus_secure : 1;
        /*  3 */ bool semihosting : 1;
#endif
    } flags;
};

//  Assert expected size of cpu_transaction_state structure.
static_assert(sizeof(union cpu_transaction_state) == 8, "cpu_transaction_state should fit in uint64_t");

//  Assert expected layout of cpu_transaction_state on the bit-level.
#if defined(__GNUC__) && !defined(__clang__) && !defined(__INTEL_COMPILER) && !defined(__INTEL_LLVM_COMPILER)
#define ATTR_FORCE_OPT __attribute__((optimize("O2")))
#else
#define ATTR_FORCE_OPT
#endif

extern void bad_bitfield_layout(void) __attribute__((error("unexpected bit-field layout")));

static __attribute__((used)) ATTR_FORCE_OPT void check_bitfield_layout(void)
{
    union cpu_transaction_state u = { .value = 0 };

    u.flags.bus_secure = true;

    if(__builtin_constant_p(u.value) && u.value != 4) {
        bad_bitfield_layout();
    }
}

uint64_t cpu_get_state_for_memory_transaction(CPUState *env, target_ulong addr, int access_type)
{
    bool idau_valid, sau_valid;
    int idau_region, sau_region;
    enum security_attribution attribution;
    union cpu_transaction_state state = { .value = 0 };
    pmsav8_get_security_attribution(env, addr, env->secure, access_type, /* access_width: */ 1, &idau_valid, &idau_region,
                                    &sau_valid, &sau_region, &attribution, /* applies_to_whole_page: */ NULL);
    state.flags.privileged = in_privileged_mode(env);
    state.flags.secure = env->secure;
    state.flags.bus_secure = attribution_is_secure(attribution);
    return state.value;
}
#else
/* Transaction filtering by state is not yet implemented for this architecture.
 * This placeholder function is here to make it clear that more CPUs are expected to support this in the future. */
uint64_t cpu_get_state_for_memory_transaction(CPUState *env, target_ulong addr, int access_type)
{
    return 0;
}
#endif

#ifdef TARGET_PROTO_ARM_M
static inline bool pmsav8_check_security_attribution(CPUState *env, uint32_t address, bool is_secure, int access_type,
                                                     bool suppress_faults, uint32_t *page_size, int *allowed_permissions,
                                                     enum security_attribution *attribution)
{
    bool idau_valid, sau_valid, applies_to_whole_page = true;
    int idau_region, sau_region;
    pmsav8_get_security_attribution(env, address, is_secure, access_type, /* access_width */ 1, &idau_valid, &idau_region,
                                    &sau_valid, &sau_region, attribution, &applies_to_whole_page);
    *page_size = applies_to_whole_page ? TARGET_PAGE_SIZE : PMSAV8_IDAU_SAU_REGION_GRANULARITY_B;

    uint32_t fault_status = 0;

    /* See: B10.2 Security attribution
     * Summary:
     *   Access:        Memory: SA_NONSECURE  SA_SECURE_NSC  SA_SECURE
     *   Secure fetch           FAULT         OK             OK
     *   Secure store/load      OK            OK             OK
     *   Non-secure fetch       OK            OK (SG only)   FAULT
     *   Non-secure store/load  OK            FAULT          FAULT
     *
     * In secure access to SA_NONSECURE memory and non-secure access to SA_SECURE_NSC memory cases
     * we can't cache fetch or store/load permissions, respectively, even when MPU would potentially
     * allow them because then the security check won't be started for the failing cases.
     *
     * In these cases MPU permissions will be restricted using `allowed_permissions`.
     */
    *allowed_permissions = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
    if(((*attribution == SA_NONSECURE) && is_secure) || ((*attribution == SA_SECURE) && !is_secure)) {
        if(access_type == ACCESS_INST_FETCH) {
            /* The fault ocurred, the fault type is determined by the direction into which domain we are crossing
             * while trying to execute code (fetch instruction for execution) */
            fault_status = is_secure ? SECURE_FAULT_INVTRAN : SECURE_FAULT_INVEP;
        } else {
            *allowed_permissions = PAGE_READ | PAGE_WRITE;
        }
    }

    if(fault_status == 0 && attribution_is_secure(*attribution) && !is_secure) {
        if(access_type != ACCESS_INST_FETCH) {
            fault_status = SECURE_FAULT_AUVIOL;
        } else {
            *allowed_permissions = PAGE_EXEC;
        }
    }

    if(fault_status != 0 && !suppress_faults) {
        fault_status |= SECURE_FAULT_SFARVALID;
        tlib_printf(LOG_LEVEL_WARNING,
                    "[PC=0x%" PRIx32 "] SecureFault while accessing address in %s state: 0x%" PRIx32
                    ", access type: %s, fault status: 0x%" PRIx32,
                    env->regs[15], is_secure ? "secure" : "non-secure", address, ACCESS_TYPE_STRING(access_type), fault_status);

        env->v7m.secure_fault_address = address;
        env->v7m.secure_fault_status |= fault_status;
        env->exception_index = EXCP_SECURE;
    }
    return fault_status == 0;
}
#endif

/* Returns TRANSLATE_SUCCESS (0x0) on success. In case of failure:
 * - for no PMSA returns c5_data/insn value
 * - for PMSA returns enum mpu_result
 * - if TrustZone is active, and SecureFault occurs, will return `TRANSLATE_FAIL` constant, and set SecureFaultStatus register */
inline int get_phys_addr(CPUState *env, uint32_t address, bool is_secure, int access_type, bool is_user, uint32_t *phys_ptr,
                         int *prot, target_ulong *page_size, int no_page_fault)
{
#ifdef TARGET_PROTO_ARM_M
    if(unlikely(access_type == ACCESS_INST_FETCH && !env->thumb && !no_page_fault)) {
        /* Armv8-M ARM rule RSQLX: attempting to execute with EPSR.T clear
         * generates an INVSTATE UsageFault before the memory access. */
        env->exception_index = EXCP_INVSTATE;
        return TRANSLATE_FAIL;
    }
#endif

    /* Fast Context Switch Extension.  */
    if(address < 0x02000000) {
        address += env->cp15.c13_fcse;
    }

    /* Resulting `page_size` should be a minimum of IDAU/SAU and MPU `page_size`.
     *
     * Generally it's a little tricky because in case no region was matched we still need to know
     * whether there are no regions on the whole page.
     *
     * `TARGET_PAGE_SIZE` is used as default for IDAU/SAU so that the final `page_size` isn't
     * changed if security attribution check wasn't performed.
     *
     * The same goes for allowed permissions which are necessary in case security check differs
     * based on access type though in this case we can just modify permissions provided by MMU/MPU.
     */
    target_ulong idau_sau_page_size = TARGET_PAGE_SIZE;
    int idau_sau_allowed_permissions = PAGE_READ | PAGE_WRITE | PAGE_EXEC;

    int ret;
#ifdef TARGET_PROTO_ARM_M
    enum security_attribution idau_sau_attribution = SA_NONSECURE;

    /* TrustZone: Security attribution happens here */
    if(env->v7m.has_trustzone) {
        if(!pmsav8_check_security_attribution(env, address, is_secure, access_type, /* suppress_faults: */ no_page_fault,
                                              &idau_sau_page_size, &idau_sau_allowed_permissions, &idau_sau_attribution)) {
            //  No need to update `page_size` in this case, we only cache successful translations.
            return TRANSLATE_FAIL;
        }
    }

    /* Handle v8M specific MPU */
    if(arm_feature(env, ARM_FEATURE_V8)) {
        *page_size = TARGET_PAGE_SIZE;
        *phys_ptr = address;
        if(env->number_of_mpu_regions == 0) {
            /* MPU not implemented */
            *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
            ret = TRANSLATE_SUCCESS;
        } else {
            //  MPU is implemented but might be disabled which is handled inside the function.
            ret = pmsav8_check_access(env, address, is_secure, access_type, is_user, prot, page_size);
        }
    } else
#endif
        if((env->cp15.c1_sys & 1) == 0) {
        /* MMU/MPU disabled.  */
        *phys_ptr = address;
        *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
        *page_size = TARGET_PAGE_SIZE;
        ret = TRANSLATE_SUCCESS;
    } else if(arm_feature(env, ARM_FEATURE_MPU)) {
        //  Set default page_size for MPU background fault checks.
        //  Size of region for background mappings is bigger than TARGET_PAGE_SIZE
        //  and our TLB does not support large pages (tlb_add_large_page is suboptimal) so it's a sane default.
        //  We could also extend pmsav7_check_default_mapping and cortexm_check_default_mapping
        //  to return region size but it doesn't bring any advantage.
        //  If MPU uses more granular permissions, it will result in `TLB_ONE_SHOT` tlb entry
        //  on successful translation.
        *page_size = TARGET_PAGE_SIZE;
        ret = get_phys_addr_mpu(env, address, access_type, is_user, phys_ptr, prot, page_size);
    } else if(env->cp15.c2_ttbcr_eae) {
        ret = get_phys_addr_lpae(env, address, access_type, is_user, phys_ptr, prot, page_size);
    } else if(env->cp15.c1_sys & (1 << 23)) {
        ret = get_phys_addr_v6(env, address, access_type, is_user, phys_ptr, prot, page_size);
    } else {
        ret = get_phys_addr_v5(env, address, access_type, is_user, phys_ptr, prot, page_size);
    }

    //  See the comment above `idau_sau_page_size` and `idau_sau_allowed_permissions` declarations.
    *page_size = MIN(idau_sau_page_size, *page_size);
    *prot = *prot & idau_sau_allowed_permissions;

#ifdef TARGET_PROTO_ARM_M
    /* TrustZone translation fetch succeeded and MPU check passed.
     * We have to check if the Non-secure instruction fetch to Secure but Non-secure callable address space will call SG
     * instruction. Because the fetch is executed twice: first for the first half of the instruction and then for the second, we
     * have to check for each half of the instruction one at a time. We take IMPDEF allowing us to abort instruction fetch when
     * first half of the instruction is invalid.
     */
    if(env->v7m.has_trustzone && ret == TRANSLATE_SUCCESS && !is_secure && access_type == ACCESS_INST_FETCH &&
       idau_sau_attribution == SA_SECURE_NSC) {
        if(!is_insn_sg_half(lduw_phys(address))) {
            if(!no_page_fault) {
                tlib_printf(LOG_LEVEL_WARNING,
                            "[PC=0x%" PRIx32
                            "] SecureFault (INVEP): Non-secure code attempted to enter Secure state at address 0x%" PRIx32
                            " which does not contain an SG instruction",
                            env->regs[15], address);
                env->v7m.secure_fault_address = address;
                env->v7m.secure_fault_status |= SECURE_FAULT_INVEP;
                env->exception_index = EXCP_SECURE;
            }
            return TRANSLATE_FAIL;
        }
    }
#endif

    if(ret == TRANSLATE_SUCCESS || no_page_fault) {
        return ret;
    }

    uint32_t c5_value = ret;
    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        c5_value = (ret == MPU_PERMISSION_FAULT) ? PERMISSION_FAULT_STATUS_BITS : BACKGROUND_FAULT_STATUS_BITS;
    }

    if(access_type == ACCESS_INST_FETCH) {
        env->cp15.c5_insn = c5_value;
        env->cp15.c6_insn = address;
        env->exception_index = EXCP_PREFETCH_ABORT;
    } else {
        env->cp15.c5_data = c5_value;
        if(access_type == ACCESS_DATA_STORE && (arm_feature(env, ARM_FEATURE_PMSA) || arm_feature(env, ARM_FEATURE_V6))) {
            env->cp15.c5_data |= (1 << 11);
        }
#ifdef TARGET_PROTO_ARM_M
        env->v7m.memory_fault_address[is_secure] = address;
#else
        env->cp15.c6_data = address;
#endif
        env->exception_index = EXCP_DATA_ABORT;
    }
    return ret;
}

int cpu_handle_mmu_fault(CPUState *env, target_ulong address, int access_type, int mmu_idx, int no_page_fault,
                         target_phys_addr_t *paddr)
{
    uint32_t phys_addr = 0;
    target_ulong page_size = 0;
    int prot = 0;
    int ret;

    MMUMode mode = mmu_index_to_mode(mmu_idx);
    ret = get_phys_addr(env, address, mode.secure, access_type, mode.user, &phys_addr, &prot, &page_size, no_page_fault);

    if(ret == TRANSLATE_SUCCESS) {
        *paddr = phys_addr;
        /* Map a single [sub]page.  */
        phys_addr &= TARGET_PAGE_MASK;
        address &= TARGET_PAGE_MASK;
        tlb_set_page(env, address, phys_addr, prot, mmu_idx, page_size);
        return TRANSLATE_SUCCESS;
    }
    return TRANSLATE_FAIL;
}

target_phys_addr_t cpu_get_phys_page_debug(CPUState *env, target_ulong addr)
{
    uint32_t phys_addr = 0;
    target_ulong page_size = 0;
    int prot = 0;
    int ret;

    ret = get_phys_addr(env, addr, env->secure, ACCESS_DATA_LOAD, in_user_mode(env), &phys_addr, &prot, &page_size,
                        /* no_page_fault: */ 1);

    if(ret != 0) {
        return -1;
    }

    return phys_addr;
}

/* Return basic MPU access permission bits.  */
uint32_t simple_mpu_ap_bits(uint32_t val)
{
    uint32_t ret;
    uint32_t mask;
    int i;
    ret = 0;
    mask = 3;
    for(i = 0; i < 16; i += 2) {
        ret |= (val >> i) & mask;
        mask <<= 2;
    }
    return ret;
}

/* Pad basic MPU access permission bits to extended format.  */
uint32_t extended_mpu_ap_bits(uint32_t val)
{
    uint32_t ret;
    uint32_t mask;
    int i;
    ret = 0;
    mask = 3;
    for(i = 0; i < 16; i += 2) {
        ret |= (val & mask) << i;
        mask <<= 2;
    }
    return ret;
}

uint64_t HELPER(get_cp15_64bit)(CPUState *env, uint32_t insn)
{
    return tlib_read_cp15_64(insn);
}

uint32_t HELPER(get_cp15_32bit)(CPUState *env, uint32_t insn)
{
    return tlib_read_cp15_32(insn);
}

void HELPER(set_cp15_64bit)(CPUState *env, uint32_t insn, uint32_t val_1, uint32_t val_2)
{
    uint64_t val = ((uint64_t)val_2 << 32) | val_1;
    tlib_write_cp15_64(insn, val);
}

void HELPER(set_cp15_32bit)(CPUState *env, uint32_t insn, uint32_t val)
{
    tlib_write_cp15_32(insn, val);
}

void HELPER(set_r13_banked)(CPUState *env, uint32_t mode, uint32_t val)
{
#ifdef TARGET_PROTO_ARM_M
    //  bits [1:0] of SP are WI or SBZP
    val &= 0xFFFFFFFC;
#endif
    if((env->uncached_cpsr & CPSR_M) == mode) {
        env->regs[13] = val;
    } else {
        env->banked_r13[bank_number(mode)] = val;
    }
}

uint32_t HELPER(get_r13_banked)(CPUState *env, uint32_t mode)
{
    if((env->uncached_cpsr & CPSR_M) == mode) {
        return env->regs[13];
    } else {
        return env->banked_r13[bank_number(mode)];
    }
}

#ifdef TARGET_PROTO_ARM_M
/* Helpful note: TrustZone sets bit 7, to indicate non-secure bank for the register
 * Otherwise, the encoding is the same (see: "Armv8-M Architecture Reference Manual")
 */
#define NON_SECURE_REG(x) (x | (1 << 7))
#define IS_REG_NS(x)      (x & (1 << 7))

/* An access to a register not ending in _NS returns the register associated with the current
 * Security state. Access to a register ending in _NS in Secure state returns the Non-secure
 * register. */
uint32_t HELPER(v7m_mrs)(CPUState *env, uint32_t reg)
{
    bool is_secure = env->secure;
    if(IS_REG_NS(reg)) {
        if(!env->secure) {
            //  Access to a register ending in _NS in Non-secure state is RAZ/WI
            return 0;
        }
        is_secure = false;
    }

    switch(reg) {
        case 0: /* APSR */
            return xpsr_read(env) & 0xf8000000;
        case 1: /* IAPSR */
            return xpsr_read(env) & 0xf80001ff;
        case 2: /* EAPSR */
            return xpsr_read(env) & 0xff00fc00;
        case 3: /* xPSR */
            return xpsr_read(env) & 0xff00fdff;
        case 5: /* IPSR */
            return xpsr_read(env) & 0x000001ff;
        case 6: /* EPSR */
            return xpsr_read(env) & 0x0700fc00;
        case 7: /* IEPSR */
            return xpsr_read(env) & 0x0700edff;
        case 8: /* MSP */
            return get_msp(env);
        case NON_SECURE_REG(8): /* MSP_NS */
            return env->v7m.other_ss_msp;
        case 9: /* PSP */
            return in_privileged_mode(env) ? get_psp(env) : 0;
        case NON_SECURE_REG(9): /* PSP_NS */
            return in_privileged_mode(env) ? env->v7m.other_ss_psp : 0;
        case NON_SECURE_REG(10):
        case 10: /* MSPLIM - armv8-m specific */
            return in_privileged_mode(env) ? env->v7m.msplim[is_secure] : 0;
        case NON_SECURE_REG(11):
        case 11: /* PSPLIM - armv8-m specific */
            return in_privileged_mode(env) ? env->v7m.psplim[is_secure] : 0;
        case NON_SECURE_REG(16):
        case 16: /* PRIMASK */
            return in_privileged_mode(env) ? ((env->v7m.primask[is_secure] & 1) != 0) : 0;
        case NON_SECURE_REG(17):
        case 17: /* BASEPRI */
        case 18: /* BASEPRI_MAX */
            return in_privileged_mode(env) ? env->v7m.basepri[is_secure] : 0;
        case NON_SECURE_REG(19):
        case 19: /* FAULTMASK */
            return in_privileged_mode(env) ? env->v7m.faultmask[is_secure] : 0;
        case NON_SECURE_REG(20):
        case 20: /* CONTROL */
            return env->v7m.control[is_secure] | (env->v7m.control[M_REG_COMMON] & ARM_CONTROL_FPCA_MASK) |
                   (is_secure ? (env->v7m.control[M_REG_COMMON] & ARM_CONTROL_SFPA_MASK) : 0);
        case NON_SECURE_REG(24): /* SP_NS */
            return is_using_process_sp(env, in_handler_mode(env), false) ? env->v7m.other_ss_psp : env->v7m.other_ss_msp;
        default:
            /* ??? For debugging only.  */
            cpu_abort(env, "Unimplemented system register read (%d)\n", reg);
            return 0;
    }
}

void HELPER(v7m_msr)(CPUState *env, uint32_t reg, uint32_t val)
{
    bool is_secure = env->secure;
    if(IS_REG_NS(reg)) {
        if(!env->secure) {
            //  Access to a register ending in _NS in Non-secure state is RAZ/WI
            return;
        }
        is_secure = false;
    }

    switch(reg) {
        case 0: /* APSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            xpsr_write(env, val, 0xf8000000);
            break;
        case 1: /* IAPSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            xpsr_write(env, val, 0xf8000000);
            break;
        case 2: /* EAPSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            xpsr_write(env, val, 0xfe00fc00);
            break;
        case 3: /* xPSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            xpsr_write(env, val, 0xfe00fc00);
            break;
        case 5: /* IPSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            /* IPSR bits are readonly.  */
            break;
        case 6: /* EPSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            xpsr_write(env, val, 0x0600fc00);
            break;
        case 7: /* IEPSR */
            if(!in_privileged_mode(env)) {
                return;
            }
            xpsr_write(env, val, 0x0600fc00);
            break;
        case 8: /* MSP */
            //  bits [1:0] of SP are WI or SBZP
            val &= 0xFFFFFFFC;
            if(!in_privileged_mode(env)) {
                return;
            } else {
                set_msp(env, val);
            }
            break;
        case NON_SECURE_REG(8): /* MSP_NS */
            //  bits [1:0] of SP are WI or SBZP
            env->v7m.other_ss_msp = val & 0xFFFFFFFC;
            break;
        case 9: /* PSP */
            //  bits [1:0] of SP are WI or SBZP
            val &= 0xFFFFFFFC;
            set_psp(env, val);
            break;
        case NON_SECURE_REG(9): /* PSP_NS */
            //  bits [1:0] of SP are WI or SBZP
            env->v7m.other_ss_psp = val & 0xFFFFFFFC;
            break;
        case NON_SECURE_REG(10):
        case 10: /* MSPLIM - armv8-m specific */
            env->v7m.msplim[is_secure] = val;
            break;
        case NON_SECURE_REG(11):
        case 11: /* PSPLIM - armv8-m specific */
            env->v7m.psplim[is_secure] = val;
            break;
        case NON_SECURE_REG(16):
        case 16: /* PRIMASK */
            if(!in_privileged_mode(env)) {
                return;
            } else if(val & 1) {
                env->v7m.primask[is_secure] |= PRIMASK_EN;
            } else {
                env->v7m.primask[is_secure] &= ~PRIMASK_EN;
            }
            tlib_nvic_find_pending_irq();
            break;
        case NON_SECURE_REG(17):
        case 17: /* BASEPRI */
            if(!in_privileged_mode(env)) {
                return;
            }
            env->v7m.basepri[is_secure] = val & 0xff;
            tlib_nvic_write_basepri(val & 0xff, is_secure);
            break;
        case 18: /* BASEPRI_MAX */
            if(!in_privileged_mode(env)) {
                return;
            }
            val &= 0xff;
            if(val != 0 && (val < env->v7m.basepri[is_secure] || env->v7m.basepri[is_secure] == 0)) {
                env->v7m.basepri[is_secure] = val;
                tlib_nvic_write_basepri(val, is_secure);
            }
            break;
        case NON_SECURE_REG(19):
        case 19: /* FAULTMASK */
            if(!in_privileged_mode(env)) {
                return;
            }
            env->v7m.faultmask[is_secure] = val & 1;
            tlib_nvic_find_pending_irq();
            break;
        case NON_SECURE_REG(20):
        case 20: /* CONTROL */
            if(is_secure) {
                env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_SFPA_MASK;
                env->v7m.control[M_REG_COMMON] |= val & ARM_CONTROL_SFPA_MASK;
            }

            if(!in_privileged_mode(env)) {
                return;
            }

            env->v7m.control[is_secure] &= ~3;
            env->v7m.control[is_secure] |= val & 3;
            if(is_secure || env->v7m.nsacr & ARM_NSACR_CP10_MASK) {
                env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_FPCA_MASK;
                env->v7m.control[M_REG_COMMON] |= val & ARM_CONTROL_FPCA_MASK;
            }
            //  only switch the stack if in thread mode (handler mode always uses MSP stack)
            if(is_secure == cpu->secure) {
                switch_v7m_sp(env, is_using_process_sp_current(env));
            }
            break;
        case NON_SECURE_REG(24): /* SP_NS */
            //  bits [1:0] of SP are WI or SBZP
            val &= 0xFFFFFFFC;
            if(is_using_process_sp(env, in_handler_mode(env), false)) {
                env->v7m.other_ss_psp = val;
            } else {
                env->v7m.other_ss_msp = val;
            }
            break;
        default:
            /* ??? For debugging only.  */
            cpu_abort(env, "Unimplemented system register write (%d)\n", reg);
            return;
    }
}
#undef NON_SECURE_REG
#undef IS_REG_NS
#endif

/* Note that signed overflow is undefined in C.  The following routines are
   careful to use unsigned types where modulo arithmetic is required.
   Failure to do so _will_ break on newer gcc.  */

/* Signed saturating arithmetic.  */

/* Perform 16-bit signed saturating addition.  */
#define PFX_Q
#include "op_addsub.h"
#undef PFX_Q

#define PFX_UQ
#include "op_addsub.h"
#undef PFX_UQ

/* Signed modulo arithmetic.  */
#define PFX_S
#define ARITH_GE
#include "op_addsub.h"
#undef ARITH_GE
#undef PFX_S

#define PFX_U
#define ARITH_GE
#include "op_addsub.h"
#undef ARITH_GE
#undef PFX_U

#define PFX_SH
#include "op_addsub.h"
#undef PFX_SH

#define PFX_UH
#include "op_addsub.h"
#undef PFX_UH

static inline uint8_t do_usad(uint8_t a, uint8_t b)
{
    if(a > b) {
        return a - b;
    } else {
        return b - a;
    }
}

/* Unsigned sum of absolute byte differences.  */
uint32_t HELPER(usad8)(uint32_t a, uint32_t b)
{
    uint32_t sum;
    sum = do_usad(a, b);
    sum += do_usad(a >> 8, b >> 8);
    sum += do_usad(a >> 16, b >> 16);
    sum += do_usad(a >> 24, b >> 24);
    return sum;
}

/* For ARMv6 SEL instruction.  */
uint32_t HELPER(sel_flags)(uint32_t flags, uint32_t a, uint32_t b)
{
    uint32_t mask;

    mask = 0;
    if(flags & 1) {
        mask |= 0xff;
    }
    if(flags & 2) {
        mask |= 0xff00;
    }
    if(flags & 4) {
        mask |= 0xff0000;
    }
    if(flags & 8) {
        mask |= 0xff000000;
    }
    return (a & mask) | (b & ~mask);
}

uint32_t HELPER(logicq_cc)(uint64_t val)
{
    return (val >> 32) | (val != 0);
}

/* Convert ARM rounding mode to softfloat */
int arm_rmode_to_sf(enum arm_fpdecoderm rmode)
{
    int output_rmode;
    switch(rmode) {
        case FPROUNDING_TIEAWAY:
            output_rmode = float_round_ties_away;
            break;
        case FPROUNDING_TIEEVEN:
            output_rmode = float_round_nearest_even;
            break;
        case FPROUNDING_POSINF:
            output_rmode = float_round_up;
            break;
        case FPROUNDING_NEGINF:
            output_rmode = float_round_down;
            break;
        case FPROUNDING_ZERO:
            output_rmode = float_round_to_zero;
            break;
        default:
            tlib_abortf("ARM FP rounding: invalid rounding mode %d", rmode);
            __builtin_unreachable();
    }
    return output_rmode;
}

/* Set the current fp rounding mode and return the old one.
 * The argument is a softfloat float_round_ value.
 */
uint32_t HELPER(set_rmode)(uint32_t rmode, void *fpstp)
{
    float_status *fp_status = fpstp;

    uint32_t prev_rmode = get_float_rounding_mode(fp_status);
    set_float_rounding_mode(rmode, fp_status);

    return prev_rmode;
}

/* VFP support.  We follow the convention used for VFP instrunctions:
   Single precition routines have a "s" suffix, double precision a
   "d" suffix.  */

//  TODO: float16 HELPER(rinth_exact)(float16 x, void *fp_status) not supported by softfloat-2

float32 HELPER(rints_exact)(float32 x, void *fp_status)
{
    return float32_round_to_int(x, fp_status);
}

float64 HELPER(rintd_exact)(float64 x, void *fp_status)
{
    return float64_round_to_int(x, fp_status);
}

//  TODO: float16 HELPER(rinth)(float16 x, void *fp_status) not supported by softfloat-2

float32 HELPER(rints)(float32 x, void *fp_status)
{
    int old_flags = get_float_exception_flags(fp_status), new_flags;
    float32 ret = float32_round_to_int(x, fp_status);

    /* Suppress any inexact exceptions the conversion produced */
    if(!(old_flags & float_flag_inexact)) {
        new_flags = get_float_exception_flags(fp_status);
        set_float_exception_flags(new_flags & ~float_flag_inexact, fp_status);
    }

    return ret;
}

float64 HELPER(rintd)(float64 x, void *fp_status)
{
    int old_flags = get_float_exception_flags(fp_status);
    int new_flags = get_float_exception_flags(fp_status);
    float64 ret = float64_round_to_int(x, fp_status);

    /* Suppress any inexact exceptions the conversion produced */
    if(!(old_flags & float_flag_inexact)) {
        new_flags = get_float_exception_flags(fp_status);
        set_float_exception_flags(new_flags & ~float_flag_inexact, fp_status);
    }

    return ret;
}

/* Convert host exception flags to vfp form.  */
static inline int vfp_exceptbits_from_host(int host_bits)
{
    int target_bits = 0;

    if(host_bits & float_flag_invalid) {
        target_bits |= 1;
    }
    if(host_bits & float_flag_divbyzero) {
        target_bits |= 2;
    }
    if(host_bits & float_flag_overflow) {
        target_bits |= 4;
    }
    if(host_bits & (float_flag_underflow | float_flag_output_denormal)) {
        target_bits |= 8;
    }
    if(host_bits & float_flag_inexact) {
        target_bits |= 0x10;
    }
    if(host_bits & float_flag_input_denormal) {
        target_bits |= 0x80;
    }
    return target_bits;
}

uint32_t HELPER(vfp_get_fpscr)(CPUState *env)
{
    int i;
    uint32_t fpscr;

    fpscr = env->vfp.xregs[ARM_VFP_FPSCR];

    fpscr = FIELD_DP32(fpscr, VFP_FPSCR, QC, env->vfp.qc);

#ifndef TARGET_PROTO_ARM_M
    fpscr = FIELD_DP32(fpscr, VFP_FPSCR, VEC_STRIDE, env->vfp.vec_stride);
    fpscr = FIELD_DP32(fpscr, VFP_FPSCR, VEC_LEN, env->vfp.vec_len);
#else
    fpscr = FIELD_DP32(fpscr, VFP_FPSCR, LTPSIZE, env->v7m.ltpsize);
#endif  //  TARGET_PROTO_ARM_M
    i = get_float_exception_flags(&env->vfp.fp_status);
    i |= get_float_exception_flags(&env->vfp.standard_fp_status);
    fpscr |= vfp_exceptbits_from_host(i);
    return fpscr;
}

uint32_t vfp_get_fpscr(CPUState *env)
{
    return HELPER(vfp_get_fpscr)(env);
}

#ifdef TARGET_PROTO_ARM_M
void vfp_trigger_exception()
{
    /* Number of an NVIC interrupt that should be triggered when an fpu exception occures
     * On some platforms this line is not physically connected (eg. STM32H7 - errata ES0392 Rev 8,
     * 2.1.2 Cortex-M7 FPU interrupt not present on NVIC line 81), so a negative value means
     * don't trigger the interrupt
     */
    if(unlikely(cpu->vfp.fpu_interrupt_irq_number >= 0)) {
        /* This interrupt is an external interrupt. We add 16 to offset this number
         * and allow the user to pass IRQ numbers from the board's documentation
         */
        tlib_nvic_set_pending_irq(16 + cpu->vfp.fpu_interrupt_irq_number);
    }
}

uint32_t HELPER(vfp_get_vpr_p0)(CPUState *env)
{
    uint32_t p0;
    p0 = env->v7m.vpr & __REGISTER_V7M_VPR_P0_MASK;

    return p0;
}

void HELPER(vfp_set_vpr_p0)(CPUState *env, uint32_t val)
{
    env->v7m.vpr = val & __REGISTER_V7M_VPR_P0_MASK;
}

uint32_t HELPER(is_fpu_context_active)(CPUState *env)
{
    /* This does not check if we have access to FPU. Check for this needs to be done separately to calling this helper. */
    bool active = !need_fp_lazy_state_preservation(env);
    if(active) {
        active = !need_fp_context(env);
    }
    return active;
}
#endif

/* Convert vfp exception flags to target form.  */
static inline int vfp_exceptbits_to_host(int target_bits)
{
    int host_bits = 0;

    if(target_bits & 1) {
        host_bits |= float_flag_invalid;
    }
    if(target_bits & 2) {
        host_bits |= float_flag_divbyzero;
    }
    if(target_bits & 4) {
        host_bits |= float_flag_overflow;
    }
    if(target_bits & 8) {
        host_bits |= float_flag_underflow;
    }
    if(target_bits & 0x10) {
        host_bits |= float_flag_inexact;
    }
    if(target_bits & 0x80) {
        host_bits |= float_flag_input_denormal;
    }
    return host_bits;
}

void HELPER(vfp_set_fpscr)(CPUState *env, uint32_t val)
{
    int i;
    uint32_t changed;

    changed = env->vfp.xregs[ARM_VFP_FPSCR];
    env->vfp.xregs[ARM_VFP_FPSCR] = val;

#ifndef TARGET_PROTO_ARM_M
    env->vfp.vec_stride = FIELD_EX32(val, VFP_FPSCR, VEC_STRIDE);
    env->vfp.vec_len = FIELD_EX32(val, VFP_FPSCR, VEC_LEN);
#else
    env->v7m.ltpsize = FIELD_EX32(val, VFP_FPSCR, LTPSIZE);
#endif
    env->vfp.qc = FIELD_EX32(val, VFP_FPSCR, QC);

    changed ^= val;
    if(changed & (3 << 22)) {
        i = (val >> 22) & 3;
        switch(i) {
            case 0:
                i = float_round_nearest_even;
                break;
            case 1:
                i = float_round_up;
                break;
            case 2:
                i = float_round_down;
                break;
            case 3:
                i = float_round_to_zero;
                break;
        }
        set_float_rounding_mode(i, &env->vfp.fp_status);
    }
    if(changed & (1 << 24)) {
        set_flush_to_zero((val & (1 << 24)) != 0, &env->vfp.fp_status);
        set_flush_inputs_to_zero((val & (1 << 24)) != 0, &env->vfp.fp_status);
    }
    if(changed & (1 << 25)) {
        set_default_nan_mode((val & (1 << 25)) != 0, &env->vfp.fp_status);
    }

    i = vfp_exceptbits_to_host(val);
    set_float_exception_flags(i, &env->vfp.fp_status);
    set_float_exception_flags(0, &env->vfp.standard_fp_status);
}

void vfp_set_fpscr(CPUState *env, uint32_t val)
{
    HELPER(vfp_set_fpscr)(env, val);
}

#define VFP_HELPER(name, p) HELPER(glue(glue(vfp_, name), p))

#define VFP_BINOP(name)                                            \
    float32 VFP_HELPER(name, s)(float32 a, float32 b, void *fpstp) \
    {                                                              \
        float_status *fpst = fpstp;                                \
        return float32_##name(a, b, fpst);                         \
    }                                                              \
    float64 VFP_HELPER(name, d)(float64 a, float64 b, void *fpstp) \
    {                                                              \
        float_status *fpst = fpstp;                                \
        return float64_##name(a, b, fpst);                         \
    }
VFP_BINOP(add)
VFP_BINOP(sub)
VFP_BINOP(mul)
VFP_BINOP(div)
VFP_BINOP(maxnum)
VFP_BINOP(minnum)
#undef VFP_BINOP

float32 VFP_HELPER(neg, s)(float32 a)
{
    return float32_chs(a);
}

float64 VFP_HELPER(neg, d)(float64 a)
{
    return float64_chs(a);
}

float32 VFP_HELPER(abs, s)(float32 a)
{
    return float32_abs(a);
}

float64 VFP_HELPER(abs, d)(float64 a)
{
    return float64_abs(a);
}

float32 VFP_HELPER(sqrt, s)(float32 a, CPUState *env)
{
    return float32_sqrt(a, &env->vfp.fp_status);
}

float64 VFP_HELPER(sqrt, d)(float64 a, CPUState *env)
{
    return float64_sqrt(a, &env->vfp.fp_status);
}

/* XXX: check quiet/signaling case */
#define DO_VFP_cmp(p, type)                                                                           \
    void VFP_HELPER(cmp, p)(type a, type b, CPUState * env)                                           \
    {                                                                                                 \
        uint32_t flags;                                                                               \
        switch(type##_compare_quiet(a, b, &env->vfp.fp_status)) {                                     \
            case 0:                                                                                   \
                flags = 0x6;                                                                          \
                break;                                                                                \
            case -1:                                                                                  \
                flags = 0x8;                                                                          \
                break;                                                                                \
            case 1:                                                                                   \
                flags = 0x2;                                                                          \
                break;                                                                                \
            default:                                                                                  \
            case 2:                                                                                   \
                flags = 0x3;                                                                          \
                break;                                                                                \
        }                                                                                             \
        env->vfp.xregs[ARM_VFP_FPSCR] = (flags << 28) | (env->vfp.xregs[ARM_VFP_FPSCR] & 0x0fffffff); \
    }                                                                                                 \
    void VFP_HELPER(cmpe, p)(type a, type b, CPUState * env)                                          \
    {                                                                                                 \
        uint32_t flags;                                                                               \
        switch(type##_compare(a, b, &env->vfp.fp_status)) {                                           \
            case 0:                                                                                   \
                flags = 0x6;                                                                          \
                break;                                                                                \
            case -1:                                                                                  \
                flags = 0x8;                                                                          \
                break;                                                                                \
            case 1:                                                                                   \
                flags = 0x2;                                                                          \
                break;                                                                                \
            default:                                                                                  \
            case 2:                                                                                   \
                flags = 0x3;                                                                          \
                break;                                                                                \
        }                                                                                             \
        env->vfp.xregs[ARM_VFP_FPSCR] = (flags << 28) | (env->vfp.xregs[ARM_VFP_FPSCR] & 0x0fffffff); \
    }
DO_VFP_cmp(s, float32) DO_VFP_cmp(d, float64)
#undef DO_VFP_cmp

/* Integer to float and float to integer conversions */

#define CONV_ITOF(name, fsz, sign)                   \
    float##fsz HELPER(name)(uint32_t x, void *fpstp) \
    {                                                \
        float_status *fpst = fpstp;                  \
        return sign##int32_to_##float##fsz(x, fpst); \
    }

#define CONV_FTOI(name, fsz, sign, round)                     \
    uint32_t HELPER(name)(float##fsz x, void *fpstp)          \
    {                                                         \
        float_status *fpst = fpstp;                           \
        if(float##fsz##_is_any_nan(x)) {                      \
            float_raise(float_flag_invalid, fpst);            \
            return 0;                                         \
        }                                                     \
        return float##fsz##_to_##sign##int32##round(x, fpst); \
    }

#define FLOAT_CONVS(name, p, fsz, sign)     \
    CONV_ITOF(vfp_##name##to##p, fsz, sign) \
    CONV_FTOI(vfp_to##name##p, fsz, sign, ) \
    CONV_FTOI(vfp_to##name##z##p, fsz, sign, _round_to_zero)

    FLOAT_CONVS(si, s, 32, ) FLOAT_CONVS(si, d, 64, ) FLOAT_CONVS(ui, s, 32, u) FLOAT_CONVS(ui, d, 64, u)

#undef CONV_ITOF
#undef CONV_FTOI
#undef FLOAT_CONVS

    /* floating point conversion */
    float64 VFP_HELPER(fcvtd, s)(float32 x, CPUState *env)
{
    float64 r = float32_to_float64(x, &env->vfp.fp_status);
    /* ARM requires that S<->D conversion of any kind of NaN generates
     * a quiet NaN by forcing the most significant frac bit to 1.
     */
    return float64_maybe_silence_nan(r, &env->vfp.fp_status);
}

float32 VFP_HELPER(fcvts, d)(float64 x, CPUState *env)
{
    float32 r = float64_to_float32(x, &env->vfp.fp_status);
    /* ARM requires that S<->D conversion of any kind of NaN generates
     * a quiet NaN by forcing the most significant frac bit to 1.
     */
    return float32_maybe_silence_nan(r, &env->vfp.fp_status);
}

/* VFP3 fixed point conversion.  */
#define VFP_CONV_FIX(name, p, fsz, itype, sign)                                        \
    float##fsz HELPER(vfp_##name##to##p)(uint##fsz##_t x, uint32_t shift, void *fpstp) \
    {                                                                                  \
        float_status *fpst = fpstp;                                                    \
        float##fsz tmp;                                                                \
        tmp = sign##int32_to_##float##fsz((itype##_t)x, fpst);                         \
        return float##fsz##_scalbn(tmp, -(int)shift, fpst);                            \
    }                                                                                  \
    uint##fsz##_t HELPER(vfp_to##name##p)(float##fsz x, uint32_t shift, void *fpstp)   \
    {                                                                                  \
        float_status *fpst = fpstp;                                                    \
        float##fsz tmp;                                                                \
        if(float##fsz##_is_any_nan(x)) {                                               \
            float_raise(float_flag_invalid, fpst);                                     \
            return 0;                                                                  \
        }                                                                              \
        tmp = float##fsz##_scalbn(x, shift, fpst);                                     \
        return float##fsz##_to_##itype##_round_to_zero(tmp, fpst);                     \
    }

VFP_CONV_FIX(sh, d, 64, int16, )
VFP_CONV_FIX(sl, d, 64, int32, )
VFP_CONV_FIX(uh, d, 64, uint16, u)
VFP_CONV_FIX(ul, d, 64, uint32, u)
VFP_CONV_FIX(sh, s, 32, int16, )
VFP_CONV_FIX(sl, s, 32, int32, )
VFP_CONV_FIX(uh, s, 32, uint16, u)
VFP_CONV_FIX(ul, s, 32, uint32, u)
#undef VFP_CONV_FIX

/* Half precision conversions.  */
static float32 do_fcvt_f16_to_f32(uint32_t a, CPUState *env, float_status *s)
{
    int ieee = (env->vfp.xregs[ARM_VFP_FPSCR] & (1 << 26)) == 0;
    float32 r = float16_to_float32(make_float16(a), ieee, s);
    if(ieee) {
        return float32_maybe_silence_nan(r, s);
    }
    return r;
}

static uint32_t do_fcvt_f32_to_f16(float32 a, CPUState *env, float_status *s)
{
    int ieee = (env->vfp.xregs[ARM_VFP_FPSCR] & (1 << 26)) == 0;
    float16 r = float32_to_float16(a, ieee, s);
    if(ieee) {
        r = float16_maybe_silence_nan(r, s);
    }
    return float16_val(r);
}

float32 HELPER(neon_fcvt_f16_to_f32)(uint32_t a, CPUState *env)
{
    return do_fcvt_f16_to_f32(a, env, &env->vfp.standard_fp_status);
}

uint32_t HELPER(neon_fcvt_f32_to_f16)(float32 a, CPUState *env)
{
    return do_fcvt_f32_to_f16(a, env, &env->vfp.standard_fp_status);
}

float32 HELPER(vfp_fcvt_f16_to_f32)(uint32_t a, CPUState *env)
{
    return do_fcvt_f16_to_f32(a, env, &env->vfp.fp_status);
}

uint32_t HELPER(vfp_fcvt_f32_to_f16)(float32 a, CPUState *env)
{
    return do_fcvt_f32_to_f16(a, env, &env->vfp.fp_status);
}

float32 HELPER(recps_f32)(float32 a, float32 b, CPUState *env)
{
    float_status *s = &env->vfp.standard_fp_status;
    if((float32_is_infinity(a) && float32_is_zero_or_denormal(b)) || (float32_is_infinity(b) && float32_is_zero_or_denormal(a))) {
        if(!(float32_is_zero(a) || float32_is_zero(b))) {
            float_raise(float_flag_input_denormal, s);
        }
        return float32_two;
    }
    return float32_sub(float32_two, float32_mul(a, b, s), s);
}

float32 HELPER(rsqrts_f32)(float32 a, float32 b, CPUState *env)
{
    float_status *s = &env->vfp.standard_fp_status;
    float32 product;
    if((float32_is_infinity(a) && float32_is_zero_or_denormal(b)) || (float32_is_infinity(b) && float32_is_zero_or_denormal(a))) {
        if(!(float32_is_zero(a) || float32_is_zero(b))) {
            float_raise(float_flag_input_denormal, s);
        }
        return float32_one_point_five;
    }
    product = float32_mul(a, b, s);
    return float32_div(float32_sub(float32_three, product, s), float32_two, s);
}

/* NEON helpers.  */

/* Constants 256 and 512 are used in some helpers; we avoid relying on
 * int->float conversions at run-time.  */
#define float64_256 make_float64(0x4070000000000000LL)
#define float64_512 make_float64(0x4080000000000000LL)

/* The algorithm that must be used to calculate the estimate
 * is specified by the ARM ARM.
 */
static float64 recip_estimate(float64 a, CPUState *env)
{
    /* These calculations mustn't set any fp exception flags,
     * so we use a local copy of the fp_status.
     */
    float_status dummy_status = env->vfp.standard_fp_status;
    float_status *s = &dummy_status;
    /* q = (int)(a * 512.0) */
    float64 q = float64_mul(float64_512, a, s);
    int64_t q_int = float64_to_int64_round_to_zero(q, s);

    /* r = 1.0 / (((double)q + 0.5) / 512.0) */
    q = int64_to_float64(q_int, s);
    q = float64_add(q, float64_half, s);
    q = float64_div(q, float64_512, s);
    q = float64_div(float64_one, q, s);

    /* s = (int)(256.0 * r + 0.5) */
    q = float64_mul(q, float64_256, s);
    q = float64_add(q, float64_half, s);
    q_int = float64_to_int64_round_to_zero(q, s);

    /* return (double)s / 256.0 */
    return float64_div(int64_to_float64(q_int, s), float64_256, s);
}

float32 HELPER(recpe_f32)(float32 a, CPUState *env)
{
    float_status *s = &env->vfp.standard_fp_status;
    float64 f64;
    uint32_t val32 = float32_val(a);

    int result_exp;
    int a_exp = (val32 & 0x7f800000) >> 23;
    int sign = val32 & 0x80000000;

    if(float32_is_any_nan(a)) {
        if(float32_is_signaling_nan(a, s)) {
            float_raise(float_flag_invalid, s);
        }
        return float32_default_nan;
    } else if(float32_is_infinity(a)) {
        return float32_set_sign(float32_zero, float32_is_neg(a));
    } else if(float32_is_zero_or_denormal(a)) {
        if(!float32_is_zero(a)) {
            float_raise(float_flag_input_denormal, s);
        }
        float_raise(float_flag_divbyzero, s);
        return float32_set_sign(float32_infinity, float32_is_neg(a));
    } else if(a_exp >= 253) {
        float_raise(float_flag_underflow, s);
        return float32_set_sign(float32_zero, float32_is_neg(a));
    }

    f64 = make_float64((0x3feULL << 52) | ((int64_t)(val32 & 0x7fffff) << 29));

    result_exp = 253 - a_exp;

    f64 = recip_estimate(f64, env);

    val32 = sign | ((result_exp & 0xff) << 23) | ((float64_val(f64) >> 29) & 0x7fffff);
    return make_float32(val32);
}

/* The algorithm that must be used to calculate the estimate
 * is specified by the ARM ARM.
 */
static float64 recip_sqrt_estimate(float64 a, CPUState *env)
{
    /* These calculations mustn't set any fp exception flags,
     * so we use a local copy of the fp_status.
     */
    float_status dummy_status = env->vfp.standard_fp_status;
    float_status *s = &dummy_status;
    float64 q;
    int64_t q_int;

    if(float64_lt(a, float64_half, s)) {
        /* range 0.25 <= a < 0.5 */

        /* a in units of 1/512 rounded down */
        /* q0 = (int)(a * 512.0);  */
        q = float64_mul(float64_512, a, s);
        q_int = float64_to_int64_round_to_zero(q, s);

        /* reciprocal root r */
        /* r = 1.0 / sqrt(((double)q0 + 0.5) / 512.0);  */
        q = int64_to_float64(q_int, s);
        q = float64_add(q, float64_half, s);
        q = float64_div(q, float64_512, s);
        q = float64_sqrt(q, s);
        q = float64_div(float64_one, q, s);
    } else {
        /* range 0.5 <= a < 1.0 */

        /* a in units of 1/256 rounded down */
        /* q1 = (int)(a * 256.0); */
        q = float64_mul(float64_256, a, s);
        int64_t q_int = float64_to_int64_round_to_zero(q, s);

        /* reciprocal root r */
        /* r = 1.0 /sqrt(((double)q1 + 0.5) / 256); */
        q = int64_to_float64(q_int, s);
        q = float64_add(q, float64_half, s);
        q = float64_div(q, float64_256, s);
        q = float64_sqrt(q, s);
        q = float64_div(float64_one, q, s);
    }
    /* r in units of 1/256 rounded to nearest */
    /* s = (int)(256.0 * r + 0.5); */

    q = float64_mul(q, float64_256, s);
    q = float64_add(q, float64_half, s);
    q_int = float64_to_int64_round_to_zero(q, s);

    /* return (double)s / 256.0;*/
    return float64_div(int64_to_float64(q_int, s), float64_256, s);
}

float32 HELPER(rsqrte_f32)(float32 a, CPUState *env)
{
    float_status *s = &env->vfp.standard_fp_status;
    int result_exp;
    float64 f64;
    uint32_t val;
    uint64_t val64;

    val = float32_val(a);

    if(float32_is_any_nan(a)) {
        if(float32_is_signaling_nan(a, s)) {
            float_raise(float_flag_invalid, s);
        }
        return float32_default_nan;
    } else if(float32_is_zero_or_denormal(a)) {
        if(!float32_is_zero(a)) {
            float_raise(float_flag_input_denormal, s);
        }
        float_raise(float_flag_divbyzero, s);
        return float32_set_sign(float32_infinity, float32_is_neg(a));
    } else if(float32_is_neg(a)) {
        float_raise(float_flag_invalid, s);
        return float32_default_nan;
    } else if(float32_is_infinity(a)) {
        return float32_zero;
    }

    /* Normalize to a double-precision value between 0.25 and 1.0,
     * preserving the parity of the exponent.  */
    if((val & 0x800000) == 0) {
        f64 = make_float64(((uint64_t)(val & 0x80000000) << 32) | (0x3feULL << 52) | ((uint64_t)(val & 0x7fffff) << 29));
    } else {
        f64 = make_float64(((uint64_t)(val & 0x80000000) << 32) | (0x3fdULL << 52) | ((uint64_t)(val & 0x7fffff) << 29));
    }

    result_exp = (380 - ((val & 0x7f800000) >> 23)) / 2;

    f64 = recip_sqrt_estimate(f64, env);

    val64 = float64_val(f64);

    val = ((result_exp & 0xff) << 23) | ((val64 >> 29) & 0x7fffff);
    return make_float32(val);
}

uint32_t HELPER(recpe_u32)(uint32_t a, CPUState *env)
{
    float64 f64;

    if((a & 0x80000000) == 0) {
        return 0xffffffff;
    }

    f64 = make_float64((0x3feULL << 52) | ((int64_t)(a & 0x7fffffff) << 21));

    f64 = recip_estimate(f64, env);

    return 0x80000000 | ((float64_val(f64) >> 21) & 0x7fffffff);
}

uint32_t HELPER(rsqrte_u32)(uint32_t a, CPUState *env)
{
    float64 f64;

    if((a & 0xc0000000) == 0) {
        return 0xffffffff;
    }

    if(a & 0x80000000) {
        f64 = make_float64((0x3feULL << 52) | ((uint64_t)(a & 0x7fffffff) << 21));
    } else { /* bits 31-30 == '01' */
        f64 = make_float64((0x3fdULL << 52) | ((uint64_t)(a & 0x3fffffff) << 22));
    }

    f64 = recip_sqrt_estimate(f64, env);

    return 0x80000000 | ((float64_val(f64) >> 21) & 0x7fffffff);
}

/* VFPv4 fused multiply-accumulate */
float32 VFP_HELPER(muladd, s)(float32 a, float32 b, float32 c, void *fpstp)
{
    float_status *fpst = fpstp;
    return float32_muladd(a, b, c, 0, fpst);
}

float64 VFP_HELPER(muladd, d)(float64 a, float64 b, float64 c, void *fpstp)
{
    float_status *fpst = fpstp;
    return float64_muladd(a, b, c, 0, fpst);
}

void HELPER(set_teecr)(CPUState *env, uint32_t val)
{
    val &= 1;
    if(env->teecr != val) {
        env->teecr = val;
        tb_flush(env);
    }
}

#ifdef TARGET_PROTO_ARM_M
uint32_t HELPER(v8m_tt)(CPUState *env, uint32_t addr, uint32_t op)
{
    int prot;
    bool a, t;
    int mpu_region;
    enum security_attribution attribution;
    bool test_privileged;
    bool test_secure;

    //  Based on TT_RESP from the ARMv8-M Architecture Reference Manual.
    union {
        struct {
#if HOST_WORDS_BIGENDIAN
            unsigned idau_region : 8;
            unsigned idau_region_valid : 1;
            unsigned target_secure : 1;
            unsigned nonsecure_readwrite_ok : 1;
            unsigned nonsecure_read_ok : 1;
            unsigned readwrite_ok : 1;
            unsigned read_ok : 1;
            unsigned sau_region_valid : 1;
            unsigned mpu_region_valid : 1;
            unsigned sau_region : 8;
            unsigned mpu_region : 8;
#else
            unsigned mpu_region : 8;
            unsigned sau_region : 8;
            unsigned mpu_region_valid : 1;
            unsigned sau_region_valid : 1;
            unsigned read_ok : 1;
            unsigned readwrite_ok : 1;
            unsigned nonsecure_read_ok : 1;
            unsigned nonsecure_readwrite_ok : 1;
            unsigned target_secure : 1;
            unsigned idau_region_valid : 1;
            unsigned idau_region : 8;
#endif
        } flags;
        uint32_t value;
    } addr_info = { .value = 0 };

    /* Decode instruction variant
     * TT:    a == 0 && t == 0
     * TTA:   a == 1 && t == 0
     * TTT:   a == 0 && t == 1
     * TTAT:  a == 1 && t == 1
     */
    a = op & 0b10;
    t = op & 0b01;

    /* Alternate Domain (A) variants are used to query the Security state and access permissions
     * of a memory location for a Non-secure access to that location. This helper is only called
     * for secure TTA and TTAT execution as they are UNDEFINED if used from non-secure state.
     */
    test_secure = a ? false : env->secure;

    /* The Arm® v8-M Architecture Reference Manual specifies that MREGION content is not valid if:
     * 1) The TT or TTT instruction variants, without the A flag specified, were executed from an unprivileged mode,
     * 2) The MPU is not implemented or MPU_CTRL.ENABLE is set to zero,
     * 3) The address specified by the TT instruction variant does not match any enabled MPU regions,
     * 4) The address matched multiple MPU regions.
     * R and RW fields are RAZ in cases 1 and 4; so with 1) we don't even need to check the access.
     */
    if(in_privileged_mode(env) || a) {
        /* T-variant instructions, i.e. TTT and TTAT, are Test Target (Alternate Mode) UNPRIVILEGED
         * so they always query access permissions for an unprivileged access to that location.
         */
        test_privileged = t ? false : in_privileged_mode(env);

        /* We're testing `ACCESS_DATA_LOAD` so the translate success means reading is allowed.
         * Store doesn't need to be tested as we can just check `prot` set by the function.
         */
        int result = pmsav8_check_access_with_region(env, addr, test_secure, ACCESS_DATA_LOAD,
                                                     /* is_user: */ !test_privileged, &prot, /* page_size: */ NULL, &mpu_region);

        if((in_privileged_mode(env) || a) && pmsav8_mpu_region_valid(mpu_region)) {
            addr_info.flags.mpu_region = mpu_region;
            addr_info.flags.mpu_region_valid = true;
        }

        /* `pmsav8_check_access_with_region` always returns `TRANSLATE_FAIL` in case multiple regions were matched
         * and `prot` has no permissions so we don't really need to know if it happened as both should be RAZ.
         */
        addr_info.flags.read_ok = result == TRANSLATE_SUCCESS;
        addr_info.flags.readwrite_ok = is_page_access_valid(prot, ACCESS_DATA_STORE) ? 1u : 0u;
    }

    /* The remaining bits are only valid if executed from secure state. */
    if(!env->secure) {
        return addr_info.value;
    }

    bool idau_valid, sau_valid;
    int idau_region, sau_region;
    pmsav8_get_security_attribution(env, addr, test_secure, ACCESS_DATA_LOAD, /* access_width: */ 1, &idau_valid, &idau_region,
                                    &sau_valid, &sau_region, &attribution, /* applies_to_whole_page: */ NULL);

    if(idau_valid) {
        addr_info.flags.idau_region_valid = true;
        addr_info.flags.idau_region = idau_region;
    }

    if(sau_valid) {
        addr_info.flags.sau_region_valid = true;
        addr_info.flags.sau_region = sau_region;
    }
    addr_info.flags.target_secure = attribution_is_secure(attribution);

    /* NSR and NSRW bits are only valid if R/RW fields are valid. */
    addr_info.flags.nonsecure_read_ok = !addr_info.flags.target_secure && addr_info.flags.read_ok;
    addr_info.flags.nonsecure_readwrite_ok = !addr_info.flags.target_secure && addr_info.flags.readwrite_ok;

    return addr_info.value;
}

void HELPER(v8m_blxns)(CPUState *env, uint32_t addr, uint32_t link)
{
    /* UNDEF should be generated in Non-secure mode */
    tlib_assert(env->secure);

    tlib_printf(LOG_LEVEL_NOISY, "B%sXNS jump at 0x%x to 0x%x", link ? "L" : "", env->regs[15], addr);

    if(!link && addr >= ARM_M_FNC_RETURN_MIN) {
        //  FNC_RETURN or EXC_RETURN is in the register, continue without clearing the lowest bit to process it later
        return;
    }

    /* Only switch to Non-Secure if bit[0] of target addr is 0 */
    if((addr & 1) == 0) {
        if(link) {
            /* According to docs "some processor state information" is pushed here
             * the ARM pseudocode specifies exactly:
             * """
             *   EPSR.B is not preserved on the stack
             *   savedPSR.Exception = IPSR.Exception;
             *   savedPSR.SFPA = CONTROL_S.SFPA;
             * """
             * RGVB: The IPSR is stacked in the partial RETPSR, and CONTROL.SFPA is stacked in bit [20] of the partial RETPSR. */
            uint32_t partialRETPSR = env->v7m.exception;
            partialRETPSR |= extract32(env->v7m.control[M_REG_COMMON], ARM_CONTROL_SFPA, 1) > 0 ? RETPSR_SFPA : 0;
            v7m_push(env, partialRETPSR);
            /* Push the return address last so it is at offset 0 of the
             * function return stack frame, below RETPSR at offset 4. */
            v7m_push(env, env->regs[15] | 1);
        }

        env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_SFPA_MASK;
        /* Now we switch stacks and jump to non-secure mode */
        switch_v7m_security_state(env, false);
        if(link) {
            env->regs[14] = FNC_RETURN;
            /* If in handler mode, we should set exception number to invalid but non-zero value
             * to prevent information leakage */
            if(in_handler_mode(env)) {
                env->v7m.exception = ARMV7M_EXCP_RESET;
            }
        }
    } else if(link) {
        /* Act like BLX */
        env->regs[14] = env->regs[15] | 1;
    }
    /* As in BX/BLX we need to clear bit[0] of address */
    env->regs[15] = addr & ~1;
}

void HELPER(v8m_sg)(CPUState *env)
{
    /* Using PC is fine, since we just synced it in "translate"
     * but we need to subtract, since we point at the next instruction */
    const uint32_t sg_pc = env->regs[15] - 2;
    if(env->secure) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "SG instruction at address: 0x%" PRIx32 " is executed in Secure state, and will be treated as NOP", sg_pc);
        return;
    }

    bool idau_valid, sau_valid;
    int idau_region, sau_region;
    enum security_attribution attribution;
    pmsav8_get_security_attribution(env, sg_pc, env->secure, ACCESS_INST_FETCH, /* access_width */ 1, &idau_valid, &idau_region,
                                    &sau_valid, &sau_region, &attribution, /* applies_to_whole_page: */ NULL);

    if(attribution != SA_SECURE_NSC) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "SG instruction at address: 0x%" PRIx32 " is not in Non-secure Callable region, and will be treated as NOP",
                    sg_pc);
        return;
    }

    uint32_t *sp_secure;
    if(is_using_process_sp(env, false, true)) {
        sp_secure = &env->v7m.other_ss_psp;
    } else {
        sp_secure = &env->v7m.other_ss_msp;
    }

    /* We're only checking if the read from this address is valid */
    uint32_t phys_ptr = 0;
    target_ulong page_size = 0;
    int prot = 0;
    if(get_phys_addr(env, *sp_secure, true, ACCESS_DATA_LOAD, !in_privileged_mode_with_security(env, true), &phys_ptr, &prot,
                     &page_size, false) == TRANSLATE_FAIL) {
        cpu_loop_exit(env);
    }

    if(env->v7m.ccr[M_REG_COMMON] & FIELD_MASK(V7M_CCR, TRD)) {
        uint32_t sp_data = ldl_phys(phys_ptr);
        if((sp_data >> 1) == (INTEGRITY_SIGN >> 1) || sp_secure == &env->v7m.other_ss_msp) {
            env->v7m.secure_fault_status |= SECURE_FAULT_INVEP;
            env->exception_index = EXCP_SECURE;
            cpu_loop_exit(env);
        }
    }

    /* Clear bit[0] of LR to indicate we will return to Non-Secure mode, if we were previously in Non-Secure state */
    env->regs[14] &= ~1;
    switch_v7m_security_state(env, true);
    env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_SFPA_MASK;
    tlib_printf(LOG_LEVEL_NOISY, "Executed SG at 0x%" PRIx32, sg_pc);
}

void HELPER(v8m_bx_update_pc)(CPUState *env, uint32_t pc)
{
    /* is not EXC_RETURN */
    if(pc < ARM_M_EXC_RETURN_MIN) {
        if((pc & 1) == 0) {
            bool raise_secure_exception = false;

            if(env->v7m.has_trustzone && !env->secure) {
                bool idau_valid, sau_valid;
                int idau_region, sau_region;
                enum security_attribution attribution;
                pmsav8_get_security_attribution(env, pc, env->secure, ACCESS_INST_FETCH, /* access_width */ 1, &idau_valid,
                                                &idau_region, &sau_valid, &sau_region, &attribution,
                                                /* applies_to_whole_page: */ NULL);
                if(attribution != SA_NONSECURE) {
                    raise_secure_exception = true;
                }
            }

            if(raise_secure_exception) {
                env->v7m.secure_fault_status |= SECURE_FAULT_INVEP;
                env->exception_index = EXCP_SECURE;
            } else {
                env->exception_index = EXCP_INVSTATE;
            }
            cpu_loop_exit(env);
        }
        pc &= ~1;
    }
    /* For EXC_RETURN, we interrupt the block in translate.c, so we will next do `do_v7m_exception_exit`
     * For FNC_RETURN there is `do_v7m_secure_return`, and low bit will be cleared, but this is fine, see this comment from the
     * manual:
     * """
     * Because FNC_RETURN is only used when calling from the Secure state, this bit is always set to 1. However, some function
     * chaining cases can result in an SG instruction clearing this bit, so the architecture ignores the state of this bit when
     * processing a branch to FNC_RETURN.
     * """
     * So we just don't care if it's cleared, as it can happen anyway out of our control in SG */
    env->regs[15] = pc;
}

static inline bool vlstm_store_helper(CPUState *env, uint32_t *address, uint64_t val)
{
    uint32_t phys_ptr = 0;
    target_ulong page_size = 0;
    int prot = 0;

    int ret = get_phys_addr(env, *address, env->secure, ACCESS_DATA_STORE, !in_privileged_mode(env), &phys_ptr, &prot, &page_size,
                            false);
    if(ret == TRANSLATE_SUCCESS) {
        stq_phys(*address, val);
        *address += sizeof(val);
        return true;
    } else {
        return false;
    }
}

static inline bool vlldm_load_helper(CPUState *env, uint32_t *address, uint64_t *val)
{
    uint32_t phys_ptr = 0;
    target_ulong page_size = 0;
    int prot = 0;

    int ret = get_phys_addr(env, *address, env->secure, ACCESS_DATA_LOAD, !in_privileged_mode(env), &phys_ptr, &prot, &page_size,
                            false);
    if(ret == TRANSLATE_SUCCESS) {
        *val = ldq_phys(*address);
        *address += sizeof(*val);
        return true;
    } else {
        return false;
    }
}

void HELPER(v8m_vlstm)(CPUState *env, uint32_t address)
{
    /* Instruction is UNDEF in Non-secure state - helper should not be called */
    tlib_assert(env->secure);

    /* This is a Thumb2 instruction, PC needs to be subtracted, since it points to next half of insn
     * the PC was synced before calling this helper */
    const uint32_t insn_pc = env->regs[15] - 2;

    /* The S bit determines who claimed FPU registers - Secure or Non-secure world */
    if((env->v7m.fpccr[(env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_S_MASK) > 0 ? M_REG_S : M_REG_NS] & ARM_FPCCR_LSPACT) > 0) {
        /* The HW raises exception here, as it's a possible attack scenario */
        env->v7m.secure_fault_address = insn_pc;
        env->v7m.secure_fault_status |= SECURE_FAULT_LSERR | SECURE_FAULT_SFARVALID;
        env->exception_index = EXCP_SECURE;
        cpu_loop_exit_restore(env, insn_pc, true);
    }

    if((env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_LSPEN_MASK) > 0) {
        /* If Lazy preservation is enabled, just update the FPCCR and FPCAR
         * Low three bits are RES0 */
        fpccr_update(env, address, 0);
    } else {
        /* We store, in this order, the following FPU registers, at the address passed in register "rn":
         *  S[0]-S[15]
         *  FPSCR
         *  VPR
         *  S[16]-S[31] */
        bool any_failed = false;
        for(int i = 0; i < 8; ++i) {
            any_failed |= !vlstm_store_helper(env, &address, env->vfp.regs[i]);
        }

        /* Pack fpscr and vpr into single 64 bit long value */
        uint64_t fpscr_vpr;
        if(arm_feature(env, ARM_FEATURE_MVE)) {
            fpscr_vpr = env->v7m.vpr;
        } else {
            /* Write UNKNOWN if MVE is not implemented */
            fpscr_vpr = 0xBADCAFFE;
        }
        fpscr_vpr = (fpscr_vpr << 32) | vfp_get_fpscr(env);
        any_failed |= !vlstm_store_helper(env, &address, fpscr_vpr);

        bool push_callee_frame = (env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) > 0;
        if(push_callee_frame) {
            for(int i = 8; i < 16; ++i) {
                any_failed |= !vlstm_store_helper(env, &address, env->vfp.regs[i]);
            }
        }

        invalidate_vfp_regs(env, push_callee_frame, push_callee_frame);

        if(any_failed) {
            env->v7m.secure_fault_address = insn_pc;
            env->v7m.secure_fault_status |= SECURE_FAULT_AUVIOL | SECURE_FAULT_SFARVALID;
            env->exception_index = EXCP_SECURE;
            cpu_loop_exit_restore(env, insn_pc, true);
        }
    }
    env->v7m.control[M_REG_COMMON] &= ~ARM_CONTROL_FPCA_MASK;
}

void HELPER(v8m_vlldm)(CPUState *env, uint32_t address)
{
    /* Instruction is UNDEF in Non-secure state - helper should not be called */
    tlib_assert(env->secure);

    /* Do writes and reads directly on FPCCR is risky, but we know what we are doing */
    if((env->v7m.fpccr[M_REG_S] & ARM_FPCCR_LSPACT_MASK) > 0) {
        /* The state is still active, doesn't need to be restored. So do nothing at all */
        env->v7m.fpccr[M_REG_S] &= ~ARM_FPCCR_LSPACT_MASK;
    } else {
        uint64_t scratch = 0;
        bool any_failed = false;
        for(int i = 0; i < 8; ++i) {
            any_failed |= !vlldm_load_helper(env, &address, &scratch);
            env->vfp.regs[i] = scratch;
        }
        /* Unpack fpscr and vpr from one 64-bit value */
        any_failed |= !vlldm_load_helper(env, &address, &scratch);
        vfp_set_fpscr(env, extract64(scratch, 0, 32));
        env->v7m.vpr = extract64(scratch, 32, 32);
        bool pop_callee_frame = (env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_TS_MASK) > 0;
        if(pop_callee_frame) {
            for(int i = 8; i < 16; ++i) {
                any_failed |= !vlldm_load_helper(env, &address, &scratch);
                env->vfp.regs[i] = scratch;
            }
        }

        if(any_failed) {
            /* This is Thumb2 instruction, PC needs to be subtracted, since it'll point to next half of insn */
            invalidate_vfp_regs(env, true, pop_callee_frame);
            const uint32_t insn_pc = env->regs[15] - 2;
            env->v7m.secure_fault_address = insn_pc;
            env->v7m.secure_fault_status |= SECURE_FAULT_AUVIOL | SECURE_FAULT_SFARVALID;
            env->exception_index = EXCP_SECURE;
            cpu_loop_exit_restore(env, insn_pc, true);
        }
    }
    env->v7m.control[M_REG_COMMON] |= ARM_CONTROL_FPCA_MASK;
}

#endif

void tlib_arch_dispose()
{
    ttable_remove(cpu->cp_regs);
}

void HELPER(set_system_event)(void)
{
    tlib_set_system_event();
}

void cpu_before_cycles_per_instruction_change(CPUState *env)
{
    pmu_recalculate_all_lazy();
}

void cpu_after_cycles_per_instruction_change(CPUState *env)
{
    pmu_take_all_snapshots();
    pmu_recalculate_cycles_instruction_limit();
}
