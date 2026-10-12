/*
 *  ARM translation for M-Profile Vector Extension (MVE)
 *
 *  Copyright (c) Antmicro
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

#pragma once

#include "translate_common.h"

typedef void MVEGenLdStFn(TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenLdStIlFn(DisasContext *, uint32_t, TCGv_i32);
typedef void MVEGenLdStSGFn(TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenTwoOpScalarFn(TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenTwoOpShiftFn(TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenTwoOpFn(TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_ptr);
typedef void MVEGenCmpFn(TCGv_ptr, TCGv_ptr, TCGv_ptr);
typedef void MVEGenScalarCmpFn(TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenVIDUPFn(TCGv_i32, TCGv_ptr, TCGv_ptr, TCGv_i32, TCGv_i32);
typedef void MVEGenVIWDUPFn(TCGv_i32, TCGv_ptr, TCGv_ptr, TCGv_i32, TCGv_i32, TCGv_i32);
typedef void MVEGenVADDVFn(TCGv_i32, TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenVABAVFn(TCGv_i32, TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEGenOneOpFn(TCGv_ptr, TCGv_ptr, TCGv_ptr);
typedef void MVEGenOneOpImmFn(TCGv_ptr, TCGv_ptr, TCGv_i64);
typedef void MVEGenLongDualAccOpFn(TCGv_i64, TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_i64);
typedef void MVEGenDualAccOpFn(TCGv_i32, TCGv_ptr, TCGv_ptr, TCGv_ptr, TCGv_i32);
typedef void MVEWideShiftImmFn(TCGv_i64, TCGv_i64, int64_t shift);
typedef void MVEWideShiftFn(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i32);
typedef void MVEShiftImmFn(TCGv_i32, TCGv_i32, int32_t shift);
typedef void MVEShiftFn(TCGv_i32, TCGv_ptr, TCGv_i32, TCGv_i32);

/* Note that the gvec expanders operate on offsets + sizes.  */
typedef void GVecGen3Fn(unsigned, uint32_t, uint32_t, uint32_t, uint32_t, uint32_t);
typedef void GVecGen2Fn(unsigned, uint32_t, uint32_t, uint32_t, uint32_t);
typedef void GVecGen2iFn(unsigned, uint32_t, uint32_t, int64_t, uint32_t, uint32_t);

/* Arguments for VMSR/VMRS system register instruction */
typedef struct {
    int rt;
    int reg;
    int l;
} arg_vmsr_vmrs;

/* Arguments for VLDR/VSTR system register instruction */
typedef struct {
    int rn;
    int reg;
    int imm;
    int a;
    int w;
    int p;
} arg_vldr_vstr_sysreg;

/* Arguments for VLSTM/VLLDM instructions */
typedef struct {
    bool low_regs_only;
    bool is_vlldm;
    int rn;
} arg_vlstm_vlldm;

/* Arguments for VSCCLRM instruction */
typedef struct {
    int vd;
    int imm;
    int size;
} arg_vscclrm;

/*
 * Arguments of stores/loads:
 * VSTRB, VSTRH, VSTRW, VLDRB, VLDRH, VLDRW
 */
typedef struct {
    int rn;
    int qd;
    int imm;
    int p;
    int a;
    int w;
    int size;
    /* Used to tell store/load apart */
    int l;
    int u;
} arg_vldr_vstr;

/*
 * Arguments of (de)interleaving stores/loads:
 * VLD2, VLD4, VST2, VST4
 */
typedef struct {
    int qd;
    int rn;
    int size;
    int pat;
    int w;
} arg_vldst_il;

/* Arguments of vabav instruction */
typedef struct {
    int rda;
    int qm;
    int qn;
    int size;
} arg_vabav;

/* Arguments of 2 operand scalar instructions */
typedef struct {
    int qd;
    int qm;
    int qn;
    int size;
} arg_2op;

/* Arguments of 2 operand scalar instructions */
typedef struct {
    int qd;
    int qn;
    int rm;
    int size;
} arg_2scalar;

/* Arguments of 1 operand scalar instructions */
typedef struct {
    int qda;
    int rm;
    int size;
} arg_shl_scalar;

/* Arguments of VDUP instruction */
typedef struct {
    int qd;
    int rt;
    /* See comment in trans function */
    int size;
} arg_vdup;

/* Arguments of VCTP instruction */
typedef struct {
    int rn;
    int size;
} arg_vctp;

/* Arguments of VPST instruction */
typedef struct {
    int mask;
} arg_vpst;

/* Arguments of VCMP (non-scalar) instruction (both floating-point and vector) */
typedef struct {
    int qm;
    int qn;
    int size;
    int mask;
} arg_vcmp;

/* Arguments of VCMP scalar instructions (both floating-point and vector) */
typedef struct {
    int qn;
    int rm;
    int size;
    int mask;
} arg_vcmp_scalar;

/* Arguments of VIDUP and VDDUP instructions */
typedef struct {
    int qd;
    int rn;
    int size;
    int imm;
} arg_vidup;

/* Arguments of VIWDUP and VDWDUP instructions */
typedef struct {
    int qd;
    int rn;
    int rm;
    int size;
    int imm;
} arg_viwdup;

/* Arguments of VMAX*V and VMIN*V instructions */
typedef struct {
    int qm;
    int rda;
    int size;
} arg_vmaxv;

/* Arguments of 1 operand vector instruction */
typedef struct {
    int qd;
    int qm;
    int size;
} arg_1op;

typedef struct {
    int qd;
    int qm;
    int shift;
    int size;
} arg_2shift;

/* Arguments of immediate value vector instruction */
typedef struct {
    int qd;
    int imm;
    int cmode;
    int op;
} arg_1imm;

/* Arguments of VMOV 2-GP<->2 V-lanes */
typedef struct {
    int rt2;
    int idx;
    int rt;
    int qd;
    int from;
} arg_vmov_2gp;

/* Arguments of VMOV GP<->V-lane */
typedef struct {
    int qn;
    int rt;
    int op1;
    int op2;
    int h;  //  high (upper) d-register
    int u;  //  unsigned
} arg_vmov_gp;

/* Arguments of VMLA(L)DAV instruction */
typedef struct {
    /* General purpose register containing high bits of 64-bit value */
    int rdahi;
    /* General purpose register containing low bits of 64-bit value */
    int rdalo;
    /* Size of the operation (.S8/S16/S32/S64 suffix) */
    int size;
    /* First vector register used for operation */
    int qn;
    /* Second vector register used for operation */
    int qm;
    /* Exchange adjecent pairs of values in Qm */
    int x;
    /* Accumulate with existing register contents */
    int a;
} arg_vmlaldav;

typedef struct {
    /* General purpose register used for operation */
    int rda;
    /* Size of the operation (.S8/S16/S32/S64 suffix) */
    int size;
    /* First vector register used for operation */
    int qn;
    /* Second vector register used for operation */
    int qm;
    /* Exchange adjecent pairs of values in Qm */
    int x;
    /* Accumulate with existing register contents */
    int a;
} arg_vmladav;

typedef struct {
    int u;
    int a;
    /* Only valid for VADDV */
    int size;
    int qm;
    int rda;
    /* Only valid for VADDLV */
    int rdahi;
} arg_vaddv;

/* Shift by immediate */
typedef struct {
    int rda;   //  Register containing the value to be shifted
    int shim;  //  The number of bits to shift by, in the range 1-32.
} arg_mve_sh_i;

/* Shift by register */
typedef struct {
    int rda;  //  Register containing the value to be shifted
    int rm;   //  Register containing the value to shift by
} arg_mve_sh_rr;

/* Arguments of long (64-bit) shifts by immediate (1-32) */
typedef struct {
    int rdalo;  //  Rda register containing lower bits of 64-bit value
    int rdahi;  //  Rda register containing upper bits of 64-bit value
    int shim;   //  "shift immediate", the immediate value
} arg_mve_shl_ri;

/* Arguments of long (64-bit) shifts by register */
typedef struct {
    int rdalo;  //  Rda register containing lower bits of 64-bit value
    int rdahi;  //  Rda register containing upper bits of 64-bit value
    int rm;     //  Register containing the amount to shift by
} arg_mve_shl_rr;

typedef struct {
    /* Vector register used as destination */
    int qd;
    /* First vector register used for operation */
    int qn;
    /* Second vector register used for operation */
    int qm;
    /* Size of the operation (.S8/S16/S32/S64 suffix) */
    int size;
    /* Exchange adjacent pairs of values in Qm */
    int x;
} arg_vqdmladh;

typedef struct {
    /* Vector register used as destination */
    int qd;
    /* Vector register containing address offsets */
    int qm;
    /* General purpose register used for base address */
    int rn;
    /* Size of the operation (.S8/S16/S32/S64 suffix) */
    int size;
    /* Width of memory transaction (B/H/W/D instruction suffix) */
    int msize;
    /* Amount by which vector offset is left shifted */
    int os;
} arg_vldst_sg;

typedef struct {
    /* Vector register used as destination */
    int qd;
    /* Vector register containing address offsets */
    int qm;
    /* Defines if offsets should be added or subtracted */
    int a;
    /* Write-back */
    int w;
    /* Base address offset (signed) */
    int imm;
} arg_vldst_sg_imm;

typedef struct {
    /* Source and destination general-purpose register for carry in and out */
    int rdm;
    /* Source and destination vector register. */
    int qd;
    /* The number of bits to shift by, in the range 1-32. */
    int imm;
} arg_vshlc;

void gen_mve_vld40b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld41b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld42b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld43b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld40h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld41h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld42h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld43h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld40w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld41w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld42w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vld43w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);

void gen_mve_vld20b(DisasContext *s, uint32_t qdindx, TCGv_i32 base);
void gen_mve_vld21b(DisasContext *s, uint32_t qdindx, TCGv_i32 base);
void gen_mve_vld20h(DisasContext *s, uint32_t qdindx, TCGv_i32 base);
void gen_mve_vld21h(DisasContext *s, uint32_t qdindx, TCGv_i32 base);
void gen_mve_vld20w(DisasContext *s, uint32_t qdindx, TCGv_i32 base);
void gen_mve_vld21w(DisasContext *s, uint32_t qdindx, TCGv_i32 base);

void gen_mve_vst40b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst41b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst42b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst43b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst40h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst41h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst42h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst43h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst40w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst41w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst42w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst43w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);

void gen_mve_vst20b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst21b(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst20h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst21h(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst20w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);
void gen_mve_vst21w(DisasContext *s, uint32_t qnindx, TCGv_i32 base);

void gen_mve_vpst(DisasContext *s, uint32_t mask);

static inline bool is_insn_vlstm_vlldm(uint32_t insn)
{
    return (insn & 0xFFE0FF7F) == 0xEC200A00;
}

static inline bool is_insn_vscclrm(uint32_t insn)
{
    /* First bit of the instruction has to be 0 for T1 */
    bool t1_check = extract32(insn, 8, 1) == 0 || extract32(insn, 0, 1) == 0;
    return t1_check && (insn & 0xFFBF0E00) == 0xEC9F0A00;
}

static inline bool is_insn_vmsr_vmrs(uint32_t insn)
{
    return (insn & 0xFFE00FFF) == 0xEEE00A10;
}

static inline bool is_insn_vldr_vstr_sysreg(uint32_t insn)
{
    uint32_t p = extract32(insn, 24, 1);
    uint32_t w = extract32(insn, 21, 1);
    bool related_encoding = p == 0 && w == 0;

    return !related_encoding && (insn & 0xFE001F80) == 0xEC000F80;
}

/*
 * Vector load/store register (encodings T5, T6, T7)
 * VLDRB, VLDRH, VLDRW, VSTRB, VSTRH, VSTRW
 */
static inline bool is_insn_vldr_vstr(uint32_t insn)
{
    uint32_t p = extract32(insn, 24, 1);
    uint32_t w = extract32(insn, 21, 1);
    bool related_encoding = p == 0 && w == 0;

    return !related_encoding && (insn & 0xFE001E00) == 0xEC001E00;
}

/*
 * Vector load/store register (encodings T1, T2)
 * VLDRB, VLDRH, VLDRW, VSTRB, VSTRH, VSTRW
 */
static inline bool is_insn_vldr_vstr_widening(uint32_t insn)
{
    uint32_t p = extract32(insn, 24, 1);
    uint32_t w = extract32(insn, 21, 1);
    uint32_t size = extract32(insn, 7, 2);
    bool related_encoding = (p == 0 && w == 0) || size == 3;

    return !related_encoding && (insn & 0xEE401E00) == 0xEC000E00;
}

static inline bool is_insn_vadd(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F51) == 0xEF000840;
}

static inline bool is_insn_vadd_scalar(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F70) == 0xEE010F40;
}

static inline bool is_insn_vadd_fp(uint32_t insn)
{
    return (insn & 0xFFA11F51) == 0xEF000D40;
}

static inline bool is_insn_vadd_fp_scalar(uint32_t insn)
{
    return (insn & 0xEFB11F70) == 0xEE300F40;
}

static inline bool is_insn_vsub(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F51) == 0xFF000840;
}

static inline bool is_insn_vsub_scalar(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F70) == 0xEE011F40;
}

static inline bool is_insn_vsub_fp(uint32_t insn)
{
    return (insn & 0xFFA11F51) == 0xEF200D40;
}

static inline bool is_insn_vsub_fp_scalar(uint32_t insn)
{
    return (insn & 0xEFB11F70) == 0xEE301F40;
}

static inline bool is_insn_vmul(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F51) == 0xEF000950;
}

static inline bool is_insn_vmul_scalar(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F70) == 0xEE011E60;
}

static inline bool is_insn_vmul_fp(uint32_t insn)
{
    return (insn & 0xFFAF1F51) == 0xFF000D50;
}

static inline bool is_insn_vmul_fp_scalar(uint32_t insn)
{
    return (insn & 0xEFB11F70) == 0xEE310E60;
}

static inline bool is_insn_vfma_scalar(uint32_t insn)
{
    return (insn & 0xEFB11F70) == 0xEE310E40;
}

static inline bool is_insn_vfmas_scalar(uint32_t insn)
{
    return (insn & 0xEFB11F70) == 0xEE311E40;
}

static inline bool is_insn_vdup(uint32_t insn)
{
    return (insn & 0xFFB10F50) == 0xEEA00B10;
}

static inline bool is_insn_vctp(uint32_t insn)
{
    /* Rn == 0b1111 is related-encoding */
    return extract32(insn, 16, 4) != 15 && (insn & 0xFFC0F801) == 0xF000E801;
}

static inline bool is_insn_vpst(uint32_t insn)
{
    uint32_t mask = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    return mask != 0 && (insn & 0xFFB10F5F) == 0xFE310F4D;
}

static inline bool is_insn_vpnot(uint32_t insn)
{
    return (insn & 0xFFFFFFFF) == 0xFE310F4D;
}

static inline bool is_insn_vcmp_fp(uint32_t insn)
{
    uint32_t fca = extract32(insn, 12, 1);
    uint32_t fcb = extract32(insn, 0, 1);

    /* fcA == 0 && fcB == 1 is related encodings */
    if(fca == 0 && fcb == 1) {
        return false;
    }
    return (insn & 0xEFB10F50) == 0xEE310F00;
}

static inline bool is_insn_vcmp_fp_scalar(uint32_t insn)
{
    uint32_t rm = extract32(insn, 0, 4);

    /* rm == 0b1101 is related encodings */
    if(rm == 13) {
        return false;
    }
    return (insn & 0xEFB10F50) == 0xEE310F40;
}

static inline bool is_insn_vcmp(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    /* size == 3 is related encoding */
    return size != 3 && (insn & 0xFF810F50) == 0xFE010F00;
}

static inline bool is_insn_vcmp_scalar(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    /* size == 3 is related encoding */
    return size != 3 && (insn & 0xFF810F50) == 0xFE010F40;
}

static inline bool is_insn_vidup(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    /* size = 0b11 is related encodings */
    if(size == 3) {
        return false;
    }
    return (insn & 0xFF811F7E) == 0xEE010F6E;
}

static inline bool is_insn_vddup(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    /* size = 0b11 is related encodings */
    if(size == 3) {
        return false;
    }
    return (insn & 0xFF811F7E) == 0xEE011F6E;
}

static inline bool is_insn_viwdup(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    /* size = 0b11 is related encodings */
    if(size == 3) {
        return false;
    }
    uint32_t rm = extract32(insn, 1, 3);
    /* rm == 0b111 related encodings */
    if(rm == 7) {
        return false;
    }
    return (insn & 0xFF811F70) == 0xEE010F60;
}

static inline bool is_insn_vdwdup(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    /* size = 0b11 is related encodings */
    if(size == 3) {
        return false;
    }
    uint32_t rm = extract32(insn, 1, 3);
    /* rm == 0b111 related encodings */
    if(rm == 7) {
        return false;
    }
    return (insn & 0xFF811F70) == 0xEE011F60;
}

static inline bool is_insn_vpsel(uint32_t insn)
{
    return (insn & 0xFFB11F51) == 0xFE310F01;
}

static inline bool is_insn_vld4(uint32_t insn)
{
    uint32_t size = extract32(insn, 7, 2);
    return size != 3 && (insn & 0xFF901E01) == 0xFC901E01;
}

static inline bool is_insn_vld2(uint32_t insn)
{
    uint32_t size = extract32(insn, 7, 2);
    return size != 3 && (insn & 0xFF901E01) == 0xFC901E00;
}

static inline bool is_insn_vst4(uint32_t insn)
{
    uint32_t size = extract32(insn, 7, 2);
    return size != 3 && (insn & 0xFF901E01) == 0xFC801E01;
}

static inline bool is_insn_vst2(uint32_t insn)
{
    uint32_t size = extract32(insn, 7, 2);
    return size != 3 && (insn & 0xFF901E01) == 0xFC801E00;
}

static inline bool is_insn_vcmul(uint32_t insn)
{
    return (insn & 0xEFB10F50) == 0xEE300E00;
}

static inline bool is_insn_vcls(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB00440;
}

static inline bool is_insn_vclz(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB004C0;
}

static inline bool is_insn_vabs(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB10340;
}

static inline bool is_insn_vneg(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB103C0;
}

static inline bool is_insn_vabs_fp(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB10740;
}

static inline bool is_insn_vneg_fp(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB107C0;
}

static inline bool is_insn_vmvn(uint32_t insn)
{
    return (insn & 0xFFBF1FD1) == 0xFFB005C0;
}

static inline bool is_insn_vmin_vmax(uint32_t insn)
{
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xEF811F41) == 0xEF000640;
}

static inline bool is_insn_vmaxa(uint32_t insn)
{
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xFFB31FD1) == 0xEE330E81;
}

static inline bool is_insn_vmina(uint32_t insn)
{
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xFFB31FD1) == 0xEE331E81;
}

static inline bool is_insn_vrev(uint32_t insn)
{
    uint32_t mask = extract32(insn, 7, 2);
    return mask != 3 && (insn & 0xFFB31E51) == 0xFFB00040;
}

static inline bool is_insn_vmaxnm(uint32_t insn)
{
    return (insn & 0xFFA11F51) == 0xFF000F50;
}

static inline bool is_insn_vminnm(uint32_t insn)
{
    return (insn & 0xFFA11F51) == 0xFF200F50;
}

static inline bool is_insn_vmaxnma(uint32_t insn)
{
    return (insn & 0xEFBF1FD1) == 0xEE3F0E81;
}

static inline bool is_insn_vminnma(uint32_t insn)
{
    return (insn & 0xEFBF1FD1) == 0xEE3F1E81;
}

/* VCVT between floating and fixed point */
static inline bool is_insn_vcvt_f_and_fixed(uint32_t insn)
{
    uint32_t upper_imm6 = extract32(insn, 19, 3);
    uint32_t fsi = extract32(insn, 9, 1);
    bool related_opcode = upper_imm6 == 0;
    bool undefined = !(upper_imm6 & 0b100) || (!fsi && (upper_imm6 & 0b110) == 0b100);
    return !related_opcode && !undefined && (insn & 0xEF801CD1) == 0xEF800C50;
}

/* VCVT between floating-point and integer */
static inline bool is_insn_vcvt_f_and_i(uint32_t insn)
{
    uint32_t size = extract32(insn, 18, 2);
    bool undefined = size == 3 || size == 0;
    return !undefined && (insn & 0xFFB31E51) == 0xFFB30640;
}

/* VCVT from floating-point to integer */
static inline bool is_insn_vcvt_fp(uint32_t insn)
{
    return (insn & 0xFFB31C51) == 0xFFB30040;
}

/* VRINT (floating-point) */
static inline bool is_insn_vrint_fp(uint32_t insn)
{
    return (insn & 0xFFB31C51) == 0xFFB20440;
}

static inline bool is_insn_vmovi(uint32_t insn)
{
    uint32_t cmode = extract32(insn, 8, 4);
    /* cmode & 1 && cmode < 12 is related encoding */
    if((cmode & 1) > 0 && cmode < 12) {
        return false;
    }

    uint32_t op = extract32(insn, 5, 1);
    if(cmode == 15 && op == 1) {
        return false;
    }

    return (insn & 0xEFB810D0) == 0xEF800050;
}

static inline bool is_insn_vandi_vorri(uint32_t insn)
{
    uint32_t cmode = extract32(insn, 8, 4);
    /* !(cmode & 1) || cmode > 12 is related encoding */
    if((cmode & 1) == 0 || cmode > 12) {
        return false;
    }
    return (insn & 0xEFB810D0) == 0xEF800050;
}

static inline bool is_insn_vand(uint32_t insn)
{
    return (insn & 0xFFB11F51) == 0xEF000150;
}
static inline bool is_insn_vbic(uint32_t insn)
{
    return (insn & 0xFFB11F51) == 0xEF100150;
}
static inline bool is_insn_vorr(uint32_t insn)
{
    return (insn & 0xFFB11F51) == 0xEF200150;
}
static inline bool is_insn_vorn(uint32_t insn)
{
    return (insn & 0xFFB11F51) == 0xEF300150;
}
static inline bool is_insn_veor(uint32_t insn)
{
    return (insn & 0xFFB11F51) == 0xFF000150;
}

static inline bool is_insn_vmov_2gp(uint32_t insn)
{
    return (insn & 0xFFA01FE0) == 0xEC000F00;
}

static inline bool is_insn_vhadd_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000040;
}

static inline bool is_insn_vhadd_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000040;
}

static inline bool is_insn_vrhadd_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000140;
}

static inline bool is_insn_vrhadd_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000140;
}

static inline bool is_insn_vhsub_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000240;
}

static inline bool is_insn_vhsub_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000240;
}

static inline bool is_insn_vhadd_s_scalar(uint32_t insn)
{
    return (insn & 0xFF811F70) == 0xEE000F40;
}

static inline bool is_insn_vhadd_u_scalar(uint32_t insn)
{
    return (insn & 0xFF811F70) == 0xFE000F40;
}

static inline bool is_insn_vhsub_s_scalar(uint32_t insn)
{
    return (insn & 0xFF811F70) == 0xEE001F40;
}

static inline bool is_insn_vhsub_u_scalar(uint32_t insn)
{
    return (insn & 0xFF811F70) == 0xFE001F40;
}

static inline bool is_insn_vmov_gp(uint32_t insn)
{
    return (insn & 0xFF000F1F) == 0xEE000B10;
}

static inline bool is_insn_vadc(uint32_t insn)
{
    return (insn & 0xFFB10F51) == 0xEE300F00;
}

static inline bool is_insn_vsbc(uint32_t insn)
{
    return (insn & 0xFFB10F51) == 0xFE300F00;
}

static inline bool is_insn_vcadd90(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFE000F00;
}

static inline bool is_insn_vcadd270(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFE001F00;
}

static inline bool is_insn_vhcadd90(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEE000F00;
}

static inline bool is_insn_vhcadd270(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEE001F00;
}

static inline bool is_insn_vshl_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000440;
}

static inline bool is_insn_vshl_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000440;
}

static inline bool is_insn_vrshl_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000540;
}

static inline bool is_insn_vrshl_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000540;
}

static inline bool is_insn_vqshl_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000450;
}

static inline bool is_insn_vqshl_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000450;
}

static inline bool is_insn_vqrshl_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000550;
}

static inline bool is_insn_vqrshl_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000550;
}

static inline bool is_insn_vshli(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xEF800550;
}

static inline bool is_insn_vqshli_s(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xEF800750;
}

static inline bool is_insn_vqshli_u(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800750;
}

static inline bool is_insn_vqshlui_s(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800650;
}

static inline bool is_insn_vshri_s(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xEF800050;
}

static inline bool is_insn_vshri_u(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800050;
}

static inline bool is_insn_vrshri_s(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800250;
}

static inline bool is_insn_vrshri_u(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800250;
}

static inline bool is_insn_vshl_s_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xEE311E60;
}

static inline bool is_insn_vrshl_s_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xEE331E60;
}

static inline bool is_insn_vqshl_s_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xEE311EE0;
}

static inline bool is_insn_vqrshl_s_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xEE331EE0;
}

static inline bool is_insn_vshl_u_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xFE311E60;
}

static inline bool is_insn_vrshl_u_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xFE331E60;
}

static inline bool is_insn_vqshl_u_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xFE311EE0;
}

static inline bool is_insn_vqrshl_u_scalar(uint32_t insn)
{
    return (insn & 0xFFB31FF0) == 0xFE331EE0;
}

static inline bool is_insn_vshll_imm(uint32_t insn)
{
    return (insn & 0xEFA00FD1) == 0xEEA00F40;
}

static inline bool is_insn_vshll(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xEFB30FD1) == 0xEE310E01;
}

static inline bool is_insn_vmovn(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xFFB30FD1) == 0xFE310E81;
}

static inline bool is_insn_vqmovn(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xEFB30FD1) == 0xEE330E01;
}

static inline bool is_insn_vqmovun(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 18, 2);
    return size != 3 && (insn & 0xFFB30FD1) == 0xEE310E81;
}

static inline bool is_insn_vmla_vmlas(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xEF810F70) == 0xEE010E40;
}

static inline bool is_insn_vmlaldav(uint32_t insn)
{
    uint32_t rdahi = extract32(insn, 20, 3);
    if(rdahi == 7) {
        /* RdaHi == '111' is related encoding */
        return false;
    }
    return (insn & 0xEF801FD1) == 0xEE800E00;
}

static inline bool is_insn_vmlsldav(uint32_t insn)
{
    uint32_t rdahi = extract32(insn, 20, 3);
    if(rdahi == 7) {
        /* RdaHi == '111' is related encoding */
        return false;
    }
    return (insn & 0xFF800FD1) == 0xEE800E01;
}

static inline bool is_insn_vmladav(uint32_t insn)
{
    uint32_t b16 = extract32(insn, 16, 1);
    uint32_t b8 = extract32(insn, 8, 1);
    if(b16 == 1 && b8 == 1) {
        /* invalid instruction */
        return false;
    }

    uint32_t u = extract32(insn, 28, 1);
    uint32_t x = extract32(insn, 12, 1);
    if(u == 1 && x == 1) {
        /* undefined */
        return false;
    }

    return (insn & 0xEFF00ED1) == 0xEEF00E00;
}

static inline bool is_insn_vmlsdav(uint32_t insn)
{
    uint32_t b28 = extract32(insn, 28, 1);
    uint32_t b16 = extract32(insn, 16, 1);
    if(b28 == 1 && b16 == 1) {
        /* invalid instruction */
        return false;
    }

    return (insn & 0xEFF00FD1) == 0xEEF00E01;
}

static inline bool is_insn_vcmla(uint32_t insn)
{
    return (insn & 0xFE211F51) == 0xFC200840;
}

static inline bool is_insn_vfcadd(uint32_t insn)
{
    return (insn & 0xFEA11F51) == 0xFC800840;
}

static inline bool is_insn_vabav(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xEFC10F51) == 0xEE800F01;
}

static inline bool is_insn_vqadd_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000050;
}

static inline bool is_insn_vqadd_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000050;
}

static inline bool is_insn_vqadd_u_scalar(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F70) == 0xFE000F60;
}

static inline bool is_insn_vqadd_s_scalar(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F70) == 0xEE000F60;
}

static inline bool is_insn_vqsub_u(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xFF000250;
}

static inline bool is_insn_vqsub_s(uint32_t insn)
{
    return (insn & 0xFF811F51) == 0xEF000250;
}

static inline bool is_insn_vqsub_u_scalar(uint32_t insn)
{
    return (insn & 0xFF811F70) == 0xFE001F60;
}

static inline bool is_insn_vqsub_s_scalar(uint32_t insn)
{
    return (insn & 0xFF811F70) == 0xEE001F60;
}

static inline bool is_insn_vqabs(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB00740;
}

static inline bool is_insn_vqneg(uint32_t insn)
{
    return (insn & 0xFFB31FD1) == 0xFFB007C0;
}

static inline bool is_insn_vmull(uint32_t insn)
{
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xEF810F51) == 0xEE010E00;
}

static inline bool is_insn_vmullp(uint32_t insn)
{
    return (insn & 0xEFB10F51) == 0xEE310E00;
}

static inline bool is_insn_vaddv(uint32_t insn)
{
    return (insn & 0xEFF31FD1) == 0xEEF10F00;
}

static inline bool is_insn_vaddlv(uint32_t insn)
{
    uint32_t related = extract32(insn, 16, 3);
    return related != 7 && (insn & 0xEF8F1FD1) == 0xEE890F00;
}

static inline bool is_insn_vabd(uint32_t insn)
{
    return (insn & 0xEF811F51) == 0xEF000740;
}

static inline bool is_insn_vabd_fp(uint32_t insn)
{
    return (insn & 0xFFA11F51) == 0xFF200D40;
}

static inline bool is_insn_lsll_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding UQSHL (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA50010F;
}

static inline bool is_insn_lsrl_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding URSHR (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA50011F;
}

static inline bool is_insn_asrl_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding SRSHR (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA50012F;
}

static inline bool is_insn_uqshll_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding UQSHL (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA51010F;
}

static inline bool is_insn_urshrl_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding URSHR (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA51011F;
}

static inline bool is_insn_srshrl_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding SRSHR (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA51012F;
}

static inline bool is_insn_sqshll_imm(uint32_t insn)
{
    bool related_encoding = extract32(insn, 9, 3) == 0b111; /* related encoding SQSHL (immediate) */
    return !related_encoding && (insn & 0xFFF1813F) == 0xEA51013F;
}

static inline bool is_insn_sqshl_imm(uint32_t insn)
{
    return (insn & 0xFFF08F3F) == 0xEA500F3F;
}

static inline bool is_insn_uqshl_imm(uint32_t insn)
{
    return (insn & 0xFFF08F3F) == 0xEA500F0F;
}

static inline bool is_insn_srshr_imm(uint32_t insn)
{
    return (insn & 0xFFF08F3F) == 0xEA500F2F;
}

static inline bool is_insn_urshr_imm(uint32_t insn)
{
    return (insn & 0xFFF08F3F) == 0xEA500F1F;
}

static inline bool is_insn_sqrshr_reg(uint32_t insn)
{
    return (insn & 0xFFF00FFF) == 0xEA500F2D;
}

static inline bool is_insn_uqrshl_reg(uint32_t insn)
{
    return (insn & 0xFFF00FFF) == 0xEA500F0D;
}

static inline bool is_insn_asrl_reg(uint32_t insn)
{
    /* RdaHi == '111' is related encoding */
    uint32_t rdahi = extract32(insn, 9, 3);
    return rdahi != 7 && (insn & 0xFFF101FF) == 0xEA50012D;
}

static inline bool is_insn_lsll_reg(uint32_t insn)
{
    /* RdaHi == '111' is related encoding */
    uint32_t rdahi = extract32(insn, 9, 3);
    return rdahi != 7 && (insn & 0xFFF101FF) == 0xEA50010D;
}

static inline bool is_insn_sqrshrl(uint32_t insn)
{
    /* RdaHi == '111' is related encoding */
    uint32_t rdahi = extract32(insn, 9, 3);
    return rdahi != 7 && (insn & 0xFFF1017F) == 0xEA51012D;
}

static inline bool is_insn_uqrshll(uint32_t insn)
{
    /* RdaHi == '111' is related encoding */
    uint32_t rdahi = extract32(insn, 9, 3);
    return rdahi != 7 && (insn & 0xFFF1017F) == 0xEA51010D;
}

static inline bool is_insn_vrmlaldavh(uint32_t insn)
{
    /* RdaHi == '11x' is related encoding */
    uint32_t rdahi = extract32(insn, 20, 3);
    if((rdahi & 6) == 6) {
        return false;
    }
    return (insn & 0xEF810FD1) == 0xEE800F00;
}

static inline bool is_insn_vrmlsldavh(uint32_t insn)
{
    /* RdaHi == '111' is related encoding */
    uint32_t rdahi = extract32(insn, 20, 3);
    return rdahi != 7 && (insn & 0xFF810FD1) == 0xFE800E01;
}

static inline bool is_insn_vfma(uint32_t insn)
{
    return (insn & 0xFFE11FF1) == 0xEF000C50;
}

static inline bool is_insn_vfms(uint32_t insn)
{
    return (insn & 0xFFE11FF1) == 0xEF200C50;
}

static inline bool is_insn_vqdmladh(uint32_t insn)
{
    /* size == '11' is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF810F50) == 0xEE000E00;
}

static inline bool is_insn_vqdmlsdh(uint32_t insn)
{
    /* size == '11' is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF810F50) == 0xFE000E00;
}

static inline bool is_insn_vsri(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800450;
}

static inline bool is_insn_vsli(uint32_t insn)
{
    return (insn & 0xFF801FD1) == 0xFF800550;
}

static inline bool is_insn_vmulh_vrmulh(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xEFC10FF1) == 0xEE010E01;
}

static inline bool is_insn_vqdmull(uint32_t insn)
{
    return (insn & 0xEFF10FF1) == 0xEE300F01;
}

static inline bool is_insn_vqdmull_scalar(uint32_t insn)
{
    return (insn & 0xEFF10FF0) == 0xEE300F60;
}

static inline bool is_insn_vqdmulh(uint32_t insn)
{
    /* size == '11' is undefined */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xEFC11FF1) == 0xEF000B40;
}

static inline bool is_insn_vqdmulh_scalar(uint32_t insn)
{
    /* size == '11' is undefined */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xEF811F70) == 0xEE010E60;
}

static inline bool is_insn_vqdmlah(uint32_t insn)
{
    /* size == '11' is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFFC11FD0) == 0xEE000E40;
}

static inline bool is_insn_vqdmlash(uint32_t insn)
{
    /* size == '11' is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFFC11FD0) == 0xEE001E40;
}

static inline bool is_insn_vqrshrn(uint32_t insn)
{
    return (insn & 0xEFA00FD1) == 0xEE800F41;
}

static inline bool is_insn_vqrshrun(uint32_t insn)
{
    return (insn & 0xFFA00FD1) == 0xFE800FC0;
}

static inline bool is_insn_vstr(uint32_t insn)
{
    return (insn & 0xFFB01E00) == 0xEC800E00;
}

static inline bool is_insn_vstr_imm(uint32_t insn)
{
    return (insn & 0xFF111E00) == 0xFD001E00;
}

static inline bool is_insn_vldr(uint32_t insn)
{
    return (insn & 0xEFB01E00) == 0xEC900E00;
}

static inline bool is_insn_vldr_imm(uint32_t insn)
{
    return (insn & 0xFF111E00) == 0xFD101E00;
}

static inline bool is_insn_vqshrn(uint32_t insn)
{
    return (insn & 0xEFA00FD1) == 0xEE800F40;
}

static inline bool is_insn_vqshrun(uint32_t insn)
{
    return (insn & 0xFFA00FD1) == 0xEE800FC0;
}

static inline bool is_insn_vbrsr(uint32_t insn)
{
    /* size == 3 is related encoding */
    uint32_t size = extract32(insn, 20, 2);
    return size != 3 && (insn & 0xFF811F70) == 0xFE011E60;
}

static inline bool is_insn_vshlc(uint32_t insn)
{
    return (insn & 0xFFA01FF0) == 0xEEA00FC0;
}

static inline bool is_insn_vshrn(uint32_t insn)
{
    return (insn & 0xFFA00FD1) == 0xEE800FC1;
}

static inline bool is_insn_vrshrn(uint32_t insn)
{
    return (insn & 0xFFA00FD1) == 0xFE800FC1;
}

static void mve_extract_vlstm_vlldm(arg_vlstm_vlldm *a, uint32_t insn)
{
    a->low_regs_only = !extract32(insn, 7, 1);
    a->is_vlldm = !!extract32(insn, 20, 1);
    a->rn = extract32(insn, 16, 4);
}

static void mve_extract_vscclrm(arg_vscclrm *a, uint32_t insn)
{
    if(extract32(insn, 8, 1)) {
        a->size = DOUBLE_PRECISION;
    } else {
        a->size = SINGLE_PRECISION;
    }

    uint32_t vd = extract32(insn, 12, 4);
    uint32_t d = extract32(insn, 22, 1);
    if(a->size == DOUBLE_PRECISION) {
        a->vd = deposit32(vd, 4, 1, d);
        a->imm = extract32(insn, 1, 7);
    } else {
        a->vd = deposit32(d, 1, 4, vd);
        a->imm = extract32(insn, 0, 8);
    }
}

static void mve_extract_vmsr_vmrs(arg_vmsr_vmrs *a, uint32_t insn)
{
    a->rt = extract32(insn, 12, 4);
    a->reg = extract32(insn, 16, 4);
    a->l = extract32(insn, 20, 1);
}

/* Extract arguments of loads/stores to system registers */
static void mve_extract_vldr_vstr_sysreg(arg_vldr_vstr_sysreg *a, uint32_t insn)
{
    a->rn = extract32(insn, 16, 4);
    a->reg = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->imm = extract32(insn, 0, 7) << 2;
    a->w = extract32(insn, 21, 1);
    a->a = extract32(insn, 23, 1);
    a->p = extract32(insn, 24, 1);
}

/* Extract arguments of loads/stores */
static void mve_extract_vldr_vstr(arg_vldr_vstr *a, uint32_t insn)
{
    a->rn = extract32(insn, 16, 4);
    a->l = extract32(insn, 20, 1);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->u = 0;
    a->imm = extract32(insn, 0, 7);
    a->p = extract32(insn, 24, 1);
    a->a = extract32(insn, 23, 1);
    a->w = extract32(insn, 21, 1);
    a->size = extract32(insn, 7, 2);
}

/* Extract arguments of widening/narrowing loads/stores */
static void mve_extract_vldr_vstr_widening(arg_vldr_vstr *a, uint32_t insn)
{
    a->rn = extract32(insn, 16, 3);
    a->l = extract32(insn, 20, 1);
    a->qd = extract32(insn, 13, 3);
    a->u = extract32(insn, 28, 1);
    a->imm = extract32(insn, 0, 7);
    a->p = extract32(insn, 24, 1);
    a->a = extract32(insn, 23, 1);
    a->w = extract32(insn, 21, 1);
    a->size = extract32(insn, 7, 2);
}

/* Extract arguments of (de)interleaving stores/loads */
static void mve_extract_vldst_il(arg_vldst_il *a, uint32_t insn)
{
    a->qd = extract32(insn, 13, 3);
    a->rn = extract32(insn, 16, 4);
    a->size = extract32(insn, 7, 2);
    a->pat = extract32(insn, 5, 2);
    a->w = extract32(insn, 21, 1);
}

/* Extract arguments of 2-operand scalar */
static void mve_extract_2op_scalar(arg_2scalar *a, uint32_t insn)
{
    a->size = extract32(insn, 20, 2);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->rm = extract32(insn, 0, 4);
}

/* Extract arguments of 2-operand scalar floating-point */
static void mve_extract_2op_fp_scalar(arg_2scalar *a, uint32_t insn)
{
    a->size = extract32(insn, 28, 1);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->rm = extract32(insn, 0, 4);
}

/* Extract arguments of 2-operand scalar with size on 28th bit */
static void mve_extract_2op_scalar_sz28(arg_2scalar *a, uint32_t insn)
{
    a->size = extract32(insn, 28, 1) + 1;
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->rm = extract32(insn, 0, 4);
}

/* Extract arguments of 2 operand floating operations */
static void mve_extract_2op_fp(arg_2op *a, uint32_t insn)
{
    a->size = extract32(insn, 20, 1);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
}

/* Extract arguments of 2 operand vector operations */
static void mve_extract_2op(arg_2op *a, uint32_t insn)
{
    a->size = extract32(insn, 20, 2);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
}

/* Extract arguments for VDUP instruction */
static void mve_extract_vdup(arg_vdup *a, uint32_t insn)
{
    a->size = deposit32(extract32(insn, 5, 1), 1, 31, extract32(insn, 22, 1));
    a->qd = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->rt = extract32(insn, 12, 4);
}

/* Extract arguments of VCTP instruction */
static void mve_extract_vctp(arg_vctp *a, uint32_t insn)
{
    a->size = extract32(insn, 20, 2);
    a->rn = extract32(insn, 16, 4);
}

/* Extract arguments of VPST instruction */
static void mve_extract_vpst(arg_vpst *a, uint32_t insn)
{
    a->mask = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
}

/* Extract arguments of VCMP instruction */
static void mve_extract_vcmp(arg_vcmp *a, uint32_t insn)
{
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qn = extract32(insn, 17, 3);
    a->size = extract32(insn, 20, 2);
    a->mask = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
}

/* Extract arguments of VCMP scalar instruction */
static void mve_extract_vcmp_scalar(arg_vcmp_scalar *a, uint32_t insn)
{
    a->mask = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = extract32(insn, 17, 3);
    a->size = extract32(insn, 20, 2);
    a->rm = extract32(insn, 0, 4);
}

/* Extract arguments of VCMP floating-point instruction */
static void mve_extract_vcmp_fp(arg_vcmp *a, uint32_t insn)
{
    a->qn = extract32(insn, 17, 3);
    a->mask = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->size = extract32(insn, 28, 1);
}

/* Extract arguments of VCMP scalar floating-point instruction */
static void mve_extract_vcmp_fp_scalar(arg_vcmp_scalar *a, uint32_t insn)
{
    a->qn = extract32(insn, 17, 3);
    a->mask = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->rm = extract32(insn, 0, 4);
    a->size = extract32(insn, 28, 1);
}

/* Extract arguments for VIDUP and VDDUP */
static void mve_extract_vidup(arg_vidup *a, uint32_t insn)
{
    uint32_t imm = deposit32(extract32(insn, 0, 1), 1, 31, extract32(insn, 7, 1));
    a->imm = 1 << imm;
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->size = extract32(insn, 20, 2);
    /* Decode for this instruction puts a zero at least significant bit */
    a->rn = extract32(insn, 17, 3) << 1;
}

/* Extract arguments of VIWDUP and VDWDUP instructions */
static void mve_extract_viwdup(arg_viwdup *a, uint32_t insn)
{
    uint32_t imm = deposit32(extract32(insn, 0, 1), 1, 31, extract32(insn, 7, 1));
    a->imm = 1 << imm;
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->size = extract32(insn, 20, 2);
    /* Decode for this instruction puts a one at least significant bit */
    a->rm = (extract32(insn, 1, 3) << 1) + 1;
    /* Decode for this instruction puts a zero at least significant bit */
    a->rn = extract32(insn, 17, 3) << 1;
}

/* Extract arguments of VMAXV/VMAXAV/VMINV/VMINAV instructions */
static void mve_extract_vmaxv(arg_vmaxv *a, uint32_t insn)
{
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->rda = extract32(insn, 12, 4);
    a->size = extract32(insn, 18, 2);
}

/* Extract arguments of VMAXNMV/VMAXNMAV/VMINNMV/VMINNMAV instructions */
static void mve_extract_vmaxnmv(arg_vmaxv *a, uint32_t insn)
{
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->rda = extract32(insn, 12, 4);
    a->size = extract32(insn, 28, 1);
}

/* Extract arguments of VMAXNMA/VMINNMA instructions */
static void mve_extract_vmaxnma(arg_2op *a, uint32_t insn)
{
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->size = extract32(insn, 28, 1);
}

/* Extract arguments of 2-operand instructions without a size */
static void mve_extract_2op_no_size(arg_2op *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->size = 0;
}

/* Extract arguments of VMUL instruction */
static void mve_extract_vmul(arg_2op *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    /*
     * We have to swap it here because for most instruction size of 1 means F32
     * but for some size of 0 means F32. We pass the size into `DO_TRANS_2OP_FP`
     * where it expect size of 0 zero to mean F32.
     */
    a->size = extract32(insn, 28, 1) == 0 ? 1 : 0;
}

/* Extract arguments of 1-operand instruction */
static void mve_extract_1op(arg_1op *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->size = extract32(insn, 18, 2);
}

/* Extract arguments of 1-operand instruction without a size */
static void mve_extract_1op_no_size(arg_1op *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->size = 0;
}

/* Extract arguments of vcvt fixed instruction */
static void mve_extract_vcvt_fixed(arg_2shift *a, uint32_t insn)
{
    /*
     * From ARMv8-M C2.4.325 reference manual:
     *
     * <fbits>  The number of fraction bits in the fixed-point number. For 16-bit fixed-point, this number
     *          must be in the range 1-16. For 32-bit fixed-point, this number must be in the range 1-32. The
     *          value of (64 - <fbits>) is encoded in imm6.
     */
    a->shift = 64 - extract32(insn, 16, 6);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->size = extract32(insn, 9, 1);
}

/* Extract arguments of immediate value instruction */
static void mve_extract_1imm(arg_1imm *a, uint32_t insn)
{
    a->imm = deposit32(deposit32(extract32(insn, 0, 4), 4, 28, extract32(insn, 16, 3)), 7, 25, extract32(insn, 28, 1));
    a->op = extract32(insn, 5, 1);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->cmode = extract32(insn, 8, 4);
}

/* Extract arguments of VMOV 2-GP<->V-lane instruction */
static void mve_extract_vmov_2gp(arg_vmov_2gp *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->rt2 = extract32(insn, 16, 4);
    a->rt = extract32(insn, 0, 4);
    a->idx = extract32(insn, 4, 1);
    a->from = extract32(insn, 20, 1);
}

/* Extract arguments of VMOV GP<->V-lane instruction */
static void mve_extract_vmov_gp(arg_vmov_gp *a, uint32_t insn)
{
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->rt = extract32(insn, 12, 4);
    a->op1 = extract32(insn, 21, 2);
    a->op2 = extract32(insn, 5, 2);
    a->h = extract32(insn, 16, 1);
    a->u = extract32(insn, 23, 1);
}
/* Extract arguments of left shift immediate instruction */
static void mve_extract_lshift_imm(arg_2shift *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    uint32_t sz = extract32(insn, 19, 3);
    uint32_t imm6 = extract32(insn, 16, 6);
    if((sz & 0b111) == 0b001) {
        a->size = 0;
        a->shift = imm6 - 8;
    } else if((sz & 0b110) == 0b010) {
        a->size = 1;
        a->shift = imm6 - 16;
    } else if((sz & 0b100) == 0b100) {
        a->size = 2;
        a->shift = imm6 - 32;
    } else {
        __builtin_unreachable();
    }
}

/* Extract arguments of right shift immediate instruction */
static void mve_extract_rshift_imm(arg_2shift *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    uint32_t sz = extract32(insn, 19, 3);
    uint32_t imm6 = deposit32(extract32(insn, 16, 3), 3, 29, extract32(insn, 19, 3));
    if((sz & 0b111) == 0b001) {
        a->size = 0;
        a->shift = 16 - imm6;
    } else if((sz & 0b110) == 0b010) {
        a->size = 1;
        a->shift = 32 - imm6;
    } else if((sz & 0b100) == 0b100) {
        a->size = 2;
        a->shift = 64 - imm6;
    } else {
        __builtin_unreachable();
    }
}

/* Extract arguments of vshll T1 encoding */
static void mve_extract_vshll_imm(arg_2shift *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    uint32_t sz = extract32(insn, 19, 2);
    uint32_t imm5 = deposit32(extract32(insn, 16, 3), 3, 29, sz);
    if(sz == 0b01) {
        a->size = 0;
        a->shift = imm5 - 8;
    } else if((sz & 0b10) == 0b10) {
        a->size = 1;
        a->shift = imm5 - 16;
    } else {
        tlib_abortf("Unsupported size (sz field) for vshll instruction: 0x%x", sz);
    }
}

/* Extract arguments of vshll T2 encoding */
static void mve_extract_vshll(arg_2shift *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->size = extract32(insn, 18, 2);

    if(a->size == 0b10) {
        tlib_abortf("Unsupported size for vshll instruction: 0x%x", a->size);
    }
    a->shift = 8 << a->size;
}

/* Extract arguments of 2-shift scalar instruction */
static void mve_extract_2shift_scalar(arg_shl_scalar *a, uint32_t insn)
{
    a->qda = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->rm = extract32(insn, 0, 4);
    a->size = extract32(insn, 18, 2);
}

/* Extract arguments of vmlaldav/vmlsldav instructions */
static void mve_extract_vmlaldav(arg_vmlaldav *a, uint32_t insn)
{
    a->a = extract32(insn, 5, 1);
    a->rdahi = 2 * extract32(insn, 20, 3) + 1;
    a->x = extract32(insn, 12, 1);
    a->size = extract32(insn, 16, 1) + 1;
    a->rdalo = 2 * extract32(insn, 13, 3);
    a->qm = extract32(insn, 1, 3);
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
}

/* Extract arguments of vmladav/vmlsdav instructions */
static void mve_extract_vmladav(arg_vmladav *a, uint32_t insn)
{
    a->a = extract32(insn, 5, 1);
    a->x = extract32(insn, 12, 1);
    a->size = extract32(insn, 16, 1) + 1;
    a->qm = extract32(insn, 1, 3);
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->rda = 2 * extract32(insn, 13, 3);
}

static void mve_extract_vabav(arg_vabav *a, uint32_t insn)
{
    a->rda = extract32(insn, 12, 4);
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->size = extract32(insn, 20, 2);
}

/* Extract arguments of VCMLA/VCADD floating-point instructions */
static void mve_extract_2op_fp_size_rev(arg_2op *a, uint32_t insn)
{
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    /*
     * We have to swap it here because for most instruction size of 1 means F32
     * but for some size of 0 means F32. We pass the size into `DO_TRANS_2OP_FP`
     * where it expect size of 0 zero to mean F32.
     */
    a->size = extract32(insn, 20, 1) == 1 ? 0 : 1;
}

/* Extract arguments of 2-operand instructions with size on 28th bit */
static void mve_extract_2op_sz28(arg_2op *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->size = extract32(insn, 28, 1) + 1;
}

/* Extract arguments of VADD(L)V(A) instructions */
static void mve_extract_vaddv(arg_vaddv *a, uint32_t insn)
{
    a->u = extract32(insn, 28, 1);
    a->a = extract32(insn, 5, 1);
    a->size = extract32(insn, 18, 2);
    a->qm = extract32(insn, 1, 3);
    a->rda = 2 * extract32(insn, 13, 3);
    a->rdahi = 2 * extract32(insn, 20, 3) + 1;
}

/* Extract arguments of shifts by immediate (1-32) */
static void extract_mve_sh_i(DisasContext *context, arg_mve_sh_i *args, uint32_t instruction)
{
    args->rda = extract32(instruction, 16, 4);
    args->shim = deposit32(extract32(instruction, 6, 2), 2, 30, extract32(instruction, 12, 3));
}

/* Extract arguments of shifts by register */
static void extract_mve_sh_rr(DisasContext *context, arg_mve_sh_rr *args, uint32_t instruction)
{
    args->rda = extract32(instruction, 16, 4);
    args->rm = extract32(instruction, 12, 4);
}

/* Extract arguments of long (64-bit) shifts by immediate (1-32) */
static void extract_mve_shl_ri(DisasContext *context, arg_mve_shl_ri *args, uint32_t instruction)
{
    args->shim = deposit32(extract32(instruction, 6, 2), 2, 30, extract32(instruction, 12, 3));
    args->rdalo = times_2(context, extract32(instruction, 17, 3));
    args->rdahi = times_2_plus_1(context, extract32(instruction, 9, 3));
}

/* Extract arguments of long (64-bit) shifts by register */
static void extract_mve_shl_rr(DisasContext *context, arg_mve_shl_rr *args, uint32_t instruction)
{
    args->rm = extract32(instruction, 12, 4);
    args->rdalo = times_2(context, extract32(instruction, 17, 3));
    args->rdahi = times_2_plus_1(context, extract32(instruction, 9, 3));
}

/* Extract arguments of VQ(R)DMLADH(X) and VQ(R)DMLSDH(X) instructions */
static void mve_extract_vqdmladh(arg_vqdmladh *a, uint32_t insn)
{
    a->size = extract32(insn, 20, 2);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->qn = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
    a->x = extract32(insn, 12, 1);
}

/* Extract arguments of VSTR/VLDST (vector) instructions */
static void mve_extract_vldst_sg(arg_vldst_sg *a, uint32_t insn)
{
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->os = extract32(insn, 0, 1);
    a->size = extract32(insn, 7, 2);
    a->rn = extract32(insn, 16, 4);
    a->msize = deposit32(extract32(insn, 4, 1), 1, 31, extract32(insn, 6, 1));
    a->qm = deposit32(extract32(insn, 1, 3), 3, 29, extract32(insn, 5, 1));
}

/* Extract arguments of VSTR/VLDST (vector/immediate) instructions */
static void mve_extract_vldst_sg_imm(arg_vldst_sg_imm *a, uint32_t insn)
{
    a->a = extract32(insn, 23, 1);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->imm = extract32(insn, 0, 7);
    a->qm = deposit32(extract32(insn, 17, 3), 3, 29, extract32(insn, 7, 1));
    a->w = extract32(insn, 21, 1);
}

/* Extract arguments of VSHLC instruction */
static void mve_extract_vshlc(arg_vshlc *a, uint32_t insn)
{
    a->rdm = extract32(insn, 0, 4);
    a->qd = deposit32(extract32(insn, 13, 3), 3, 29, extract32(insn, 22, 1));
    a->imm = extract32(insn, 16, 5);
}
