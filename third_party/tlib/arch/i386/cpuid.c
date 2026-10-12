/*
 *  i386 CPUID helper functions
 *
 *  Copyright (c) 2003 Fabrice Bellard
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <inttypes.h>
#include <ctype.h>
#include <math.h>

#include "cpu.h"
#include "infrastructure.h"

/* feature flags taken from "Intel Processor Identification and the CPUID
 * Instruction" and AMD's "CPUID Specification".  In cases of disagreement
 * between feature naming conventions, aliases may be added.
 */
static const char *feature_name[] = {
    "fpu",
    "vme",
    "de",
    "pse",
    "tsc",
    "msr",
    "pae",
    "mce",
    "cx8",
    "apic",
    NULL,
    "sep",
    "mtrr",
    "pge",
    "mca",
    "cmov",
    "pat",
    "pse36",
    "pn" /* Intel psn */,
    "clflush" /* Intel clfsh */,
    NULL,
    "ds" /* Intel dts */,
    "acpi",
    "mmx",
    "fxsr",
    "sse",
    "sse2",
    "ss",
    "ht" /* Intel htt */,
    "tm",
    "ia64",
    "pbe",
};
static const char *ext_feature_name[] = {
    "pni|sse3" /* Intel,AMD sse3 */,
    "pclmuldq",
    "dtes64",
    "monitor",
    "ds_cpl",
    "vmx",
    "smx",
    "est",
    "tm2",
    "ssse3",
    "cid",
    NULL,
    "fma",
    "cx16",
    "xtpr",
    "pdcm",
    NULL,
    NULL,
    "dca",
    "sse4.1|sse4_1",
    "sse4.2|sse4_2",
    "x2apic",
    "movbe",
    "popcnt",
    NULL,
    "aes",
    "xsave",
    "osxsave",
    "avx",
    NULL,
    NULL,
    "hypervisor",
};
static const char *ext2_feature_name[] = {
    "fpu",
    "vme",
    "de",
    "pse",
    "tsc",
    "msr",
    "pae",
    "mce",
    "cx8" /* AMD CMPXCHG8B */,
    "apic",
    NULL,
    "syscall",
    "mtrr",
    "pge",
    "mca",
    "cmov",
    "pat",
    "pse36",
    NULL,
    NULL /* Linux mp */,
    "nx" /* Intel xd */,
    NULL,
    "mmxext",
    "mmx",
    "fxsr",
    "fxsr_opt" /* AMD ffxsr */,
    "pdpe1gb" /* AMD Page1GB */,
    "rdtscp",
    NULL,
    "lm" /* Intel 64 */,
    "3dnowext",
    "3dnow",
};
static const char *ext3_feature_name[] = {
    "lahf_lm" /* AMD LahfSahf */,
    "cmp_legacy",
    "svm",
    "extapic" /* AMD ExtApicSpace */,
    "cr8legacy" /* AMD AltMovCr8 */,
    "abm",
    "sse4a",
    "misalignsse",
    "3dnowprefetch",
    "osvw",
    "ibs",
    "xop",
    "skinit",
    "wdt",
    NULL,
    NULL,
    "fma4",
    NULL,
    "cvt16",
    "nodeid_msr",
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
};

static const char *svm_feature_name[] = {
    "npt", "lbrv", "svm_lock",     "nrip_save", "tsc_scale",   "vmcb_clean", "flushbyasid", "decodeassists",
    NULL,  NULL,   "pause_filter", NULL,        "pfthreshold", NULL,         NULL,          NULL,
    NULL,  NULL,   NULL,           NULL,        NULL,          NULL,         NULL,          NULL,
    NULL,  NULL,   NULL,           NULL,        NULL,          NULL,         NULL,          NULL,
};

/* collects per-function cpuid data
 */
typedef struct model_features_t {
    uint32_t *guest_feat;
    uint32_t *host_feat;
    uint32_t check_feat;
    const char **flag_names;
    uint32_t cpuid;
} model_features_t;

#define iswhite(c) ((c) && ((c) <= ' ' || '~' < (c)))

/* general substring compare of *[s1..e1) and *[s2..e2).  sx is start of
 * a substring.  ex if !NULL points to the first char after a substring,
 * otherwise the string is assumed to sized by a terminating nul.
 * Return lexical ordering of *s1:*s2.
 */
static int sstrcmp(const char *s1, const char *e1, const char *s2, const char *e2)
{
    for(;;) {
        if(!*s1 || !*s2 || *s1 != *s2) {
            return (*s1 - *s2);
        }
        ++s1, ++s2;
        if(s1 == e1 && s2 == e2) {
            return (0);
        } else if(s1 == e1) {
            return (*s2);
        } else if(s2 == e2) {
            return (*s1);
        }
    }
}

/* compare *[s..e) to *altstr.  *altstr may be a simple string or multiple
 * '|' delimited (possibly empty) strings in which case search for a match
 * within the alternatives proceeds left to right.  Return 0 for success,
 * non-zero otherwise.
 */
static int altcmp(const char *s, const char *e, const char *altstr)
{
    const char *p, *q;

    for(q = p = altstr;;) {
        while(*p && *p != '|') {
            ++p;
        }
        if((q == p && !*s) || (q != p && !sstrcmp(s, e, q, p))) {
            return (0);
        }
        if(!*p) {
            return (1);
        } else {
            q = ++p;
        }
    }
}

/* search featureset for flag *[s..e), if found set corresponding bit in
 * *pval and return true, otherwise return false
 */
static bool lookup_feature(uint32_t *pval, const char *s, const char *e, const char **featureset)
{
    uint32_t mask;
    const char **ppc;
    bool found = false;

    for(mask = 1, ppc = featureset; mask; mask <<= 1, ++ppc) {
        if(*ppc && !altcmp(s, e, *ppc)) {
            *pval |= mask;
            found = true;
        }
    }
    return found;
}

static void add_flagname_to_bitmaps(const char *flagname, uint32_t *features, uint32_t *ext_features, uint32_t *ext2_features,
                                    uint32_t *ext3_features, uint32_t *svm_features)
{
    if(!lookup_feature(features, flagname, NULL, feature_name) &&
       !lookup_feature(ext_features, flagname, NULL, ext_feature_name) &&
       !lookup_feature(ext2_features, flagname, NULL, ext2_feature_name) &&
       !lookup_feature(ext3_features, flagname, NULL, ext3_feature_name) &&
       !lookup_feature(svm_features, flagname, NULL, svm_feature_name)) {
        tlib_printf(LOG_LEVEL_ERROR, "CPU feature %s not found\n", flagname);
    }
}

typedef struct x86_def_t {
    struct x86_def_t *next;
    const char *name;
    uint32_t level;
    uint32_t vendor1, vendor2, vendor3;
    int family;
    int model;
    int stepping;
    int tsc_khz;
    uint32_t features, ext_features, ext2_features, ext3_features;
    uint32_t svm_features;
    uint32_t xlevel;
    char model_id[48];
    int vendor_override;
    uint32_t flags;
    /* Store the results of Centaur's CPUID instructions */
    uint32_t ext4_features;
    uint32_t xlevel2;
} x86_def_t;

#define I486_FEATURES    (CPUID_FP87 | CPUID_VME | CPUID_PSE)
#define PENTIUM_FEATURES (I486_FEATURES | CPUID_DE | CPUID_TSC | CPUID_MSR | CPUID_MCE | CPUID_CX8 | CPUID_MMX | CPUID_APIC)
#define PENTIUM2_FEATURES                                                                                                   \
    (PENTIUM_FEATURES | CPUID_PAE | CPUID_SEP | CPUID_MTRR | CPUID_PGE | CPUID_MCA | CPUID_CMOV | CPUID_PAT | CPUID_PSE36 | \
     CPUID_FXSR)
#define PENTIUM3_FEATURES (PENTIUM2_FEATURES | CPUID_SSE)
#define PPRO_FEATURES                                                                                                         \
    (CPUID_FP87 | CPUID_DE | CPUID_PSE | CPUID_TSC | CPUID_MSR | CPUID_MCE | CPUID_CX8 | CPUID_PGE | CPUID_CMOV | CPUID_PAT | \
     CPUID_FXSR | CPUID_MMX | CPUID_SSE | CPUID_SSE2 | CPUID_PAE | CPUID_SEP | CPUID_APIC)
#define EXT2_FEATURE_MASK 0x0183F3FF

#define TCG_FEATURES                                                                                                            \
    (CPUID_FP87 | CPUID_PSE | CPUID_TSC | CPUID_MSR | CPUID_PAE | CPUID_MCE | CPUID_CX8 | CPUID_APIC | CPUID_SEP | CPUID_MTRR | \
     CPUID_PGE | CPUID_MCA | CPUID_CMOV | CPUID_PAT | CPUID_PSE36 | CPUID_CLFLUSH | CPUID_ACPI | CPUID_MMX | CPUID_FXSR |       \
     CPUID_SSE | CPUID_SSE2 | CPUID_SS)
/* partly implemented:
   CPUID_MTRR, CPUID_MCA, CPUID_CLFLUSH (needed for Win64)
   CPUID_PSE36 (needed for Solaris) */
/* missing:
   CPUID_VME, CPUID_DTS, CPUID_SS, CPUID_HT, CPUID_TM, CPUID_PBE */
#define TCG_EXT_FEATURES (CPUID_EXT_SSE3 | CPUID_EXT_MONITOR | CPUID_EXT_CX16 | CPUID_EXT_POPCNT | CPUID_EXT_HYPERVISOR)
/* missing:
   CPUID_EXT_DTES64, CPUID_EXT_DSCPL, CPUID_EXT_VMX, CPUID_EXT_EST,
   CPUID_EXT_TM2, CPUID_EXT_XTPR, CPUID_EXT_PDCM, CPUID_EXT_XSAVE */
#define TCG_EXT2_FEATURES                                                                                            \
    ((TCG_FEATURES & EXT2_FEATURE_MASK) | CPUID_EXT2_NX | CPUID_EXT2_MMXEXT | CPUID_EXT2_RDTSCP | CPUID_EXT2_3DNOW | \
     CPUID_EXT2_3DNOWEXT)
/* missing:
   CPUID_EXT2_PDPE1GB */
#define TCG_EXT3_FEATURES (CPUID_EXT3_LAHF_LM | CPUID_EXT3_SVM | CPUID_EXT3_CR8LEG | CPUID_EXT3_ABM | CPUID_EXT3_SSE4A)
#define TCG_SVM_FEATURES  0

/* maintains list of cpu model definitions
 */
static x86_def_t *x86_defs = { NULL };

/* built-in cpu model definitions (deprecated)
 */
static x86_def_t builtin_x86_defs[] = {
    {
     .name = "x86",
     .level = 4,
     .family = 6,
     .model = 3,
     .stepping = 3,
     .features = PPRO_FEATURES,
     .ext_features = CPUID_EXT_SSE3 | CPUID_EXT_POPCNT,
     .xlevel = 0x80000004,
     .model_id = "QEMU Virtual CPU version 0",
     },
    {
     .name = "coreduo",
     .level = 10,
     .family = 6,
     .model = 14,
     .stepping = 8,
     .features = PPRO_FEATURES | CPUID_VME | CPUID_MTRR | CPUID_CLFLUSH | CPUID_MCA | CPUID_DTS | CPUID_ACPI | CPUID_SS |
                    CPUID_HT | CPUID_TM | CPUID_PBE,
     .ext_features = CPUID_EXT_SSE3 | CPUID_EXT_MONITOR | CPUID_EXT_VMX | CPUID_EXT_EST | CPUID_EXT_TM2 | CPUID_EXT_XTPR |
                        CPUID_EXT_PDCM, .ext2_features = CPUID_EXT2_NX,
     .xlevel = 0x80000008,
     .model_id = "Genuine Intel(R) CPU           T2600  @ 2.16GHz",
     },
    {
     .name = "486",
     .level = 1,
     .family = 4,
     .model = 0,
     .stepping = 0,
     .features = I486_FEATURES,
     .xlevel = 0,
     },
    {
     .name = "pentium",
     .level = 1,
     .family = 5,
     .model = 4,
     .stepping = 3,
     .features = PENTIUM_FEATURES,
     .xlevel = 0,
     },
    {
     .name = "pentium2",
     .level = 2,
     .family = 6,
     .model = 5,
     .stepping = 2,
     .features = PENTIUM2_FEATURES,
     .xlevel = 0,
     },
    {
     .name = "pentium3",
     .level = 2,
     .family = 6,
     .model = 7,
     .stepping = 3,
     .features = PENTIUM3_FEATURES,
     .xlevel = 0,
     },
    {
     .name = "athlon",
     .level = 2,
     .vendor1 = CPUID_VENDOR_AMD_1,
     .vendor2 = CPUID_VENDOR_AMD_2,
     .vendor3 = CPUID_VENDOR_AMD_3,
     .family = 6,
     .model = 2,
     .stepping = 3,
     .features = PPRO_FEATURES | CPUID_PSE36 | CPUID_VME | CPUID_MTRR | CPUID_MCA,
     .ext2_features = (PPRO_FEATURES & EXT2_FEATURE_MASK) | CPUID_EXT2_MMXEXT | CPUID_EXT2_3DNOW | CPUID_EXT2_3DNOWEXT,
     .xlevel = 0x80000008,
     /* XXX: put another string ? */
        .model_id = "QEMU Virtual CPU version 0",
     },
    {
     .name = "n270",
     /* original is on level 10 */
        .level = 5,
     .family = 6,
     .model = 28,
     .stepping = 2,
     .features = PPRO_FEATURES | CPUID_MTRR | CPUID_CLFLUSH | CPUID_MCA | CPUID_VME | CPUID_DTS | CPUID_ACPI | CPUID_SS |
                    CPUID_HT | CPUID_TM | CPUID_PBE,
     /* Some CPUs got no CPUID_SEP */
        .ext_features = CPUID_EXT_SSE3 | CPUID_EXT_MONITOR | CPUID_EXT_SSSE3 | CPUID_EXT_DSCPL | CPUID_EXT_EST | CPUID_EXT_TM2 |
                        CPUID_EXT_XTPR, .ext2_features = (PPRO_FEATURES & EXT2_FEATURE_MASK) | CPUID_EXT2_NX,
     .ext3_features = CPUID_EXT3_LAHF_LM,
     .xlevel = 0x8000000A,
     .model_id = "Intel(R) Atom(TM) CPU N270   @ 1.60GHz",
     },
    {
     .name = "x86_64",
     .level = 5,
     .family = 6,
     .model = 28,
     .stepping = 2,
     .features = PPRO_FEATURES | CPUID_MTRR | CPUID_CLFLUSH | CPUID_MCA | CPUID_VME | CPUID_DTS | CPUID_ACPI | CPUID_SS |
                    CPUID_HT | CPUID_TM | CPUID_PBE,
     /* Some CPUs got no CPUID_SEP */
        .ext_features = CPUID_EXT_SSE3 | CPUID_EXT_MONITOR | CPUID_EXT_SSSE3 | CPUID_EXT_DSCPL | CPUID_EXT_EST | CPUID_EXT_TM2 |
                        CPUID_EXT_XTPR | CPUID_EXT_X2APIC | CPUID_EXT_TSC_DEADLINE,
     .ext2_features = (PPRO_FEATURES & EXT2_FEATURE_MASK) | CPUID_EXT2_NX | CPUID_EXT2_LM | CPUID_EXT2_SYSCALL,
     .ext3_features = CPUID_EXT3_LAHF_LM,
     .xlevel = 0x8000000A,
     .model_id = "Virtual x86_64 CPU",
     },
};

static int cpu_x86_find_by_name(x86_def_t *x86_cpu_def, const char *cpu_model)
{
    x86_def_t *def;
    char *name = tlib_strdup(cpu_model);
    /* Features to be added*/
    uint32_t plus_features = 0, plus_ext_features = 0;
    uint32_t plus_ext2_features = 0, plus_ext3_features = 0;
    uint32_t plus_svm_features = 0;
    /* Features to be removed */
    uint32_t minus_features = 0, minus_ext_features = 0;
    uint32_t minus_ext2_features = 0, minus_ext3_features = 0;
    uint32_t minus_svm_features = 0;
    x86_defs = NULL;
    x86_cpudef_setup();
    for(def = x86_defs; def; def = def->next) {
        if(name && !strcmp(name, def->name)) {
            break;
        }
    }
    if(!def) {
        goto error;
    } else {
        memcpy(x86_cpu_def, def, sizeof(*def));
    }

    add_flagname_to_bitmaps("hypervisor", &plus_features, &plus_ext_features, &plus_ext2_features, &plus_ext3_features,
                            &plus_svm_features);

    x86_cpu_def->features |= plus_features;
    x86_cpu_def->ext_features |= plus_ext_features;
    x86_cpu_def->ext2_features |= plus_ext2_features;
    x86_cpu_def->ext3_features |= plus_ext3_features;
    x86_cpu_def->svm_features |= plus_svm_features;
    x86_cpu_def->features &= ~minus_features;
    x86_cpu_def->ext_features &= ~minus_ext_features;
    x86_cpu_def->ext2_features &= ~minus_ext2_features;
    x86_cpu_def->ext3_features &= ~minus_ext3_features;
    x86_cpu_def->svm_features &= ~minus_svm_features;
    tlib_free(name);
    return 0;

error:
    tlib_free(name);
    return -1;
}

int cpu_x86_register(CPUState *env, const char *cpu_model)
{
    x86_def_t def1, *def = &def1;

    memset(def, 0, sizeof(*def));
    if(cpu_x86_find_by_name(def, cpu_model) < 0) {
        return -1;
    }
    if(def->vendor1) {
        env->cpuid_vendor1 = def->vendor1;
        env->cpuid_vendor2 = def->vendor2;
        env->cpuid_vendor3 = def->vendor3;
    } else {
        env->cpuid_vendor1 = CPUID_VENDOR_INTEL_1;
        env->cpuid_vendor2 = CPUID_VENDOR_INTEL_2;
        env->cpuid_vendor3 = CPUID_VENDOR_INTEL_3;
    }
    env->cpuid_vendor_override = def->vendor_override;
    env->cpuid_level = def->level;
    if(def->family > 0x0f) {
        env->cpuid_version = 0xf00 | ((def->family - 0x0f) << 20);
    } else {
        env->cpuid_version = def->family << 8;
    }
    env->cpuid_version |= ((def->model & 0xf) << 4) | ((def->model >> 4) << 16);
    env->cpuid_version |= def->stepping;
    env->cpuid_features = def->features;
    env->cpuid_ext_features = def->ext_features;
    env->cpuid_ext2_features = def->ext2_features;
    env->cpuid_ext3_features = def->ext3_features;
    env->cpuid_xlevel = def->xlevel;
    env->cpuid_svm_features = def->svm_features;
    env->cpuid_ext4_features = def->ext4_features;
    env->cpuid_xlevel2 = def->xlevel2;
    env->tsc_khz = def->tsc_khz;
    {
        const char *model_id = def->model_id;
        int c, len, i;
        if(!model_id) {
            model_id = "";
        }
        len = strlen(model_id);
        for(i = 0; i < 48; i++) {
            if(i >= len) {
                c = '\0';
            } else {
                c = (uint8_t)model_id[i];
            }
            env->cpuid_model[i >> 2] |= c << (8 * (i & 3));
        }
    }
    return 0;
}

/* interpret radix and convert from string to arbitrary scalar,
 * otherwise flag failure
 */
#define setscalar(pval, str, perr)                   \
    {                                                \
        char *pend;                                  \
        uintptr_t ul;                                \
                                                     \
        ul = strtoul(str, &pend, 0);                 \
        *str && !*pend ? (*pval = ul) : (*perr = 1); \
    }

void cpu_clear_apic_feature(CPUState *env)
{
    env->cpuid_features &= ~CPUID_APIC;
}

/* register "cpudef" models defined in configuration file.  Here we first
 * preload any built-in definitions
 */
void x86_cpudef_setup(void)
{
    int i;
    for(i = 0; i < ARRAY_SIZE(builtin_x86_defs); ++i) {
        builtin_x86_defs[i].next = x86_defs;
        builtin_x86_defs[i].flags = 1;
        x86_defs = &builtin_x86_defs[i];
    }
}

static void get_cpuid_vendor(CPUState *env, uint32_t *ebx, uint32_t *ecx, uint32_t *edx)
{
    *ebx = env->cpuid_vendor1;
    *edx = env->cpuid_vendor2;
    *ecx = env->cpuid_vendor3;
}

void cpu_x86_cpuid(CPUState *env, uint32_t index, uint32_t count, uint32_t *eax, uint32_t *ebx, uint32_t *ecx, uint32_t *edx)
{
    /* test if maximum index reached */
    if(index & 0x80000000) {
        if(index > env->cpuid_xlevel) {
            if(env->cpuid_xlevel2 > 0) {
                /* Handle the Centaur's CPUID instruction. */
                if(index > env->cpuid_xlevel2) {
                    index = env->cpuid_xlevel2;
                } else if(index < 0xC0000000) {
                    index = env->cpuid_xlevel;
                }
            } else {
                index = env->cpuid_xlevel;
            }
        }
    } else {
        if(index > env->cpuid_level) {
            index = env->cpuid_level;
        }
    }

    switch(index) {
        case 0:
            *eax = env->cpuid_level;
            get_cpuid_vendor(env, ebx, ecx, edx);
            break;
        case 1:
            *eax = env->cpuid_version;
            *ebx = (tlib_get_mp_index() << 24) | 8 << 8; /* CLFLUSH size in quad words, Linux wants it. */
            *ecx = env->cpuid_ext_features;
            *edx = env->cpuid_features;
            if(env->nr_cores * env->nr_threads > 1) {
                *ebx |= (env->nr_cores * env->nr_threads) << 16;
                *edx |= 1 << 28; /* HTT bit */
            }
            break;
        case 2:
            /* cache info: needed for Pentium Pro compatibility */
            *eax = 1;
            *ebx = 0;
            *ecx = 0;
            *edx = 0x2c307d;
            break;
        case 4:
            /* cache info: needed for Core compatibility */
            if(env->nr_cores > 1) {
                *eax = (env->nr_cores - 1) << 26;
            } else {
                *eax = 0;
            }
            switch(count) {
                case 0: /* L1 dcache info */
                    *eax |= 0x0000121;
                    *ebx = 0x1c0003f;
                    *ecx = 0x000003f;
                    *edx = 0x0000001;
                    break;
                case 1: /* L1 icache info */
                    *eax |= 0x0000122;
                    *ebx = 0x1c0003f;
                    *ecx = 0x000003f;
                    *edx = 0x0000001;
                    break;
                case 2: /* L2 cache info */
                    *eax |= 0x0000143;
                    if(env->nr_threads > 1) {
                        *eax |= (env->nr_threads - 1) << 14;
                    }
                    *ebx = 0x3c0003f;
                    *ecx = 0x0000fff;
                    *edx = 0x0000001;
                    break;
                default: /* end of info */
                    *eax = 0;
                    *ebx = 0;
                    *ecx = 0;
                    *edx = 0;
                    break;
            }
            break;
        case 5:
            /* mwait info: needed for Core compatibility */
            *eax = 0; /* Smallest monitor-line size in bytes */
            *ebx = 0; /* Largest monitor-line size in bytes */
            *ecx = CPUID_MWAIT_EMX | CPUID_MWAIT_IBE;
            *edx = 0;
            break;
        case 6:
            /* Thermal and Power Leaf */
            *eax = 0;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        case 7:
            *eax = 0;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        case 9:
            /* Direct Cache Access Information Leaf */
            *eax = 0; /* Bits 0-31 in DCA_CAP MSR */
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        case 0xA:
            /* Architectural Performance Monitoring Leaf */
            *eax = 0;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        case 0xD:
            /* Processor Extended State */
            if(!(env->cpuid_ext_features & CPUID_EXT_XSAVE)) {
                *eax = 0;
                *ebx = 0;
                *ecx = 0;
                *edx = 0;
                break;
            }
            *eax = 0;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        case 0x80000000:
            *eax = env->cpuid_xlevel;
            *ebx = env->cpuid_vendor1;
            *edx = env->cpuid_vendor2;
            *ecx = env->cpuid_vendor3;
            break;
        case 0x80000001:
            *eax = env->cpuid_version;
            *ebx = 0;
            *ecx = env->cpuid_ext3_features;
            *edx = env->cpuid_ext2_features;

            /* The Linux kernel checks for the CMPLegacy bit and
             * discards multiple thread information if it is set.
             * So dont set it here for Intel to make Linux guests happy.
             */
            if(env->nr_cores * env->nr_threads > 1) {
                uint32_t tebx, tecx, tedx;
                get_cpuid_vendor(env, &tebx, &tecx, &tedx);
                if(tebx != CPUID_VENDOR_INTEL_1 || tedx != CPUID_VENDOR_INTEL_2 || tecx != CPUID_VENDOR_INTEL_3) {
                    *ecx |= 1 << 1; /* CmpLegacy bit */
                }
            }
            break;
        case 0x80000002:
        case 0x80000003:
        case 0x80000004:
            *eax = env->cpuid_model[(index - 0x80000002) * 4 + 0];
            *ebx = env->cpuid_model[(index - 0x80000002) * 4 + 1];
            *ecx = env->cpuid_model[(index - 0x80000002) * 4 + 2];
            *edx = env->cpuid_model[(index - 0x80000002) * 4 + 3];
            break;
        case 0x80000005:
            /* cache info (L1 cache) */
            *eax = 0x01ff01ff;
            *ebx = 0x01ff01ff;
            *ecx = 0x40020140;
            *edx = 0x40020140;
            break;
        case 0x80000006:
            /* cache info (L2 cache) */
            *eax = 0;
            *ebx = 0x42004200;
            *ecx = 0x02008140;
            *edx = 0;
            break;
        case 0x80000007:
            /* Processor Power Management Information and RAS Capabilities */
            *eax = 0;
            *ebx = 0;
            *ecx = 0;

            if(env->cpuid_ext_features & CPUID_EXT_TSC_DEADLINE) {
                //  Bit 8 - Invariant TSC
                //  This feature is not implemented
                //  when software enable this feature
                //  TSC deadline mode will be using same timer
                *edx = 0x00000100;
            } else {
                *edx = 0;
            }
            break;
        case 0x80000008:
            /* virtual & phys address size in low 2 bytes. */
            /* XXX: This value must match the one used in the MMU code. */
            if(env->cpuid_ext2_features & CPUID_EXT2_LM) {
                /* 64 bit processor */
                /* XXX: The physical address space is limited to 42 bits in exec.c. */
                *eax = 0x00003028; /* 48 bits virtual, 40 bits physical */
            } else {
                if(env->cpuid_features & CPUID_PSE36) {
                    *eax = 0x00000024; /* 36 bits physical */
                } else {
                    *eax = 0x00000020; /* 32 bits physical */
                }
            }
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            if(env->nr_cores * env->nr_threads > 1) {
                *ecx |= (env->nr_cores * env->nr_threads) - 1;
            }
            break;
        case 0x8000000A:
            if(env->cpuid_ext3_features & CPUID_EXT3_SVM) {
                *eax = 0x00000001; /* SVM Revision */
                *ebx = 0x00000010; /* nr of ASIDs */
                *ecx = 0;
                *edx = env->cpuid_svm_features; /* optional features */
            } else {
                *eax = 0;
                *ebx = 0;
                *ecx = 0;
                *edx = 0;
            }
            break;
        case 0xC0000000:
            *eax = env->cpuid_xlevel2;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        case 0xC0000001:
            /* Support for VIA CPU's CPUID instruction */
            *eax = env->cpuid_version;
            *ebx = 0;
            *ecx = 0;
            *edx = env->cpuid_ext4_features;
            break;
        case 0xC0000002:
        case 0xC0000003:
        case 0xC0000004:
            /* Reserved for the future, and now filled with zero */
            *eax = 0;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
        default:
            /* reserved values: zero */
            *eax = 0;
            *ebx = 0;
            *ecx = 0;
            *edx = 0;
            break;
    }
}
