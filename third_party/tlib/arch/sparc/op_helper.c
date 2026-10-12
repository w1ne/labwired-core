#include "cpu.h"
#define ALIGNED_ONLY
#include "softmmu_exec.h"
#include "helper.h"
#include "arch_callbacks.h"

void do_unaligned_access(target_ulong addr, int is_write, int is_user, void *retaddr);

#define DT0 (env->dt0)
#define DT1 (env->dt1)
#define QT0 (env->qt0)
#define QT1 (env->qt1)

#define BIG_ENDIAN_CONVERT(value) \
    (((value >> 24) & 0xFF) | ((value >> 8) & 0xFF00) | ((value << 8) & 0xFF0000) | ((value << 24) & 0xFF000000))

/* Leon3 cache control */

/* Cache control: emulate the behavior of cache control registers but without
   any effect on the emulated */

#define CACHE_STATE_MASK 0x3
#define CACHE_DISABLED   0x0
#define CACHE_FROZEN     0x1
#define CACHE_ENABLED    0x3

/* Cache Control register fields */

#define CACHE_CTRL_IF (1 << 4)  /* Instruction Cache Freeze on Interrupt */
#define CACHE_CTRL_DF (1 << 5)  /* Data Cache Freeze on Interrupt */
#define CACHE_CTRL_DP (1 << 14) /* Data cache flush pending */
#define CACHE_CTRL_IP (1 << 15) /* Instruction cache flush pending */
#define CACHE_CTRL_IB (1 << 16) /* Instruction burst fetch */
#define CACHE_CTRL_FI (1 << 21) /* Flush Instruction cache (Write only) */
#define CACHE_CTRL_FD (1 << 22) /* Flush Data cache (Write only) */
#define CACHE_CTRL_DS (1 << 23) /* Data cache snoop enable */

static void do_unassigned_access(target_phys_addr_t addr, int is_write, int is_exec, int is_asi, int size);

static void raise_exception(int tt)
{
    env->exception_index = tt;
    cpu_loop_exit(env);
}

void HELPER(raise_exception)(int tt)
{
    raise_exception(tt);
}

target_ulong HELPER(ldstub)(uint32_t addr)
{
    int page_index;
    uintptr_t physaddr;
    int mmu_idx;
    void *retaddr;
    uint8_t ret;
    uint8_t value = 0xFF;

    retaddr = GETPC();
    page_index = (addr >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);
    mmu_idx = env->psrs;
    if(unlikely(env->tlb_table[mmu_idx][page_index].addr_write != (addr & (TARGET_PAGE_MASK)))) {
        /* the page is not in the TLB : fill it */
        tlb_fill(env, addr, 1, mmu_idx, retaddr, 0, 1);
    }

    if(unlikely(env->tlb_table[mmu_idx][page_index].addr_read != (addr & (TARGET_PAGE_MASK)))) {
        /* the page is not in the TLB : fill it */
        tlb_fill(env, addr, 0, mmu_idx, retaddr, 0, 1);
    }

    physaddr = addr + env->tlb_table[mmu_idx][page_index].addend;

    __atomic_exchange((uint8_t *)physaddr, &value, &ret, __ATOMIC_SEQ_CST);

    return (target_ulong)ret;
}

target_ulong HELPER(swap)(target_ulong value, uint32_t addr)
{
    int page_index;
    uintptr_t physaddr;
    int mmu_idx;
    void *retaddr;
    uint32_t ret;

    retaddr = GETPC();
    page_index = (addr >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);
    mmu_idx = env->psrs;
    if(unlikely(env->tlb_table[mmu_idx][page_index].addr_write != (addr & (TARGET_PAGE_MASK)))) {
        /* the page is not in the TLB : fill it */
        tlb_fill(env, addr, 1, mmu_idx, retaddr, 0, 4);
    }

    if(unlikely(env->tlb_table[mmu_idx][page_index].addr_read != (addr & (TARGET_PAGE_MASK)))) {
        /* the page is not in the TLB : fill it */
        tlb_fill(env, addr, 0, mmu_idx, retaddr, 0, 4);
    }

    physaddr = addr + env->tlb_table[mmu_idx][page_index].addend;

#ifdef TARGET_WORDS_BIGENDIAN
    value = BIG_ENDIAN_CONVERT(value);
#endif
    __atomic_exchange((uint32_t *)physaddr, &value, &ret, __ATOMIC_SEQ_CST);

#ifdef TARGET_WORDS_BIGENDIAN
    ret = BIG_ENDIAN_CONVERT(ret);
#endif

    return (target_ulong)ret;
}

void HELPER(power_down)(void)
{
    tlib_on_cpu_power_down();

    env->exception_index = EXCP_WFI;
    env->wfi = 1;
    env->pc = env->npc;
    env->npc = env->pc + 4;
    cpu_loop_exit(env);
}
void helper_shutdown(void)
{
    tlib_on_cpu_power_down();

    env->wfi = 1;
    env->exception_index = EXCP_WFI;
    cpu_loop_exit(env);
}

void helper_check_align(target_ulong addr, uint32_t align)
{
    if(addr & align) {
        raise_exception(TT_UNALIGNED);
    }
}

#define F_HELPER(name, p) void helper_f##name##p(void)

#define F_BINOP(name)                                       \
    float32 helper_f##name##s(float32 src1, float32 src2)   \
    {                                                       \
        return float32_##name(src1, src2, &env->fp_status); \
    }                                                       \
    F_HELPER(name, d)                                       \
    {                                                       \
        DT0 = float64_##name(DT0, DT1, &env->fp_status);    \
    }                                                       \
    F_HELPER(name, q)                                       \
    {                                                       \
        QT0 = float128_##name(QT0, QT1, &env->fp_status);   \
    }

F_BINOP(add);
F_BINOP(sub);
F_BINOP(mul);
F_BINOP(div);
#undef F_BINOP

void helper_fsmuld(float32 src1, float32 src2)
{
    DT0 = float64_mul(float32_to_float64(src1, &env->fp_status), float32_to_float64(src2, &env->fp_status), &env->fp_status);
}

void helper_fdmulq(void)
{
    QT0 = float128_mul(float64_to_float128(DT0, &env->fp_status), float64_to_float128(DT1, &env->fp_status), &env->fp_status);
}

float32 helper_fnegs(float32 src)
{
    return float32_chs(src);
}

/* Integer to float conversion.  */
float32 helper_fitos(int32_t src)
{
    return int32_to_float32(src, &env->fp_status);
}

void helper_fitod(int32_t src)
{
    DT0 = int32_to_float64(src, &env->fp_status);
}

void helper_fitoq(int32_t src)
{
    QT0 = int32_to_float128(src, &env->fp_status);
}

#undef F_HELPER

/* floating point conversion */
float32 helper_fdtos(void)
{
    return float64_to_float32(DT1, &env->fp_status);
}

void helper_fstod(float32 src)
{
    DT0 = float32_to_float64(src, &env->fp_status);
}

float32 helper_fqtos(void)
{
    return float128_to_float32(QT1, &env->fp_status);
}

void helper_fstoq(float32 src)
{
    QT0 = float32_to_float128(src, &env->fp_status);
}

void helper_fqtod(void)
{
    DT0 = float128_to_float64(QT1, &env->fp_status);
}

void helper_fdtoq(void)
{
    QT0 = float64_to_float128(DT1, &env->fp_status);
}

/* Float to integer conversion.  */
int32_t helper_fstoi(float32 src)
{
    return float32_to_int32_round_to_zero(src, &env->fp_status);
}

int32_t helper_fdtoi(void)
{
    return float64_to_int32_round_to_zero(DT1, &env->fp_status);
}

int32_t helper_fqtoi(void)
{
    return float128_to_int32_round_to_zero(QT1, &env->fp_status);
}

void helper_check_ieee_exceptions(void)
{
    target_ulong status;

    status = get_float_exception_flags(&env->fp_status);
    if(status) {
        /* Copy IEEE 754 flags into FSR */
        if(status & float_flag_invalid) {
            env->fsr |= FSR_NVC;
        }
        if(status & float_flag_overflow) {
            env->fsr |= FSR_OFC;
        }
        if(status & float_flag_underflow) {
            env->fsr |= FSR_UFC;
        }
        if(status & float_flag_divbyzero) {
            env->fsr |= FSR_DZC;
        }
        if(status & float_flag_inexact) {
            env->fsr |= FSR_NXC;
        }

        if((env->fsr & FSR_CEXC_MASK) & ((env->fsr & FSR_TEM_MASK) >> 23)) {
            /* Unmasked exception, generate a trap */
            env->fsr |= FSR_FTT_IEEE_EXCP;
            raise_exception(TT_FP_EXCP);
        } else {
            /* Accumulate exceptions */
            env->fsr |= (env->fsr & FSR_CEXC_MASK) << 5;
        }
    }
}

void helper_clear_float_exceptions(void)
{
    set_float_exception_flags(0, &env->fp_status);
}

float32 helper_fabss(float32 src)
{
    return float32_abs(src);
}

float32 helper_fsqrts(float32 src)
{
    return float32_sqrt(src, &env->fp_status);
}

void helper_fsqrtd(void)
{
    DT0 = float64_sqrt(DT1, &env->fp_status);
}

void helper_fsqrtq(void)
{
    QT0 = float128_sqrt(QT1, &env->fp_status);
}

#define GEN_FCMP(name, size, reg1, reg2, FS, E)                                                             \
    void glue(helper_, name)(void)                                                                          \
    {                                                                                                       \
        env->fsr &= FSR_FTT_NMASK;                                                                          \
        if(E && (glue(size, _is_any_nan)(reg1) || glue(size, _is_any_nan)(reg2)) && (env->fsr & FSR_NVM)) { \
            env->fsr |= FSR_NVC;                                                                            \
            env->fsr |= FSR_FTT_IEEE_EXCP;                                                                  \
            raise_exception(TT_FP_EXCP);                                                                    \
        }                                                                                                   \
        switch(glue(size, _compare)(reg1, reg2, &env->fp_status)) {                                         \
            case float_relation_unordered:                                                                  \
                if((env->fsr & FSR_NVM)) {                                                                  \
                    env->fsr |= FSR_NVC;                                                                    \
                    env->fsr |= FSR_FTT_IEEE_EXCP;                                                          \
                    raise_exception(TT_FP_EXCP);                                                            \
                } else {                                                                                    \
                    env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                             \
                    env->fsr |= (FSR_FCC1 | FSR_FCC0) << FS;                                                \
                    env->fsr |= FSR_NVA;                                                                    \
                }                                                                                           \
                break;                                                                                      \
            case float_relation_less:                                                                       \
                env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                                 \
                env->fsr |= FSR_FCC0 << FS;                                                                 \
                break;                                                                                      \
            case float_relation_greater:                                                                    \
                env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                                 \
                env->fsr |= FSR_FCC1 << FS;                                                                 \
                break;                                                                                      \
            default:                                                                                        \
                env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                                 \
                break;                                                                                      \
        }                                                                                                   \
    }
#define GEN_FCMPS(name, size, FS, E)                                                                        \
    void glue(helper_, name)(float32 src1, float32 src2)                                                    \
    {                                                                                                       \
        env->fsr &= FSR_FTT_NMASK;                                                                          \
        if(E && (glue(size, _is_any_nan)(src1) || glue(size, _is_any_nan)(src2)) && (env->fsr & FSR_NVM)) { \
            env->fsr |= FSR_NVC;                                                                            \
            env->fsr |= FSR_FTT_IEEE_EXCP;                                                                  \
            raise_exception(TT_FP_EXCP);                                                                    \
        }                                                                                                   \
        switch(glue(size, _compare)(src1, src2, &env->fp_status)) {                                         \
            case float_relation_unordered:                                                                  \
                if((env->fsr & FSR_NVM)) {                                                                  \
                    env->fsr |= FSR_NVC;                                                                    \
                    env->fsr |= FSR_FTT_IEEE_EXCP;                                                          \
                    raise_exception(TT_FP_EXCP);                                                            \
                } else {                                                                                    \
                    env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                             \
                    env->fsr |= (FSR_FCC1 | FSR_FCC0) << FS;                                                \
                    env->fsr |= FSR_NVA;                                                                    \
                }                                                                                           \
                break;                                                                                      \
            case float_relation_less:                                                                       \
                env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                                 \
                env->fsr |= FSR_FCC0 << FS;                                                                 \
                break;                                                                                      \
            case float_relation_greater:                                                                    \
                env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                                 \
                env->fsr |= FSR_FCC1 << FS;                                                                 \
                break;                                                                                      \
            default:                                                                                        \
                env->fsr &= ~((FSR_FCC1 | FSR_FCC0) << FS);                                                 \
                break;                                                                                      \
        }                                                                                                   \
    }

GEN_FCMPS(fcmps, float32, 0, 0);
GEN_FCMP(fcmpd, float64, DT0, DT1, 0, 0);

GEN_FCMPS(fcmpes, float32, 0, 1);
GEN_FCMP(fcmped, float64, DT0, DT1, 0, 1);

GEN_FCMP(fcmpq, float128, QT0, QT1, 0, 0);
GEN_FCMP(fcmpeq, float128, QT0, QT1, 0, 1);

static uint32_t compute_all_flags(void)
{
    return env->psr & PSR_ICC;
}

static uint32_t compute_C_flags(void)
{
    return env->psr & PSR_CARRY;
}

static inline uint32_t get_NZ_icc(int32_t dst)
{
    uint32_t ret = 0;

    if(dst == 0) {
        ret = PSR_ZERO;
    } else if(dst < 0) {
        ret = PSR_NEG;
    }
    return ret;
}

static inline uint32_t get_V_div_icc(target_ulong src2)
{
    uint32_t ret = 0;

    if(src2 != 0) {
        ret = PSR_OVF;
    }
    return ret;
}

static uint32_t compute_all_div(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_V_div_icc(CC_SRC2);
    return ret;
}

static uint32_t compute_C_div(void)
{
    return 0;
}

static inline uint32_t get_C_add_icc(uint32_t dst, uint32_t src1)
{
    uint32_t ret = 0;

    if(dst < src1) {
        ret = PSR_CARRY;
    }
    return ret;
}

static inline uint32_t get_C_addx_icc(uint32_t dst, uint32_t src1, uint32_t src2)
{
    uint32_t ret = 0;

    if(((src1 & src2) | (~dst & (src1 | src2))) & (1U << 31)) {
        ret = PSR_CARRY;
    }
    return ret;
}

static inline uint32_t get_V_add_icc(uint32_t dst, uint32_t src1, uint32_t src2)
{
    uint32_t ret = 0;

    if(((src1 ^ src2 ^ -1) & (src1 ^ dst)) & (1U << 31)) {
        ret = PSR_OVF;
    }
    return ret;
}

static uint32_t compute_all_add(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_add_icc(CC_DST, CC_SRC);
    ret |= get_V_add_icc(CC_DST, CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_C_add(void)
{
    return get_C_add_icc(CC_DST, CC_SRC);
}

static uint32_t compute_all_addx(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_addx_icc(CC_DST, CC_SRC, CC_SRC2);
    ret |= get_V_add_icc(CC_DST, CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_C_addx(void)
{
    uint32_t ret;

    ret = get_C_addx_icc(CC_DST, CC_SRC, CC_SRC2);
    return ret;
}

static inline uint32_t get_V_tag_icc(target_ulong src1, target_ulong src2)
{
    uint32_t ret = 0;

    if((src1 | src2) & 0x3) {
        ret = PSR_OVF;
    }
    return ret;
}

static uint32_t compute_all_tadd(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_add_icc(CC_DST, CC_SRC);
    ret |= get_V_add_icc(CC_DST, CC_SRC, CC_SRC2);
    ret |= get_V_tag_icc(CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_all_taddtv(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_add_icc(CC_DST, CC_SRC);
    return ret;
}

static inline uint32_t get_C_sub_icc(uint32_t src1, uint32_t src2)
{
    uint32_t ret = 0;

    if(src1 < src2) {
        ret = PSR_CARRY;
    }
    return ret;
}

static inline uint32_t get_C_subx_icc(uint32_t dst, uint32_t src1, uint32_t src2)
{
    uint32_t ret = 0;

    if(((~src1 & src2) | (dst & (~src1 | src2))) & (1U << 31)) {
        ret = PSR_CARRY;
    }
    return ret;
}

static inline uint32_t get_V_sub_icc(uint32_t dst, uint32_t src1, uint32_t src2)
{
    uint32_t ret = 0;

    if(((src1 ^ src2) & (src1 ^ dst)) & (1U << 31)) {
        ret = PSR_OVF;
    }
    return ret;
}

static uint32_t compute_all_sub(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_sub_icc(CC_SRC, CC_SRC2);
    ret |= get_V_sub_icc(CC_DST, CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_C_sub(void)
{
    return get_C_sub_icc(CC_SRC, CC_SRC2);
}

static uint32_t compute_all_subx(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_subx_icc(CC_DST, CC_SRC, CC_SRC2);
    ret |= get_V_sub_icc(CC_DST, CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_C_subx(void)
{
    uint32_t ret;

    ret = get_C_subx_icc(CC_DST, CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_all_tsub(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_sub_icc(CC_SRC, CC_SRC2);
    ret |= get_V_sub_icc(CC_DST, CC_SRC, CC_SRC2);
    ret |= get_V_tag_icc(CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_all_tsubtv(void)
{
    uint32_t ret;

    ret = get_NZ_icc(CC_DST);
    ret |= get_C_sub_icc(CC_SRC, CC_SRC2);
    return ret;
}

static uint32_t compute_all_logic(void)
{
    return get_NZ_icc(CC_DST);
}

static uint32_t compute_C_logic(void)
{
    return 0;
}

typedef struct CCTable {
    uint32_t (*compute_all)(void); /* return all the flags */
    uint32_t (*compute_c)(void);   /* return the C flag */
} CCTable;

static const CCTable icc_table[CC_OP_NB] = {
    /* CC_OP_DYNAMIC should never happen */
    [CC_OP_FLAGS] = { compute_all_flags,  compute_C_flags },
      [CC_OP_DIV] = { compute_all_div,    compute_C_div   },
    [CC_OP_ADD] = { compute_all_add,    compute_C_add   },
      [CC_OP_ADDX] = { compute_all_addx,   compute_C_addx  },
    [CC_OP_TADD] = { compute_all_tadd,   compute_C_add   },
      [CC_OP_TADDTV] = { compute_all_taddtv, compute_C_add   },
    [CC_OP_SUB] = { compute_all_sub,    compute_C_sub   },
      [CC_OP_SUBX] = { compute_all_subx,   compute_C_subx  },
    [CC_OP_TSUB] = { compute_all_tsub,   compute_C_sub   },
      [CC_OP_TSUBTV] = { compute_all_tsubtv, compute_C_sub   },
    [CC_OP_LOGIC] = { compute_all_logic,  compute_C_logic },
};

void helper_compute_psr(void)
{
    uint32_t new_psr;

    new_psr = icc_table[CC_OP].compute_all();
    env->psr = new_psr;
    CC_OP = CC_OP_FLAGS;
}

uint32_t helper_compute_C_icc(void)
{
    uint32_t ret;

    ret = icc_table[CC_OP].compute_c() >> PSR_CARRY_SHIFT;
    return ret;
}

static inline void memcpy32(target_ulong *dst, const target_ulong *src)
{
    dst[0] = src[0];
    dst[1] = src[1];
    dst[2] = src[2];
    dst[3] = src[3];
    dst[4] = src[4];
    dst[5] = src[5];
    dst[6] = src[6];
    dst[7] = src[7];
}

static void set_cwp(int new_cwp)
{
    /* put the modified wrap registers at their proper location */
    if(env->cwp == env->nwindows - 1) {
        memcpy32(env->regbase, env->regbase + env->nwindows * 16);
    }
    env->cwp = new_cwp;

    /* put the wrap registers at their temporary location */
    if(new_cwp == env->nwindows - 1) {
        memcpy32(env->regbase + env->nwindows * 16, env->regbase);
    }
    env->regwptr = env->regbase + (new_cwp * 16);
}

void cpu_set_cwp(CPUState *env1, int new_cwp)
{
    CPUState *saved_env;

    saved_env = env;
    env = env1;
    set_cwp(new_cwp);
    env = saved_env;
}

static target_ulong get_psr(void)
{
    helper_compute_psr();

    return env->version | (env->psr & PSR_ICC) | (env->psref ? PSR_EF : 0) | (env->psrpil << 8) | (env->psrs ? PSR_S : 0) |
           (env->psrps ? PSR_PS : 0) | (env->psret ? PSR_ET : 0) | env->cwp;
}

target_ulong cpu_get_psr(CPUState *env1)
{
    CPUState *saved_env;
    target_ulong ret;

    saved_env = env;
    env = env1;
    ret = get_psr();
    env = saved_env;
    return ret;
}

static void put_psr(target_ulong val)
{
    env->psr = val & PSR_ICC;
    env->psref = (val & PSR_EF) ? 1 : 0;
    env->psrpil = (val & PSR_PIL) >> 8;
    env->psrs = (val & PSR_S) ? 1 : 0;
    env->psrps = (val & PSR_PS) ? 1 : 0;
    env->psret = (val & PSR_ET) ? 1 : 0;
    set_cwp(val & PSR_CWP);
    env->cc_op = CC_OP_FLAGS;
}

void cpu_put_psr(CPUState *env1, target_ulong val)
{
    CPUState *saved_env;

    saved_env = env;
    env = env1;
    put_psr(val);
    env = saved_env;
}

static int cwp_inc(int cwp)
{
    if(unlikely(cwp >= env->nwindows)) {
        cwp -= env->nwindows;
    }
    return cwp;
}

int cpu_cwp_inc(CPUState *env1, int cwp)
{
    CPUState *saved_env;
    target_ulong ret;

    saved_env = env;
    env = env1;
    ret = cwp_inc(cwp);
    env = saved_env;
    return ret;
}

static int cwp_dec(int cwp)
{
    if(unlikely(cwp < 0)) {
        cwp += env->nwindows;
    }
    return cwp;
}

int cpu_cwp_dec(CPUState *env1, int cwp)
{
    CPUState *saved_env;
    target_ulong ret;

    saved_env = env;
    env = env1;
    ret = cwp_dec(cwp);
    env = saved_env;
    return ret;
}

#undef GEN_FCMPS

/* Leon3 cache control */

static void leon3_cache_control_st(target_ulong addr, uint64_t val, int size)
{
    if(size != 4) {
        return;
    }

    switch(addr) {
        case 0x00: /* Cache control */

            /* These values must always be read as zeros */
            val &= ~CACHE_CTRL_FD;
            val &= ~CACHE_CTRL_FI;
            val &= ~CACHE_CTRL_IB;
            val &= ~CACHE_CTRL_IP;
            val &= ~CACHE_CTRL_DP;

            env->cache_control = val;
            break;
        case 0x08: /* Instruction cache configuration */
        case 0x0C: /* Data cache configuration */
            /* Read Only */
            break;
        default:
            break;
    };
}

static uint64_t leon3_cache_control_ld(target_ulong addr, int size)
{
    uint64_t ret = 0;

    if(size != 4) {
        return 0;
    }

    switch(addr) {
        case 0x00: /* Cache control */
            ret = env->cache_control;
            break;

            /* Configuration registers are read and only always keep those
               predefined values */

        case 0x08: /* Instruction cache configuration */
            ret = 0x10220000;
            break;
        case 0x0C: /* Data cache configuration */
            ret = 0x18220000;
            break;
        default:
            break;
    };
    return ret;
}

uint64_t helper_ld_asi(target_ulong addr, int asi, int size, int sign)
{
    uint64_t ret = 0;

    helper_check_align(addr, size - 1);
    switch(asi) {
        case 1:
            /* XXX: hack
               copied from case 20
             */
            {
                switch(size) {
                    case 1:
                        ret = ldub_kernel(addr);
                        break;
                    case 2:
                        ret = lduw_kernel(addr);
                        break;
                    default:
                    case 4:
                        ret = ldl_kernel(addr);
                        break;
                    case 8:
                        ret = ldq_kernel(addr);
                        break;
                }
                break;
            }

        case 2: /* SuperSparc MXCC registers and Leon3 cache control */
            switch(addr) {
                case 0x00: /* Leon3 Cache Control */
                case 0x08: /* Leon3 Instruction Cache config */
                case 0x0C: /* Leon3 Date Cache config */
                    if(env->def->features & CPU_FEATURE_CACHE_CTRL) {
                        ret = leon3_cache_control_ld(addr, size);
                    }
                    break;
                case 0x01c00a00: /* MXCC control register */
                    if(size == 8) {
                        ret = env->mxccregs[3];
                    }
                    break;
                case 0x01c00a04: /* MXCC control register */
                    if(size == 4) {
                        ret = env->mxccregs[3];
                    }
                    break;
                case 0x01c00c00: /* Module reset register */
                    if(size == 8) {
                        ret = env->mxccregs[5];
                        //  should we do something here?
                    }
                    break;
                case 0x01c00f00: /* MBus port address register */
                    if(size == 8) {
                        ret = env->mxccregs[7];
                    }
                    break;
            }
            break;
        case 3:    /* MMU probe */
        case 0x18: /* LEON3 MMU probe */
        {
            int mmulev;

            mmulev = (addr >> 8) & 15;
            if(mmulev > 4) {
                ret = 0;
            } else {
                ret = mmu_probe(env, addr, mmulev);
            }
        } break;
        case 4:    /* read MMU regs */
        case 0x19: /* LEON3 MMU regs */
        {
            int reg = (addr >> 8) & 0x1f;

            ret = env->mmuregs[reg];
            if(reg == 3) { /* Fault status cleared on read */
                env->mmuregs[3] = 0;
            } else if(reg == 0x13) { /* Fault status read */
                ret = env->mmuregs[3];
            } else if(reg == 0x14) { /* Fault address read */
                ret = env->mmuregs[4];
            }
        } break;
        case 5:  //  Turbosparc ITLB Diagnostic
        case 6:  //  Turbosparc DTLB Diagnostic
        case 7:  //  Turbosparc IOTLB Diagnostic
            break;
        case 9: /* Supervisor code access */
            switch(size) {
                case 1:
                    ret = ldub_code(addr);
                    break;
                case 2:
                    ret = lduw_code(addr);
                    break;
                default:
                case 4:
                    ret = ldl_code(addr);
                    break;
                case 8:
                    ret = ldq_code(addr);
                    break;
            }
            break;
        case 0xa: /* User data access */
            switch(size) {
                case 1:
                    ret = ldub_user(addr);
                    break;
                case 2:
                    ret = lduw_user(addr);
                    break;
                default:
                case 4:
                    ret = ldl_user(addr);
                    break;
                case 8:
                    ret = ldq_user(addr);
                    break;
            }
            break;
        case 0xb: /* Supervisor data access */
            switch(size) {
                case 1:
                    ret = ldub_kernel(addr);
                    break;
                case 2:
                    ret = lduw_kernel(addr);
                    break;
                default:
                case 4:
                    ret = ldl_kernel(addr);
                    break;
                case 8:
                    ret = ldq_kernel(addr);
                    break;
            }
            break;
        case 0xc: /* I-cache tag */
        case 0xd: /* I-cache data */
        case 0xe: /* D-cache tag */
        case 0xf: /* D-cache data */
            break;
        case 0x20: /* MMU passthrough */
        case 0x1c: /* LEON3 MMU passthrougth */
            switch(size) {
                case 1:
                    ret = ldub_phys(addr);
                    break;
                case 2:
                    ret = lduw_phys(addr);
                    break;
                default:
                case 4:
                    ret = ldl_phys(addr);
                    break;
                case 8:
                    ret = ldq_phys(addr);
                    break;
            }
            break;
        case 0x21 ... 0x2f: /* MMU passthrough, 0x100000000 to 0xfffffffff */
            switch(size) {
                case 1:
                    ret = ldub_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32));
                    break;
                case 2:
                    ret = lduw_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32));
                    break;
                default:
                case 4:
                    ret = ldl_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32));
                    break;
                case 8:
                    ret = ldq_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32));
                    break;
            }
            break;
        case 0x30:  //  Turbosparc secondary cache diagnostic
        case 0x31:  //  Turbosparc RAM snoop
        case 0x32:  //  Turbosparc page table descriptor diagnostic
        case 0x39:  /* data cache diagnostic register */
            ret = 0;
            break;
        case 0x38: /* SuperSPARC MMU Breakpoint Control Registers */
        {
            int reg = (addr >> 8) & 3;

            switch(reg) {
                case 0: /* Breakpoint Value (Addr) */
                    ret = env->mmubpregs[reg];
                    break;
                case 1: /* Breakpoint Mask */
                    ret = env->mmubpregs[reg];
                    break;
                case 2: /* Breakpoint Control */
                    ret = env->mmubpregs[reg];
                    break;
                case 3: /* Breakpoint Status */
                    ret = env->mmubpregs[reg];
                    env->mmubpregs[reg] = 0ULL;
                    break;
            }
        } break;
        case 0x49: /* SuperSPARC MMU Counter Breakpoint Value */
            ret = env->mmubpctrv;
            break;
        case 0x4a: /* SuperSPARC MMU Counter Breakpoint Control */
            ret = env->mmubpctrc;
            break;
        case 0x4b: /* SuperSPARC MMU Counter Breakpoint Status */
            ret = env->mmubpctrs;
            break;
        case 0x4c: /* SuperSPARC MMU Breakpoint Action */
            ret = env->mmubpaction;
            break;
        case 8: /* User code access, XXX */
        default:
            do_unassigned_access(addr, 0, 0, asi, size);
            ret = 0;
            break;
    }
    if(sign) {
        switch(size) {
            case 1:
                ret = (int8_t)ret;
                break;
            case 2:
                ret = (int16_t)ret;
                break;
            case 4:
                ret = (int32_t)ret;
                break;
            default:
                break;
        }
    }
    return ret;
}

void helper_st_asi(target_ulong addr, uint64_t val, int asi, int size)
{
    helper_check_align(addr, size - 1);
    switch(asi) {
        case 2: /* SuperSparc MXCC registers and Leon3 cache control */
            switch(addr) {
                case 0x00: /* Leon3 Cache Control */
                case 0x08: /* Leon3 Instruction Cache config */
                case 0x0C: /* Leon3 Date Cache config */
                    if(env->def->features & CPU_FEATURE_CACHE_CTRL) {
                        leon3_cache_control_st(addr, val, size);
                    }
                    break;

                case 0x01c00000: /* MXCC stream data register 0 */
                    if(size == 8) {
                        env->mxccdata[0] = val;
                    }
                    break;
                case 0x01c00008: /* MXCC stream data register 1 */
                    if(size == 8) {
                        env->mxccdata[1] = val;
                    }
                    break;
                case 0x01c00010: /* MXCC stream data register 2 */
                    if(size == 8) {
                        env->mxccdata[2] = val;
                    }
                    break;
                case 0x01c00018: /* MXCC stream data register 3 */
                    if(size == 8) {
                        env->mxccdata[3] = val;
                    }
                    break;
                case 0x01c00100: /* MXCC stream source */
                    if(size == 8) {
                        env->mxccregs[0] = val;
                    }
                    env->mxccdata[0] = ldq_phys((env->mxccregs[0] & 0xffffffffULL) + 0);
                    env->mxccdata[1] = ldq_phys((env->mxccregs[0] & 0xffffffffULL) + 8);
                    env->mxccdata[2] = ldq_phys((env->mxccregs[0] & 0xffffffffULL) + 16);
                    env->mxccdata[3] = ldq_phys((env->mxccregs[0] & 0xffffffffULL) + 24);
                    break;
                case 0x01c00200: /* MXCC stream destination */
                    if(size == 8) {
                        env->mxccregs[1] = val;
                    }
                    stq_phys((env->mxccregs[1] & 0xffffffffULL) + 0, env->mxccdata[0]);
                    stq_phys((env->mxccregs[1] & 0xffffffffULL) + 8, env->mxccdata[1]);
                    stq_phys((env->mxccregs[1] & 0xffffffffULL) + 16, env->mxccdata[2]);
                    stq_phys((env->mxccregs[1] & 0xffffffffULL) + 24, env->mxccdata[3]);
                    break;
                case 0x01c00a00: /* MXCC control register */
                    if(size == 8) {
                        env->mxccregs[3] = val;
                    }
                    break;
                case 0x01c00a04: /* MXCC control register */
                    if(size == 4) {
                        env->mxccregs[3] = (env->mxccregs[3] & 0xffffffff00000000ULL) | val;
                    }
                    break;
                case 0x01c00e00: /* MXCC error register  */
                    //  writing a 1 bit clears the error
                    if(size == 8) {
                        env->mxccregs[6] &= ~val;
                    }
                    break;
                case 0x01c00f00: /* MBus port address register */
                    if(size == 8) {
                        env->mxccregs[7] = val;
                    }
                    break;
            }
            break;
        case 3:    /* MMU flush */
        case 0x18: /*LEON3 MMU flush*/
        {
            int mmulev;

            mmulev = (addr >> 8) & 15;
            switch(mmulev) {
                case 0:  //  flush page
                    tlb_flush_page(env, addr & 0xfffff000, true);
                    break;
                case 1:  //  flush segment (256k)
                case 2:  //  flush region (16M)
                case 3:  //  flush context (4G)
                case 4:  //  flush entire
                    tlb_flush(env, 1, true);
                    break;
                default:
                    break;
            }
        } break;
        case 4:    /* write MMU regs */
        case 0x19: /*LEON3 write MMU regs */
        {
            int reg = (addr >> 8) & 0x1f;
            uint32_t oldreg;

            oldreg = env->mmuregs[reg];
            switch(reg) {
                case 0:  //  Control Register
                    env->mmuregs[reg] = (env->mmuregs[reg] & 0xff000000) | (val & 0x00ffffff);
                    //  Mappings generated during no-fault mode or MMU
                    //  disabled mode are invalid in normal mode
                    if((oldreg & (MMU_E | MMU_NF | env->def->mmu_bm)) !=
                       (env->mmuregs[reg] & (MMU_E | MMU_NF | env->def->mmu_bm))) {
                        tlb_flush(env, 1, true);
                    }
                    break;
                case 1:  //  Context Table Pointer Register
                    env->mmuregs[reg] = val & env->def->mmu_ctpr_mask;
                    break;
                case 2:  //  Context Register
                    env->mmuregs[reg] = val & env->def->mmu_cxr_mask;
                    if(oldreg != env->mmuregs[reg]) {
                        /* we flush when the MMU context changes because
                           QEMU has no MMU context support */
                        tlb_flush(env, 1, true);
                    }
                    break;
                case 3:  //  Synchronous Fault Status Register with Clear
                case 4:  //  Synchronous Fault Address Register
                    break;
                case 0x10:  //  TLB Replacement Control Register
                    env->mmuregs[reg] = val & env->def->mmu_trcr_mask;
                    break;
                case 0x13:  //  Synchronous Fault Status Register with Read and Clear
                    env->mmuregs[3] = val & env->def->mmu_sfsr_mask;
                    break;
                case 0x14:  //  Synchronous Fault Address Register
                    env->mmuregs[4] = val;
                    break;
                default:
                    env->mmuregs[reg] = val;
                    break;
            }
            if(oldreg != env->mmuregs[reg]) { }
        } break;
        case 5:  //  Turbosparc ITLB Diagnostic
        case 6:  //  Turbosparc DTLB Diagnostic
        case 7:  //  Turbosparc IOTLB Diagnostic
            break;
        case 0xa: /* User data access */
            switch(size) {
                case 1:
                    stb_user(addr, val);
                    break;
                case 2:
                    stw_user(addr, val);
                    break;
                default:
                case 4:
                    stl_user(addr, val);
                    break;
                case 8:
                    stq_user(addr, val);
                    break;
            }
            break;
        case 0xb: /* Supervisor data access */
            switch(size) {
                case 1:
                    stb_kernel(addr, val);
                    break;
                case 2:
                    stw_kernel(addr, val);
                    break;
                default:
                case 4:
                    stl_kernel(addr, val);
                    break;
                case 8:
                    stq_kernel(addr, val);
                    break;
            }
            break;
        case 0xc:  /* I-cache tag */
        case 0xd:  /* I-cache data */
        case 0xe:  /* D-cache tag */
        case 0xf:  /* D-cache data */
        case 0x10: /* I/D-cache flush page */
        case 0x11: /* I/D-cache flush segment */
        case 0x12: /* I/D-cache flush region */
        case 0x13: /* I/D-cache flush context */
        case 0x14: /* I/D-cache flush user */
            break;
        case 0x17: /* Block copy, sta access */
        {
            //  val = src
            //  addr = dst
            //  copy 32 bytes
            unsigned int i;
            uint32_t src = val & ~3, dst = addr & ~3, temp;

            for(i = 0; i < 32; i += 4, src += 4, dst += 4) {
                temp = ldl_kernel(src);
                stl_kernel(dst, temp);
            }
        } break;
        case 0x1f: /* Block fill, stda access */
        {
            //  addr = dst
            //  fill 32 bytes with val
            unsigned int i;
            uint32_t dst = addr & 7;

            for(i = 0; i < 32; i += 8, dst += 8) {
                stq_kernel(dst, val);
            }
        } break;
        case 0x1: {
            /* The default case in the switch below just follows the coding
             * convention in this function meaning that no other values on
             * size are expected than 1, 2, 4 and 8 */
            switch(size) {
                case 1:
                    stb_kernel(addr, val);
                    break;
                case 2:
                    stw_kernel(addr, val);
                    break;
                default:
                case 4:
                    stl_kernel(addr, val);
                    break;
                case 8:
                    stq_kernel(addr, val);
                    break;
            }
        } break;
        case 0x20: /* MMU passthrough */
        case 0x1c: /* LEON3 MMU passthrougth */
        {
            switch(size) {
                case 1:
                    stb_phys(addr, val);
                    break;
                case 2:
                    stw_phys(addr, val);
                    break;
                case 4:
                default:
                    stl_phys(addr, val);
                    break;
                case 8:
                    stq_phys(addr, val);
                    break;
            }
        } break;
        case 0x21 ... 0x2f: /* MMU passthrough, 0x100000000 to 0xfffffffff */
        {
            switch(size) {
                case 1:
                    stb_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32), val);
                    break;
                case 2:
                    stw_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32), val);
                    break;
                case 4:
                default:
                    stl_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32), val);
                    break;
                case 8:
                    stq_phys((target_phys_addr_t)addr | ((target_phys_addr_t)(asi & 0xf) << 32), val);
                    break;
            }
        } break;
        case 0x30:  //  store buffer tags or Turbosparc secondary cache diagnostic
        case 0x31:  //  store buffer data, Ross RT620 I-cache flush or
        //  Turbosparc snoop RAM
        case 0x32:  //  store buffer control or Turbosparc page table
        //  descriptor diagnostic
        case 0x36: /* I-cache flash clear */
        case 0x37: /* D-cache flash clear */
            break;
        case 0x38: /* SuperSPARC MMU Breakpoint Control Registers*/
        {
            int reg = (addr >> 8) & 3;

            switch(reg) {
                case 0: /* Breakpoint Value (Addr) */
                    env->mmubpregs[reg] = (val & 0xfffffffffULL);
                    break;
                case 1: /* Breakpoint Mask */
                    env->mmubpregs[reg] = (val & 0xfffffffffULL);
                    break;
                case 2: /* Breakpoint Control */
                    env->mmubpregs[reg] = (val & 0x7fULL);
                    break;
                case 3: /* Breakpoint Status */
                    env->mmubpregs[reg] = (val & 0xfULL);
                    break;
            }
        } break;
        case 0x49: /* SuperSPARC MMU Counter Breakpoint Value */
            env->mmubpctrv = val & 0xffffffff;
            break;
        case 0x4a: /* SuperSPARC MMU Counter Breakpoint Control */
            env->mmubpctrc = val & 0x3;
            break;
        case 0x4b: /* SuperSPARC MMU Counter Breakpoint Status */
            env->mmubpctrs = val & 0x3;
            break;
        case 0x4c: /* SuperSPARC MMU Breakpoint Action */
            env->mmubpaction = val & 0x1fff;
            break;
        case 8: /* User code access, XXX */
        case 9: /* Supervisor code access, XXX */
        default:
            do_unassigned_access(addr, 1, 0, asi, size);
            break;
    }
}

target_ulong helper_cas_asi(target_ulong addr, target_ulong val1, target_ulong val2, uint32_t asi)
{
    target_ulong ret;

    val1 &= 0xffffffffUL;
    ret = helper_ld_asi(addr, asi, 4, 0);
    ret &= 0xffffffffUL;
    if(val1 == ret) {
        helper_st_asi(addr, val2 & 0xffffffffUL, asi, 4);
    }
    return ret;
}

void helper_rett(void)
{
    unsigned int cwp;

    if(env->psret == 1) {
        raise_exception(TT_ILL_INSN);
    }

    env->psret = 1;
    cwp = cwp_inc(env->cwp + 1);
    if(env->wim & (1 << cwp)) {
        raise_exception(TT_WIN_UNF);
    }
    set_cwp(cwp);
    env->psrs = env->psrps;

    if(env->interrupt_end_callback_enabled) {
        tlib_on_interrupt_end(env->interrupt_index);
    }
}

static target_ulong helper_udiv_common(target_ulong a, target_ulong b, int cc)
{
    int overflow = 0;
    uint64_t x0;
    uint32_t x1;

    x0 = (a & 0xffffffff) | ((int64_t)(env->y) << 32);
    x1 = (b & 0xffffffff);

    if(x1 == 0) {
        raise_exception(TT_DIV_ZERO);
    }

    x0 = x0 / x1;
    if(x0 > 0xffffffff) {
        x0 = 0xffffffff;
        overflow = 1;
    }

    if(cc) {
        env->cc_dst = x0;
        env->cc_src2 = overflow;
        env->cc_op = CC_OP_DIV;
    }
    return x0;
}

target_ulong helper_udiv(target_ulong a, target_ulong b)
{
    return helper_udiv_common(a, b, 0);
}

target_ulong helper_udiv_cc(target_ulong a, target_ulong b)
{
    return helper_udiv_common(a, b, 1);
}

static target_ulong helper_sdiv_common(target_ulong a, target_ulong b, int cc)
{
    int overflow = 0;
    int64_t x0;
    int32_t x1;

    x0 = (a & 0xffffffff) | ((int64_t)(env->y) << 32);
    x1 = (b & 0xffffffff);

    if(x1 == 0) {
        raise_exception(TT_DIV_ZERO);
    }

    x0 = x0 / x1;
    if((int32_t)x0 != x0) {
        x0 = x0 < 0 ? 0x80000000 : 0x7fffffff;
        overflow = 1;
    }

    if(cc) {
        env->cc_dst = x0;
        env->cc_src2 = overflow;
        env->cc_op = CC_OP_DIV;
    }
    return x0;
}

target_ulong helper_sdiv(target_ulong a, target_ulong b)
{
    return helper_sdiv_common(a, b, 0);
}

target_ulong helper_sdiv_cc(target_ulong a, target_ulong b)
{
    return helper_sdiv_common(a, b, 1);
}

void helper_stdf(target_ulong addr, int mem_idx)
{
    helper_check_align(addr, 7);
    switch(mem_idx) {
        case MMU_USER_IDX:
            stfq_user(addr, DT0);
            break;
        case MMU_KERNEL_IDX:
            stfq_kernel(addr, DT0);
            break;
    }
}

void helper_lddf(target_ulong addr, int mem_idx)
{
    helper_check_align(addr, 7);
    switch(mem_idx) {
        case MMU_USER_IDX:
            DT0 = ldfq_user(addr);
            break;
        case MMU_KERNEL_IDX:
            DT0 = ldfq_kernel(addr);
            break;
    }
}

void helper_ldqf(target_ulong addr, int mem_idx)
{
    //  XXX add 128 bit load
    CPU_QuadU u;

    helper_check_align(addr, 7);
    switch(mem_idx) {
        case MMU_USER_IDX:
            u.ll.upper = ldq_user(addr);
            u.ll.lower = ldq_user(addr + 8);
            QT0 = u.q;
            break;
        case MMU_KERNEL_IDX:
            u.ll.upper = ldq_kernel(addr);
            u.ll.lower = ldq_kernel(addr + 8);
            QT0 = u.q;
            break;
    }
}

void helper_stqf(target_ulong addr, int mem_idx)
{
    //  XXX add 128 bit store
    CPU_QuadU u;

    helper_check_align(addr, 7);
    switch(mem_idx) {
        case MMU_USER_IDX:
            u.q = QT0;
            stq_user(addr, u.ll.upper);
            stq_user(addr + 8, u.ll.lower);
            break;
        case MMU_KERNEL_IDX:
            u.q = QT0;
            stq_kernel(addr, u.ll.upper);
            stq_kernel(addr + 8, u.ll.lower);
            break;
    }
}

static inline void set_fsr(void)
{
    int rnd_mode;

    switch(env->fsr & FSR_RD_MASK) {
        case FSR_RD_NEAREST:
            rnd_mode = float_round_nearest_even;
            break;
        default:
        case FSR_RD_ZERO:
            rnd_mode = float_round_to_zero;
            break;
        case FSR_RD_POS:
            rnd_mode = float_round_up;
            break;
        case FSR_RD_NEG:
            rnd_mode = float_round_down;
            break;
    }
    set_float_rounding_mode(rnd_mode, &env->fp_status);
}

void helper_ldfsr(uint32_t new_fsr)
{
    env->fsr = (new_fsr & FSR_LDFSR_MASK) | (env->fsr & FSR_LDFSR_OLDMASK);
    set_fsr();
}

void helper_debug(void)
{
    env->exception_index = EXCP_DEBUG;
    cpu_loop_exit(env);
}

/* XXX: use another pointer for %iN registers to avoid slow wrapping
   handling ? */
void helper_save(void)
{
    uint32_t cwp;

    cwp = cwp_dec(env->cwp - 1);
    if(env->wim & (1 << cwp)) {
        raise_exception(TT_WIN_OVF);
    }
    set_cwp(cwp);
}

void helper_restore(void)
{
    uint32_t cwp;

    cwp = cwp_inc(env->cwp + 1);
    if(env->wim & (1 << cwp)) {
        raise_exception(TT_WIN_UNF);
    }
    set_cwp(cwp);
}

void helper_wrpsr(target_ulong new_psr)
{
    if((new_psr & PSR_CWP) >= env->nwindows) {
        raise_exception(TT_ILL_INSN);
    } else {
        cpu_put_psr(env, new_psr);
    }
}

target_ulong helper_rdpsr(void)
{
    return get_psr();
}

void do_unaligned_access(target_ulong addr, int is_write, int is_user, void *retaddr)
{
    cpu_restore_state(env, retaddr);
    raise_exception(TT_UNALIGNED);
}

void arch_raise_mmu_fault_exception(CPUState *env, int errcode, int access_type, target_ulong address, void *retaddr)
{
    cpu_restore_state(env, retaddr);
    cpu_loop_exit(env);
}

/* try to fill the TLB and return an exception if error. If retaddr is
   NULL, it means that the function was called in C code (i.e. not
   from generated code or from helper.c) */
/* XXX: fix it to restore all registers */
int arch_tlb_fill(CPUState *env, target_ulong addr, int access_type, int mmu_idx, void *retaddr, int no_page_fault,
                  int access_width, target_phys_addr_t *paddr)
{
    return cpu_handle_mmu_fault(env, addr, access_type, mmu_idx, 1, no_page_fault, paddr);
}

static void do_unassigned_access(target_phys_addr_t addr, int is_write, int is_exec, int is_asi, int size)
{
    int fault_type;

    /* XXX: hack to restore env in all cases, even if not called from
       generated code */
    /* Don't overwrite translation and access faults */
    fault_type = (env->mmuregs[3] & 0x1c) >> 2;
    if((fault_type > 4) || (fault_type == 0)) {
        env->mmuregs[3] = 0; /* Fault status register */
        if(is_asi) {
            env->mmuregs[3] |= 1 << 16;
        }
        if(env->psrs) {
            env->mmuregs[3] |= 1 << 5;
        }
        if(is_exec) {
            env->mmuregs[3] |= 1 << 6;
        }
        if(is_write) {
            env->mmuregs[3] |= 1 << 7;
        }
        env->mmuregs[3] |= (5 << 2) | 2;
        /* SuperSPARC will never place instruction fault addresses in the FAR */
        if(!is_exec) {
            env->mmuregs[4] = addr; /* Fault address register */
        }
    }
    /* overflow (same type fault was not read before another fault) */
    if(fault_type == ((env->mmuregs[3] & 0x1c)) >> 2) {
        env->mmuregs[3] |= 1;
    }

    if((env->mmuregs[0] & MMU_E) && !(env->mmuregs[0] & MMU_NF)) {
        if(is_exec) {
            raise_exception(TT_CODE_ACCESS);
        } else {
            raise_exception(TT_DATA_ACCESS);
        }
    }

    /* flush neverland mappings created during no-fault mode,
       so the sequential MMU faults report proper fault types */
    if(env->mmuregs[0] & MMU_NF) {
        tlb_flush(env, 1, true);
    }
}
