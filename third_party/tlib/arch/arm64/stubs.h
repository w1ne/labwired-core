//  STUB HELPERS

//  Because pre C23 requires one fixed argument before the variadic arguments two different stub macros are needed
#define FUNC_STUB(name)                    \
    static inline int name(int dummy, ...) \
    {                                      \
        (void)dummy;                       \
        return stub_abort(#name);          \
    }

#define FUNC_STUB_PTR(name)                        \
    static inline int name(const void *dummy, ...) \
    {                                              \
        (void)dummy;                               \
        return stub_abort(#name);                  \
    }

#define FUNC_STUB_GENERIC(name, type)       \
    static inline type name(int dummy, ...) \
    {                                       \
        (void)dummy;                        \
        return (type)stub_abort(#name);     \
    }

#define FUNC_STUB_GENERIC_PTR(name, type)           \
    static inline type name(const void *dummy, ...) \
    {                                               \
        (void)dummy;                                \
        return (type)stub_abort(#name);             \
    }

//  Call directly for consts.
static inline int64_t stub_abort(char *name)
{
    tlib_abortf("Stub encountered: %s", name);
    return 0;
}

#define unimplemented(...)                     \
    tlib_abortf("%s unimplemented", __func__); \
    __builtin_unreachable()

//  STUBS emitting warnings instead of aborting simulation (e.g. used by Linux).
//  TODO: Implement properly.

static inline bool semihosting_enabled(bool arg)
{
    tlib_printf(LOG_LEVEL_DEBUG, "Stub encountered: semihosting_enabled(); returning false");
    return false;
}

#define DisasContext void
static inline bool disas_mve(DisasContext *dc, uint32_t insn)
{
    tlib_printf(LOG_LEVEL_DEBUG, "Stub encountered: disas_mve(); returning false");
    return false;
}
#undef DisasContext

//  STUBS

//  These were declared in 'cpu.h' and used by non-skipped sources but we have no
//  implementations. Remember to reenable their declarations after implementing.
FUNC_STUB_PTR(write_v7m_exception)

typedef struct MemTxAttrs {
    int secure;
    int target_tlb_bit0;
    int target_tlb_bit1;
} MemTxAttrs;

#define HAVE_CMPXCHG128 stub_abort("HAVE_CMPXCHG128")
FUNC_STUB(cpu_atomic_cmpxchgo_be_mmu)
FUNC_STUB(cpu_atomic_cmpxchgo_le_mmu)
FUNC_STUB_PTR(probe_access)
FUNC_STUB_GENERIC_PTR(probe_write, void *)
FUNC_STUB_GENERIC(tlb_vaddr_to_host, void *)

#define MO_128 stub_abort("MO_128")

//  Couldn't have been ported easily because of a complex softfloat licensing.
#define float16_one_point_five stub_abort("float16_one_point_five")
#define float16_three          stub_abort("float16_three")
#define float16_two            stub_abort("float16_two")
FUNC_STUB(float16_abs)
FUNC_STUB(float16_add)
FUNC_STUB(float16_chs)
FUNC_STUB(float16_compare_quiet)
FUNC_STUB(float16_compare)
FUNC_STUB(float16_div)
FUNC_STUB(float16_is_any_nan)
FUNC_STUB(float16_is_infinity)
FUNC_STUB(float16_is_zero)
FUNC_STUB(float16_max)
FUNC_STUB(float16_maxnum)
FUNC_STUB(float16_min)
FUNC_STUB(float16_minnum)
FUNC_STUB(float16_mul)
FUNC_STUB(float16_muladd)
FUNC_STUB(float16_round_to_int)
FUNC_STUB(float16_sqrt)
FUNC_STUB(float16_squash_input_denormal)
FUNC_STUB(float16_sub)
FUNC_STUB(float16_to_int16)
FUNC_STUB(float16_to_uint16)

FUNC_STUB(cpu_stb_mmuidx_ra)

/* mte_helper.c */

typedef int AddressSpace;
typedef int Error;
typedef int hwaddr;

typedef struct {
    int addr;
    void *container;
} MemoryRegion;

typedef struct {
    MemTxAttrs attrs;
} CPUIOTLBEntry;

typedef struct {
    CPUIOTLBEntry *iotlb;
} env_tlb_d_struct;

typedef struct {
    env_tlb_d_struct *d;
} env_tlb_struct;

FUNC_STUB_GENERIC_PTR(address_space_translate, void *)
FUNC_STUB(address_with_allocation_tag)
FUNC_STUB(allocation_tag_from_addr)
FUNC_STUB_PTR(arm_cpu_do_unaligned_access)
FUNC_STUB_PTR(cpu_check_watchpoint)
FUNC_STUB_GENERIC_PTR(cpu_get_address_space, void *)
FUNC_STUB(cpu_physical_memory_set_dirty_flag)
FUNC_STUB_GENERIC_PTR(env_tlb, env_tlb_struct *)
FUNC_STUB_PTR(error_free)
FUNC_STUB_PTR(error_get_pretty)
FUNC_STUB_GENERIC_PTR(memory_region_from_host, void *)
FUNC_STUB_PTR(memory_region_get_ram_addr)
FUNC_STUB_GENERIC_PTR(memory_region_get_ram_ptr, void *)
FUNC_STUB_PTR(memory_region_is_ram)
FUNC_STUB_PTR(probe_access_flags)
FUNC_STUB_PTR(qatomic_cmpxchg)
FUNC_STUB_PTR(qatomic_read)
FUNC_STUB_PTR(qatomic_set)
FUNC_STUB_PTR(qemu_guest_getrandom)
FUNC_STUB_PTR(regime_el)
FUNC_STUB(tbi_check)
FUNC_STUB(tcma_check)
FUNC_STUB_PTR(tlb_index)
FUNC_STUB(useronly_clean_ptr)

#define BP_MEM_READ            stub_abort("BP_MEM_READ")
#define BP_MEM_WRITE           stub_abort("BP_MEM_WRITE")
#define DIRTY_MEMORY_MIGRATION stub_abort("DIRTY_MEMORY_MIGRATION")
#define LOG2_TAG_GRANULE       stub_abort("LOG2_TAG_GRANULE")
#define TAG_GRANULE            stub_abort("TAG_GRANULE")
#define TLB_WATCHPOINT         stub_abort("TLB_WATCHPOINT")

#define HWADDR_PRIx "d"

/* op_helper.c */

typedef struct {
    int type;
} ARMMMUFaultInfo;

#define ARMFault_AsyncExternal stub_abort("ARMFault_AsyncExternal")

FUNC_STUB_PTR(arm_cpreg_in_idspace)
FUNC_STUB_PTR(arm_fi_to_lfsc)
FUNC_STUB_PTR(arm_fi_to_sfsc)
FUNC_STUB_PTR(extended_addresses_enabled)
FUNC_STUB(syn_bxjtrap)
FUNC_STUB_PTR(v7m_sp_limit)

/* vfp_helper.c */

typedef int FloatRelation;
typedef int FloatRoundMode;

FUNC_STUB(float16_to_float64)
FUNC_STUB(float16_to_int16_scalbn)
FUNC_STUB(float16_to_int32_round_to_zero)
FUNC_STUB(float16_to_int32_scalbn)
FUNC_STUB(float16_to_int32)
FUNC_STUB(float16_to_int64_scalbn)
FUNC_STUB(float16_to_uint16_scalbn)
FUNC_STUB(float16_to_uint32_round_to_zero)
FUNC_STUB(float16_to_uint32_scalbn)
FUNC_STUB(float16_to_uint32)
FUNC_STUB(float16_to_uint64_scalbn)
FUNC_STUB(float32_to_int16_scalbn)
FUNC_STUB(float32_to_uint16_scalbn)
FUNC_STUB(float64_to_int16_scalbn)
FUNC_STUB(float64_to_uint16_scalbn)
FUNC_STUB(int16_to_float16_scalbn)
FUNC_STUB(int16_to_float32_scalbn)
FUNC_STUB(int16_to_float64_scalbn)
FUNC_STUB(int32_to_float16_scalbn)
FUNC_STUB(int64_to_float16_scalbn)
FUNC_STUB(uint16_to_float16_scalbn)
FUNC_STUB(uint16_to_float32_scalbn)
FUNC_STUB(uint16_to_float64_scalbn)
FUNC_STUB(uint32_to_float16_scalbn)
FUNC_STUB(uint64_to_float16_scalbn)

/* vec_helper.c */

typedef int bfloat16;

#define float16_zero           stub_abort("float16_zero")
#define float_round_to_odd_inf stub_abort("float_round_to_odd_inf")

FUNC_STUB(float16_eq_quiet)
FUNC_STUB(float16_le)
FUNC_STUB(float16_lt)
FUNC_STUB(float16_set_sign)
FUNC_STUB(float16_to_int16_round_to_zero)
FUNC_STUB(float16_to_uint16_round_to_zero)
FUNC_STUB(int16_to_float16)
FUNC_STUB(uint16_to_float16)

/* sve_helper.c */

#define float16_infinity  stub_abort("float16_infinity")
#define float16_one       stub_abort("float16_one")
#define SVE_MTEDESC_SHIFT stub_abort("SVE_MTEDESC_SHIFT")

FUNC_STUB_PTR(cpu_ldl_be_data_ra)
FUNC_STUB_PTR(cpu_ldl_le_data_ra)
FUNC_STUB_PTR(cpu_ldq_be_data_ra)
FUNC_STUB_PTR(cpu_ldq_le_data_ra)
FUNC_STUB_PTR(cpu_ldub_data_ra)
FUNC_STUB_PTR(cpu_lduw_be_data_ra)
FUNC_STUB_PTR(cpu_lduw_le_data_ra)
FUNC_STUB_PTR(cpu_stb_data_ra)
FUNC_STUB_PTR(cpu_stl_be_data_ra)
FUNC_STUB_PTR(cpu_stl_le_data_ra)
FUNC_STUB_PTR(cpu_stq_be_data_ra)
FUNC_STUB_PTR(cpu_stq_le_data_ra)
FUNC_STUB_PTR(cpu_stw_be_data_ra)
FUNC_STUB_PTR(cpu_stw_le_data_ra)
FUNC_STUB_PTR(cpu_watchpoint_address_matches)
FUNC_STUB(float16_is_neg)
FUNC_STUB(float16_scalbn)
FUNC_STUB(float16_to_int64_round_to_zero)
FUNC_STUB(float16_to_uint64_round_to_zero)
FUNC_STUB(float32_to_bfloat16)
FUNC_STUB(float32_to_uint64_round_to_zero)
FUNC_STUB(hswap32)
FUNC_STUB(hswap64)
FUNC_STUB(int32_to_float16)
FUNC_STUB(int64_to_float16)
FUNC_STUB(pow2floor)
FUNC_STUB(uint32_to_float16)
FUNC_STUB(uint64_to_float16)
FUNC_STUB(wswap64)

/* translate-a64.c */

typedef int TCGOp;
typedef struct {
    void *init_disas_context;
    void *tb_start;
    void *insn_start;
    void *translate_insn;
    void *tb_stop;
    void *disas_log;
} TranslatorOps;

#define R_SVCR_SM_MASK      stub_abort("R_SVCR_SM_MASK")
#define R_SVCR_ZA_MASK      stub_abort("R_SVCR_ZA_MASK")
#define SME_ET_AccessTrap   stub_abort("SME_ET_AccessTrap")
#define SME_ET_InactiveZA   stub_abort("SME_ET_InactiveZA")
#define SME_ET_NotStreaming stub_abort("SME_ET_NotStreaming")
#define SME_ET_Streaming    stub_abort("SME_ET_Streaming")
#define gen_io_start()      stub_abort("gen_io_start")
FUNC_STUB_GENERIC(get_arm_cp_reginfo, void *)
FUNC_STUB(target_disas)
#define tcg_last_op() (void *)stub_abort("tcg_last_op")
FUNC_STUB_GENERIC_PTR(tlb_entry, void *)
FUNC_STUB(tlb_hit)
FUNC_STUB(translator_ldl_swap)
FUNC_STUB(translator_lduw_swap)
FUNC_STUB(translator_loop_temp_check)
FUNC_STUB(translator_use_goto_tb)
FUNC_STUB_GENERIC(crypto_sm3tt2b, void)
FUNC_STUB(gen_helper_autda)
FUNC_STUB(gen_helper_autdb)
FUNC_STUB(gen_helper_autia)
FUNC_STUB(gen_helper_autib)
FUNC_STUB(gen_helper_exception_pc_alignment)
FUNC_STUB(gen_helper_exception_swstep)
FUNC_STUB(gen_helper_exit_atomic)
FUNC_STUB(gen_helper_neon_acge_f64)
FUNC_STUB(gen_helper_neon_acgt_f64)
FUNC_STUB(gen_helper_neon_qabs_s64)
FUNC_STUB(gen_helper_neon_qneg_s64)
FUNC_STUB(gen_helper_neon_rbit_u8)
FUNC_STUB(gen_helper_neon_sqadd_u16)
FUNC_STUB(gen_helper_neon_sqadd_u32)
FUNC_STUB(gen_helper_neon_sqadd_u64)
FUNC_STUB(gen_helper_neon_sqadd_u8)
FUNC_STUB(gen_helper_neon_uqadd_s16)
FUNC_STUB(gen_helper_neon_uqadd_s32)
FUNC_STUB(gen_helper_neon_uqadd_s64)
FUNC_STUB(gen_helper_neon_uqadd_s8)
FUNC_STUB(gen_helper_pacda)
FUNC_STUB(gen_helper_pacdb)
FUNC_STUB(gen_helper_pacga)
FUNC_STUB(gen_helper_pacia)
FUNC_STUB(gen_helper_pacib)
FUNC_STUB(gen_helper_set_pstate_sm)
FUNC_STUB(gen_helper_set_pstate_za)
FUNC_STUB(gen_helper_xpacd)
FUNC_STUB(gen_helper_xpaci)
FUNC_STUB_PTR(disas_m_nocp)

/* translate.c */

#define EXC_RETURN_MIN_MAGIC stub_abort("EXC_RETURN_MIN_MAGIC")
#define FNC_RETURN_MIN_MAGIC stub_abort("FNC_RETURN_MIN_MAGIC")

#define TCG_TARGET_HAS_add2_i32 0  //  TODO: Port add2_i32 from TCG

FUNC_STUB_PTR(regime_is_secure)
FUNC_STUB_PTR(tcg_remove_ops_after)
FUNC_STUB(translator_loop)

/* translate.c */

FUNC_STUB(gen_helper_crc32)
FUNC_STUB(gen_helper_crc32c)
FUNC_STUB(gen_helper_mve_sqshl)
FUNC_STUB(gen_helper_mve_sqshll)
FUNC_STUB(gen_helper_mve_uqshl)
FUNC_STUB(gen_helper_mve_uqshll)
FUNC_STUB(gen_helper_mve_vctp)
FUNC_STUB(gen_helper_rebuild_hflags_a32_newel)
FUNC_STUB(gen_helper_rebuild_hflags_m32)
FUNC_STUB(gen_helper_rebuild_hflags_m32_newel)
FUNC_STUB(gen_helper_v7m_blxns)
FUNC_STUB(gen_helper_v7m_bxns)
FUNC_STUB(gen_helper_v7m_mrs)
FUNC_STUB(gen_helper_v7m_msr)
FUNC_STUB(gen_helper_v7m_tt)

/* translate-neon.c */

FUNC_STUB_GENERIC(gvec_fceq0_h, void)
FUNC_STUB_GENERIC(gvec_fcge0_h, void)
FUNC_STUB_GENERIC(gvec_fcgt0_h, void)
FUNC_STUB_GENERIC(gvec_fcle0_h, void)
FUNC_STUB_GENERIC(gvec_fclt0_h, void)

/* translate-sve.c */

#define float16_half              stub_abort("float16_half")
#define float_round_to_odd        stub_abort("float_round_to_odd")
#define TCG_TARGET_HAS_bitsel_vec 0  //  TODO: Port bitsel_vec from TCG

FUNC_STUB(pow2ceil)
FUNC_STUB(tcg_const_local_ptr)
FUNC_STUB(tcg_gen_brcondi_ptr)
FUNC_STUB(tcg_gen_ctpop_i64)
FUNC_STUB(tcg_gen_mov_ptr)
FUNC_STUB(tcg_gen_trunc_i64_ptr)
FUNC_STUB(tcg_temp_local_new_ptr)

/* translate-vfp.c */

#define ECI_A0A1A2B0            stub_abort("ECI_A0A1A2B0")
#define ECI_A0A1A2              stub_abort("ECI_A0A1A2")
#define ECI_A0A1                stub_abort("ECI_A0A1")
#define ECI_A0                  stub_abort("ECI_A0")
#define ECI_NONE                stub_abort("ECI_NONE")
#define R_V7M_CONTROL_FPCA_MASK stub_abort("R_V7M_CONTROL_FPCA_MASK")
#define R_V7M_CONTROL_SFPA_MASK stub_abort("R_V7M_CONTROL_SFPA_MASK")
#define R_V7M_FPCCR_S_MASK      stub_abort("R_V7M_FPCCR_S_MASK")

FUNC_STUB(gen_helper_v7m_preserve_fp_state)

//  In 'helper.c' there are additional stubs for functions declared but unimplemented.

/* decode-sve.c.inc included in translate-sve.c */

FUNC_STUB_PTR(trans_FADD_zpzi)
FUNC_STUB_PTR(trans_FSUB_zpzi)
FUNC_STUB_PTR(trans_FMUL_zpzi)
FUNC_STUB_PTR(trans_FSUBR_zpzi)
FUNC_STUB_PTR(trans_FMAXNM_zpzi)
FUNC_STUB_PTR(trans_FMINNM_zpzi)
FUNC_STUB_PTR(trans_FMAX_zpzi)
FUNC_STUB_PTR(trans_FMIN_zpzi)

/* mte_helper.c */

#define __REGISTER_MTEDESC_MIDX_START   stub_abort("__REGISTER_MTEDESC_MIDX_START")
#define __REGISTER_MTEDESC_MIDX_WIDTH   stub_abort("__REGISTER_MTEDESC_MIDX_WIDTH")
#define __REGISTER_MTEDESC_SIZEM1_START stub_abort("__REGISTER_MTEDESC_SIZEM1_START")
#define __REGISTER_MTEDESC_SIZEM1_WIDTH stub_abort("__REGISTER_MTEDESC_SIZEM1_WIDTH")
#define __REGISTER_MTEDESC_WRITE_START  stub_abort("__REGISTER_MTEDESC_WRITE_START")
#define __REGISTER_MTEDESC_WRITE_WIDTH  stub_abort("__REGISTER_MTEDESC_WRITE_WIDTH")

/* sve_helper.c */

#define __REGISTER_PREDDESC_DATA_START  stub_abort("__REGISTER_PREDDESC_DATA_START")
#define __REGISTER_PREDDESC_DATA_WIDTH  stub_abort("__REGISTER_PREDDESC_DATA_WIDTH")
#define __REGISTER_PREDDESC_ESZ_START   stub_abort("__REGISTER_PREDDESC_ESZ_START")
#define __REGISTER_PREDDESC_ESZ_WIDTH   stub_abort("__REGISTER_PREDDESC_ESZ_WIDTH")
#define __REGISTER_PREDDESC_OPRSZ_START stub_abort("__REGISTER_PREDDESC_OPRSZ_START")
#define __REGISTER_PREDDESC_OPRSZ_WIDTH stub_abort("__REGISTER_PREDDESC_OPRSZ_WIDTH")

/* translate-a64.c */

#define __REGISTER_MTEDESC_TBI_START  stub_abort("__REGISTER_MTEDESC_TBI_START")
#define __REGISTER_MTEDESC_TBI_WIDTH  stub_abort("__REGISTER_MTEDESC_TBI_WIDTH")
#define __REGISTER_MTEDESC_TCMA_START stub_abort("__REGISTER_MTEDESC_TCMA_START")
#define __REGISTER_MTEDESC_TCMA_WIDTH stub_abort("__REGISTER_MTEDESC_TCMA_WIDTH")

//  Prototyped in translate-a32.h
FUNC_STUB_PTR(mve_eci_check)
FUNC_STUB_PTR(mve_update_eci)
FUNC_STUB_PTR(mve_update_and_store_eci)
