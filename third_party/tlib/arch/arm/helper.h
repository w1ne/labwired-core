#include <stdint.h>

int get_phys_addr(CPUState *env, uint32_t address, bool is_secure, int access_type, bool is_user, uint32_t *phys_ptr, int *prot,
                  target_ulong *page_size, int no_page_fault);

bool is_impl_def_exempt_from_attribution(uint32_t address, bool *applies_to_whole_page);
bool try_get_impl_def_attr_exemption_region(uint32_t address, uint32_t start_at, uint32_t *found_index,
                                            bool *applies_to_whole_page);

int arm_rmode_to_sf(enum arm_fpdecoderm rmode);

#include "def-helper.h"

DEF_HELPER_1(clz, i32, i32)
DEF_HELPER_1(sxtb16, i32, i32)
DEF_HELPER_1(uxtb16, i32, i32)

DEF_HELPER_2(add_setq, i32, i32, i32)
DEF_HELPER_2(add_saturate, i32, i32, i32)
DEF_HELPER_2(sub_saturate, i32, i32, i32)
DEF_HELPER_2(add_usaturate, i32, i32, i32)
DEF_HELPER_2(sub_usaturate, i32, i32, i32)
DEF_HELPER_1(double_saturate, i32, s32)
DEF_HELPER_3(sdiv, s32, env, s32, s32)
DEF_HELPER_3(udiv, i32, env, i32, i32)
DEF_HELPER_1(rbit, i32, i32)
DEF_HELPER_1(abs, i32, i32)

#define PAS_OP(pfx)                                \
    DEF_HELPER_3(pfx##add8, i32, i32, i32, ptr)    \
    DEF_HELPER_3(pfx##sub8, i32, i32, i32, ptr)    \
    DEF_HELPER_3(pfx##sub16, i32, i32, i32, ptr)   \
    DEF_HELPER_3(pfx##add16, i32, i32, i32, ptr)   \
    DEF_HELPER_3(pfx##addsubx, i32, i32, i32, ptr) \
    DEF_HELPER_3(pfx##subaddx, i32, i32, i32, ptr)

PAS_OP(s)
PAS_OP(u)
#undef PAS_OP

#define PAS_OP(pfx)                           \
    DEF_HELPER_2(pfx##add8, i32, i32, i32)    \
    DEF_HELPER_2(pfx##sub8, i32, i32, i32)    \
    DEF_HELPER_2(pfx##sub16, i32, i32, i32)   \
    DEF_HELPER_2(pfx##add16, i32, i32, i32)   \
    DEF_HELPER_2(pfx##addsubx, i32, i32, i32) \
    DEF_HELPER_2(pfx##subaddx, i32, i32, i32)
PAS_OP(q)
PAS_OP(sh)
PAS_OP(uq)
PAS_OP(uh)
#undef PAS_OP

DEF_HELPER_2(ssat, i32, i32, i32)
DEF_HELPER_2(usat, i32, i32, i32)
DEF_HELPER_2(ssat16, i32, i32, i32)
DEF_HELPER_2(usat16, i32, i32, i32)

DEF_HELPER_2(usad8, i32, i32, i32)

DEF_HELPER_1(logicq_cc, i32, i64)

DEF_HELPER_3(sel_flags, i32, i32, i32, i32)
DEF_HELPER_1(exception, void, i32)
DEF_HELPER_0(wfi, void)
DEF_HELPER_0(wfe, void)

DEF_HELPER_2(cpsr_write, void, i32, i32)
DEF_HELPER_0(cpsr_read, i32)

DEF_HELPER_2(set_rmode, i32, i32, ptr)

#ifdef TARGET_PROTO_ARM_M
DEF_HELPER_3(v7m_msr, void, env, i32, i32)
DEF_HELPER_2(v7m_mrs, i32, env, i32)

DEF_HELPER_1(fp_lsp_no_context, void, env)
DEF_HELPER_1(fp_lsp, void, env)

DEF_HELPER_3(v8m_blxns, void, env, i32, i32)
DEF_HELPER_1(v8m_sg, void, env)

DEF_HELPER_2(v8m_bx_update_pc, void, env, i32)

DEF_HELPER_2(v8m_vlstm, void, env, i32)
DEF_HELPER_2(v8m_vlldm, void, env, i32)
#endif

DEF_HELPER_4(set_cp15_64bit, void, env, i32, i32, i32)
DEF_HELPER_2(get_cp15_64bit, i64, env, i32)

DEF_HELPER_3(set_cp15_32bit, void, env, i32, i32)
DEF_HELPER_2(get_cp15_32bit, i32, env, i32)

DEF_HELPER_2(get_r13_banked, i32, env, i32)
DEF_HELPER_3(set_r13_banked, void, env, i32, i32)

DEF_HELPER_1(get_user_reg, i32, i32)
DEF_HELPER_2(set_user_reg, void, i32, i32)

DEF_HELPER_1(vfp_get_fpscr, i32, env)
DEF_HELPER_2(vfp_set_fpscr, void, env, i32)

#ifdef TARGET_PROTO_ARM_M
DEF_HELPER_1(vfp_get_vpr_p0, i32, env)
DEF_HELPER_2(vfp_set_vpr_p0, void, env, i32)
DEF_HELPER_1(is_fpu_context_active, i32, env)
#endif

DEF_HELPER_3(vfp_adds, f32, f32, f32, ptr)
DEF_HELPER_3(vfp_addd, f64, f64, f64, ptr)
DEF_HELPER_3(vfp_subs, f32, f32, f32, ptr)
DEF_HELPER_3(vfp_subd, f64, f64, f64, ptr)
DEF_HELPER_3(vfp_muls, f32, f32, f32, ptr)
DEF_HELPER_3(vfp_muld, f64, f64, f64, ptr)
DEF_HELPER_3(vfp_divs, f32, f32, f32, ptr)
DEF_HELPER_3(vfp_divd, f64, f64, f64, ptr)
DEF_HELPER_1(vfp_negs, f32, f32)
DEF_HELPER_1(vfp_negd, f64, f64)
DEF_HELPER_1(vfp_abss, f32, f32)
DEF_HELPER_1(vfp_absd, f64, f64)
DEF_HELPER_2(vfp_sqrts, f32, f32, env)
DEF_HELPER_2(vfp_sqrtd, f64, f64, env)
DEF_HELPER_3(vfp_cmps, void, f32, f32, env)
DEF_HELPER_3(vfp_cmpd, void, f64, f64, env)
DEF_HELPER_3(vfp_cmpes, void, f32, f32, env)
DEF_HELPER_3(vfp_cmped, void, f64, f64, env)
DEF_HELPER_3(vfp_maxnums, f32, f32, f32, ptr)
DEF_HELPER_3(vfp_maxnumd, f64, f64, f64, ptr)
DEF_HELPER_3(vfp_minnums, f32, f32, f32, ptr)
DEF_HELPER_3(vfp_minnumd, f64, f64, f64, ptr)

DEF_HELPER_2(vfp_fcvtds, f64, f32, env)
DEF_HELPER_2(vfp_fcvtsd, f32, f64, env)

DEF_HELPER_2(vfp_uitos, f32, i32, ptr)
DEF_HELPER_2(vfp_uitod, f64, i32, ptr)
DEF_HELPER_2(vfp_sitos, f32, i32, ptr)
DEF_HELPER_2(vfp_sitod, f64, i32, ptr)

DEF_HELPER_2(vfp_touis, i32, f32, ptr)
DEF_HELPER_2(vfp_touid, i32, f64, ptr)
DEF_HELPER_2(vfp_touizs, i32, f32, ptr)
DEF_HELPER_2(vfp_touizd, i32, f64, ptr)
DEF_HELPER_2(vfp_tosis, i32, f32, ptr)
DEF_HELPER_2(vfp_tosid, i32, f64, ptr)
DEF_HELPER_2(vfp_tosizs, i32, f32, ptr)
DEF_HELPER_2(vfp_tosizd, i32, f64, ptr)

DEF_HELPER_3(vfp_toshs, i32, f32, i32, ptr)
DEF_HELPER_3(vfp_tosls, i32, f32, i32, ptr)
DEF_HELPER_3(vfp_touhs, i32, f32, i32, ptr)
DEF_HELPER_3(vfp_touls, i32, f32, i32, ptr)
DEF_HELPER_3(vfp_toshd, i64, f64, i32, ptr)
DEF_HELPER_3(vfp_tosld, i64, f64, i32, ptr)
DEF_HELPER_3(vfp_touhd, i64, f64, i32, ptr)
DEF_HELPER_3(vfp_tould, i64, f64, i32, ptr)
DEF_HELPER_3(vfp_shtos, f32, i32, i32, ptr)
DEF_HELPER_3(vfp_sltos, f32, i32, i32, ptr)
DEF_HELPER_3(vfp_uhtos, f32, i32, i32, ptr)
DEF_HELPER_3(vfp_ultos, f32, i32, i32, ptr)
DEF_HELPER_3(vfp_shtod, f64, i64, i32, ptr)
DEF_HELPER_3(vfp_sltod, f64, i64, i32, ptr)
DEF_HELPER_3(vfp_uhtod, f64, i64, i32, ptr)
DEF_HELPER_3(vfp_ultod, f64, i64, i32, ptr)

DEF_HELPER_2(vfp_fcvt_f16_to_f32, f32, i32, env)
DEF_HELPER_2(vfp_fcvt_f32_to_f16, i32, f32, env)
DEF_HELPER_2(neon_fcvt_f16_to_f32, f32, i32, env)
DEF_HELPER_2(neon_fcvt_f32_to_f16, i32, f32, env)

DEF_HELPER_4(vfp_muladdd, f64, f64, f64, f64, ptr)
DEF_HELPER_4(vfp_muladds, f32, f32, f32, f32, ptr)

DEF_HELPER_2(rints_exact, f32, f32, ptr)
DEF_HELPER_2(rintd_exact, f64, f64, ptr)
DEF_HELPER_2(rints, f32, f32, ptr)
DEF_HELPER_2(rintd, f64, f64, ptr)

DEF_HELPER_3(recps_f32, f32, f32, f32, env)
DEF_HELPER_3(rsqrts_f32, f32, f32, f32, env)
DEF_HELPER_2(recpe_f32, f32, f32, env)
DEF_HELPER_2(rsqrte_f32, f32, f32, env)
DEF_HELPER_2(recpe_u32, i32, i32, env)
DEF_HELPER_2(rsqrte_u32, i32, i32, env)
DEF_HELPER_4(neon_tbl, i32, i32, i32, i32, i32)

DEF_HELPER_2(add_cc, i32, i32, i32)
DEF_HELPER_2(adc_cc, i32, i32, i32)
DEF_HELPER_2(sub_cc, i32, i32, i32)
DEF_HELPER_2(sbc_cc, i32, i32, i32)

DEF_HELPER_2(shl, i32, i32, i32)
DEF_HELPER_2(shr, i32, i32, i32)
DEF_HELPER_2(sar, i32, i32, i32)
DEF_HELPER_2(shl_cc, i32, i32, i32)
DEF_HELPER_2(shr_cc, i32, i32, i32)
DEF_HELPER_2(sar_cc, i32, i32, i32)
DEF_HELPER_2(ror_cc, i32, i32, i32)

/* neon_helper.c */
DEF_HELPER_3(neon_qadd_u8, i32, env, i32, i32)
DEF_HELPER_3(neon_qadd_s8, i32, env, i32, i32)
DEF_HELPER_3(neon_qadd_u16, i32, env, i32, i32)
DEF_HELPER_3(neon_qadd_s16, i32, env, i32, i32)
DEF_HELPER_3(neon_qadd_u32, i32, env, i32, i32)
DEF_HELPER_3(neon_qadd_s32, i32, env, i32, i32)
DEF_HELPER_3(neon_qsub_u8, i32, env, i32, i32)
DEF_HELPER_3(neon_qsub_s8, i32, env, i32, i32)
DEF_HELPER_3(neon_qsub_u16, i32, env, i32, i32)
DEF_HELPER_3(neon_qsub_s16, i32, env, i32, i32)
DEF_HELPER_3(neon_qsub_u32, i32, env, i32, i32)
DEF_HELPER_3(neon_qsub_s32, i32, env, i32, i32)
DEF_HELPER_3(neon_qadd_u64, i64, env, i64, i64)
DEF_HELPER_3(neon_qadd_s64, i64, env, i64, i64)
DEF_HELPER_3(neon_qsub_u64, i64, env, i64, i64)
DEF_HELPER_3(neon_qsub_s64, i64, env, i64, i64)

DEF_HELPER_2(neon_hadd_s8, i32, i32, i32)
DEF_HELPER_2(neon_hadd_u8, i32, i32, i32)
DEF_HELPER_2(neon_hadd_s16, i32, i32, i32)
DEF_HELPER_2(neon_hadd_u16, i32, i32, i32)
DEF_HELPER_2(neon_hadd_s32, s32, s32, s32)
DEF_HELPER_2(neon_hadd_u32, i32, i32, i32)
DEF_HELPER_2(neon_rhadd_s8, i32, i32, i32)
DEF_HELPER_2(neon_rhadd_u8, i32, i32, i32)
DEF_HELPER_2(neon_rhadd_s16, i32, i32, i32)
DEF_HELPER_2(neon_rhadd_u16, i32, i32, i32)
DEF_HELPER_2(neon_rhadd_s32, s32, s32, s32)
DEF_HELPER_2(neon_rhadd_u32, i32, i32, i32)
DEF_HELPER_2(neon_hsub_s8, i32, i32, i32)
DEF_HELPER_2(neon_hsub_u8, i32, i32, i32)
DEF_HELPER_2(neon_hsub_s16, i32, i32, i32)
DEF_HELPER_2(neon_hsub_u16, i32, i32, i32)
DEF_HELPER_2(neon_hsub_s32, s32, s32, s32)
DEF_HELPER_2(neon_hsub_u32, i32, i32, i32)

DEF_HELPER_2(neon_cgt_u8, i32, i32, i32)
DEF_HELPER_2(neon_cgt_s8, i32, i32, i32)
DEF_HELPER_2(neon_cgt_u16, i32, i32, i32)
DEF_HELPER_2(neon_cgt_s16, i32, i32, i32)
DEF_HELPER_2(neon_cgt_u32, i32, i32, i32)
DEF_HELPER_2(neon_cgt_s32, i32, i32, i32)
DEF_HELPER_2(neon_cge_u8, i32, i32, i32)
DEF_HELPER_2(neon_cge_s8, i32, i32, i32)
DEF_HELPER_2(neon_cge_u16, i32, i32, i32)
DEF_HELPER_2(neon_cge_s16, i32, i32, i32)
DEF_HELPER_2(neon_cge_u32, i32, i32, i32)
DEF_HELPER_2(neon_cge_s32, i32, i32, i32)

DEF_HELPER_2(neon_min_u8, i32, i32, i32)
DEF_HELPER_2(neon_min_s8, i32, i32, i32)
DEF_HELPER_2(neon_min_u16, i32, i32, i32)
DEF_HELPER_2(neon_min_s16, i32, i32, i32)
DEF_HELPER_2(neon_min_u32, i32, i32, i32)
DEF_HELPER_2(neon_min_s32, i32, s32, s32)
DEF_HELPER_2(neon_max_u8, i32, i32, i32)
DEF_HELPER_2(neon_max_s8, i32, i32, i32)
DEF_HELPER_2(neon_max_u16, i32, i32, i32)
DEF_HELPER_2(neon_max_s16, i32, i32, i32)
DEF_HELPER_2(neon_max_u32, i32, i32, i32)
DEF_HELPER_2(neon_max_s32, i32, s32, s32)
DEF_HELPER_2(neon_pmin_u8, i32, i32, i32)
DEF_HELPER_2(neon_pmin_s8, i32, i32, i32)
DEF_HELPER_2(neon_pmin_u16, i32, i32, i32)
DEF_HELPER_2(neon_pmin_s16, i32, i32, i32)
DEF_HELPER_2(neon_pmax_u8, i32, i32, i32)
DEF_HELPER_2(neon_pmax_s8, i32, i32, i32)
DEF_HELPER_2(neon_pmax_u16, i32, i32, i32)
DEF_HELPER_2(neon_pmax_s16, i32, i32, i32)

DEF_HELPER_2(neon_abd_u8, i32, i32, i32)
DEF_HELPER_2(neon_abd_s8, i32, i32, i32)
DEF_HELPER_2(neon_abd_u16, i32, i32, i32)
DEF_HELPER_2(neon_abd_s16, i32, i32, i32)
DEF_HELPER_2(neon_abd_u32, i32, i32, i32)
DEF_HELPER_2(neon_abd_s32, i32, s32, s32)

DEF_HELPER_2(neon_shl_u8, i32, i32, i32)
DEF_HELPER_2(neon_shl_s8, i32, i32, i32)
DEF_HELPER_2(neon_shl_u16, i32, i32, i32)
DEF_HELPER_2(neon_shl_s16, i32, i32, i32)
DEF_HELPER_2(neon_shl_u32, i32, i32, i32)
DEF_HELPER_2(neon_shl_s32, i32, i32, i32)
DEF_HELPER_2(neon_shl_u64, i64, i64, i64)
DEF_HELPER_2(neon_shl_s64, i64, i64, i64)
DEF_HELPER_2(neon_rshl_u8, i32, i32, i32)
DEF_HELPER_2(neon_rshl_s8, i32, i32, i32)
DEF_HELPER_2(neon_rshl_u16, i32, i32, i32)
DEF_HELPER_2(neon_rshl_s16, i32, i32, i32)
DEF_HELPER_2(neon_rshl_u32, i32, i32, i32)
DEF_HELPER_2(neon_rshl_s32, i32, i32, i32)
DEF_HELPER_2(neon_rshl_u64, i64, i64, i64)
DEF_HELPER_2(neon_rshl_s64, i64, i64, i64)
DEF_HELPER_3(neon_qshl_u8, i32, env, i32, i32)
DEF_HELPER_3(neon_qshl_s8, i32, env, i32, i32)
DEF_HELPER_3(neon_qshl_u16, i32, env, i32, i32)
DEF_HELPER_3(neon_qshl_s16, i32, env, i32, i32)
DEF_HELPER_3(neon_qshl_u32, i32, env, i32, i32)
DEF_HELPER_3(neon_qshl_s32, i32, env, i32, i32)
DEF_HELPER_3(neon_qshl_u64, i64, env, i64, i64)
DEF_HELPER_3(neon_qshl_s64, i64, env, i64, i64)
DEF_HELPER_3(neon_qshlu_s8, i32, env, i32, i32);
DEF_HELPER_3(neon_qshlu_s16, i32, env, i32, i32);
DEF_HELPER_3(neon_qshlu_s32, i32, env, i32, i32);
DEF_HELPER_3(neon_qshlu_s64, i64, env, i64, i64);
DEF_HELPER_3(neon_qrshl_u8, i32, env, i32, i32)
DEF_HELPER_3(neon_qrshl_s8, i32, env, i32, i32)
DEF_HELPER_3(neon_qrshl_u16, i32, env, i32, i32)
DEF_HELPER_3(neon_qrshl_s16, i32, env, i32, i32)
DEF_HELPER_3(neon_qrshl_u32, i32, env, i32, i32)
DEF_HELPER_3(neon_qrshl_s32, i32, env, i32, i32)
DEF_HELPER_3(neon_qrshl_u64, i64, env, i64, i64)
DEF_HELPER_3(neon_qrshl_s64, i64, env, i64, i64)

DEF_HELPER_2(neon_add_u8, i32, i32, i32)
DEF_HELPER_2(neon_add_u16, i32, i32, i32)
DEF_HELPER_2(neon_padd_u8, i32, i32, i32)
DEF_HELPER_2(neon_padd_u16, i32, i32, i32)
DEF_HELPER_2(neon_sub_u8, i32, i32, i32)
DEF_HELPER_2(neon_sub_u16, i32, i32, i32)
DEF_HELPER_2(neon_mul_u8, i32, i32, i32)
DEF_HELPER_2(neon_mul_u16, i32, i32, i32)
DEF_HELPER_2(neon_mul_p8, i32, i32, i32)
DEF_HELPER_2(neon_mull_p8, i64, i32, i32)

DEF_HELPER_2(neon_tst_u8, i32, i32, i32)
DEF_HELPER_2(neon_tst_u16, i32, i32, i32)
DEF_HELPER_2(neon_tst_u32, i32, i32, i32)
DEF_HELPER_2(neon_ceq_u8, i32, i32, i32)
DEF_HELPER_2(neon_ceq_u16, i32, i32, i32)
DEF_HELPER_2(neon_ceq_u32, i32, i32, i32)

DEF_HELPER_1(neon_abs_s8, i32, i32)
DEF_HELPER_1(neon_abs_s16, i32, i32)
DEF_HELPER_1(neon_clz_u8, i32, i32)
DEF_HELPER_1(neon_clz_u16, i32, i32)
DEF_HELPER_1(neon_cls_s8, i32, i32)
DEF_HELPER_1(neon_cls_s16, i32, i32)
DEF_HELPER_1(neon_cls_s32, i32, i32)
DEF_HELPER_1(neon_cnt_u8, i32, i32)

DEF_HELPER_3(neon_qdmulh_s16, i32, env, i32, i32)
DEF_HELPER_3(neon_qrdmulh_s16, i32, env, i32, i32)
DEF_HELPER_3(neon_qdmulh_s32, i32, env, i32, i32)
DEF_HELPER_3(neon_qrdmulh_s32, i32, env, i32, i32)

DEF_HELPER_1(neon_narrow_u8, i32, i64)
DEF_HELPER_1(neon_narrow_u16, i32, i64)
DEF_HELPER_2(neon_unarrow_sat8, i32, env, i64)
DEF_HELPER_2(neon_narrow_sat_u8, i32, env, i64)
DEF_HELPER_2(neon_narrow_sat_s8, i32, env, i64)
DEF_HELPER_2(neon_unarrow_sat16, i32, env, i64)
DEF_HELPER_2(neon_narrow_sat_u16, i32, env, i64)
DEF_HELPER_2(neon_narrow_sat_s16, i32, env, i64)
DEF_HELPER_2(neon_unarrow_sat32, i32, env, i64)
DEF_HELPER_2(neon_narrow_sat_u32, i32, env, i64)
DEF_HELPER_2(neon_narrow_sat_s32, i32, env, i64)
DEF_HELPER_1(neon_narrow_high_u8, i32, i64)
DEF_HELPER_1(neon_narrow_high_u16, i32, i64)
DEF_HELPER_1(neon_narrow_round_high_u8, i32, i64)
DEF_HELPER_1(neon_narrow_round_high_u16, i32, i64)
DEF_HELPER_1(neon_widen_u8, i64, i32)
DEF_HELPER_1(neon_widen_s8, i64, i32)
DEF_HELPER_1(neon_widen_u16, i64, i32)
DEF_HELPER_1(neon_widen_s16, i64, i32)

DEF_HELPER_2(neon_addl_u16, i64, i64, i64)
DEF_HELPER_2(neon_addl_u32, i64, i64, i64)
DEF_HELPER_2(neon_paddl_u16, i64, i64, i64)
DEF_HELPER_2(neon_paddl_u32, i64, i64, i64)
DEF_HELPER_2(neon_subl_u16, i64, i64, i64)
DEF_HELPER_2(neon_subl_u32, i64, i64, i64)
DEF_HELPER_3(neon_addl_saturate_s32, i64, env, i64, i64)
DEF_HELPER_3(neon_addl_saturate_s64, i64, env, i64, i64)
DEF_HELPER_2(neon_abdl_u16, i64, i32, i32)
DEF_HELPER_2(neon_abdl_s16, i64, i32, i32)
DEF_HELPER_2(neon_abdl_u32, i64, i32, i32)
DEF_HELPER_2(neon_abdl_s32, i64, i32, i32)
DEF_HELPER_2(neon_abdl_u64, i64, i32, i32)
DEF_HELPER_2(neon_abdl_s64, i64, i32, i32)
DEF_HELPER_2(neon_mull_u8, i64, i32, i32)
DEF_HELPER_2(neon_mull_s8, i64, i32, i32)
DEF_HELPER_2(neon_mull_u16, i64, i32, i32)
DEF_HELPER_2(neon_mull_s16, i64, i32, i32)

DEF_HELPER_1(neon_negl_u16, i64, i64)
DEF_HELPER_1(neon_negl_u32, i64, i64)

DEF_HELPER_2(neon_qabs_s8, i32, env, i32)
DEF_HELPER_2(neon_qabs_s16, i32, env, i32)
DEF_HELPER_2(neon_qabs_s32, i32, env, i32)
DEF_HELPER_2(neon_qneg_s8, i32, env, i32)
DEF_HELPER_2(neon_qneg_s16, i32, env, i32)
DEF_HELPER_2(neon_qneg_s32, i32, env, i32)

DEF_HELPER_3(neon_min_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_max_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_abd_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_ceq_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_cge_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_cgt_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_acge_f32, i32, i32, i32, ptr)
DEF_HELPER_3(neon_acgt_f32, i32, i32, i32, ptr)

/* iwmmxt_helper.c */
DEF_HELPER_2(iwmmxt_maddsq, i64, i64, i64)
DEF_HELPER_2(iwmmxt_madduq, i64, i64, i64)
DEF_HELPER_2(iwmmxt_sadb, i64, i64, i64)
DEF_HELPER_2(iwmmxt_sadw, i64, i64, i64)
DEF_HELPER_2(iwmmxt_mulslw, i64, i64, i64)
DEF_HELPER_2(iwmmxt_mulshw, i64, i64, i64)
DEF_HELPER_2(iwmmxt_mululw, i64, i64, i64)
DEF_HELPER_2(iwmmxt_muluhw, i64, i64, i64)
DEF_HELPER_2(iwmmxt_macsw, i64, i64, i64)
DEF_HELPER_2(iwmmxt_macuw, i64, i64, i64)
DEF_HELPER_1(iwmmxt_setpsr_nz, i32, i64)

#define DEF_IWMMXT_HELPER_SIZE_ENV(name)               \
    DEF_HELPER_3(iwmmxt_##name##b, i64, env, i64, i64) \
    DEF_HELPER_3(iwmmxt_##name##w, i64, env, i64, i64) \
    DEF_HELPER_3(iwmmxt_##name##l, i64, env, i64, i64)

DEF_IWMMXT_HELPER_SIZE_ENV(unpackl)
DEF_IWMMXT_HELPER_SIZE_ENV(unpackh)

DEF_HELPER_2(iwmmxt_unpacklub, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackluw, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpacklul, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackhub, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackhuw, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackhul, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpacklsb, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpacklsw, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpacklsl, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackhsb, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackhsw, i64, env, i64)
DEF_HELPER_2(iwmmxt_unpackhsl, i64, env, i64)

DEF_IWMMXT_HELPER_SIZE_ENV(cmpeq)
DEF_IWMMXT_HELPER_SIZE_ENV(cmpgtu)
DEF_IWMMXT_HELPER_SIZE_ENV(cmpgts)

DEF_IWMMXT_HELPER_SIZE_ENV(mins)
DEF_IWMMXT_HELPER_SIZE_ENV(minu)
DEF_IWMMXT_HELPER_SIZE_ENV(maxs)
DEF_IWMMXT_HELPER_SIZE_ENV(maxu)

DEF_IWMMXT_HELPER_SIZE_ENV(subn)
DEF_IWMMXT_HELPER_SIZE_ENV(addn)
DEF_IWMMXT_HELPER_SIZE_ENV(subu)
DEF_IWMMXT_HELPER_SIZE_ENV(addu)
DEF_IWMMXT_HELPER_SIZE_ENV(subs)
DEF_IWMMXT_HELPER_SIZE_ENV(adds)

DEF_HELPER_3(iwmmxt_avgb0, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_avgb1, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_avgw0, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_avgw1, i64, env, i64, i64)

DEF_HELPER_2(iwmmxt_msadb, i64, i64, i64)

DEF_HELPER_3(iwmmxt_align, i64, i64, i64, i32)
DEF_HELPER_4(iwmmxt_insr, i64, i64, i32, i32, i32)

DEF_HELPER_1(iwmmxt_bcstb, i64, i32)
DEF_HELPER_1(iwmmxt_bcstw, i64, i32)
DEF_HELPER_1(iwmmxt_bcstl, i64, i32)

DEF_HELPER_1(iwmmxt_addcb, i64, i64)
DEF_HELPER_1(iwmmxt_addcw, i64, i64)
DEF_HELPER_1(iwmmxt_addcl, i64, i64)

DEF_HELPER_1(iwmmxt_msbb, i32, i64)
DEF_HELPER_1(iwmmxt_msbw, i32, i64)
DEF_HELPER_1(iwmmxt_msbl, i32, i64)

DEF_HELPER_3(iwmmxt_srlw, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_srll, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_srlq, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_sllw, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_slll, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_sllq, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_sraw, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_sral, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_sraq, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_rorw, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_rorl, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_rorq, i64, env, i64, i32)
DEF_HELPER_3(iwmmxt_shufh, i64, env, i64, i32)

DEF_HELPER_3(iwmmxt_packuw, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_packul, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_packuq, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_packsw, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_packsl, i64, env, i64, i64)
DEF_HELPER_3(iwmmxt_packsq, i64, env, i64, i64)

DEF_HELPER_3(iwmmxt_muladdsl, i64, i64, i32, i32)
DEF_HELPER_3(iwmmxt_muladdsw, i64, i64, i32, i32)
DEF_HELPER_3(iwmmxt_muladdswl, i64, i64, i32, i32)

DEF_HELPER_2(set_teecr, void, env, i32)

DEF_HELPER_3(neon_unzip8, void, env, i32, i32)
DEF_HELPER_3(neon_unzip16, void, env, i32, i32)
DEF_HELPER_3(neon_qunzip8, void, env, i32, i32)
DEF_HELPER_3(neon_qunzip16, void, env, i32, i32)
DEF_HELPER_3(neon_qunzip32, void, env, i32, i32)
DEF_HELPER_3(neon_zip8, void, env, i32, i32)
DEF_HELPER_3(neon_zip16, void, env, i32, i32)
DEF_HELPER_3(neon_qzip8, void, env, i32, i32)
DEF_HELPER_3(neon_qzip16, void, env, i32, i32)
DEF_HELPER_3(neon_qzip32, void, env, i32, i32)

DEF_HELPER_0(set_system_event, void)

#ifdef TARGET_PROTO_ARM_M
DEF_HELPER_3(v8m_tt, i32, env, i32, i32)
#endif
DEF_HELPER_3(set_cp_reg, void, env, ptr, i32)
DEF_HELPER_3(set_cp_reg64, void, env, ptr, i64)
DEF_HELPER_2(get_cp_reg, i32, env, ptr)
DEF_HELPER_2(get_cp_reg64, i64, env, ptr)

DEF_HELPER_3(pmu_update_event_counters, void, env, int, i32)
DEF_HELPER_1(pmu_count_instructions_cycles, void, i32)

#ifdef TARGET_PROTO_ARM_M
DEF_HELPER_2(mve_vctp, void, env, i32)

DEF_HELPER_3(mve_vldrb, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrh, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrw, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrb_sh, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrb_sw, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrb_uh, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrb_uw, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrh_sw, void, env, ptr, i32)
DEF_HELPER_3(mve_vldrh_uw, void, env, ptr, i32)

DEF_HELPER_3(mve_vstrb, void, env, ptr, i32)
DEF_HELPER_3(mve_vstrh, void, env, ptr, i32)
DEF_HELPER_3(mve_vstrw, void, env, ptr, i32)
DEF_HELPER_3(mve_vstrb_h, void, env, ptr, i32)
DEF_HELPER_3(mve_vstrb_w, void, env, ptr, i32)
DEF_HELPER_3(mve_vstrh_w, void, env, ptr, i32)

DEF_HELPER_3(mve_vdup, void, env, ptr, i32)

DEF_HELPER_3(mve_vfcmp_eqs, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfcmp_nes, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfcmp_ges, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfcmp_les, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfcmp_gts, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfcmp_lts, void, env, ptr, ptr)

DEF_HELPER_3(mve_vfcmp_eq_scalars, void, env, ptr, i32)
DEF_HELPER_3(mve_vfcmp_ne_scalars, void, env, ptr, i32)
DEF_HELPER_3(mve_vfcmp_ge_scalars, void, env, ptr, i32)
DEF_HELPER_3(mve_vfcmp_le_scalars, void, env, ptr, i32)
DEF_HELPER_3(mve_vfcmp_gt_scalars, void, env, ptr, i32)
DEF_HELPER_3(mve_vfcmp_lt_scalars, void, env, ptr, i32)

DEF_HELPER_4(mve_vhaddsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhaddsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhaddsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhaddub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhadduh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhadduw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vrhaddsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrhaddsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrhaddsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrhaddub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrhadduh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrhadduw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vhsubsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhsubsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhsubsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhsubub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhsubuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhsubuw, void, env, ptr, ptr, ptr)

DEF_HELPER_3(mve_vmaxvsb, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxvsh, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxvsw, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxvub, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxvuh, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxvuw, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxavb, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxavh, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxavw, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminvsb, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminvsh, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminvsw, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminvub, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminvuh, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminvuw, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminavb, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminavh, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminavw, i32, env, ptr, i32)
DEF_HELPER_4(mve_vmaxnms, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminnms, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmaxnmas, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminnmas, void, env, ptr, ptr, ptr)
DEF_HELPER_3(mve_vmaxnmvs, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminnmvs, i32, env, ptr, i32)
DEF_HELPER_3(mve_vmaxnmavs, i32, env, ptr, i32)
DEF_HELPER_3(mve_vminnmavs, i32, env, ptr, i32)

DEF_HELPER_4(mve_vfmul_scalars, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vfadd_scalars, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vfsub_scalars, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vfma_scalars, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vfmas_scalars, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vfadds, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vfsubs, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vfmuls, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vidupb, i32, env, ptr, i32, i32)
DEF_HELPER_4(mve_viduph, i32, env, ptr, i32, i32)
DEF_HELPER_4(mve_vidupw, i32, env, ptr, i32, i32)

DEF_HELPER_4(mve_vand, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vbic, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vorr, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vorn, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_veor, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vpsel, void, env, ptr, ptr, ptr)
DEF_HELPER_1(mve_vpnot, void, env)

DEF_HELPER_4(mve_vcmul0s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcmul90s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcmul180s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcmul270s, void, env, ptr, ptr, ptr)

DEF_HELPER_5(mve_viwdupb, i32, env, ptr, i32, i32, i32)
DEF_HELPER_5(mve_viwduph, i32, env, ptr, i32, i32, i32)
DEF_HELPER_5(mve_viwdupw, i32, env, ptr, i32, i32, i32)

DEF_HELPER_5(mve_vdwdupb, i32, env, ptr, i32, i32, i32)
DEF_HELPER_5(mve_vdwduph, i32, env, ptr, i32, i32, i32)
DEF_HELPER_5(mve_vdwdupw, i32, env, ptr, i32, i32, i32)

DEF_HELPER_3(mve_vclzb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vclzh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vclzw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vclsb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vclsh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vclsw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vabsb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vabsh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vabsw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vnegb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vnegh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vnegw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vfabss, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfabsh, void, env, ptr, ptr)

DEF_HELPER_3(mve_vfnegs, void, env, ptr, ptr)
DEF_HELPER_3(mve_vfnegh, void, env, ptr, ptr)

DEF_HELPER_3(mve_vqabsb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqabsh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqabsw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vqnegb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqnegh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqnegw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vmvn, void, env, ptr, ptr)

DEF_HELPER_3(mve_vmaxab, void, env, ptr, ptr)
DEF_HELPER_3(mve_vmaxah, void, env, ptr, ptr)
DEF_HELPER_3(mve_vmaxaw, void, env, ptr, ptr)

DEF_HELPER_3(mve_vminab, void, env, ptr, ptr)
DEF_HELPER_3(mve_vminah, void, env, ptr, ptr)
DEF_HELPER_3(mve_vminaw, void, env, ptr, ptr)

DEF_HELPER_4(mve_vcvt_s_rm_s, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vcvt_u_rm_s, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vrint_rm_s, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrintx_rm_s, void, env, ptr, ptr, i32)

DEF_HELPER_3(mve_vrev16b, void, env, ptr, ptr)
DEF_HELPER_3(mve_vrev32b, void, env, ptr, ptr)
DEF_HELPER_3(mve_vrev32h, void, env, ptr, ptr)
DEF_HELPER_3(mve_vrev64b, void, env, ptr, ptr)
DEF_HELPER_3(mve_vrev64h, void, env, ptr, ptr)
DEF_HELPER_3(mve_vrev64w, void, env, ptr, ptr)

DEF_HELPER_4(mve_vaddb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vaddh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vaddw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vadc, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vadci, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vsbc, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vsbci, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vcadd90b, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcadd90h, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcadd90w, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vcadd270b, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcadd270h, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcadd270w, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vhcadd90b, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhcadd90h, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhcadd90w, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vhcadd270b, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhcadd270h, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vhcadd270w, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vadd_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vadd_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vadd_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vsub_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vsub_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vsub_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vmul_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmul_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmul_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vhadds_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhadds_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhadds_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vhaddu_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhaddu_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhaddu_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vhsubs_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhsubs_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhsubs_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vhsubu_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhsubu_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vhsubu_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vsubb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vsubh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vsubw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmulb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmaxsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmaxsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmaxsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmaxub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmaxuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmaxuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vminsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vminuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vcvt_sf, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vcvt_uf, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vcvt_fs, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vcvt_fu, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vabavsb, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vabavsh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vabavsw, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vabavub, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vabavuh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vabavuw, i32, env, ptr, ptr, i32)

DEF_HELPER_3(mve_vmovnbb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vmovnbh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vmovntb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vmovnth, void, env, ptr, ptr)

DEF_HELPER_3(mve_vqmovunbb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovunbh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovuntb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovunth, void, env, ptr, ptr)

DEF_HELPER_3(mve_vqmovnbsb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovnbsh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovntsb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovntsh, void, env, ptr, ptr)

DEF_HELPER_3(mve_vqmovnbub, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovnbuh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovntub, void, env, ptr, ptr)
DEF_HELPER_3(mve_vqmovntuh, void, env, ptr, ptr)

DEF_HELPER_3(mve_vmovi, void, env, ptr, i64)
DEF_HELPER_3(mve_vandi, void, env, ptr, i64)
DEF_HELPER_3(mve_vorri, void, env, ptr, i64)

DEF_HELPER_4(mve_vshlsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vshlsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vshlsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vshlub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vshluh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vshluw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vrshlsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrshlsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrshlsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vrshlub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrshluh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrshluw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqaddsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqaddsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqaddsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqaddub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqadduh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqadduw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqadds_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqadds_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqadds_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqaddu_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqaddu_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqaddu_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqsubsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqsubsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqsubsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqsubub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqsubuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqsubuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqsubs_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqsubs_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqsubs_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqsubu_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqsubu_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqsubu_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqdmulh_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmulh_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmulh_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqrdmulh_scalarb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmulh_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmulh_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqshlsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqshlsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqshlsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqshlub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqshluh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqshluw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqrshlsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrshlsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrshlsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqrshlub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrshluh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrshluw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vshli_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshli_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshli_uw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vshli_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshli_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshli_sw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vrshli_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshli_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshli_sw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vrshli_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshli_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshli_uw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqshli_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshli_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshli_sw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqshli_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshli_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshli_uw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqshlui_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshlui_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshlui_sw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqrshli_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshli_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshli_sw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqrshli_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshli_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshli_uw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vshllbsb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshllbsh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshllbub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshllbuh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshlltsb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshlltsh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshlltub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshlltuh, void, env, ptr, ptr, i32)

DEF_HELPER_3(mve_vcmpeqb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpeqh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpeqw, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpneb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpneh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpnew, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpcsb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpcsh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpcsw, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmphib, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmphih, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmphiw, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpgeb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpgeh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpgew, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpltb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmplth, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpltw, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpgtb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpgth, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpgtw, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpleb, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmpleh, void, env, ptr, ptr)
DEF_HELPER_3(mve_vcmplew, void, env, ptr, ptr)

DEF_HELPER_3(mve_vcmpeq_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpeq_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpeq_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpne_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpne_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpne_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpcs_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpcs_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpcs_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmphi_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmphi_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmphi_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpge_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpge_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpge_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmplt_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmplt_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmplt_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpgt_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpgt_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmpgt_scalarw, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmple_scalarb, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmple_scalarh, void, env, ptr, i32)
DEF_HELPER_3(mve_vcmple_scalarw, void, env, ptr, i32)

DEF_HELPER_4(mve_vmlab, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlah, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlaw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vmlasb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlash, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlasw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vmlaldavsh, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlaldavxsh, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlaldavsw, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlaldavxsw, i64, env, ptr, ptr, i64)

DEF_HELPER_4(mve_vmlaldavuh, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlaldavuw, i64, env, ptr, ptr, i64)

DEF_HELPER_4(mve_vmlsldavsh, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlsldavxsh, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlsldavsw, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vmlsldavxsw, i64, env, ptr, ptr, i64)

DEF_HELPER_4(mve_vmladavsb, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavsh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavsw, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavsxb, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavsxh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavsxw, i32, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vmlsdavb, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlsdavh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlsdavw, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlsdavxb, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlsdavxh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmlsdavxw, i32, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vmladavub, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavuh, i32, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vmladavuw, i32, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vrmlaldavhsw, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vrmlaldavhxsw, i64, env, ptr, ptr, i64)

DEF_HELPER_4(mve_vrmlaldavhuw, i64, env, ptr, ptr, i64)

DEF_HELPER_4(mve_vrmlsldavhsw, i64, env, ptr, ptr, i64)
DEF_HELPER_4(mve_vrmlsldavhxsw, i64, env, ptr, ptr, i64)

DEF_HELPER_4(mve_vcmla0s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcmla90s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcmla180s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vcmla270s, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vfcadd90s, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vfcadd270s, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmullbsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmullbsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmullbsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmullbub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmullbuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmullbuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmulltsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulltsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulltsw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmulltub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulltuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulltuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmullpbh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmullpbw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vmullpth, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmullptw, void, env, ptr, ptr, ptr)

DEF_HELPER_3(mve_vaddvsb, i32, env, ptr, i32);
DEF_HELPER_3(mve_vaddvsh, i32, env, ptr, i32);
DEF_HELPER_3(mve_vaddvsw, i32, env, ptr, i32);
DEF_HELPER_3(mve_vaddvub, i32, env, ptr, i32);
DEF_HELPER_3(mve_vaddvuh, i32, env, ptr, i32);
DEF_HELPER_3(mve_vaddvuw, i32, env, ptr, i32);

DEF_HELPER_3(mve_vaddlvs, i64, env, ptr, i64)
DEF_HELPER_3(mve_vaddlvu, i64, env, ptr, i64)

DEF_HELPER_4(mve_vabdsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vabdsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vabdsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vabdub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vabduh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vabduw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vfabds, void, env, ptr, ptr, ptr)

DEF_HELPER_3(mve_uqshl, i32, env, i32, i32)
DEF_HELPER_3(mve_sqshl, i32, env, i32, i32)
DEF_HELPER_3(mve_uqrshl, i32, env, i32, i32)
DEF_HELPER_3(mve_sqrshr, i32, env, i32, i32)

DEF_HELPER_3(mve_sshrl, i64, env, i64, i32)
DEF_HELPER_3(mve_ushll, i64, env, i64, i32)
DEF_HELPER_3(mve_sqshll, i64, env, i64, i32)
DEF_HELPER_3(mve_uqshll, i64, env, i64, i32)

DEF_HELPER_3(mve_sqrshrl, i64, env, i64, i32)
DEF_HELPER_3(mve_uqrshll, i64, env, i64, i32)
DEF_HELPER_3(mve_sqrshrl48, i64, env, i64, i32)
DEF_HELPER_3(mve_uqrshll48, i64, env, i64, i32)

DEF_HELPER_4(mve_vfmas, void, env, ptr, ptr, ptr);
DEF_HELPER_4(mve_vfmss, void, env, ptr, ptr, ptr);

DEF_HELPER_4(mve_vqdmladhb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmladhh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmladhw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmladhxb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmladhxh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmladhxw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqrdmladhb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmladhh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmladhw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmladhxb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmladhxh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmladhxw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqdmlsdhb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmlsdhh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmlsdhw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmlsdhxb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmlsdhxh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmlsdhxw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqrdmlsdhb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmlsdhh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmlsdhw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmlsdhxb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmlsdhxh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmlsdhxw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vsrib, void, env, ptr, ptr, i32);
DEF_HELPER_4(mve_vsrih, void, env, ptr, ptr, i32);
DEF_HELPER_4(mve_vsriw, void, env, ptr, ptr, i32);

DEF_HELPER_4(mve_vslib, void, env, ptr, ptr, i32);
DEF_HELPER_4(mve_vslih, void, env, ptr, ptr, i32);
DEF_HELPER_4(mve_vsliw, void, env, ptr, ptr, i32);

DEF_HELPER_4(mve_vmulhsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulhsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulhsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulhub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulhuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vmulhuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vrmulhsb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrmulhsh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrmulhsw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrmulhub, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrmulhuh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vrmulhuw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqdmullbh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmullbw, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmullth, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmulltw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqdmullb_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmullb_scalarw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmullt_scalarh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmullt_scalarw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqdmulhb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmulhh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqdmulhw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqrdmulhb, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmulhh, void, env, ptr, ptr, ptr)
DEF_HELPER_4(mve_vqrdmulhw, void, env, ptr, ptr, ptr)

DEF_HELPER_4(mve_vqdmlahb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmlahh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmlahw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmlahb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmlahh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmlahw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqdmlashb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmlashh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqdmlashw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmlashb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmlashh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrdmlashw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqrshrnb_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnb_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnt_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnt_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnb_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnb_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnt_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrnt_uh, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqrshrunbb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrunbh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshruntb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqrshrunth, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vstrb_sg_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrb_sg_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrb_sg_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrh_sg_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrh_sg_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrw_sg_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrd_sg_ud, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vstrh_sg_os_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrh_sg_os_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrw_sg_os_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrd_sg_os_ud, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vstrw_sg_wb_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vstrd_sg_wb_ud, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vldrb_sg_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrb_sg_sw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrh_sg_sw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrb_sg_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrb_sg_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrb_sg_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrh_sg_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrh_sg_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrw_sg_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrd_sg_ud, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vldrh_sg_os_sw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrh_sg_os_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrh_sg_os_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrw_sg_os_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrd_sg_os_ud, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vldrw_sg_wb_uw, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vldrd_sg_wb_ud, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqshrnb_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnb_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnt_sb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnt_sh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnb_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnb_uh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnt_ub, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrnt_uh, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vqshrunbb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshruntb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrunbh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vqshrunth, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vbrsrb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vbrsrh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vbrsrw, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vshlc, i32, env, ptr, i32, i32)

DEF_HELPER_4(mve_vshrnbb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshrnbh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshrntb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vshrnth, void, env, ptr, ptr, i32)

DEF_HELPER_4(mve_vrshrnbb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshrnbh, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshrntb, void, env, ptr, ptr, i32)
DEF_HELPER_4(mve_vrshrnth, void, env, ptr, ptr, i32)
#endif

#include "def-helper.h"
