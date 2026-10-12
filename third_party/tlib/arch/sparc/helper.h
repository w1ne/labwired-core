#include "def-helper.h"

DEF_HELPER_0(power_down, void)
DEF_HELPER_0(rett, void)
DEF_HELPER_1(wrpsr, void, tl)
DEF_HELPER_0(rdpsr, tl)
DEF_HELPER_2(check_align, void, tl, i32)
DEF_HELPER_0(debug, void)
DEF_HELPER_0(save, void)
DEF_HELPER_0(restore, void)
DEF_HELPER_2(udiv, tl, tl, tl)
DEF_HELPER_2(udiv_cc, tl, tl, tl)
DEF_HELPER_2(sdiv, tl, tl, tl)
DEF_HELPER_2(sdiv_cc, tl, tl, tl)
DEF_HELPER_2(stdf, void, tl, int)
DEF_HELPER_2(lddf, void, tl, int)
DEF_HELPER_2(ldqf, void, tl, int)
DEF_HELPER_2(stqf, void, tl, int)
DEF_HELPER_4(ld_asi, i64, tl, int, int, int)
DEF_HELPER_4(st_asi, void, tl, i64, int, int)
DEF_HELPER_4(cas_asi, tl, tl, tl, tl, i32)
DEF_HELPER_1(ldfsr, void, i32)
DEF_HELPER_0(check_ieee_exceptions, void)
DEF_HELPER_0(clear_float_exceptions, void)
DEF_HELPER_1(fabss, f32, f32)
DEF_HELPER_1(fsqrts, f32, f32)
DEF_HELPER_0(fsqrtd, void)
DEF_HELPER_2(fcmps, void, f32, f32)
DEF_HELPER_0(fcmpd, void)
DEF_HELPER_2(fcmpes, void, f32, f32)
DEF_HELPER_0(fcmped, void)
DEF_HELPER_0(fsqrtq, void)
DEF_HELPER_0(fcmpq, void)
DEF_HELPER_0(fcmpeq, void)
DEF_HELPER_1(raise_exception, void, int)
DEF_HELPER_0(shutdown, void)
DEF_HELPER_1(ldstub, tl, i32)
DEF_HELPER_2(swap, tl, tl, i32)
#define F_HELPER_0_0(name) DEF_HELPER_0(f##name, void)
#define F_HELPER_DQ_0_0(name) \
    F_HELPER_0_0(name##d);    \
    F_HELPER_0_0(name##q)

F_HELPER_DQ_0_0(add);
F_HELPER_DQ_0_0(sub);
F_HELPER_DQ_0_0(mul);
F_HELPER_DQ_0_0(div);

DEF_HELPER_2(fadds, f32, f32, f32)
DEF_HELPER_2(fsubs, f32, f32, f32)
DEF_HELPER_2(fmuls, f32, f32, f32)
DEF_HELPER_2(fdivs, f32, f32, f32)

DEF_HELPER_2(fsmuld, void, f32, f32)
F_HELPER_0_0(dmulq);

DEF_HELPER_1(fnegs, f32, f32)
DEF_HELPER_1(fitod, void, s32)
DEF_HELPER_1(fitoq, void, s32)

DEF_HELPER_1(fitos, f32, s32)

DEF_HELPER_0(fdtos, f32)
DEF_HELPER_1(fstod, void, f32)
DEF_HELPER_0(fqtos, f32)
DEF_HELPER_1(fstoq, void, f32)
F_HELPER_0_0(qtod);
F_HELPER_0_0(dtoq);
DEF_HELPER_1(fstoi, s32, f32)
DEF_HELPER_0(fdtoi, s32)
DEF_HELPER_0(fqtoi, s32)
#undef F_HELPER_0_0
#undef F_HELPER_DQ_0_0
#undef VIS_HELPER
#undef VIS_CMPHELPER
DEF_HELPER_0(compute_psr, void);
DEF_HELPER_0(compute_C_icc, i32);
#include "def-helper.h"
