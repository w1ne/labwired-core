/*
 *  RISC-V main translation routines
 *
 *  Author: Sagar Karandikar, sagark@eecs.berkeley.edu
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
#include "bit_helper.h"
#include "cpu.h"

#include "instmap.h"
#include "debug.h"
#include "arch_callbacks.h"
#include "cpu_registers.h"
#include "atomic-intrinsics.h"
#include "tcg-op-atomic.h"
#include "hash-table-store-test.h"
#include "tcg-op.h"

/* global register indices */
static TCGv cpu_gpr[32], cpu_pc, cpu_opcode;
static TCGv_i64 cpu_fpr[32]; /* assume F and D extensions */
static TCGv cpu_vstart;
static TCGv cpu_prev_sp;
static TCGv reserved_address;

#include "tb-helper.h"

void translate_init(void)
{
    int i;

    static const char *const regnames[] = { "zero", "ra  ", "sp  ", "gp  ", "tp  ", "t0  ", "t1  ", "t2  ",
                                            "s0  ", "s1  ", "a0  ", "a1  ", "a2  ", "a3  ", "a4  ", "a5  ",
                                            "a6  ", "a7  ", "s2  ", "s3  ", "s4  ", "s5  ", "s6  ", "s7  ",
                                            "s8  ", "s9  ", "s10 ", "s11 ", "t3  ", "t4  ", "t5  ", "t6  " };

    static const char *const fpr_regnames[] = { "ft0", "ft1", "ft2", "ft3", "ft4",  "ft5",  "ft6", "ft7", "fs0",  "fs1", "fa0",
                                                "fa1", "fa2", "fa3", "fa4", "fa5",  "fa6",  "fa7", "fs2", "fs3",  "fs4", "fs5",
                                                "fs6", "fs7", "fs8", "fs9", "fs10", "fs11", "ft8", "ft9", "ft10", "ft11" };

    /* cpu_gpr[0] is a placeholder for the zero register. Do not use it. */
    /* Use the gen_set_gpr and gen_get_gpr helper functions when accessing */
    /* registers, unless you specifically block reads/writes to reg 0 */
    TCGV_UNUSED(cpu_gpr[0]);
    for(i = 1; i < 32; i++) {
        cpu_gpr[i] = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, gpr[i]), regnames[i]);
    }

    for(i = 0; i < 32; i++) {
        cpu_fpr[i] = tcg_global_mem_new_i64(TCG_AREG0, offsetof(CPUState, fpr[i]), fpr_regnames[i]);
    }

    cpu_pc = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, pc), "pc");
    cpu_opcode = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, opcode), "opcode");
    cpu_vstart = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, vstart), "vstart");

    cpu_prev_sp = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, prev_sp), "previous_sp");

    reserved_address = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, reserved_address), "reserved_address");
}

static inline void kill_unknown(DisasContext *dc, int excp);

//  Other values are defined in `exec-all.h`
#define DISAS_STOP   4 /* Need to exit tb for syscall, sret, etc. */
#define DISAS_NONE   5 /* When seen outside of translation while loop, indicates need to exit tb due to end of page. */
#define DISAS_BRANCH 6 /* Need to exit tb for branch, jal, etc. */

#ifdef TARGET_RISCV64
#define CASE_OP_32_64(X) \
    case X:              \
    case glue(X, W)
#define BITMANIP_SHAMT_MASK 0x3F
#else
#define CASE_OP_32_64(X)    case X
#define BITMANIP_SHAMT_MASK 0x1F
#define BITMANIP_ZIP_GROUP  0x8F
#endif

//  RISC-V User ISA, Release 2.2, section 1.2 Instruction Length Encoding
static int decode_instruction_length(uint16_t opcode_first_word)
{
    int instruction_length = 0;
    if((opcode_first_word & 0b11) != 0b11) {
        instruction_length = 2;
    } else if((opcode_first_word & 0b11100) != 0b11100) {
        instruction_length = 4;
    } else if((opcode_first_word & 0b111111) == 0b011111) {
        instruction_length = 6;
    } else if((opcode_first_word & 0b1111111) == 0b0111111) {
        instruction_length = 8;
    } else if(extract16(opcode_first_word, 12, 3) != 0b111) {
        instruction_length = 10 + 2 * extract16(opcode_first_word, 12, 3);
    } else {
        //  Reserved for >=192 bits, this function returns 0 in that case.
    }
    return instruction_length;
}

static inline uint64_t format_opcode(uint64_t opcode, int instruction_length)
{
    return opcode & (((typeof(opcode))1 << (8 * instruction_length)) - 1);
}

static void log_disabled_extension_and_kill_unknown(DisasContext *dc, enum riscv_feature ext, const char *message)
{
    if(!riscv_silent_ext(cpu, ext)) {
        char letter = 0;
        riscv_features_to_string(ext, &letter, 1);

        int instruction_length = decode_instruction_length(dc->opcode);
        if(message == NULL) {
            tlib_printf(LOG_LEVEL_ERROR, "PC: 0x%llx, opcode: 0x%0*llx, RISC-V '%c' instruction set is not enabled for this CPU!",
                        dc->base.pc, /* padding */ 2 * instruction_length, format_opcode(dc->opcode, instruction_length), letter);
        } else {
            tlib_printf(LOG_LEVEL_ERROR,
                        "PC: 0x%llx, opcode: 0x%0*llx, RISC-V '%c' instruction set is not enabled for this CPU! %s", dc->base.pc,
                        /* padding */ 2 * instruction_length, format_opcode(dc->opcode, instruction_length), letter, message);
        }
    }

    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
}

static int ensure_extension(DisasContext *dc, target_ulong ext)
{
    if(riscv_has_ext(cpu, ext)) {
        return 1;
    }
    log_disabled_extension_and_kill_unknown(dc, ext, NULL);
    return 0;
}

static int ensure_additional_extension(DisasContext *dc, enum riscv_additional_feature ext)
{
    if(riscv_has_additional_ext(cpu, ext)) {
        return 1;
    }

    char *encoding = NULL;
    switch(ext) {
        case RISCV_FEATURE_ZBA:
            encoding = "ba";
            break;
        case RISCV_FEATURE_ZBB:
            encoding = "bb";
            break;
        case RISCV_FEATURE_ZBC:
            encoding = "bc";
            break;
        case RISCV_FEATURE_ZBS:
            encoding = "bs";
            break;
        case RISCV_FEATURE_ZICSR:
            encoding = "icsr";
            break;
        case RISCV_FEATURE_ZIFENCEI:
            encoding = "ifencei";
            break;
        case RISCV_FEATURE_ZFH:
            encoding = "fh";
            break;
        case RISCV_FEATURE_ZVFH:
            encoding = "vfh";
            break;
        case RISCV_FEATURE_ZVE32X:
            encoding = "ve32x";
            break;
        case RISCV_FEATURE_ZVE32F:
            encoding = "ve32f";
            break;
        case RISCV_FEATURE_ZVE64X:
            encoding = "ve64x";
            break;
        case RISCV_FEATURE_ZVE64F:
            encoding = "ve64f";
            break;
        case RISCV_FEATURE_ZVE64D:
            encoding = "ve64d";
            break;
        case RISCV_FEATURE_ZACAS:
            encoding = "acas";
            break;
        case RISCV_FEATURE_ZCB:
            encoding = "cb";
            break;
        case RISCV_FEATURE_ZCMP:
            encoding = "cmp";
            break;
        case RISCV_FEATURE_ZCMT:
            encoding = "cmt";
            break;
        case RISCV_FEATURE_ZICOND:
            encoding = "icond";
            break;
        default:
            tlib_printf(LOG_LEVEL_ERROR, "Unexpected additional extension encoding: %d", ext);
            break;
    }

    int instruction_length = decode_instruction_length(dc->opcode);
    tlib_printf(LOG_LEVEL_ERROR, "RISC-V Z%s instruction set is not enabled for this CPU! PC: 0x%llx, opcode: 0x%0*llx", encoding,
                dc->base.pc, /* padding */ 2 * instruction_length, format_opcode(dc->opcode, instruction_length));

    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
    return 0;
}

static int ensure_fp_extension(DisasContext *dc, int precision_bit)
{
    switch((enum riscv_floating_point_precision)extract64(dc->opcode, precision_bit, 2)) {
        case RISCV_HALF_PRECISION:
            return ensure_additional_extension(dc, RISCV_FEATURE_ZFH);
        case RISCV_SINGLE_PRECISION:
            return ensure_extension(dc, RISCV_FEATURE_RVF);
        case RISCV_DOUBLE_PRECISION:
            return ensure_extension(dc, RISCV_FEATURE_RVD);
        default:
            tlib_printf(LOG_LEVEL_ERROR, "Unknown floating point instruction encoding! PC: 0x%llx", dc->base.pc);
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            return false;
    }
}

static int ensure_fp_extension_for_load_store(DisasContext *dc, uint32_t opc)
{
    //  The FS/L W and D has compressed variants with a different way of encoding width,
    //  so they have to be handled seperatly
    switch(opc) {
        case OPC_RISC_FSW:
        case OPC_RISC_FLW:
            return ensure_extension(dc, RISCV_FEATURE_RVF);
        case OPC_RISC_FSD:
        case OPC_RISC_FLD:
            return ensure_extension(dc, RISCV_FEATURE_RVD);
        default:
            switch(extract64(dc->opcode, 12, 3)) {
                case 1:
                    return ensure_additional_extension(dc, RISCV_FEATURE_ZFH);
                case 2:
                    return ensure_extension(dc, RISCV_FEATURE_RVF);
                case 3:
                    return ensure_extension(dc, RISCV_FEATURE_RVD);
                default:
                    tlib_printf(LOG_LEVEL_ERROR, "Unknown floating point instruction encoding! PC: 0x%llx", dc->base.pc);
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    return false;
            }
    }
}

static inline bool ensure_vector_embedded_extension_or_kill_unknown(DisasContext *dc)
{
    //  Check if the most basic extension is supported.
    if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE32X)) {
        return true;
    }

    log_disabled_extension_and_kill_unknown(dc, RISCV_FEATURE_RVV, NULL);
    return false;
}

static inline bool ensure_vector_embedded_extension_for_vsew_or_kill_unknown(DisasContext *dc, target_ulong vsew)
{
    //  Assume there is no EEW larger than 64.
    if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE64X)) {
        return true;
    }

    if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE32X)) {
        if(vsew < 0b11) {
            return true;
        }
        log_disabled_extension_and_kill_unknown(dc, RISCV_FEATURE_RVV, "vsew is too large for the Zve32x extension");
    } else {
        log_disabled_extension_and_kill_unknown(dc, RISCV_FEATURE_RVV, NULL);
    }
    return false;
}

void gen_sync_pc(DisasContext *dc)
{
    tcg_gen_movi_tl(cpu_pc, dc->base.pc);
    tcg_gen_movi_tl(cpu_opcode, dc->opcode);
}

static inline void generate_exception(DisasContext *dc, int excp)
{
    gen_sync_pc(dc);
    TCGv_i32 helper_tmp = tcg_const_i32(excp);
    gen_helper_raise_exception(cpu_env, helper_tmp);
    tcg_temp_free_i32(helper_tmp);
}

static inline void generate_exception_mbadaddr(DisasContext *dc, int excp)
{
    gen_sync_pc(dc);
    TCGv_i32 helper_tmp = tcg_const_i32(excp);
    gen_helper_raise_exception_mbadaddr(cpu_env, helper_tmp, cpu_pc);
    tcg_temp_free_i32(helper_tmp);
}

/* unknown instruction */
static inline void kill_unknown(DisasContext *dc, int excp)
{
    gen_sync_pc(dc);

    //  According to the RISC-V ISA manual,
    //  for Illegal Instruction, mtval
    //  should contain an opcode of the faulting instruction.
    TCGv_i32 helper_tmp = tcg_const_i32(excp);
    TCGv_i32 helper_bdinstr = tcg_const_i32(dc->opcode);
    gen_helper_raise_exception_mbadaddr(cpu_env, helper_tmp, helper_bdinstr);
    tcg_temp_free_i32(helper_tmp);
    tcg_temp_free_i32(helper_bdinstr);

    dc->base.is_jmp = DISAS_STOP;
}

static inline bool use_goto_tb(DisasContext *dc, target_ulong dest)
{
    return (dc->base.tb->pc & TARGET_PAGE_MASK) == (dest & TARGET_PAGE_MASK);
}

static inline void gen_goto_tb(DisasContext *dc, int n, target_ulong dest)
{
    if(use_goto_tb(dc, dest)) {
        /* chaining is only allowed when the jump is to the same page */
        tcg_gen_goto_tb(n);
        tcg_gen_movi_tl(cpu_pc, dest);
        gen_exit_tb(dc->base.tb, n);
    } else {
        tcg_gen_movi_tl(cpu_pc, dest);
        gen_exit_tb_no_chaining(dc->base.tb);
    }
}

static inline void try_run_gpr_access_hook(int reg_num, int is_write)
{
    if(unlikely(env->are_post_gpr_access_hooks_enabled)) {
        if(env->post_gpr_access_hook_mask & (1u << reg_num)) {
            TCGv_i32 register_index = tcg_const_i32(reg_num);
            TCGv_i32 is_write_const = tcg_const_i32(is_write);
            gen_helper_handle_post_gpr_access_hook(register_index, is_write_const);
            tcg_temp_free_i32(register_index);
            tcg_temp_free_i32(is_write_const);
        }
    }
}

/* Wrapper for getting reg values - need to check of reg is zero since
 * cpu_gpr[0] is not actually allocated
 */
static inline void gen_get_gpr(TCGv t, int reg_num)
{
    try_run_gpr_access_hook(reg_num, 0);

    if(reg_num == 0) {
        tcg_gen_movi_tl(t, 0);
    } else {
        tcg_gen_mov_tl(t, cpu_gpr[reg_num]);
    }
}

static inline void gen_get_fpr(TCGv_i64 t, int reg_num)
{
    tcg_gen_mov_tl(t, cpu_fpr[reg_num]);
}

/* Wrapper for setting reg values - need to check of reg is zero since
 * cpu_gpr[0] is not actually allocated. this is more for safety purposes,
 * since we usually avoid calling the OP_TYPE_gen function if we see a write to
 * $zero
 */
static inline void gen_set_gpr(int reg_num_dst, TCGv t)
{
    if(reg_num_dst != 0) {
        tcg_gen_mov_tl(cpu_gpr[reg_num_dst], t);
    }

    try_run_gpr_access_hook(reg_num_dst, 1);
}

static inline uint32_t opc_to_width(uint32_t opc)
{
    switch(opc) {
        case OPC_RISC_SB:
        case OPC_RISC_LB:
        case OPC_RISC_LBU:
            return 8;
        case OPC_RISC_SH:
        case OPC_RISC_LH:
        case OPC_RISC_LHU:
        case OPC_RISC_FLH:
        case OPC_RISC_FSH:
            return 16;
        case OPC_RISC_SW:
        case OPC_RISC_LW:
        case OPC_RISC_LWU:
        case OPC_RISC_FLW:
        case OPC_RISC_FSW:
            return 32;
        case OPC_RISC_SD:
        case OPC_RISC_LD:
        case OPC_RISC_FLD:
        case OPC_RISC_FSD:
            return 64;
        default:
            tlib_printf(LOG_LEVEL_WARNING, "opc_to_width called on unsupported opcode %x", opc);
            return 0;
    }
}

static inline void try_run_pre_stack_access_hook(TCGv addr, uint32_t width, bool is_write)
{
    //  Don't fire the hook is the function was passed an invalide width
    if(unlikely(env->is_pre_stack_access_hook_enabled) && width > 0) {
        TCGv_i32 width_tmp = tcg_const_i32(width);
        TCGv_i32 is_write_tmp = tcg_const_i32(is_write);
        gen_helper_handle_pre_stack_access_hook(addr, width_tmp, is_write_tmp);
        tcg_temp_free_i32(width_tmp);
        tcg_temp_free_i32(is_write_tmp);
    }
}

static inline void gen_orcb(TCGv source1, int max_byte_index)
{
    target_ulong byte_mask = 0xff;
    TCGv t0 = tcg_temp_local_new();
    for(int i = max_byte_index; i >= 0; i--) {
        int next_byte = gen_new_label();
        tcg_gen_movi_tl(t0, byte_mask << i * 8);
        tcg_gen_and_tl(t0, t0, source1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, next_byte);
        tcg_gen_ori_tl(source1, source1, byte_mask << i * 8);
        gen_set_label(next_byte);
    }
    tcg_temp_free(t0);
}

static inline void gen_cpopx(TCGv source1, int length)
{
    TCGv t0 = tcg_temp_local_new();
    TCGv t1 = tcg_temp_local_new();
    TCGv i = tcg_temp_local_new();
    int loop = gen_new_label();

    tcg_gen_movi_tl(t0, 0);
    tcg_gen_movi_tl(i, 0);

    gen_set_label(loop);
    tcg_gen_andi_tl(t1, source1, 1);
    tcg_gen_add_tl(t0, t0, t1);
    tcg_gen_shri_tl(source1, source1, 1);
    tcg_gen_addi_tl(i, i, 1);
    tcg_gen_brcondi_tl(TCG_COND_LT, i, length, loop);
    tcg_gen_mov_tl(source1, t0);

    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(i);
}

static inline void gen_clmulx(TCGv source1, TCGv source2, int width, int reversed, int high_bits)
{
    TCGv t0, i, result;
    int next_bit, loop;
    t0 = tcg_temp_local_new();
    i = tcg_temp_local_new();
    result = tcg_temp_local_new();
    next_bit = gen_new_label();
    loop = gen_new_label();

    tcg_gen_movi_tl(result, 0);
    tcg_gen_movi_tl(i, high_bits - reversed);

    gen_set_label(loop);
    tcg_gen_shr_tl(t0, source2, i);
    tcg_gen_andi_tl(t0, t0, 1);
    tcg_gen_brcondi_tl(TCG_COND_NE, t0, 1, next_bit);

    if(high_bits) {
        tcg_gen_movi_tl(t0, width - reversed);
        tcg_gen_sub_tl(t0, t0, i);
        tcg_gen_shr_tl(t0, source1, t0);
    } else {
        tcg_gen_shl_tl(t0, source1, i);
    }
    tcg_gen_xor_tl(result, result, t0);

    gen_set_label(next_bit);
    tcg_gen_addi_tl(i, i, 1);
    tcg_gen_brcondi_tl(TCG_COND_LT, i, width, loop);

    tcg_gen_mov_tl(source1, result);

    tcg_temp_free(t0);
    tcg_temp_free(i);
    tcg_temp_free(result);
}

static inline void gen_ctzx(TCGv source1, int width)
{
    TCGv t0, i;
    t0 = tcg_temp_new();
    i = tcg_temp_local_new();
    int finish = gen_new_label();
    int loop = gen_new_label();

    tcg_gen_movi_tl(i, 0);

    gen_set_label(loop);
    tcg_gen_andi_tl(t0, source1, 1);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 1, finish);
    tcg_gen_shri_tl(source1, source1, 1);
    tcg_gen_addi_tl(i, i, 1);
    tcg_gen_brcondi_tl(TCG_COND_LT, i, width, loop);

    gen_set_label(finish);
    tcg_gen_mov_tl(source1, i);
    tcg_temp_free(t0);
    tcg_temp_free(i);
}

static inline void get_set_gpr_imm(int reg_num_dst, target_ulong value)
{
    if(reg_num_dst != 0) {
        tcg_gen_movi_tl(cpu_gpr[reg_num_dst], value);
    }

    try_run_gpr_access_hook(reg_num_dst, 1);
}

/* Some instructions don't allow NFIELDS value to be different from 1, 2, 4 or 8.
 * As NFIELDS can be expressed as `nf + 1` this function checks if the above condition is true, while saving a few clock cycles
 * */
static inline bool is_nfields_power_of_two(uint32_t nf)
{
    return (nf & (nf + 1)) == 0;
}

static inline void generate_vill_check(DisasContext *dc)
{
    TCGv t0 = tcg_temp_local_new();
    int done = gen_new_label();

    tcg_gen_ld_tl(t0, cpu_env, offsetof(CPUState, vill));
    tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0x0, done);

    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);

    gen_set_label(done);
    tcg_temp_free(t0);
}

static void gen_mulhsu(TCGv ret, TCGv arg1, TCGv arg2)
{
    TCGv rl = tcg_temp_new();
    TCGv rh = tcg_temp_new();

    tcg_gen_mulu2_tl(rl, rh, arg1, arg2);
    /* fix up for one negative */
    tcg_gen_sari_tl(rl, arg1, TARGET_LONG_BITS - 1);
    tcg_gen_and_tl(rl, rl, arg2);
    tcg_gen_sub_tl(ret, rh, rl);

    tcg_temp_free(rl);
    tcg_temp_free(rh);
}

/* Zcmp helper functions */
static void gen_push_pop(DisasContext *dc, uint8_t rlist, target_ulong spimm, bool is_push)
{
    //  Validate rlist range
    if(rlist < 4 || rlist > 15) {
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
        return;
    }

    //  For RV32E only rlist values 4-6 are legal
    if(riscv_has_ext(env, RISCV_FEATURE_RVE) && rlist > 6) {
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
        return;
    }

    TCGv sp = tcg_temp_local_new();
    TCGv addr = tcg_temp_local_new();
    TCGv reg = tcg_temp_local_new();

    //  Calculate stack adjustment according to RISC-V Zcmp specification
    target_ulong base_adj;

#if defined(TARGET_RISCV64)
    //  RV64 base adjustments
    if(rlist >= 4 && rlist <= 5) {
        base_adj = 16;
    } else if(rlist >= 6 && rlist <= 7) {
        base_adj = 32;
    } else if(rlist >= 8 && rlist <= 9) {
        base_adj = 48;
    } else if(rlist >= 10 && rlist <= 11) {
        base_adj = 64;
    } else if(rlist >= 12 && rlist <= 13) {
        base_adj = 80;
    } else if(rlist == 14) {
        base_adj = 96;
    } else {  //  rlist == 15
        base_adj = 112;
    }
#else
    //  RV32 base adjustments
    if(rlist >= 4 && rlist <= 7) {
        base_adj = 16;
    } else if(rlist >= 8 && rlist <= 11) {
        base_adj = 32;
    } else if(rlist >= 12 && rlist <= 14) {
        base_adj = 48;
    } else {  //  rlist == 15
        base_adj = 64;
    }
#endif

    //  Total stack adjustment: base + spimm * 16 bytes
    target_ulong total_adj = base_adj + (spimm * 16);

    gen_get_gpr(sp, 2);  //  x2 is stack pointer

    const int reg_size = sizeof(target_ulong);

    TCGv new_sp = tcg_temp_local_new();
    if(is_push) {
        //  PUSH: Per spec, addr=sp-bytes, then store in reverse order
        //  Start from sp - reg_size (before the first register slot)
        tcg_gen_subi_tl(new_sp, sp, total_adj);
        tcg_gen_subi_tl(addr, sp, reg_size);
    } else {
        //  POP: load from sp + stack_adj - reg_size (per Zcmp spec)
        //  The spec says: addr=sp+stack_adj-bytes, then work backwards
        tcg_gen_addi_tl(addr, sp, total_adj - reg_size);
    }

    //  Process registers
    //  Both PUSH and POP iterate in REVERSE order per Zcmp spec
    //  Spec: for(i in 27,26,25,24,23,22,21,20,19,18,9,8,1)
    //  This means: s11,s10,...,s1,s0,ra (reverse of storage order)

    //  Number of s-registers: rlist=15 includes s0-s11 (12 regs), others follow rlist-4
    int s_regs = (rlist == 15) ? 12 : (rlist - 4);

    if(is_push) {
        //  PUSH: Reverse order per spec - store s11 down to s0, then ra
        //  Start address is sp-reg_size, store, then decrement
        for(int i = s_regs - 1; i >= 0; i--) {
            int reg_num = (i <= 1) ? (8 + i) : (16 + i);
            gen_get_gpr(reg, reg_num);
            tcg_gen_qemu_st_tl(reg, addr, dc->base.mem_idx, MO_TETL);
            tcg_gen_subi_tl(addr, addr, reg_size);
        }

        //  Process ra last (stored at lowest address)
        gen_get_gpr(reg, RA);
        tcg_gen_qemu_st_tl(reg, addr, dc->base.mem_idx, MO_TETL);
    } else {
        //  POP: Reverse order per spec - s11-s0, then ra
        //  Start address is sp+stack_adj-reg_size, load, then decrement
        for(int i = s_regs - 1; i >= 0; i--) {
            int reg_num = (i <= 1) ? (8 + i) : (16 + i);

            tcg_gen_qemu_ld_tl(reg, addr, dc->base.mem_idx, MO_TETL);
            gen_set_gpr(reg_num, reg);
            tcg_gen_subi_tl(addr, addr, reg_size);
        }

        //  Process ra last
        tcg_gen_qemu_ld_tl(reg, addr, dc->base.mem_idx, MO_TETL);
        gen_set_gpr(RA, reg);
    }

    //  Update stack pointer atomically after all operations complete
    if(is_push) {
        gen_set_gpr(SP, new_sp);
    } else {
        tcg_gen_addi_tl(sp, sp, total_adj);
        gen_set_gpr(SP, sp);
    }

    tcg_temp_free(new_sp);
    tcg_temp_free(sp);
    tcg_temp_free(addr);
    tcg_temp_free(reg);
}

static inline void gen_push(DisasContext *dc, uint8_t rlist, target_ulong spimm)
{
    gen_push_pop(dc, rlist, spimm, true);
}

static inline void gen_pop(DisasContext *dc, uint8_t rlist, target_ulong spimm)
{
    gen_push_pop(dc, rlist, spimm, false);
}

static void gen_popret_popretz(DisasContext *dc, uint8_t rlist, target_ulong spimm, bool clear_a0)
{
    //  C.POPRET/C.PORETZ combines the functionality of C.POP followed by a return jump

    //  Restore registers from stack and update stack pointer
    gen_pop(dc, rlist, spimm);

    if(clear_a0) {
        //  Set a0 (x10) to 0
        TCGv zero = tcg_const_tl(0);
        gen_set_gpr(10, zero);
        tcg_temp_free(zero);
    }

    //  Then jump to the return address that was loaded into RA register
    //  Load RA (x1) into PC and clear LSB (like JALR does)
    gen_get_gpr(cpu_pc, 1);
    tcg_gen_andi_tl(cpu_pc, cpu_pc, ~(target_ulong)1);

    //  Properly exit the translation block
    gen_exit_tb_no_chaining(dc->base.tb);
    dc->base.is_jmp = DISAS_BRANCH;
}

static inline void gen_popret(DisasContext *dc, uint8_t rlist, target_ulong spimm)
{
    gen_popret_popretz(dc, rlist, spimm, false);
}

static inline void gen_popretz(DisasContext *dc, uint8_t rlist, target_ulong spimm)
{
    gen_popret_popretz(dc, rlist, spimm, true);
}

//  Map s-register selector (3-bit) to architectural x-register number
//  rsc[2:0] encodes s0-s7 using non-contiguous x-register mapping:
//    s0-s1 → x8-x9, s2-s7 → x18-x23
//  Formula from spec: xreg = { rsc[2:1] > 0, rsc[2:1] == 0, rsc[2:0] }
static inline uint8_t sreg_selector_to_xreg(uint8_t rsc)
{
    if((rsc >> 1) > 0) {
        return 16 + rsc;  //  s2-s7 → x18-x23
    } else {
        return 8 + rsc;  //  s0-s1 → x8-x9
    }
}

/* Zcmt helper functions */
static inline void gen_jt(DisasContext *dc, uint8_t index, bool save_ra)
{
    TCGv jvt = tcg_temp_local_new();
    TCGv table_addr = tcg_temp_local_new();
    TCGv target_addr = tcg_temp_local_new();

    //  Save return address if needed (for CM.JALT)
    if(save_ra) {
        TCGv ra = tcg_temp_local_new();
        tcg_gen_movi_tl(ra, dc->base.pc + 2);
        gen_set_gpr(1, ra);  //  x1 is return address register
        tcg_temp_free(ra);
    }

    //  Read JVT CSR directly from CPU state
    tcg_gen_ld_tl(jvt, cpu_env, offsetof(CPUState, jvt));

    //  Calculate table address based on XLEN
    //  RV32: table_address = jvt.base + (index<<2)
    //  RV64: table_address = jvt.base + (index<<3)
#if defined(TARGET_RISCV64)
    const int index_shift = index << 3;
#else
    const int index_shift = index << 2;
#endif
    tcg_gen_mov_tl(table_addr, jvt);
    tcg_gen_addi_tl(table_addr, table_addr, index_shift);

    //  Load jump target from table (XLEN-bit load)
    tcg_gen_qemu_ld_tl(target_addr, table_addr, dc->base.mem_idx, MO_TETL);

    //  Clear LSB and jump to target
    tcg_gen_andi_tl(cpu_pc, target_addr, ~0x1);

    //  Exit translation block
    gen_exit_tb_no_chaining(dc->base.tb);
    dc->base.is_jmp = DISAS_BRANCH;

    tcg_temp_free(jvt);
    tcg_temp_free(table_addr);
    tcg_temp_free(target_addr);
}

static void gen_fsgnj(DisasContext *dc, uint32_t rd, uint32_t rs1, uint32_t rs2, int rm,
                      enum riscv_floating_point_precision precision)
{
    TCGv t0 = tcg_temp_new();
    int fp_ok = gen_new_label();
    int done = gen_new_label();

    int64_t sign_mask = get_float_sign_mask(precision);

    //  check MSTATUS.FS
    tcg_gen_ld_tl(t0, cpu_env, offsetof(CPUState, mstatus));
    tcg_gen_andi_tl(t0, t0, MSTATUS_FS);
    tcg_gen_brcondi_tl(TCG_COND_NE, t0, 0x0, fp_ok);
    //  MSTATUS_FS field was zero:
    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
    tcg_gen_br(done);

    //  proceed with operation
    gen_set_label(fp_ok);
    TCGv_i64 src1 = tcg_temp_local_new_i64();
    TCGv_i64 src2 = tcg_temp_new_i64();

    gen_unbox_float(precision, env, src1, cpu_fpr[rs1]);
    tcg_gen_mov_i64(src2, cpu_fpr[rs2]);

    switch(rm) {
        case 0: /* fsgnj */

            if(rs1 == rs2) { /* FMOV */
                tcg_gen_mov_i64(cpu_fpr[rd], src1);
            }

            tcg_gen_andi_i64(src1, src1, ~sign_mask);
            tcg_gen_andi_i64(src2, src2, sign_mask);
            tcg_gen_or_i64(cpu_fpr[rd], src1, src2);
            gen_box_float(precision, cpu_fpr[rd]);
            break;
        case 1: /* fsgnjn */
            tcg_gen_andi_i64(src1, src1, ~sign_mask);
            tcg_gen_not_i64(src2, src2);
            tcg_gen_andi_i64(src2, src2, sign_mask);
            tcg_gen_or_i64(cpu_fpr[rd], src1, src2);
            gen_box_float(precision, cpu_fpr[rd]);
            break;
        case 2: /* fsgnjx */
            tcg_gen_andi_i64(src2, src2, sign_mask);
            tcg_gen_xor_i64(cpu_fpr[rd], src1, src2);
            gen_box_float(precision, cpu_fpr[rd]);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
    }

    tcg_temp_free_i64(src1);
    tcg_temp_free_i64(src2);
    gen_set_label(done);
    tcg_temp_free(t0);
}

#define SEXT_RESULT_IF_W(res)            \
    do {                                 \
        if(opc & (1 << 3)) {             \
            tcg_gen_ext32s_tl(res, res); \
        }                                \
    } while(0)

static void gen_arith(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2)
{
    TCGv source1, source2, cond1, cond2, zeroreg, resultopt1;
    target_ulong mask;
    source1 = tcg_temp_local_new();
    source2 = tcg_temp_local_new();
    gen_get_gpr(source1, rs1);
    gen_get_gpr(source2, rs2);

    switch(opc) {
        CASE_OP_32_64(OPC_RISC_ADD)
            : tcg_gen_add_tl(source1, source1, source2);
        SEXT_RESULT_IF_W(source1);
        break;
        CASE_OP_32_64(OPC_RISC_SUB)
            : tcg_gen_sub_tl(source1, source1, source2);
        SEXT_RESULT_IF_W(source1);
        break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_SLLW:
            tcg_gen_andi_tl(source2, source2, 0x1F);
            tcg_gen_shl_tl(source1, source1, source2);
            tcg_gen_ext32s_tl(source1, source1);
            break;
#endif
        case OPC_RISC_SLL:
            tcg_gen_andi_tl(source2, source2, TARGET_LONG_BITS - 1);
            tcg_gen_shl_tl(source1, source1, source2);
            break;
        case OPC_RISC_SLT:
            tcg_gen_setcond_tl(TCG_COND_LT, source1, source1, source2);
            break;
        case OPC_RISC_SLTU:
            tcg_gen_setcond_tl(TCG_COND_LTU, source1, source1, source2);
            break;
        case OPC_RISC_XOR:
            tcg_gen_xor_tl(source1, source1, source2);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_SRLW:
            /* clear upper 32 */
            tcg_gen_ext32u_tl(source1, source1);
            tcg_gen_andi_tl(source2, source2, 0x1F);
            tcg_gen_shr_tl(source1, source1, source2);
            tcg_gen_ext32s_tl(source1, source1);
            break;
#endif
        case OPC_RISC_SRL:
            tcg_gen_andi_tl(source2, source2, TARGET_LONG_BITS - 1);
            tcg_gen_shr_tl(source1, source1, source2);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_SRAW:
            /* first, trick to get it to act like working on 32 bits (get rid of
               upper 32, sign extend to fill space) */
            tcg_gen_ext32s_tl(source1, source1);
            tcg_gen_andi_tl(source2, source2, 0x1F);
            tcg_gen_sar_tl(source1, source1, source2);
            tcg_gen_ext32s_tl(source1, source1);
            break;
#endif
        case OPC_RISC_SRA:
            tcg_gen_andi_tl(source2, source2, TARGET_LONG_BITS - 1);
            tcg_gen_sar_tl(source1, source1, source2);
            break;
        case OPC_RISC_OR:
            tcg_gen_or_tl(source1, source1, source2);
            break;
        case OPC_RISC_AND:
            tcg_gen_and_tl(source1, source1, source2);
            break;
            CASE_OP_32_64(OPC_RISC_MUL)
                : tcg_gen_mul_tl(source1, source1, source2);
            SEXT_RESULT_IF_W(source1);
            break;
        case OPC_RISC_MULH:
            tcg_gen_muls2_tl(source2, source1, source1, source2);
            break;
        case OPC_RISC_MULHSU:
            gen_mulhsu(source1, source1, source2);
            break;
        case OPC_RISC_MULHU:
            tcg_gen_mulu2_tl(source2, source1, source1, source2);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_DIVW:
            tcg_gen_ext32s_tl(source1, source1);
            tcg_gen_ext32s_tl(source2, source2);
            /* fall through to DIV */
#endif
        /* fallthrough */
        case OPC_RISC_DIV:
            /* Handle by altering args to tcg_gen_div to produce req'd results:
             * For overflow: want source1 in source1 and 1 in source2
             * For div by zero: want -1 in source1 and 1 in source2 -> -1 result */
            cond1 = tcg_temp_new();
            cond2 = tcg_temp_new();
            zeroreg = tcg_const_tl(0);
            resultopt1 = tcg_temp_new();

            tcg_gen_movi_tl(resultopt1, (target_ulong)-1);
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond2, source2, (target_ulong)(~0L));
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond1, source1, ((target_ulong)1) << (TARGET_LONG_BITS - 1));
            tcg_gen_and_tl(cond1, cond1, cond2);                 /* cond1 = overflow */
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond2, source2, 0); /* cond2 = div 0 */
            /* if div by zero, set source1 to -1, otherwise don't change */
            tcg_gen_movcond_tl(TCG_COND_EQ, source1, cond2, zeroreg, source1, resultopt1);
            /* if overflow or div by zero, set source2 to 1, else don't change */
            tcg_gen_or_tl(cond1, cond1, cond2);
            tcg_gen_movi_tl(resultopt1, (target_ulong)1);
            tcg_gen_movcond_tl(TCG_COND_EQ, source2, cond1, zeroreg, source2, resultopt1);
            tcg_gen_div_tl(source1, source1, source2);
            SEXT_RESULT_IF_W(source1);

            tcg_temp_free(cond1);
            tcg_temp_free(cond2);
            tcg_temp_free(zeroreg);
            tcg_temp_free(resultopt1);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_DIVUW:
            tcg_gen_ext32u_tl(source1, source1);
            tcg_gen_ext32u_tl(source2, source2);
            /* fall through to DIVU */
#endif
        /* fallthrough */
        case OPC_RISC_DIVU:
            cond1 = tcg_temp_new();
            zeroreg = tcg_const_tl(0);
            resultopt1 = tcg_temp_new();

            tcg_gen_setcondi_tl(TCG_COND_EQ, cond1, source2, 0);
            tcg_gen_movi_tl(resultopt1, (target_ulong)-1);
            tcg_gen_movcond_tl(TCG_COND_EQ, source1, cond1, zeroreg, source1, resultopt1);
            tcg_gen_movi_tl(resultopt1, (target_ulong)1);
            tcg_gen_movcond_tl(TCG_COND_EQ, source2, cond1, zeroreg, source2, resultopt1);
            tcg_gen_divu_tl(source1, source1, source2);
            SEXT_RESULT_IF_W(source1);

            tcg_temp_free(cond1);
            tcg_temp_free(zeroreg);
            tcg_temp_free(resultopt1);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_REMW:
            tcg_gen_ext32s_tl(source1, source1);
            tcg_gen_ext32s_tl(source2, source2);
            /* fall through to REM */
#endif
        /* fallthrough */
        case OPC_RISC_REM:
            cond1 = tcg_temp_new();
            cond2 = tcg_temp_new();
            zeroreg = tcg_const_tl(0);
            resultopt1 = tcg_temp_new();

            tcg_gen_movi_tl(resultopt1, 1L);
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond2, source2, (target_ulong)-1);
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond1, source1, (target_ulong)1 << (TARGET_LONG_BITS - 1));
            tcg_gen_and_tl(cond2, cond1, cond2);                 /* cond1 = overflow */
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond1, source2, 0); /* cond2 = div 0 */
            /* if overflow or div by zero, set source2 to 1, else don't change */
            tcg_gen_or_tl(cond2, cond1, cond2);
            tcg_gen_movcond_tl(TCG_COND_EQ, source2, cond2, zeroreg, source2, resultopt1);
            tcg_gen_rem_tl(resultopt1, source1, source2);
            /* if div by zero, just return the original dividend */
            tcg_gen_movcond_tl(TCG_COND_EQ, source1, cond1, zeroreg, resultopt1, source1);
            SEXT_RESULT_IF_W(source1);

            tcg_temp_free(cond1);
            tcg_temp_free(cond2);
            tcg_temp_free(zeroreg);
            tcg_temp_free(resultopt1);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_REMUW:
            tcg_gen_ext32u_tl(source1, source1);
            tcg_gen_ext32u_tl(source2, source2);
            /* fall through to REMU */
#endif
        /* fallthrough */
        case OPC_RISC_REMU:
            cond1 = tcg_temp_new();
            zeroreg = tcg_const_tl(0);
            resultopt1 = tcg_temp_new();

            tcg_gen_movi_tl(resultopt1, (target_ulong)1);
            tcg_gen_setcondi_tl(TCG_COND_EQ, cond1, source2, 0);
            tcg_gen_movcond_tl(TCG_COND_EQ, source2, cond1, zeroreg, source2, resultopt1);
            tcg_gen_remu_tl(resultopt1, source1, source2);
            /* if div by zero, just return the original dividend */
            tcg_gen_movcond_tl(TCG_COND_EQ, source1, cond1, zeroreg, resultopt1, source1);
            SEXT_RESULT_IF_W(source1);

            tcg_temp_free(cond1);
            tcg_temp_free(zeroreg);
            tcg_temp_free(resultopt1);
            break;
        case OPC_RISC_ADD_UW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_andi_tl(source1, source1, 0xFFFFFFFF);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_SH1ADD:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_shli_tl(source1, source1, 1);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_SH1ADD_UW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_andi_tl(source1, source1, 0xFFFFFFFF);
            tcg_gen_shli_tl(source1, source1, 1);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_SH2ADD:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_shli_tl(source1, source1, 2);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_SH2ADD_UW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_andi_tl(source1, source1, 0xFFFFFFFF);
            tcg_gen_shli_tl(source1, source1, 2);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_SH3ADD:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_shli_tl(source1, source1, 3);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_SH3ADD_UW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_andi_tl(source1, source1, 0xFFFFFFFF);
            tcg_gen_shli_tl(source1, source1, 3);
            tcg_gen_add_tl(source1, source1, source2);
            break;
        case OPC_RISC_ANDN:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_not_tl(source2, source2);
            tcg_gen_and_tl(source1, source1, source2);
            break;
        case OPC_RISC_ORN:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_not_tl(source2, source2);
            tcg_gen_or_tl(source1, source1, source2);
            break;
        case OPC_RISC_XNOR:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_xor_tl(source1, source1, source2);
            tcg_gen_not_tl(source1, source1);
            break;
        case OPC_RISC_MAX:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
#if defined(TARGET_RISCV32)
            tcg_gen_smax_i32(source1, source1, source2);
#elif defined(TARGET_RISCV64)
            tcg_gen_smax_i64(source1, source1, source2);
#endif
            break;
        case OPC_RISC_MAXU:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
#if defined(TARGET_RISCV32)
            tcg_gen_umax_i32(source1, source1, source2);
#elif defined(TARGET_RISCV64)
            tcg_gen_umax_i64(source1, source1, source2);
#endif
            break;
        case OPC_RISC_MIN:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
#if defined(TARGET_RISCV32)
            tcg_gen_smin_i32(source1, source1, source2);
#elif defined(TARGET_RISCV64)
            tcg_gen_smin_i64(source1, source1, source2);
#endif
            break;
        case OPC_RISC_MINU:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
#if defined(TARGET_RISCV32)
            tcg_gen_umin_i32(source1, source1, source2);
#elif defined(TARGET_RISCV64)
            tcg_gen_umin_i64(source1, source1, source2);
#endif
            break;
        case OPC_RISC_CZERO_EQZ:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZICOND)) {
                return;
            }
            zeroreg = tcg_const_tl(0);
            /* rd = (rs2 == 0) ? 0 : rs1 */
            tcg_gen_movcond_tl(TCG_COND_EQ, source1, source2, zeroreg, zeroreg, source1);
            tcg_temp_free(zeroreg);
            break;
        case OPC_RISC_CZERO_NEZ:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZICOND)) {
                return;
            }
            zeroreg = tcg_const_tl(0);
            /* rd = (rs2 != 0) ? 0 : rs1 */
            tcg_gen_movcond_tl(TCG_COND_NE, source1, source2, zeroreg, zeroreg, source1);
            tcg_temp_free(zeroreg);
            break;
        case OPC_RISC_PACK:
#if defined(TARGET_RISCV32)
            //  On RV32 `pack rd, rs1, x0` is encoded the same way as `zext.h rd, rs1`
            //  so a core with only Zbb can only use the zext.h variant,
            //  while a core with Zbkb can use the full version
            if(!riscv_has_additional_ext(cpu, RISCV_FEATURE_ZBKB)) {
                if(rs2 == 0) {
                    if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                        return;
                    }
                } else {
                    if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                        return;
                    }
                }
            }
#elif defined(TARGET_RISCV64)
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
#endif
#if defined(TARGET_RISCV32)
            mask = 0xFFFF;
#elif defined(TARGET_RISCV64)
            mask = 0xFFFFFFFF;
#endif
            tcg_gen_andi_tl(source1, source1, mask);
            tcg_gen_andi_tl(source2, source2, mask);
            tcg_gen_shli_tl(source2, source2, TARGET_LONG_BITS / 2);
            tcg_gen_or_tl(source1, source1, source2);
            break;
        case OPC_RISC_PACKH:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            mask = 0xFF;
            tcg_gen_andi_tl(source1, source1, mask);
            tcg_gen_andi_tl(source2, source2, mask);
            tcg_gen_shli_tl(source2, source2, 8);
            tcg_gen_or_tl(source1, source1, source2);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_PACKW:
            //  On RV64 `packw rd, rs1, x0` is encoded the same way as `zext.h rd, rs1`
            //  so a core with only Zbb can only use the zext.h variant,
            //  while a core with Zbkb can use the full version
            if(!riscv_has_additional_ext(cpu, RISCV_FEATURE_ZBKB)) {
                if(rs2 == 0) {
                    if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                        return;
                    }
                } else {
                    if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                        return;
                    }
                }
            }
            mask = 0xFFFF;
            tcg_gen_andi_tl(source1, source1, mask);
            tcg_gen_andi_tl(source2, source2, mask);
            tcg_gen_shli_tl(source2, source2, 16);
            tcg_gen_or_tl(source1, source1, source2);
            break;
#endif
        case OPC_RISC_ROL:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            cond1 = tcg_temp_new();
            tcg_gen_andi_tl(cond1, source2, TARGET_LONG_BITS - 1);
            tcg_gen_rotl_tl(source1, source1, cond1);
            tcg_temp_free(cond1);
            break;
        case OPC_RISC_ROLW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            cond1 = tcg_temp_new_i64();
            tcg_gen_shli_tl(cond1, source1, 32);
            tcg_gen_shri_tl(source1, cond1, 32);
            tcg_gen_rotl_i64(cond1, cond1, source2);
            tcg_gen_rotl_i64(source1, source1, source2);
            tcg_gen_or_i64(source1, source1, cond1);
            tcg_gen_ext32s_tl(source1, source1);
            tcg_temp_free(cond1);
            break;

        case OPC_RISC_ROR:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_rotr_tl(source1, source1, source2);
            break;
        case OPC_RISC_RORW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            cond1 = tcg_temp_new_i64();
            tcg_gen_shli_tl(source1, source1, 32);
            tcg_gen_rotr_i64(cond1, source1, source2);
            tcg_gen_shr_i64(source1, source1, source2);
            tcg_gen_shri_i64(cond1, cond1, 32);
            tcg_gen_or_i64(source1, source1, cond1);
            tcg_gen_ext32s_tl(source1, source1);
            tcg_temp_free(cond1);
            break;

        case OPC_RISC_BCLR:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
#if defined(TARGET_RISCV32)
            TCGv bclr_t = tcg_temp_new_internal_i32(0);
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#elif defined(TARGET_RISCV64)
            TCGv bclr_t = tcg_temp_new_internal_i64(0);
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#endif
            tcg_gen_movi_tl(bclr_t, 1);
            tcg_gen_shl_tl(bclr_t, bclr_t, source2);
#if defined(TARGET_RISCV32)
            tcg_gen_not_i32(bclr_t, bclr_t);
#elif defined(TARGET_RISCV64)
            tcg_gen_not_i64(bclr_t, bclr_t);
#endif
            tcg_gen_and_tl(source1, source1, bclr_t);
            tcg_temp_free(bclr_t);
            break;
        case OPC_RISC_BEXT:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
#if defined(TARGET_RISCV32)
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#elif defined(TARGET_RISCV64)
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#endif
            tcg_gen_shr_tl(source1, source1, source2);
            tcg_gen_andi_tl(source1, source1, 1);
            break;
        case OPC_RISC_BINV:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
#if defined(TARGET_RISCV32)
            TCGv binv_t = tcg_temp_new_internal_i32(0);
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#elif defined(TARGET_RISCV64)
            TCGv binv_t = tcg_temp_new_internal_i64(0);
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#endif
            tcg_gen_movi_tl(binv_t, 1);
            tcg_gen_shl_tl(binv_t, binv_t, source2);
            tcg_gen_xor_tl(source1, source1, binv_t);

            tcg_temp_free(binv_t);
            break;
        case OPC_RISC_BSET:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
#if defined(TARGET_RISCV32)
            TCGv test = tcg_temp_new_internal_i32(0);
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#elif defined(TARGET_RISCV64)
            TCGv test = tcg_temp_new_internal_i64(0);
            tcg_gen_andi_tl(source2, source2, BITMANIP_SHAMT_MASK);
#endif
            tcg_gen_movi_tl(test, 1UL);
            tcg_gen_shl_tl(test, test, source2);
            tcg_gen_or_tl(source1, source1, test);

            tcg_temp_free(test);
            break;
        case OPC_RISC_CLMUL:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBC)) {
                return;
            }
            gen_clmulx(source1, source2, TARGET_LONG_BITS, 0, 0);
            break;
        case OPC_RISC_CLMULR:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBC)) {
                return;
            }
            gen_clmulx(source1, source2, TARGET_LONG_BITS, 1, 1);
            break;
        case OPC_RISC_CLMULH:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBC)) {
                return;
            }
            gen_clmulx(source1, source2, TARGET_LONG_BITS, 0, 1);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    gen_set_gpr(rd, source1);
    tcg_temp_free(source1);
    tcg_temp_free(source2);
}

static void gen_synch(DisasContext *dc, uint32_t opc)
{
    switch(opc) {
        case OPC_RISC_FENCE:
            /* standard fence = NOP */
            break;
        case OPC_RISC_FENCE_I:
            if(!riscv_has_additional_ext(cpu, RISCV_FEATURE_ZIFENCEI)) {
                int instruction_length = decode_instruction_length(dc->opcode);
                tlib_printf(LOG_LEVEL_ERROR,
                            "RISC-V Zifencei instruction set is not enabled for this CPU! In future release this configuration "
                            "will lead to an illegal instruction exception. PC: 0x%llx, opcode: 0x%0*llx",
                            dc->base.pc, /* padding */ 2 * instruction_length, format_opcode(dc->opcode, instruction_length));
            }
            gen_helper_fence_i(cpu_env);
            tcg_gen_movi_tl(cpu_pc, dc->npc);
            gen_exit_tb_no_chaining(dc->base.tb);
            dc->base.is_jmp = DISAS_BRANCH;
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void gen_arith_bitmanip(DisasContext *dc, int rd, int rs1, target_long imm, TCGv source1)
{
    TCGv_i64 t0, t1;
#if defined(TARGET_RISCV32)
    TCGv result, temp;
#endif
    uint32_t opc = 0;
    switch((dc->opcode >> 12) & 0x7) {
        case 0x1:
#if defined(TARGET_RISCV32)
            if(dc->opcode >> 20 == BITMANIP_ZIP_GROUP) {
                opc = MASK_OP_ARITH_IMM_ZB_1_12(dc->opcode);
                break;
            }
#endif
            switch((dc->opcode >> 26) & BITMANIP_SHAMT_MASK) {
                case 0b011000:
                    opc = MASK_OP_ARITH_IMM_ZB_1_12(dc->opcode);
                    break;
                case 0b010010:  //  bclri
                case 0b011010:  //  binvi
                case 0b001010:  //  bseti
                case 0b000010:  //  slli.uw
                    opc = MASK_OP_ARITH_IMM_ZB_1_12_SHAMT(dc->opcode);
                    break;
            }
            break;
        case 0x5:
            if(((dc->opcode) & OPC_RISC_RORIW) == 0) {
                opc = MASK_OP_ARITH_IMM_ZB_5_12_SHAMT_LAST_7(dc->opcode);
                break;
            }
#if defined(TARGET_RISCV32)
            if(dc->opcode >> 20 == BITMANIP_ZIP_GROUP) {
                opc = MASK_OP_ARITH_IMM_ZB_1_12(dc->opcode);
                break;
            }
#endif
            switch((dc->opcode >> 26) & BITMANIP_SHAMT_MASK) {
                case 0b001010:  //  orc.b
                case 0b011010:  //  rev8
                    opc = MASK_OP_ARITH_IMM_ZB_5_12(dc->opcode);
                    break;
                case 0b010010:  //  bexti
                case 0b011000:  //  rori
                    opc = MASK_OP_ARITH_IMM_ZB_5_12_SHAMT(dc->opcode);
                    break;
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    switch(opc) {
        case OPC_RISC_CLZW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_clzi_i32(source1, source1, 32);
            tcg_gen_ext32s_i64(source1, source1);
            break;
        case OPC_RISC_RORI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_rotri_tl(source1, source1, (imm & BITMANIP_SHAMT_MASK));
            break;
        case OPC_RISC_RORIW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            t0 = tcg_temp_new_i64();
            tcg_gen_rotri_i64(t0, source1, (imm & BITMANIP_SHAMT_MASK));
            tcg_gen_rotri_i64(source1, source1, 32 + (imm & BITMANIP_SHAMT_MASK));
            tcg_gen_or_i64(source1, source1, t0);
            tcg_gen_ext32s_i64(source1, source1);
            tcg_temp_free(t0);
            break;
        case OPC_RISC_SLLI_UW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBA)) {
                return;
            }
            tcg_gen_andi_i64(source1, source1, 0xFFFFFFFF);
            tcg_gen_shli_i64(source1, source1, (imm & BITMANIP_SHAMT_MASK));
            break;
        case OPC_RISC_REV8_32:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_bswap32_i32(source1, source1);
            break;
        case OPC_RISC_REV8_64:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB) && !ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            tcg_gen_bswap64_i64(source1, source1);
            break;

        case OPC_RISC_BREV8:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            t0 = tcg_temp_new_i64();
            t1 = tcg_temp_new_i64();

            /* Consider a 16bit value with bits indexed by a hex digit:
             * FEDCBA9876543210
             * After each step (and, shift, or) the indices change as follows:
             * EFCDAB8976452301 first pairs of bits are swapped
             * DCEF89AB45670123 then pairs of two bits are swapped
             * 89ABCDEF01234567 last nibbles are swapped
             */
            tcg_gen_andi_i64(t0, source1, 0xAAAAAAAAAAAAAAAA);
            tcg_gen_shri_i64(t0, t0, 1);
            tcg_gen_andi_i64(t1, source1, 0x5555555555555555);
            tcg_gen_shli_i64(t1, t1, 1);
            tcg_gen_or_i64(source1, t0, t1);

            tcg_gen_andi_i64(t0, source1, 0xCCCCCCCCCCCCCCCC);
            tcg_gen_shri_i64(t0, t0, 2);
            tcg_gen_andi_i64(t1, source1, 0x3333333333333333);
            tcg_gen_shli_i64(t1, t1, 2);
            tcg_gen_or_i64(source1, t0, t1);

            tcg_gen_andi_i64(t0, source1, 0xF0F0F0F0F0F0F0F0);
            tcg_gen_shri_i64(t0, t0, 4);
            tcg_gen_andi_i64(t1, source1, 0x0F0F0F0F0F0F0F0F);
            tcg_gen_shli_i64(t1, t1, 4);
            tcg_gen_or_i64(source1, t0, t1);

            tcg_temp_free_i64(t0);
            tcg_temp_free_i64(t1);
            break;
        case OPC_RISC_CTZ:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            gen_ctzx(source1, TARGET_LONG_BITS);
            break;
        case OPC_RISC_CPOP:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            gen_cpopx(source1, TARGET_LONG_BITS);
            break;
#if defined(TARGET_RISCV32)
        case OPC_RISC_CLZ:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_clzi_i32(source1, source1, 32);
            break;
        case OPC_RISC_SEXT_B:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_sextract_i32(source1, source1, 0, 8);
            break;
        case OPC_RISC_SEXT_H:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_sextract_i32(source1, source1, 0, 16);
            break;
        case OPC_RISC_ORC_B:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            gen_orcb(source1, 3);
            break;
        case OPC_RISC_BCLRI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_andi_tl(source1, source1, ~(1UL << (imm & BITMANIP_SHAMT_MASK)));
            break;
        case OPC_RISC_BEXTI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_shri_tl(source1, source1, imm & BITMANIP_SHAMT_MASK);
            tcg_gen_andi_tl(source1, source1, 1UL);
            break;
        case OPC_RISC_BINVI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_xori_tl(source1, source1, 1UL << (imm & BITMANIP_SHAMT_MASK));
            break;
        case OPC_RISC_BSETI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_ori_tl(source1, source1, 1UL << (imm & BITMANIP_SHAMT_MASK));
            break;
        case OPC_RISC_ZIP:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            result = tcg_temp_new();
            temp = tcg_temp_new();
            for(target_ulong i = 0; i < TARGET_LONG_BITS / 2; i++) {
                tcg_gen_extract_i32(temp, source1, i, 1);
                tcg_gen_deposit_i32(result, result, temp, 2 * i, 1);

                tcg_gen_extract_i32(temp, source1, i + (TARGET_LONG_BITS / 2), 1);
                tcg_gen_deposit_i32(result, result, temp, (2 * i) + 1, 1);
            }
            tcg_gen_mov_tl(source1, result);
            tcg_temp_free(temp);
            tcg_temp_free(result);
            break;
        case OPC_RISC_UNZIP:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBKB)) {
                return;
            }
            result = tcg_temp_new();
            temp = tcg_temp_new();
            for(target_ulong i = 0; i < TARGET_LONG_BITS / 2; i++) {
                tcg_gen_extract_i32(temp, source1, 2 * i, 1);
                tcg_gen_deposit_i32(result, result, temp, i, 1);

                tcg_gen_extract_i32(temp, source1, (2 * i) + 1, 1);
                tcg_gen_deposit_i32(result, result, temp, i + (TARGET_LONG_BITS / 2), 1);
            }
            tcg_gen_mov_tl(source1, result);
            tcg_temp_free(temp);
            tcg_temp_free(result);
            break;
#elif defined(TARGET_RISCV64)
        case OPC_RISC_CLZ:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_clzi_i64(source1, source1, 64);
            break;
        case OPC_RISC_CTZW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            gen_ctzx(source1, 32);
            break;
        case OPC_RISC_CPOPW:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            gen_cpopx(source1, 32);
            break;
        case OPC_RISC_SEXT_B:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_sextract_i64(source1, source1, 0, 8);
            break;
        case OPC_RISC_SEXT_H:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            tcg_gen_sextract_i64(source1, source1, 0, 16);
            break;
        case OPC_RISC_ORC_B:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBB)) {
                return;
            }
            gen_orcb(source1, 7);
            break;
        case OPC_RISC_BCLRI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_andi_tl(source1, source1, ~(1UL << (imm & BITMANIP_SHAMT_MASK)));
            break;
        case OPC_RISC_BEXTI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_shri_tl(source1, source1, imm & BITMANIP_SHAMT_MASK);
            tcg_gen_andi_tl(source1, source1, 1UL);
            break;
        case OPC_RISC_BINVI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_xori_tl(source1, source1, 1UL << (imm & BITMANIP_SHAMT_MASK));
            break;
        case OPC_RISC_BSETI:
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZBS)) {
                return;
            }
            tcg_gen_ori_tl(source1, source1, 1UL << (imm & BITMANIP_SHAMT_MASK));
            break;
#endif
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void gen_arith_imm(DisasContext *dc, uint32_t opc, int rd, int rs1, target_long imm)
{
    TCGv source1;
    source1 = tcg_temp_local_new();
    gen_get_gpr(source1, rs1);
    target_long extra_shamt = 0;

    switch(opc) {
        case OPC_RISC_ADDI:
#if defined(TARGET_RISCV64)
        case OPC_RISC_ADDIW:
#endif
            tcg_gen_addi_tl(source1, source1, imm);
            SEXT_RESULT_IF_W(source1);
            break;
        case OPC_RISC_SLTI:
            tcg_gen_setcondi_tl(TCG_COND_LT, source1, source1, imm);
            break;
        case OPC_RISC_SLTIU:
            tcg_gen_setcondi_tl(TCG_COND_LTU, source1, source1, imm);
            break;
        case OPC_RISC_XORI:
            tcg_gen_xori_tl(source1, source1, imm);
            break;
        case OPC_RISC_ORI:
            tcg_gen_ori_tl(source1, source1, imm);
            break;
        case OPC_RISC_ANDI:
            tcg_gen_andi_tl(source1, source1, imm);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_SLLIW:
            /* fall through to SLLI */
#endif
        /* fallthrough */
        case OPC_RISC_SLLI:
#if defined(TARGET_RISCV32)
            if(imm >> 5) {
#elif defined(TARGET_RISCV64)
            if(imm >> 6) {
#endif
                gen_arith_bitmanip(dc, rd, rs1, imm, source1);
            } else {
                tcg_gen_shli_tl(source1, source1, imm);
            }
            if(MASK_OP_ARITH_IMM_ZB_5_12_SHAMT(dc->opcode) != OPC_RISC_SLLI_UW) {
                SEXT_RESULT_IF_W(source1);
            }
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_SHIFT_RIGHT_IW:
            tcg_gen_shli_tl(source1, source1, 32);
            extra_shamt = 32;
            /* fall through to SHIFT_RIGHT_I */
#endif
        /* fallthrough */
        case OPC_RISC_SHIFT_RIGHT_I:
            /* differentiate on IMM */
#if defined(TARGET_RISCV32)
            if(imm >> 5) {
                if((imm >> 5) == 0x20) {
#elif defined(TARGET_RISCV64)
            if(imm >> 6) {
                if((imm >> 6) == 0x10) {
#endif
                    /* SRAI[W] */
                    tcg_gen_sari_tl(source1, source1, (imm ^ 0x400) + extra_shamt);
                    SEXT_RESULT_IF_W(source1);
                } else {
                    gen_arith_bitmanip(dc, rd, rs1, imm, source1);
                }
            } else {
                /* SRLI[W] */
                tcg_gen_shri_tl(source1, source1, imm + extra_shamt);
                SEXT_RESULT_IF_W(source1);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    gen_set_gpr(rd, source1);
    tcg_temp_free(source1);
}

static inline int is_jal_an_ret_pseudoinsn(int rd, int rs1, int imm)
{
    //  ret => jalr x0, 0(x1)
    return (rs1 == 1) && (rd == 0) && (imm == 0);
}

static inline int is_jal_RA_based(int rd)
{
    //  jalr x1, NN(XX)
    return (rd == 1);
}

static inline void announce_if_jump_or_ret(int rd, int rs1, target_long imm, target_ulong next_pc)
{
    int type = STACK_FRAME_NO_CHANGE;

    if(is_jal_an_ret_pseudoinsn(rd, rs1, imm)) {
        type = STACK_FRAME_POP;
    } else if(is_jal_RA_based(rd)) {
        type = STACK_FRAME_ADD;
    }

    if(next_pc == PROFILER_TCG_PC) {
        generate_stack_announcement(cpu_pc, type, false);
    } else {
        generate_stack_announcement_imm_i64(next_pc, type, false);
    }
}

static void gen_jal(CPUState *env, DisasContext *dc, int rd, target_ulong imm)
{
    target_ulong next_pc;

    /* check misaligned: */
    next_pc = dc->base.pc + imm;

    if(!riscv_has_ext(env, RISCV_FEATURE_RVC)) {
        if((next_pc & 0x3) != 0) {
            generate_exception_mbadaddr(dc, RISCV_EXCP_INST_ADDR_MIS);
        }
    }

    get_set_gpr_imm(rd, dc->npc);

    if(unlikely(dc->base.guest_profile)) {
        announce_if_jump_or_ret(rd, RA, imm, next_pc);
    }

    gen_goto_tb(dc, 0, dc->base.pc + imm); /* must use this for safety */
    dc->base.is_jmp = DISAS_BRANCH;
}

static void gen_jalr(CPUState *env, DisasContext *dc, uint32_t opc, int rd, int rs1, target_long imm)
{
    /* no chaining with JALR */
    int misaligned = gen_new_label();
    TCGv t0;
    t0 = tcg_temp_new();

    switch(opc) {
        case OPC_RISC_JALR:
            gen_get_gpr(cpu_pc, rs1);
            tcg_gen_addi_tl(cpu_pc, cpu_pc, imm);
            tcg_gen_andi_tl(cpu_pc, cpu_pc, ~(target_ulong)1);

            if(!riscv_has_ext(env, RISCV_FEATURE_RVC)) {
                tcg_gen_andi_tl(t0, cpu_pc, 0x2);
                tcg_gen_brcondi_tl(TCG_COND_NE, t0, 0x0, misaligned);
            }

            get_set_gpr_imm(rd, dc->npc);
            if(unlikely(dc->base.guest_profile)) {
                announce_if_jump_or_ret(rd, rs1, imm, PROFILER_TCG_PC);
            }

            gen_exit_tb_no_chaining(dc->base.tb);

            gen_set_label(misaligned);
            generate_exception_mbadaddr(dc, RISCV_EXCP_INST_ADDR_MIS);
            gen_exit_tb_no_chaining(dc->base.tb);
            dc->base.is_jmp = DISAS_BRANCH;
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free(t0);
}

static void gen_branch(CPUState *env, DisasContext *dc, uint32_t opc, int rs1, int rs2, target_long bimm)
{
    int l = gen_new_label();
    TCGv source1, source2;
    source1 = tcg_temp_new();
    source2 = tcg_temp_new();
    gen_get_gpr(source1, rs1);
    gen_get_gpr(source2, rs2);

    switch(opc) {
        case OPC_RISC_BEQ:
            tcg_gen_brcond_tl(TCG_COND_EQ, source1, source2, l);
            break;
        case OPC_RISC_BNE:
            tcg_gen_brcond_tl(TCG_COND_NE, source1, source2, l);
            break;
        case OPC_RISC_BLT:
            tcg_gen_brcond_tl(TCG_COND_LT, source1, source2, l);
            break;
        case OPC_RISC_BGE:
            tcg_gen_brcond_tl(TCG_COND_GE, source1, source2, l);
            break;
        case OPC_RISC_BLTU:
            tcg_gen_brcond_tl(TCG_COND_LTU, source1, source2, l);
            break;
        case OPC_RISC_BGEU:
            tcg_gen_brcond_tl(TCG_COND_GEU, source1, source2, l);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    gen_goto_tb(dc, 1, dc->npc);
    gen_set_label(l); /* branch taken */
    if(!riscv_has_ext(env, RISCV_FEATURE_RVC) && ((dc->base.pc + bimm) & 0x3)) {
        /* misaligned */
        generate_exception_mbadaddr(dc, RISCV_EXCP_INST_ADDR_MIS);
        gen_exit_tb_no_chaining(dc->base.tb);
    } else {
        gen_goto_tb(dc, 0, dc->base.pc + bimm);
    }
    tcg_temp_free(source1);
    tcg_temp_free(source2);
    dc->base.is_jmp = DISAS_BRANCH;
}

static void gen_load(DisasContext *dc, uint32_t opc, int rd, int rs1, target_long imm)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();

    gen_get_gpr(t0, rs1);

    tcg_gen_addi_tl(t0, t0, imm);

    if(rs1 == SP) {
        try_run_pre_stack_access_hook(t0, opc_to_width(opc), false);
    }

    gen_sync_pc(dc);
    switch(opc) {
        case OPC_RISC_LB:
            tcg_gen_qemu_ld8s(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        case OPC_RISC_LH:
            tcg_gen_qemu_ld16s(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        case OPC_RISC_LW:
            tcg_gen_qemu_ld32s(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        case OPC_RISC_LD:
            tcg_gen_qemu_ld64(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        case OPC_RISC_LBU:
            tcg_gen_qemu_ld8u(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        case OPC_RISC_LHU:
            tcg_gen_qemu_ld16u(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        case OPC_RISC_LWU:
            tcg_gen_qemu_ld32u(t1, t0, dc->base.mem_idx);
            gen_set_gpr(rd, t1);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    tcg_temp_free(t0);
    tcg_temp_free(t1);
}

static void gen_store(DisasContext *dc, uint32_t opc, int rs1, int rs2, target_long imm)
{
    gen_sync_pc(dc);

    TCGv t0 = tcg_temp_local_new();
    TCGv dat = tcg_temp_local_new();
    gen_get_gpr(t0, rs1);
    tcg_gen_addi_tl(t0, t0, imm);
    gen_get_gpr(dat, rs2);

    if(rs1 == SP) {
        try_run_pre_stack_access_hook(t0, opc_to_width(opc), true);
    }

    switch(opc) {
        case OPC_RISC_SB:
            tcg_gen_qemu_st8(dat, t0, dc->base.mem_idx);
            break;
        case OPC_RISC_SH:
            tcg_gen_qemu_st16(dat, t0, dc->base.mem_idx);
            break;
        case OPC_RISC_SW:
            tcg_gen_qemu_st32(dat, t0, dc->base.mem_idx);
            break;
        case OPC_RISC_SD:
            tcg_gen_qemu_st64(dat, t0, dc->base.mem_idx);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    tcg_temp_free(t0);
    tcg_temp_free(dat);
}

static void gen_fp_load(DisasContext *dc, uint32_t opc, int rd, int rs1, target_long imm)
{
    if(!ensure_fp_extension_for_load_store(dc, opc)) {
        return;
    }

    TCGv t0 = tcg_temp_local_new();
    int fp_ok = gen_new_label();
    int done = gen_new_label();

    //  check MSTATUS.FS
    tcg_gen_ld_tl(t0, cpu_env, offsetof(CPUState, mstatus));
    tcg_gen_andi_tl(t0, t0, MSTATUS_FS);
    tcg_gen_brcondi_tl(TCG_COND_NE, t0, 0x0, fp_ok);
    //  MSTATUS_FS field was zero:
    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
    tcg_gen_br(done);

    //  proceed with operation
    gen_set_label(fp_ok);
    gen_get_gpr(t0, rs1);
    tcg_gen_addi_tl(t0, t0, imm);

    if(rs1 == SP) {
        try_run_pre_stack_access_hook(t0, opc_to_width(opc), false);
    }

    TCGv destination = tcg_temp_new();
    switch(opc) {
        case OPC_RISC_FLH:
            tcg_gen_qemu_ld16u(destination, t0, dc->base.mem_idx);
            tcg_gen_extu_tl_i64(cpu_fpr[rd], destination);
            gen_box_float(RISCV_HALF_PRECISION, cpu_fpr[rd]);
            break;
        case OPC_RISC_FLW:
            tcg_gen_qemu_ld32u(destination, t0, dc->base.mem_idx);
            tcg_gen_extu_tl_i64(cpu_fpr[rd], destination);
            gen_box_float(RISCV_SINGLE_PRECISION, cpu_fpr[rd]);
            break;
        case OPC_RISC_FLD:
            tcg_gen_qemu_ld64(cpu_fpr[rd], t0, dc->base.mem_idx);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    //  mark MSTATUS.FS as dirty
    tcg_gen_ld_tl(t0, cpu_env, offsetof(CPUState, mstatus));
    tcg_gen_ori_tl(t0, t0, 3 << 13);
    tcg_gen_st_tl(t0, cpu_env, offsetof(CPUState, mstatus));

    tcg_temp_free(destination);
    gen_set_label(done);
    tcg_temp_free(t0);
}

static void gen_v_load(DisasContext *dc, uint32_t opc, uint32_t rest, uint32_t vd, uint32_t rs1, uint32_t rs2, uint32_t width)
{
//  Vector helpers require 128-bit ints which aren't supported on 32-bit hosts.
#if HOST_LONG_BITS == 32
    tlib_abort("Vector extension isn't available on 32-bit hosts.");
#else
    uint32_t vm = extract32(rest, 0, 1);
    uint32_t mew = extract32(rest, 3, 1);  //  1 is a currently reserved encoding
    uint32_t nf = extract32(rest, 4, 3);

    if(!ensure_vector_embedded_extension_for_vsew_or_kill_unknown(dc, width)) {
        return;
    }
    if(mew) {
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
        return;
    }

    if(MASK_OP_V_LOAD_US(dc->opcode) != OPC_RISC_VL_US_WR) {
        generate_vill_check(dc);
    }
    TCGv_i32 t_vd, t_rs1, t_rs2, t_nf;
    t_vd = tcg_temp_new_i32();
    t_rs1 = tcg_temp_new_i32();
    t_rs2 = tcg_temp_new_i32();
    t_nf = tcg_temp_new_i32();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_rs1, rs1);
    tcg_gen_movi_i32(t_rs2, rs2);
    tcg_gen_movi_i32(t_nf, nf);

    switch(opc) {
        case OPC_RISC_VL_US:  //  unit-stride
            switch(MASK_OP_V_LOAD_US(dc->opcode)) {
                case OPC_RISC_VL_US:
                    switch(width & 0x3) {
                        case 0:
                            if(vm) {
                                gen_helper_vle8(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle8_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 1:
                            if(vm) {
                                gen_helper_vle16(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle16_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 2:
                            if(vm) {
                                gen_helper_vle32(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle32_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 3:
                            if(vm) {
                                gen_helper_vle64(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle64_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                    }
                    break;
                case OPC_RISC_VL_US_WR:
                    if(!vm || !is_nfields_power_of_two(nf)) {
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                    }
                    gen_helper_vl_wr(cpu_env, t_vd, t_rs1, t_nf);
                    break;
                case OPC_RISC_VL_US_MASK:
                    if(!vm || width || nf) {
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                    }
                    gen_helper_vlm(cpu_env, t_vd, t_rs1);
                    break;
                case OPC_RISC_VL_US_FOF:
                    switch(width & 0x3) {
                        case 0:
                            if(vm) {
                                gen_helper_vle8ff(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle8ff_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 1:
                            if(vm) {
                                gen_helper_vle16ff(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle16ff_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 2:
                            if(vm) {
                                gen_helper_vle32ff(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle32ff_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 3:
                            if(vm) {
                                gen_helper_vle64ff(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vle64ff_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                    }
                    break;
            }
            break;
        case OPC_RISC_VL_VS:  //  vector-strided
            switch(width & 0x3) {
                case 0:
                    if(vm) {
                        gen_helper_vlse8(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlse8_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 1:
                    if(vm) {
                        gen_helper_vlse16(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlse16_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 2:
                    if(vm) {
                        gen_helper_vlse32(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlse32_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 3:
                    if(vm) {
                        gen_helper_vlse64(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlse64_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
            }
            break;
        case OPC_RISC_VL_UVI:  //  unordered vector-indexed
        case OPC_RISC_VL_OVI:  //  ordered vector-indexed
            switch(width & 0x3) {
                case 0:
                    if(vm) {
                        gen_helper_vlxei8(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlxei8_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 1:
                    if(vm) {
                        gen_helper_vlxei16(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlxei16_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 2:
                    if(vm) {
                        gen_helper_vlxei32(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlxei32_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 3:
#if defined(TARGET_RISCV32)
                    //  Indexed instructions for EEW=64 and XLEN=32 aren't supported.
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
#else
                    if(vm) {
                        gen_helper_vlxei64(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vlxei64_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
#endif
                    break;
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_gen_movi_tl(cpu_vstart, 0);
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_rs1);
    tcg_temp_free_i32(t_rs2);
    tcg_temp_free_i32(t_nf);
#endif  //  HOST_LONG_BITS != 32
}

static void gen_fp_store(DisasContext *dc, uint32_t opc, int rs1, int rs2, target_long imm)
{
    if(!ensure_fp_extension_for_load_store(dc, opc)) {
        return;
    }

    TCGv t0 = tcg_temp_local_new();
    TCGv t1 = tcg_temp_local_new();
    int fp_ok = gen_new_label();
    int done = gen_new_label();

    //  check MSTATUS.FS
    tcg_gen_ld_tl(t0, cpu_env, offsetof(CPUState, mstatus));
    tcg_gen_andi_tl(t0, t0, MSTATUS_FS);
    tcg_gen_brcondi_tl(TCG_COND_NE, t0, 0x0, fp_ok);
    //  MSTATUS_FS field was zero:
    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
    tcg_gen_br(done);

    //  proceed with operation
    gen_set_label(fp_ok);
    gen_get_gpr(t0, rs1);
    tcg_gen_addi_tl(t0, t0, imm);

    try_run_pre_stack_access_hook(t0, opc_to_width(opc), true);

    switch(opc) {
        case OPC_RISC_FSH:
            tcg_gen_qemu_st16(cpu_fpr[rs2], t0, dc->base.mem_idx);
            break;
        case OPC_RISC_FSW:
            tcg_gen_qemu_st32(cpu_fpr[rs2], t0, dc->base.mem_idx);
            break;
        case OPC_RISC_FSD:
            tcg_gen_qemu_st64(cpu_fpr[rs2], t0, dc->base.mem_idx);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    gen_set_label(done);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
}

static void gen_v_store(DisasContext *dc, uint32_t opc, uint32_t rest, uint32_t vd, uint32_t rs1, uint32_t rs2, uint32_t width)
{
//  Vector helpers require 128-bit ints which aren't supported on 32-bit hosts.
#if HOST_LONG_BITS == 32
    tlib_abort("Vector extension isn't available on 32-bit hosts.");
#else
    uint32_t vm = extract32(rest, 0, 1);
    uint32_t mew = extract32(rest, 3, 1);
    uint32_t nf = extract32(rest, 4, 3);

    if(!ensure_vector_embedded_extension_for_vsew_or_kill_unknown(dc, width)) {
        return;
    }
    if(mew) {
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
        return;
    }

    if(MASK_OP_V_STORE_US(dc->opcode) != OPC_RISC_VS_US_WR) {
        generate_vill_check(dc);
    }
    TCGv_i32 t_vd, t_rs1, t_rs2, t_nf;
    t_vd = tcg_temp_new_i32();
    t_rs1 = tcg_temp_new_i32();
    t_rs2 = tcg_temp_new_i32();
    t_nf = tcg_temp_new_i32();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_rs1, rs1);
    tcg_gen_movi_i32(t_rs2, rs2);
    tcg_gen_movi_i32(t_nf, nf);

    switch(opc) {
        case OPC_RISC_VS_US:  //  unit-stride
            switch(MASK_OP_V_STORE_US(dc->opcode)) {
                case OPC_RISC_VS_US:
                    switch(width & 0x3) {
                        case 0:
                            if(vm) {
                                gen_helper_vse8(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vse8_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 1:
                            if(vm) {
                                gen_helper_vse16(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vse16_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 2:
                            if(vm) {
                                gen_helper_vse32(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vse32_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                        case 3:
                            if(vm) {
                                gen_helper_vse64(cpu_env, t_vd, t_rs1, t_nf);
                            } else {
                                gen_helper_vse64_m(cpu_env, t_vd, t_rs1, t_nf);
                            }
                            break;
                    }
                    break;
                case OPC_RISC_VS_US_WR:
                    if(!vm || width || !is_nfields_power_of_two(nf)) {
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                    }
                    gen_helper_vs_wr(cpu_env, t_vd, t_rs1, t_nf);
                    break;
                case OPC_RISC_VS_US_MASK:
                    if(!vm || width || nf) {
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                    }
                    gen_helper_vsm(cpu_env, t_vd, t_rs1);
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case OPC_RISC_VS_VS:  //  vector-strided
            switch(width & 0x3) {
                case 0:
                    if(vm) {
                        gen_helper_vsse8(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsse8_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 1:
                    if(vm) {
                        gen_helper_vsse16(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsse16_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 2:
                    if(vm) {
                        gen_helper_vsse32(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsse32_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 3:
                    if(vm) {
                        gen_helper_vsse64(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsse64_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
            }
            break;
        case OPC_RISC_VS_UVI:  //  unordered vector-indexed
        case OPC_RISC_VS_OVI:  //  ordered vector-indexed
            switch(width & 0x3) {
                case 0:
                    if(vm) {
                        gen_helper_vsxei8(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsxei8_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 1:
                    if(vm) {
                        gen_helper_vsxei16(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsxei16_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 2:
                    if(vm) {
                        gen_helper_vsxei32(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsxei32_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
                    break;
                case 3:
#if defined(TARGET_RISCV32)
                    //  Indexed instructions for EEW=64 and XLEN=32 aren't supported.
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
#else
                    if(vm) {
                        gen_helper_vsxei64(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    } else {
                        gen_helper_vsxei64_m(cpu_env, t_vd, t_rs1, t_rs2, t_nf);
                    }
#endif
                    break;
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_gen_movi_tl(cpu_vstart, 0);
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_rs1);
    tcg_temp_free_i32(t_rs2);
    tcg_temp_free_i32(t_nf);
#endif  //  HOST_LONG_BITS != 32
}

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static void gen_sc(CPUState *env, TCGv result, TCGv value, TCGv guest_address, TCGv reserved_result, int mem_index, int size)
{
    tlib_assert(size == 32 || size == 64);

    int done = gen_new_label();
    int fail = gen_new_label();

    /*  An SC may fail for two reasons: either there is no matching LR on
     *  the same address to pair with, or the reservation has been invalidated.
     *
     *                "can the SC pair with a previous LR?"
     *                ┌───────────────────────────────────┐
     *                | reserved_address == guest_address ┼──────────────┐
     *                └──────────────────┬────────────────┘ false        │
     *                                   │ true                          │
     *                         ┌─────────▼────────────┐                  │
     * "is reservation valid?" │ reserved_result == 1 ┼──────────────────┤
     *                         └─────────┬────────────┘ false            │
     *                                   │ true                          │
     *                           ┌───────▼────────┐                      │
     *       "perform the store" │ qemu_st(32|64) │                      │
     *                           └───────┬────────┘                      │
     *                                   │                               │
     *                            ┌──────┘       ┌───────────────────────┘
     *                            │              │
     *                     ┌──────▼─────┐ ┌──────▼─────┐
     *     "Zero indicates │ result = 0 │ │ result = 1 │ "Non-zero result
     *      success"       └──────┬─────┘ └──────┬─────┘  indicates SC failure"
     *                            │              │
     *                      ┌─────▼──────────────▼────┐
     *                      │ reserved_address = null │ "Always invalidate reservation"
     *                      └─────────────────────────┘
     */

    int address_reserved = gen_new_label();
    tcg_gen_brcond_tl(TCG_COND_EQ, reserved_address, guest_address, address_reserved);

    //  Didn't branch above, then the address wasn't reserved, so we fail...
    tcg_gen_br(fail);

    gen_set_label(address_reserved);

    //  If address is not reserved (i.e. reserved_result = 0), jump to fail block.
    tcg_gen_brcondi_tl(TCG_COND_EQ, reserved_result, 0, fail);

    /* Didn't branch to fail block, this means SC should perform the store... */
    if(size == 64) {
        tcg_gen_qemu_st64_unsafe(value, guest_address, mem_index);
    } else {
        tcg_gen_qemu_st32_unsafe(value, guest_address, mem_index);
    }
    gen_store_table_set(env, guest_address);

    //  Zero value in rd indicates SC success.
    tcg_gen_movi_tl(result, 0);
    //  Skip over the fail block below.
    tcg_gen_br(done);

    gen_set_label(fail);
    //  Non-zero value in rd indicates SC failure.
    tcg_gen_movi_tl(result, 1);

    gen_set_label(done);

    //  Regardless of success or failure, executing an SC instruction invalidates any reservation held by this hart.
    tcg_gen_movi_tl(reserved_address, 0);
}

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static inline void gen_atomic_non_intrinsic(DisasContext *dc, uint32_t opc, TCGv dat, TCGv source1, TCGv source2)
{
    int done;
    done = gen_new_label();

    switch(opc) {
        case OPC_RISC_AMOSWAP_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOXOR_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_xor_tl(source2, dat, source2);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOAND_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_and_tl(source2, dat, source2);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOOR_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_or_tl(source2, dat, source2);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMIN_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_i32(TCG_COND_LT, dat, source2, done);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMAX_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_i32(TCG_COND_GT, dat, source2, done);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMINU_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_i32(TCG_COND_LTU, dat, source2, done);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMAXU_W:
            tcg_gen_qemu_ld32s(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_i32(TCG_COND_GTU, dat, source2, done);
            tcg_gen_qemu_st32_unsafe(source2, source1, dc->base.mem_idx);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_AMOSWAP_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOXOR_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_xor_tl(source2, dat, source2);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOAND_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_and_tl(source2, dat, source2);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOOR_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_or_tl(source2, dat, source2);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMIN_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_tl(TCG_COND_LT, dat, source2, done);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMAX_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_tl(TCG_COND_GT, dat, source2, done);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMINU_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_tl(TCG_COND_LTU, dat, source2, done);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_AMOMAXU_D:
            tcg_gen_qemu_ld64(dat, source1, dc->base.mem_idx);
            tcg_gen_brcond_tl(TCG_COND_GTU, dat, source2, done);
            tcg_gen_qemu_st64_unsafe(source2, source1, dc->base.mem_idx);
            break;
#endif
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    gen_set_label(done);
}

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static inline void gen_amoadd(TCGv result, TCGv guestAddress, TCGv toAdd, uint32_t memIndex, uint8_t size)
{
    tlib_assert(size == 64 || size == 32);

    int fallback = gen_new_label();
    //  Use host intrinsic if possible.
    if(size == 64) {
        tcg_try_gen_atomic_fetch_add_intrinsic_i64(result, guestAddress, toAdd, memIndex, fallback);
    } else {
        tcg_try_gen_atomic_fetch_add_intrinsic_i32(result, guestAddress, toAdd, memIndex, fallback);
    }

    int done = gen_new_label();
    tcg_gen_br(done);

    /*
     * If it's not possible to utilize host intrinsics, fall back to slower version:
     */
    gen_set_label(fallback);

    if(size == 64) {
        tcg_gen_qemu_ld64(result, guestAddress, memIndex);
        tcg_gen_add_i64(toAdd, result, toAdd);
        tcg_gen_qemu_st64_unsafe(toAdd, guestAddress, memIndex);
    } else {
        tcg_gen_qemu_ld32s(result, guestAddress, memIndex);
        tcg_gen_add_i32(toAdd, result, toAdd);
        tcg_gen_qemu_st32_unsafe(toAdd, guestAddress, memIndex);
    }

    gen_set_label(done);
}

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static void gen_atomic_fetch_and_op(DisasContext *dc, uint32_t opc, TCGv result, TCGv source1, TCGv source2)
{
    if(!ensure_extension(dc, RISCV_FEATURE_RVA)) {
        return;
    }
    bool is_sc = (opc == OPC_RISC_SC_W) | (opc == OPC_RISC_SC_D);
    TCGv reserved_result = tcg_temp_local_new();

    if(is_sc) {
        gen_store_table_check(env, reserved_result, source1);
    }

    switch(opc) {
        case OPC_RISC_LR_W:
            tcg_gen_mov_tl(reserved_address, source1);
            tcg_gen_qemu_ld32s(result, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_SC_W:
            gen_sc(env, result, source2, source1, reserved_result, dc->base.mem_idx, 32);
            break;
        /*
         * AMO instructions:
         * rd = *rs1
         * *rs1 = rd OP rs2
         */
        case OPC_RISC_AMOADD_W:
            gen_amoadd(result, source1, source2, dc->base.mem_idx, 32);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_LR_D:
            tcg_gen_mov_tl(reserved_address, source1);
            tcg_gen_qemu_ld64(result, source1, dc->base.mem_idx);
            break;
        case OPC_RISC_SC_D:
            gen_sc(env, result, source2, source1, reserved_result, dc->base.mem_idx, 64);
            break;
        /*
         * AMO instructions:
         * rd = *rs1
         * *rs1 = rd OP rs2
         */
        case OPC_RISC_AMOADD_D:
            gen_amoadd(result, source1, source2, dc->base.mem_idx, 64);
            break;
#endif
        default:
            gen_atomic_non_intrinsic(dc, opc, result, source1, source2);
            break;
    }

    if(!is_sc) {
        gen_store_table_set(env, source1);
    }
    tcg_temp_free(reserved_result);
}

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static inline void gen_amocas(TCGv_i64 expectedValue, TCGv guestAddress, TCGv newValue, uint32_t memIndex, uint8_t size)
{
    tlib_assert(size == 64 || size == 32);

    int fallback = gen_new_label();
    //  Needs to be 64-bit even for 32-bit guests, as they may do amocas.d
    TCGv_i64 result = tcg_temp_local_new_i64();

    //  Use host intrinsic if possible.
    if(size == 64) {
        tcg_try_gen_atomic_compare_and_swap_intrinsic_i64(result, expectedValue, guestAddress, newValue, memIndex, fallback);
    } else {
        tcg_try_gen_atomic_compare_and_swap_intrinsic_i32(result, expectedValue, guestAddress, newValue, memIndex, fallback);
    }

    int done = gen_new_label();
    tcg_gen_br(done);

    /*
     * If it's not possible to utilize host intrinsics, fall back to slower version:
     */
    gen_set_label(fallback);

    if(size == 64) {
        tcg_gen_atomic_cmpxchg_i64_unsafe(result, guestAddress, expectedValue, newValue, memIndex, MO_64);
    } else {
        tcg_gen_atomic_cmpxchg_i32_unsafe(result, guestAddress, expectedValue, newValue, memIndex, MO_32);
    }

    gen_set_label(done);

    tcg_gen_mov_i64(expectedValue, result);
    tcg_temp_free_i64(result);
}

static inline void amocas_ensure_even_register(DisasContext *dc, int registerId)
{
    //  Encodings with odd numbered registers specified in rs2 and rd are reserved.
    if(registerId % 2 != 0) {
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
    }
}

#ifdef TARGET_RISCV64
/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static inline void gen_amocas_128(TCGv_i64 expectedValueLow, TCGv_i64 guestAddress, TCGv_i64 newValueLow, uint32_t memIndex,
                                  int newValueHighRegister, int destinationHighRegister)
{
    int fallback = gen_new_label();
    TCGv_i128 result = tcg_temp_local_new_i128();

    //  The lower parts of expectedValue and newValue are already loaded, but we still need to load in the high parts.
    TCGv_i128 expectedValue = { .high = tcg_temp_local_new_i64(), .low = expectedValueLow };
    TCGv_i128 newValue = { .high = tcg_temp_local_new_i64(), .low = newValueLow };
    gen_get_gpr(expectedValue.high, destinationHighRegister);
    gen_get_gpr(newValue.high, newValueHighRegister);

    //  Use host intrinsic if possible.
    tcg_try_gen_atomic_compare_and_swap_intrinsic_i128(result, expectedValue, guestAddress, newValue, memIndex, fallback);

    int done = gen_new_label();
    tcg_gen_br(done);

    /*
     * If it's not possible to utilize host intrinsics, fall back to slower version:
     */
    gen_set_label(fallback);

    tcg_gen_atomic_cmpxchg_i128_unsafe(result, guestAddress, expectedValue, newValue, memIndex);

    gen_set_label(done);

    //  the rd register gets set from `expectedValueLow` (in `gen_atomic`), so all we need to do here is move `resultLow` into it
    //  and set rd+1 to `resultHigh`.
    tcg_gen_mov_i64(expectedValue.low, result.low);
    //  the rd+1 register is not already being set, so do it here.
    gen_set_gpr(destinationHighRegister, result.high);

    tcg_temp_free_i128(result);
    //  The 'low' parts are freed up in `gen_atomic` where they were allocated.
    tcg_temp_free_i64(expectedValue.high);
    tcg_temp_free_i64(newValue.high);
}
#endif

#if defined(TARGET_RISCV32) && HOST_LONG_BITS == 64
static inline void consolidate_32_registers_to_64(TCGv_i64 result, TCGv_i32 upper, TCGv_i32 lower)
{
    //  result = upper << 32
    tcg_gen_mov_i32(result, upper);
    tcg_gen_shli_i64(result, result, 32);
    //  result |= lower
    tcg_gen_or_i64(result, result, lower);
}

static inline void extract_and_consolidate_32_registers_to_64(TCGv_i64 result, TCGv_i32 lowerValue, int upperRegisterId)
{
    TCGv_i32 upperValue = tcg_temp_new();
    gen_get_gpr(upperValue, upperRegisterId);
    consolidate_32_registers_to_64(result, upperValue, lowerValue);
    tcg_temp_free_i32(upperValue);
}

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static inline void gen_amocas_d_on_rv32(TCGv_i32 lowerExpectedValue, TCGv_i32 guestAddress, TCGv_i32 lowerNewValue,
                                        uint32_t memIndex, int newValueHighRegister, int destinationHighRegister)
{
    /*
     * 32-bit guests split the 64-bit expected value in rd+1:rd, so
     * before we can use the host's 64-bit operation we need to consolidate them.
     */
    TCGv_i64 expectedValue64 = tcg_temp_local_new_i64();
    extract_and_consolidate_32_registers_to_64(expectedValue64, lowerExpectedValue, destinationHighRegister);

    /*
     * 32-bit guests split the 64-bit new value in rs2+1:rs2, so
     * before we can use the host's 64-bit operation we need to consolidate them.
     */
    TCGv_i64 newValue64 = tcg_temp_local_new_i64();
    extract_and_consolidate_32_registers_to_64(newValue64, lowerNewValue, newValueHighRegister);

    //  Use 64-bit host intrinsic.
    gen_amocas(expectedValue64, guestAddress, newValue64, memIndex, 64);

    /*
     * 32-bit guests expect the actual value to be placed in rd+1:rd, so
     * before finishing we must split up the 64-bit result we've gotten.
     */
    //  the rd register gets set from `lowerExpectedValue` (in `gen_atomic`),
    //  so all we need to do here is set rd+1 to the upper 32 bits of `expectedValue64`.
    tcg_gen_mov_i32(lowerExpectedValue, expectedValue64);
    //  chop off the lower 32 bits. BAM
    tcg_gen_shri_i64(expectedValue64, expectedValue64, 32);
    //  the rd+1 register is not already being set, so do it here.
    gen_set_gpr(destinationHighRegister, expectedValue64);

    tcg_temp_free_i64(expectedValue64);
    tcg_temp_free_i64(newValue64);
}
#endif

/*
 * Code generated by this function assumes that the store table entry lock for the associated address is held.
 */
static void gen_atomic_compare_and_swap(DisasContext *dc, uint32_t opc, TCGv result, TCGv source1, TCGv source2,
                                        int newValueHighRegister, int destinationHighRegister)
{
    if(!ensure_additional_extension(dc, RISCV_FEATURE_ZACAS)) {
        return;
    }

    switch(opc) {
        case OPC_RISC_AMOCAS_W:
        case OPC_RISC_AMOCAS_D:
#if defined(TARGET_RISCV64)
        case OPC_RISC_AMOCAS_Q:
#endif
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            return;
    }

    switch(opc) {
        case OPC_RISC_AMOCAS_W:
            gen_amocas(result, source1, source2, dc->base.mem_idx, 32);
            break;
        case OPC_RISC_AMOCAS_D:  //  64-bit amocas.d is available on RV32
#if defined(TARGET_RISCV64) && HOST_LONG_BITS == 64
            gen_amocas(result, source1, source2, dc->base.mem_idx, 64);
#elif defined(TARGET_RISCV32) && HOST_LONG_BITS == 64
            amocas_ensure_even_register(dc, newValueHighRegister == 0 ? 0 : newValueHighRegister - 1);
            amocas_ensure_even_register(dc, destinationHighRegister == 0 ? 0 : destinationHighRegister - 1);
            gen_amocas_d_on_rv32(result, source1, source2, dc->base.mem_idx, newValueHighRegister, destinationHighRegister);
#else
#error 64-bit amocas.d is not implemented for 32-bit hosts.
#endif
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_AMOCAS_Q:
            amocas_ensure_even_register(dc, newValueHighRegister == 0 ? 0 : newValueHighRegister - 1);
            amocas_ensure_even_register(dc, destinationHighRegister == 0 ? 0 : destinationHighRegister - 1);
            gen_amocas_128(result, source1, source2, dc->base.mem_idx, newValueHighRegister, destinationHighRegister);
            break;
#endif
    }

    gen_store_table_set(env, source1);
}

static void gen_alignment_check(DisasContext *dc, TCGv address, target_ulong alignment, int exception)
{
    int aligned = gen_new_label();
    TCGv misalignment = tcg_temp_new();

    tcg_gen_andi_tl(misalignment, address, alignment);
    tcg_gen_brcondi_tl(TCG_COND_EQ, misalignment, 0, aligned);
    generate_exception_mbadaddr(dc, exception);
    gen_set_label(aligned);

    tcg_temp_free(misalignment);
}

static void gen_atomic_alignment_check(DisasContext *dc, uint32_t opc, TCGv address)
{
    target_ulong alignment_mask = 0;

    switch(opc) {
        case OPC_RISC_LR_W:
            alignment_mask = 0x3;
            gen_alignment_check(dc, address, alignment_mask, RISCV_EXCP_LOAD_ADDR_MIS);
            break;
        case OPC_RISC_SC_W:
            alignment_mask = 0x3;
            gen_alignment_check(dc, address, alignment_mask, RISCV_EXCP_STORE_AMO_ADDR_MIS);
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_LR_D:
            alignment_mask = 0x7;
            gen_alignment_check(dc, address, alignment_mask, RISCV_EXCP_LOAD_ADDR_MIS);
            break;
        case OPC_RISC_SC_D:
            alignment_mask = 0x7;
            gen_alignment_check(dc, address, alignment_mask, RISCV_EXCP_STORE_AMO_ADDR_MIS);
            break;
#endif
        default:
            return;
    }
}

static void gen_atomic(CPUState *env, DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2)
{
    if(!ensure_extension(dc, RISCV_FEATURE_RVA)) {
        return;
    }

    /* TODO: handle aq, rl bits? - for now just get rid of them: */
    opc = MASK_OP_ATOMIC_NO_AQ_RL(opc);

    gen_sync_pc(dc);

    TCGv source1, source2, result;
    source1 = tcg_temp_local_new();
    source2 = tcg_temp_local_new();
    result = tcg_temp_local_new();

    gen_get_gpr(source1, rs1);
    gen_get_gpr(source2, rs2);
    gen_get_gpr(result, rd);

    gen_atomic_alignment_check(dc, opc, source1);

    if(rs1 == SP) {
        //  LR is the only pure read atomic instruciton
        bool is_write = MASK_FUNCT5(opc) != FUNCT5_LR;
        switch(MASK_FUNCT3(opc)) {
            case FUNCT3_WORD:
                try_run_pre_stack_access_hook(source1, 32, is_write);
                break;
            case FUNCT3_DOUBLEWORD:
                try_run_pre_stack_access_hook(source1, 64, is_write);
                break;
            case FUNCT3_QUADWORD:
                try_run_pre_stack_access_hook(source1, 128, is_write);
                break;
            default:
                //  Can only happen if the instruction is malformed somehow
                //  so it will throw an illegal instruction exception later
                break;
        }
    }

    gen_store_table_lock(cpu, source1);

    int done = gen_new_label();

    int funct5_bits = MASK_FUNCT5(opc);
    switch(funct5_bits) {
        case FUNCT5_AMOCAS:
            //  gen_store_table_set() is called inside gen_atomic_compare_and_swap().
            gen_atomic_compare_and_swap(dc, opc, result, source1, source2, rs2 == 0 ? 0 : rs2 + 1, rd == 0 ? 0 : rd + 1);
            break;
        default:
            //  gen_store_table_set() is called inside gen_atomic_fetch_and_op().
            gen_atomic_fetch_and_op(dc, opc, result, source1, source2);
            break;
    }

    gen_set_label(done);

    gen_store_table_unlock(cpu, source1);

    gen_set_gpr(rd, result);
    tcg_temp_free(source1);
    tcg_temp_free(source2);
    tcg_temp_free(result);
}

static void gen_fp_helper_fpr_3fpr_1imm(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64, TCGv_i64, TCGv_i64),
                                        enum riscv_floating_point_precision float_precision, int rd, int rs1, int rs2, int rs3,
                                        uint64_t rm)
{
    TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
    TCGv_i64 rs2_boxed = tcg_temp_local_new_i64();
    TCGv_i64 rs3_boxed = tcg_temp_local_new_i64();
    gen_unbox_float(float_precision, env, rs1_boxed, cpu_fpr[rs1]);
    gen_unbox_float(float_precision, env, rs2_boxed, cpu_fpr[rs2]);
    gen_unbox_float(float_precision, env, rs3_boxed, cpu_fpr[rs3]);

    TCGv_i64 rm_reg = tcg_temp_new_i64();
    tcg_gen_movi_i64(rm_reg, rm);

    gen_fp_helper(cpu_fpr[rd], cpu_env, rs1_boxed, rs2_boxed, rs3_boxed, rm_reg);
    gen_box_float(float_precision, cpu_fpr[rd]);

    tcg_temp_free_i64(rm_reg);
    tcg_temp_free_i64(rs1_boxed);
    tcg_temp_free_i64(rs2_boxed);
    tcg_temp_free_i64(rs3_boxed);
}

static void gen_fp_helper_fpr_2fpr_1tcg(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64, TCGv_i64),
                                        enum riscv_floating_point_precision float_precision, int rd, int rs1, int rs2,
                                        TCGv_i64 rm_reg)
{
    TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
    TCGv_i64 rs2_boxed = tcg_temp_local_new_i64();
    gen_unbox_float(float_precision, env, rs1_boxed, cpu_fpr[rs1]);
    gen_unbox_float(float_precision, env, rs2_boxed, cpu_fpr[rs2]);

    gen_fp_helper(cpu_fpr[rd], cpu_env, rs1_boxed, rs2_boxed, rm_reg);

    gen_box_float(float_precision, cpu_fpr[rd]);
    tcg_temp_free_i64(rs1_boxed);
    tcg_temp_free_i64(rs2_boxed);
}

static void gen_fp_helper_fpr_2fpr(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64),
                                   enum riscv_floating_point_precision float_precision, int rd, int rs1, int rs2)
{
    TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
    TCGv_i64 rs2_boxed = tcg_temp_local_new_i64();
    gen_unbox_float(float_precision, env, rs1_boxed, cpu_fpr[rs1]);
    gen_unbox_float(float_precision, env, rs2_boxed, cpu_fpr[rs2]);

    gen_fp_helper(cpu_fpr[rd], cpu_env, rs1_boxed, rs2_boxed);

    gen_box_float(float_precision, cpu_fpr[rd]);
    tcg_temp_free_i64(rs1_boxed);
    tcg_temp_free_i64(rs2_boxed);
}

static void gen_fp_helper_fpr_1fpr_1tcg(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64),
                                        enum riscv_floating_point_precision float_precision, int rd, int rs1, TCGv_i64 rm_reg)
{
    TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
    gen_unbox_float(float_precision, env, rs1_boxed, cpu_fpr[rs1]);

    gen_fp_helper(cpu_fpr[rd], cpu_env, rs1_boxed, rm_reg);

    gen_box_float(float_precision, cpu_fpr[rd]);
    tcg_temp_free_i64(rs1_boxed);
}

static void gen_fp_helper_gpr_2fpr(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64),
                                   enum riscv_floating_point_precision float_precision, TCGv_i64 rd_reg, int rd, int rs1, int rs2)
{
    TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
    TCGv_i64 rs2_boxed = tcg_temp_local_new_i64();
    gen_unbox_float(float_precision, env, rs1_boxed, cpu_fpr[rs1]);
    gen_unbox_float(float_precision, env, rs2_boxed, cpu_fpr[rs2]);

    gen_fp_helper(rd_reg, cpu_env, rs1_boxed, rs2_boxed);

    gen_set_gpr(rd, rd_reg);
    tcg_temp_free_i64(rs1_boxed);
    tcg_temp_free_i64(rs2_boxed);
}

static void gen_fp_helper_gpr_1fpr_1tcg(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64),
                                        enum riscv_floating_point_precision float_precision, TCGv_i64 rd_reg, int rd, int rs1,
                                        TCGv_i64 rm_reg)
{
    TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
    gen_unbox_float(float_precision, env, rs1_boxed, cpu_fpr[rs1]);

    gen_fp_helper(rd_reg, cpu_env, rs1_boxed, rm_reg);

    gen_set_gpr(rd, rd_reg);
    tcg_temp_free_i64(rs1_boxed);
}

static void gen_fp_helper_fpr_1gpr_1tcg(void (*gen_fp_helper)(TCGv_i64, TCGv_ptr, TCGv_i64, TCGv_i64),
                                        enum riscv_floating_point_precision float_precision, TCGv_i64 tmp_reg, int rd, int rs1,
                                        TCGv_i64 rm_reg)
{
    gen_get_gpr(tmp_reg, rs1);

    gen_fp_helper(cpu_fpr[rd], cpu_env, tmp_reg, rm_reg);

    gen_box_float(float_precision, cpu_fpr[rd]);
}

static void gen_fp_fmadd(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int rs3, int rm)
{
    if(!ensure_fp_extension(dc, 25)) {
        return;
    }

    switch(opc) {
        case OPC_RISC_FMADD_S:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fmadd_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        case OPC_RISC_FMADD_D: {
            TCGv_i64 rm_reg = tcg_temp_new_i64();
            tcg_gen_movi_i64(rm_reg, rm);
            gen_helper_fmadd_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], cpu_fpr[rs3], rm_reg);
            tcg_temp_free_i64(rm_reg);
            break;
        }
        case OPC_RISC_FMADD_H:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fmadd_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void gen_fp_fmsub(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int rs3, int rm)
{
    if(!ensure_fp_extension(dc, 25)) {
        return;
    }

    switch(opc) {
        case OPC_RISC_FMSUB_S:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fmsub_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        case OPC_RISC_FMSUB_D: {
            TCGv_i64 rm_reg = tcg_temp_new_i64();
            tcg_gen_movi_i64(rm_reg, rm);
            gen_helper_fmsub_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], cpu_fpr[rs3], rm_reg);
            tcg_temp_free_i64(rm_reg);
            break;
        }
        case OPC_RISC_FMSUB_H:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fmsub_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void gen_fp_fnmsub(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int rs3, int rm)
{
    if(!ensure_fp_extension(dc, 25)) {
        return;
    }

    switch(opc) {
        case OPC_RISC_FNMSUB_S:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fnmsub_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        case OPC_RISC_FNMSUB_D: {
            TCGv_i64 rm_reg = tcg_temp_new_i64();
            tcg_gen_movi_i64(rm_reg, rm);
            gen_helper_fnmsub_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], cpu_fpr[rs3], rm_reg);
            tcg_temp_free_i64(rm_reg);
            break;
        }
        case OPC_RISC_FNMSUB_H:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fnmsub_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void gen_fp_fnmadd(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int rs3, int rm)
{
    if(!ensure_fp_extension(dc, 25)) {
        return;
    }

    switch(opc) {
        case OPC_RISC_FNMADD_S:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fnmadd_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        case OPC_RISC_FNMADD_D: {
            TCGv_i64 rm_reg = tcg_temp_new_i64();
            tcg_gen_movi_i64(rm_reg, rm);
            gen_helper_fnmadd_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], cpu_fpr[rs3], rm_reg);
            tcg_temp_free_i64(rm_reg);
            break;
        }
        case OPC_RISC_FNMADD_H:
            gen_fp_helper_fpr_3fpr_1imm(gen_helper_fnmadd_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rs3, rm);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void gen_fp_arith(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int rm)
{
    if(!ensure_fp_extension(dc, 25)) {
        return;
    }

    gen_sync_pc(dc);

    TCGv_i64 rm_reg = tcg_temp_local_new_i64();
    TCGv write_int_rd = tcg_temp_local_new();
    tcg_gen_movi_i64(rm_reg, rm);
    switch(opc) {
        case OPC_RISC_FADD_S:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fadd_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FSUB_S:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fsub_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FMUL_S:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fmul_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FDIV_S:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fdiv_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FSGNJ_S:
            gen_fsgnj(dc, rd, rs1, rs2, rm, RISCV_SINGLE_PRECISION);
            break;
        case OPC_RISC_FMIN_S:
            /* also handles: OPC_RISC_FMAX_S */
            if(rm == 0x0) {
                gen_fp_helper_fpr_2fpr(gen_helper_fmin_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2);
            } else if(rm == 0x1) {
                gen_fp_helper_fpr_2fpr(gen_helper_fmax_s, RISCV_SINGLE_PRECISION, rd, rs1, rs2);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FSQRT_S:
            gen_fp_helper_fpr_1fpr_1tcg(gen_helper_fsqrt_s, RISCV_SINGLE_PRECISION, rd, rs1, rm_reg);
            break;
        case OPC_RISC_FEQ_S:
            /* also handles: OPC_RISC_FLT_S, OPC_RISC_FLE_S */
            if(rm == 0x0) {
                gen_fp_helper_gpr_2fpr(gen_helper_fle_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rs2);
            } else if(rm == 0x1) {
                gen_fp_helper_gpr_2fpr(gen_helper_flt_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rs2);
            } else if(rm == 0x2) {
                gen_fp_helper_gpr_2fpr(gen_helper_feq_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rs2);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FCVT_W_S:
            /* also OPC_RISC_FCVT_WU_S, OPC_RISC_FCVT_L_S, OPC_RISC_FCVT_LU_S */
            switch(rs2) {
                case 0x0: /* FCVT_W_S */
                    gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_w_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
                case 0x1: /* FCVT_WU_S */
                    gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_wu_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
#if defined(TARGET_RISCV64)
                case 0x2: /* FCVT_L_S */
                    gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_l_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
                case 0x3: /* FCVT_LU_S */
                    gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_lu_s, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
#endif
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case OPC_RISC_FCVT_S_W:
            /* also OPC_RISC_FCVT_S_WU, OPC_RISC_FCVT_S_L, OPC_RISC_FCVT_S_LU */
            gen_get_gpr(write_int_rd, rs1);
            switch(rs2) {
                case 0x0: /* FCVT_S_W */
                    gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_s_w, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
                case 0x1: /* FCVT_S_WU */
                    gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_s_wu, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
#if defined(TARGET_RISCV64)
                case 0x2: /* FCVT_S_L */
                    gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_s_l, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
                case 0x3: /* FCVT_S_LU */
                    gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_s_lu, RISCV_SINGLE_PRECISION, write_int_rd, rd, rs1, rm_reg);
                    break;
#endif
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case OPC_RISC_FMV_X_S: {
            int fp_ok = gen_new_label();
            int done = gen_new_label();

            //  check MSTATUS.FS
            tcg_gen_ld_tl(write_int_rd, cpu_env, offsetof(CPUState, mstatus));
            tcg_gen_andi_tl(write_int_rd, write_int_rd, MSTATUS_FS);
            tcg_gen_brcondi_tl(TCG_COND_NE, write_int_rd, 0x0, fp_ok);
            //  MSTATUS_FS field was zero:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            tcg_gen_br(done);

            //  proceed with operation
            gen_set_label(fp_ok);
            /* also OPC_RISC_FCLASS_S */
            if(rm == 0x0) { /* FMV */
#if defined(TARGET_RISCV64)
                tcg_gen_ext32s_tl(write_int_rd, cpu_fpr[rs1]);
#else
                tcg_gen_trunc_i64_i32(write_int_rd, cpu_fpr[rs1]);
#endif
            } else if(rm == 0x1) {
                gen_helper_fclass_s(write_int_rd, cpu_env, cpu_fpr[rs1]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            gen_set_gpr(rd, write_int_rd);
            gen_set_label(done);
            break;
        }
        case OPC_RISC_FMV_S_X: {
            int fp_ok = gen_new_label();
            int done = gen_new_label();

            //  check MSTATUS.FS
            tcg_gen_ld_tl(write_int_rd, cpu_env, offsetof(CPUState, mstatus));
            tcg_gen_andi_tl(write_int_rd, write_int_rd, MSTATUS_FS);
            tcg_gen_brcondi_tl(TCG_COND_NE, write_int_rd, 0x0, fp_ok);
            //  MSTATUS_FS field was zero:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            tcg_gen_br(done);

            //  proceed with operation
            gen_set_label(fp_ok);
            gen_get_gpr(write_int_rd, rs1);
#if defined(TARGET_RISCV64)
            tcg_gen_mov_tl(cpu_fpr[rd], write_int_rd);
#else
            tcg_gen_extu_i32_i64(cpu_fpr[rd], write_int_rd);
#endif
            gen_box_float(RISCV_SINGLE_PRECISION, cpu_fpr[rd]);
            gen_set_label(done);
            break;
        }
        /* double */
        case OPC_RISC_FADD_D:
            gen_helper_fadd_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], rm_reg);
            break;
        case OPC_RISC_FSUB_D:
            gen_helper_fsub_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], rm_reg);
            break;
        case OPC_RISC_FMUL_D:
            gen_helper_fmul_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], rm_reg);
            break;
        case OPC_RISC_FDIV_D:
            gen_helper_fdiv_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2], rm_reg);
            break;
        case OPC_RISC_FSGNJ_D:
            gen_fsgnj(dc, rd, rs1, rs2, rm, RISCV_DOUBLE_PRECISION);
            break;
        case OPC_RISC_FMIN_D:
            /* also OPC_RISC_FMAX_D */
            if(rm == 0x0) {
                gen_helper_fmin_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2]);
            } else if(rm == 0x1) {
                gen_helper_fmax_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], cpu_fpr[rs2]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FCVT_S_D: {
            TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
            if(rs2 == 0x1) {
                gen_helper_fcvt_s_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], rm_reg);
                gen_box_float(RISCV_SINGLE_PRECISION, cpu_fpr[rd]);
            } else if(rs2 == 0x2) {
                gen_unbox_float(RISCV_HALF_PRECISION, env, rs1_boxed, cpu_fpr[rs1]);
                gen_helper_fcvt_s_h(cpu_fpr[rd], cpu_env, rs1_boxed, rm_reg);
                gen_box_float(RISCV_SINGLE_PRECISION, cpu_fpr[rd]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            tcg_temp_free_i64(rs1_boxed);
            break;
        }
        case OPC_RISC_FCVT_D_S: {
            TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
            if(rs2 == 0x0) {
                gen_unbox_float(RISCV_SINGLE_PRECISION, env, rs1_boxed, cpu_fpr[rs1]);
                gen_helper_fcvt_d_s(cpu_fpr[rd], cpu_env, rs1_boxed, rm_reg);
            } else if(rs2 == 0x2) {
                gen_unbox_float(RISCV_HALF_PRECISION, env, rs1_boxed, cpu_fpr[rs1]);
                gen_helper_fcvt_d_h(cpu_fpr[rd], cpu_env, rs1_boxed, rm_reg);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            tcg_temp_free_i64(rs1_boxed);
            break;
        }
        case OPC_RISC_FSQRT_D:
            gen_helper_fsqrt_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], rm_reg);
            break;
        case OPC_RISC_FEQ_D:
            /* also OPC_RISC_FLT_D, OPC_RISC_FLE_D */
            if(rm == 0x0) {
                gen_helper_fle_d(write_int_rd, cpu_env, cpu_fpr[rs1], cpu_fpr[rs2]);
            } else if(rm == 0x1) {
                gen_helper_flt_d(write_int_rd, cpu_env, cpu_fpr[rs1], cpu_fpr[rs2]);
            } else if(rm == 0x2) {
                gen_helper_feq_d(write_int_rd, cpu_env, cpu_fpr[rs1], cpu_fpr[rs2]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            gen_set_gpr(rd, write_int_rd);
            break;
        case OPC_RISC_FCVT_W_D:
            /* also OPC_RISC_FCVT_WU_D, OPC_RISC_FCVT_L_D, OPC_RISC_FCVT_LU_D */
            switch(rs2) {
                case 0x0:
                    gen_helper_fcvt_w_d(write_int_rd, cpu_env, cpu_fpr[rs1], rm_reg);
                    break;
                case 0x1:
                    gen_helper_fcvt_wu_d(write_int_rd, cpu_env, cpu_fpr[rs1], rm_reg);
                    break;
#if defined(TARGET_RISCV64)
                case 0x2:
                    gen_helper_fcvt_l_d(write_int_rd, cpu_env, cpu_fpr[rs1], rm_reg);
                    break;
                case 0x3:
                    gen_helper_fcvt_lu_d(write_int_rd, cpu_env, cpu_fpr[rs1], rm_reg);
                    break;
#endif
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            gen_set_gpr(rd, write_int_rd);
            break;
        case OPC_RISC_FCVT_D_W:
            /* also OPC_RISC_FCVT_D_WU, OPC_RISC_FCVT_D_L, OPC_RISC_FCVT_D_LU */
            gen_get_gpr(write_int_rd, rs1);
            switch(rs2) {
                case 0x0:
                    gen_helper_fcvt_d_w(cpu_fpr[rd], cpu_env, write_int_rd, rm_reg);
                    break;
                case 0x1:
                    gen_helper_fcvt_d_wu(cpu_fpr[rd], cpu_env, write_int_rd, rm_reg);
                    break;
#if defined(TARGET_RISCV64)
                case 0x2:
                    gen_helper_fcvt_d_l(cpu_fpr[rd], cpu_env, write_int_rd, rm_reg);
                    break;
                case 0x3:
                    gen_helper_fcvt_d_lu(cpu_fpr[rd], cpu_env, write_int_rd, rm_reg);
                    break;
#endif
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
#if defined(TARGET_RISCV64)
        case OPC_RISC_FMV_X_D: {
            int fp_ok = gen_new_label();
            int done = gen_new_label();

            //  check MSTATUS.FS
            tcg_gen_ld_tl(write_int_rd, cpu_env, offsetof(CPUState, mstatus));
            tcg_gen_andi_tl(write_int_rd, write_int_rd, MSTATUS_FS);
            tcg_gen_brcondi_tl(TCG_COND_NE, write_int_rd, 0x0, fp_ok);
            //  MSTATUS_FS field was zero:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            tcg_gen_br(done);

            //  proceed with operation
            gen_set_label(fp_ok);
            /* also OPC_RISC_FCLASS_D */
            if(rm == 0x0) { /* FMV */
                tcg_gen_mov_tl(write_int_rd, cpu_fpr[rs1]);
            } else if(rm == 0x1) {
                gen_helper_fclass_d(write_int_rd, cpu_env, cpu_fpr[rs1]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            gen_set_gpr(rd, write_int_rd);
            gen_set_label(done);
            break;
        }
        case OPC_RISC_FMV_D_X: {
            int fp_ok = gen_new_label();
            int done = gen_new_label();

            //  check MSTATUS.FS
            tcg_gen_ld_tl(write_int_rd, cpu_env, offsetof(CPUState, mstatus));
            tcg_gen_andi_tl(write_int_rd, write_int_rd, MSTATUS_FS);
            tcg_gen_brcondi_tl(TCG_COND_NE, write_int_rd, 0x0, fp_ok);
            //  MSTATUS_FS field was zero:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            tcg_gen_br(done);

            //  proceed with operation
            gen_set_label(fp_ok);
            gen_get_gpr(write_int_rd, rs1);
            tcg_gen_mov_tl(cpu_fpr[rd], write_int_rd);
            gen_set_label(done);
            break;
        }
#endif
        /* half-precision */
        case OPC_RISC_FADD_H:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fadd_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FSUB_H:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fsub_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FMUL_H:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fmul_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FDIV_H:
            gen_fp_helper_fpr_2fpr_1tcg(gen_helper_fdiv_h, RISCV_HALF_PRECISION, rd, rs1, rs2, rm_reg);
            break;
        case OPC_RISC_FSGNJ_H:
            gen_fsgnj(dc, rd, rs1, rs2, rm, RISCV_HALF_PRECISION);
            break;
        case OPC_RISC_FMIN_H:
            /* also OPC_RISC_FMAX_H */
            if(rm == 0x0) {
                gen_fp_helper_fpr_2fpr(gen_helper_fmin_h, RISCV_HALF_PRECISION, rd, rs1, rs2);
            } else if(rm == 0x1) {
                gen_fp_helper_fpr_2fpr(gen_helper_fmax_h, RISCV_HALF_PRECISION, rd, rs1, rs2);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FCVT_H_S: {
            TCGv_i64 rs1_boxed = tcg_temp_local_new_i64();
            if(rs2 == 0x0) {
                gen_unbox_float(RISCV_SINGLE_PRECISION, env, rs1_boxed, cpu_fpr[rs1]);
                gen_helper_fcvt_h_s(cpu_fpr[rd], cpu_env, rs1_boxed, rm_reg);
                gen_box_float(RISCV_HALF_PRECISION, cpu_fpr[rd]);
            } else if(rs2 == 0x1) {
                gen_helper_fcvt_h_d(cpu_fpr[rd], cpu_env, cpu_fpr[rs1], rm_reg);
                gen_box_float(RISCV_HALF_PRECISION, cpu_fpr[rd]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            tcg_temp_free_i64(rs1_boxed);
            break;
        }
        case OPC_RISC_FSQRT_H:
            gen_fp_helper_fpr_1fpr_1tcg(gen_helper_fsqrt_h, RISCV_HALF_PRECISION, rd, rs1, rm_reg);
            break;
        case OPC_RISC_FEQ_H:
            /* also OPC_RISC_FLT_H, OPC_RISC_FLE_H */
            if(rm == 0x0) {
                gen_fp_helper_gpr_2fpr(gen_helper_fle_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rs2);
            } else if(rm == 0x1) {
                gen_fp_helper_gpr_2fpr(gen_helper_flt_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rs2);
            } else if(rm == 0x2) {
                gen_fp_helper_gpr_2fpr(gen_helper_feq_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rs2);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FCVT_W_H:
            /* also OPC_RISC_FCVT_WU_H, OPC_RISC_FCVT_L_H, OPC_RISC_FCVT_LU_H */
            if(rs2 == 0x0) {
                gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_w_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
            } else if(rs2 == 0x1) {
                gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_wu_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
            } else if(rs2 == 0x2) {
#if defined(TARGET_RISCV64)
                gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_l_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
#else
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
#endif
            } else if(rs2 == 0x3) {
#if defined(TARGET_RISCV64)
                gen_fp_helper_gpr_1fpr_1tcg(gen_helper_fcvt_lu_h, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
#else
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
#endif
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FCVT_H_W:
            /* also OPC_RISC_FCVT_H_WU, OPC_RISC_FCVT_H_L, OPC_RISC_FCVT_H_LU */
            if(rs2 == 0x0) {
                gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_h_w, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
            } else if(rs2 == 0x1) {
                gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_h_wu, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
            } else if(rs2 == 0x2) {
#if defined(TARGET_RISCV64)
                gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_h_l, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
#else
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
#endif
            } else if(rs2 == 0x3) {
#if defined(TARGET_RISCV64)
                gen_fp_helper_fpr_1gpr_1tcg(gen_helper_fcvt_h_lu, RISCV_HALF_PRECISION, write_int_rd, rd, rs1, rm_reg);
#else
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
#endif
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case OPC_RISC_FMV_X_H:
            /* also OPC_RISC_FCLASS_H */
            if(rm == 0x0) { /* FMV */
                gen_helper_fmv_x_h(write_int_rd, cpu_env, cpu_fpr[rs1], rm_reg);
            } else if(rm == 0x1) {
                gen_helper_fclass_h(write_int_rd, cpu_env, cpu_fpr[rs1]);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            gen_set_gpr(rd, write_int_rd);
            break;
        case OPC_RISC_FMV_H_X:
            gen_get_gpr(write_int_rd, rs1);
            gen_helper_fmv_h_x(cpu_fpr[rd], cpu_env, write_int_rd, rm_reg);
            gen_box_float(RISCV_HALF_PRECISION, cpu_fpr[rd]);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free_i64(rm_reg);
    tcg_temp_free(write_int_rd);
}

static void gen_system(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int funct12)
{
    gen_sync_pc(dc);
    if(opc == OPC_RISC_ECALL) {
        //  This group uses both `I-type` and `R-type` instruction formats
        //  It's easier to start narrowing with the shorter function code
        int funct7 = funct12 >> 5;

        switch(funct7) {
            case 0x0:
                switch(rs2) {
                    case 0x0: /* ECALL */
                        /* always generates U-level ECALL, fixed in do_interrupt handler */
                        generate_exception(dc, RISCV_EXCP_U_ECALL);
                        gen_exit_tb_no_chaining(dc->base.tb);
                        dc->base.is_jmp = DISAS_BRANCH;
                        break;
                    case 0x1: /* EBREAK */
                        generate_exception(dc, RISCV_EXCP_BREAKPOINT);
                        gen_exit_tb_no_chaining(dc->base.tb);
                        dc->base.is_jmp = DISAS_BRANCH;
                        break;
                    case 0x2: /* URET */
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                    default:
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                }
                break;
            case 0x8:
                switch(rs2) {
                    case 0x2: /* SRET */
                        gen_helper_sret(cpu_pc, cpu_env, cpu_pc);
                        gen_exit_tb_no_chaining(dc->base.tb);
                        dc->base.is_jmp = DISAS_BRANCH;
                        break;
                    case 0x4: /* SFENCE.VM */
                        gen_helper_tlb_flush(cpu_env);
                        break;
                    case 0x5: /* WFI */
                        tcg_gen_movi_tl(cpu_pc, dc->npc);
                        gen_helper_wfi(cpu_env);
                        gen_exit_tb_no_chaining(dc->base.tb);
                        dc->base.is_jmp = DISAS_BRANCH;
                        break;
                    default:
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                        break;
                }
                break;
            case 0x9: /* SFENCE.VMA */
                gen_helper_sfence_vma(cpu_env);
                break;
            case 0x10: /* HRET */
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                break;
            case 0x18: /* MRET */
                gen_helper_mret(cpu_pc, cpu_env, cpu_pc);
                gen_exit_tb_no_chaining(dc->base.tb);
                dc->base.is_jmp = DISAS_BRANCH;
                break;
            case 0x3d: /* DRET */
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                break;
            default:
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    } else {
        if(!riscv_has_additional_ext(cpu, RISCV_FEATURE_ZICSR)) {
            int instruction_length = decode_instruction_length(dc->opcode);
            tlib_printf(LOG_LEVEL_ERROR,
                        "RISC-V Zicsr instruction set is not enabled for this CPU! In future release this configuration will "
                        "lead to an illegal instruction exception. PC: 0x%llx, opcode: 0x%0*llx",
                        dc->base.pc, /* padding */ 2 * instruction_length, format_opcode(dc->opcode, instruction_length));
        }
        TCGv source1, csr_store, dest, rs1_pass, imm_rs1;
        source1 = tcg_temp_new();
        csr_store = tcg_temp_new();
        dest = tcg_temp_new();
        rs1_pass = tcg_temp_new();
        imm_rs1 = tcg_temp_new();
        gen_get_gpr(source1, rs1);
        tcg_gen_movi_tl(rs1_pass, rs1);
        tcg_gen_movi_tl(csr_store, funct12); /* copy into temp reg to feed to helper */
        tcg_gen_movi_tl(imm_rs1, rs1);

        switch(opc) {
            case OPC_RISC_CSRRW:
                if(rd == 0) {
                    gen_helper_csrrw_no_read(cpu_env, source1, csr_store);
                } else {
                    gen_helper_csrrw(dest, cpu_env, source1, csr_store);
                    gen_set_gpr(rd, dest);
                }
                break;
            case OPC_RISC_CSRRS:
                gen_helper_csrrs(dest, cpu_env, source1, csr_store, rs1_pass);
                gen_set_gpr(rd, dest);
                break;
            case OPC_RISC_CSRRC:
                gen_helper_csrrc(dest, cpu_env, source1, csr_store, rs1_pass);
                gen_set_gpr(rd, dest);
                break;
            case OPC_RISC_CSRRWI:
                if(rd == 0) {
                    gen_helper_csrrw_no_read(cpu_env, imm_rs1, csr_store);
                } else {
                    gen_helper_csrrw(dest, cpu_env, imm_rs1, csr_store);
                    gen_set_gpr(rd, dest);
                }
                break;
            case OPC_RISC_CSRRSI:
                gen_helper_csrrs(dest, cpu_env, imm_rs1, csr_store, rs1_pass);
                gen_set_gpr(rd, dest);
                break;
            case OPC_RISC_CSRRCI:
                gen_helper_csrrc(dest, cpu_env, imm_rs1, csr_store, rs1_pass);
                gen_set_gpr(rd, dest);
                break;
            default:
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                break;
        }

        /* end tb since we may be changing priv modes, to get mmu_index right */
        tcg_gen_movi_tl(cpu_pc, dc->npc);
        gen_exit_tb_no_chaining(dc->base.tb);
        dc->base.is_jmp = DISAS_BRANCH;

        tcg_temp_free(source1);
        tcg_temp_free(csr_store);
        tcg_temp_free(dest);
        tcg_temp_free(rs1_pass);
        tcg_temp_free(imm_rs1);
    }
}

//  Vector helpers require 128-bit ints which aren't supported on 32-bit hosts.
#if HOST_LONG_BITS != 32
static void gen_v_cfg(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int imm)
{
    TCGv rs1_value, rs2_value, zimm, returned_vl, rd_index, rs1_index, rs1_is_uimm;

    if(!ensure_vector_embedded_extension_or_kill_unknown(dc)) {
        return;
    }

    zimm = tcg_temp_new();
    rd_index = tcg_temp_new();
    rs1_index = tcg_temp_new();
    rs1_value = tcg_temp_new();
    rs2_value = tcg_temp_new();

    if(opc == OPC_RISC_VSETIVLI) {
        //  in vsetivli the imm field is [9:0] rather than [11:0]
        imm = imm & ((1 << 10) - 1);
    } else if(opc == OPC_RISC_VSETVLI_0 || opc == OPC_RISC_VSETVLI_1) {
        //  in vsetvli the imm field is [10:0] rather than [11:0]
        imm = imm & ((1 << 11) - 1);
    }

    tcg_gen_movi_tl(rd_index, rd);
    tcg_gen_movi_tl(rs1_index, rs1);
    tcg_gen_movi_tl(zimm, imm);
    gen_get_gpr(rs1_value, rs1);
    gen_get_gpr(rs2_value, rs2);

    rs1_is_uimm = tcg_temp_new();
    if(opc == OPC_RISC_VSETIVLI) {
        tcg_gen_movi_i32(rs1_is_uimm, 1);
    } else {
        tcg_gen_movi_i32(rs1_is_uimm, 0);
    }

    gen_sync_pc(dc);
    returned_vl = tcg_temp_new();

    switch(opc) {
        case OPC_RISC_VSETVL:
            gen_helper_vsetvl(returned_vl, cpu_env, rd_index, rs1_index, rs1_value, rs2_value, rs1_is_uimm);
            gen_set_gpr(rd, returned_vl);
            break;
        case OPC_RISC_VSETVLI_0:
        case OPC_RISC_VSETVLI_1:
            gen_helper_vsetvl(returned_vl, cpu_env, rd_index, rs1_index, rs1_value, zimm, rs1_is_uimm);
            gen_set_gpr(rd, returned_vl);
            break;
        case OPC_RISC_VSETIVLI:
            gen_helper_vsetvl(returned_vl, cpu_env, rd_index, rs1_index, rs1_index, zimm, rs1_is_uimm);
            gen_set_gpr(rd, returned_vl);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }

    tcg_temp_free(rs1_value);
    tcg_temp_free(rs2_value);
    tcg_temp_free(returned_vl);
    tcg_temp_free(rs1_index);
    tcg_temp_free(rs1_is_uimm);
}

static void gen_v_opivv(DisasContext *dc, uint8_t funct6, int vd, int vs1, int vs2, uint8_t vm)
{
    generate_vill_check(dc);
    TCGv_i32 t_vd, t_vs1, t_vs2;
    t_vd = tcg_temp_new_i32();
    t_vs1 = tcg_temp_new_i32();
    t_vs2 = tcg_temp_new_i32();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_vs1, vs1);
    tcg_gen_movi_i32(t_vs2, vs2);

    switch(funct6) {
        case RISC_V_FUNCT_ADD:
            if(vm) {
                gen_helper_vadd_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vadd_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SUB:
            if(vm) {
                gen_helper_vsub_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsub_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MINU:
            if(vm) {
                gen_helper_vminu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vminu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MIN:
            if(vm) {
                gen_helper_vmin_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmin_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MAXU:
            if(vm) {
                gen_helper_vmaxu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmaxu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MAX:
            if(vm) {
                gen_helper_vmax_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmax_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_AND:
            if(vm) {
                gen_helper_vand_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vand_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_OR:
            if(vm) {
                gen_helper_vor_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vor_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_XOR:
            if(vm) {
                gen_helper_vxor_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vxor_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_RGATHER:
            if(vm) {
                gen_helper_vrgather_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vrgather_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_RGATHEREI16:
            if(vm) {
                gen_helper_vrgatherei16_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vrgatherei16_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_ADC:
            if(vm) {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                if(!vd) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
                }
                gen_helper_vadc_vvm(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MADC:
            if(vm) {
                gen_helper_vmadc_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmadc_vvm(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SBC:
            if(vm) {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                if(!vd) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
                }
                gen_helper_vsbc_vvm(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSBC:
            if(vm) {
                gen_helper_vmsbc_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmsbc_vvm(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MERGE_MV:
            if(vm) {
                if(vs2) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                } else {
                    gen_helper_vmv_ivv(cpu_env, t_vd, t_vs1);
                }
            } else {
                gen_helper_vmerge_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSEQ:
            if(vm) {
                gen_helper_vmseq_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmseq_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSNE:
            if(vm) {
                gen_helper_vmsne_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmsne_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSLTU:
            if(vm) {
                gen_helper_vmsltu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmsltu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSLT:
            if(vm) {
                gen_helper_vmslt_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmslt_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSLEU:
            if(vm) {
                gen_helper_vmsleu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmsleu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MSLE:
            if(vm) {
                gen_helper_vmsle_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmsle_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SADDU:
            if(vm) {
                gen_helper_vsaddu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsaddu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SADD:
            if(vm) {
                gen_helper_vsadd_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsadd_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SSUBU:
            if(vm) {
                gen_helper_vssubu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vssubu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SSUB:
            if(vm) {
                gen_helper_vssub_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vssub_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SLL:
            if(vm) {
                gen_helper_vsll_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsll_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SMUL:
            if(vm) {
                gen_helper_vsmul_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsmul_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SRL:
            if(vm) {
                gen_helper_vsrl_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsrl_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SRA:
            if(vm) {
                gen_helper_vsra_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vsra_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SSRL:
            if(vm) {
                gen_helper_vssrl_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vssrl_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_SSRA:
            if(vm) {
                gen_helper_vssra_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vssra_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_NSRL:
            if(vm) {
                gen_helper_vnsrl_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vnsrl_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_NSRA:
            if(vm) {
                gen_helper_vnsra_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vnsra_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_NCLIPU:
            if(vm) {
                gen_helper_vnclipu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vnclipu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_NCLIP:
            if(vm) {
                gen_helper_vnclip_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vnclip_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WREDSUMU:
            if(vm) {
                gen_helper_vwredsumu_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwredsumu_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WREDSUM:
            if(vm) {
                gen_helper_vwredsum_ivv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwredsum_ivv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_vs1);
    tcg_temp_free_i32(t_vs2);
}

//  common or mutually exclusive operations for vi and vx
static void gen_v_opivt(DisasContext *dc, uint8_t funct6, int vd, int vs2, TCGv t, uint8_t vm)
{
    TCGv_i32 t_vd, t_vs2;
    t_vd = tcg_temp_new_i32();
    t_vs2 = tcg_temp_new_i32();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_vs2, vs2);

    switch(funct6) {
        //  Common for vi and vx
        case RISC_V_FUNCT_ADD:
            if(vm) {
                gen_helper_vadd_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vadd_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_RSUB:
            if(vm) {
                gen_helper_vrsub_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vrsub_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_AND:
            if(vm) {
                gen_helper_vand_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vand_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_OR:
            if(vm) {
                gen_helper_vor_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vor_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_XOR:
            if(vm) {
                gen_helper_vxor_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vxor_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_RGATHER:
            if(vm) {
                gen_helper_vrgather_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vrgather_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SLIDEUP:
            if(vm) {
                gen_helper_vslideup_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vslideup_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SLIDEDOWN:
            if(vm) {
                gen_helper_vslidedown_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vslidedown_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_ADC:
            if(vm) {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                if(!vd) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
                }
                gen_helper_vadc_vi(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MADC:
            if(vm) {
                gen_helper_vmadc_vi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmadc_vim(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MERGE_MV:
            if(vm) {
                if(vs2) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                } else {
                    gen_helper_vmv_ivi(cpu_env, t_vd, t);
                }
            } else {
                gen_helper_vmerge_ivi(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSEQ:
            if(vm) {
                gen_helper_vmseq_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmseq_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSNE:
            if(vm) {
                gen_helper_vmsne_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsne_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSLEU:
            if(vm) {
                gen_helper_vmsleu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsleu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSLE:
            if(vm) {
                gen_helper_vmsle_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsle_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSGTU:
            if(vm) {
                gen_helper_vmsgtu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsgtu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSGT:
            if(vm) {
                gen_helper_vmsgt_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsgt_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SADDU:
            if(vm) {
                gen_helper_vsaddu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vsaddu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SADD:
            if(vm) {
                gen_helper_vsadd_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vsadd_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SLL:
            if(vm) {
                gen_helper_vsll_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vsll_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SRL:
            if(vm) {
                gen_helper_vsrl_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vsrl_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SRA:
            if(vm) {
                gen_helper_vsra_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vsra_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SSRL:
            if(vm) {
                gen_helper_vssrl_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vssrl_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SSRA:
            if(vm) {
                gen_helper_vssra_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vssra_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_NSRL:
            if(vm) {
                gen_helper_vnsrl_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vnsrl_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_NSRA:
            if(vm) {
                gen_helper_vnsra_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vnsra_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_NCLIPU:
            if(vm) {
                gen_helper_vnclipu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vnclipu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_NCLIP:
            if(vm) {
                gen_helper_vnclip_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vnclip_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        //  defined for vi and reserved for vx
        //  reserved for vi and defined for vx
        case RISC_V_FUNCT_SUB:
            tcg_gen_neg_i64(t, t);
            if(vm) {
                gen_helper_vadd_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vadd_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MINU:
            if(vm) {
                gen_helper_vminu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vminu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MIN:
            if(vm) {
                gen_helper_vmin_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmin_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MAXU:
            if(vm) {
                gen_helper_vmaxu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmaxu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MAX:
            if(vm) {
                gen_helper_vmax_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmax_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SBC:
            if(vm) {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                if(!vd) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
                }
                gen_helper_vsbc_vi(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSBC:
            if(vm) {
                gen_helper_vmsbc_vi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsbc_vim(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSLTU:
            if(vm) {
                gen_helper_vmsltu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmsltu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_MSLT:
            if(vm) {
                gen_helper_vmslt_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vmslt_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SSUBU:
            if(vm) {
                gen_helper_vssubu_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vssubu_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        case RISC_V_FUNCT_SSUB:
            if(vm) {
                gen_helper_vssub_ivi(cpu_env, t_vd, t_vs2, t);
            } else {
                gen_helper_vssub_ivi_m(cpu_env, t_vd, t_vs2, t);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_vs2);
}

static void gen_v_opivi(DisasContext *dc, uint8_t funct6, int vd, int rs1, int vs2, uint8_t vm)
{
    if(funct6 != RISC_V_FUNCT_MV_NF_R) {
        generate_vill_check(dc);
    }
    int64_t simm5 = rs1;
    TCGv t_simm5;
    TCGv_i32 t_vd, t_vs2;
    t_simm5 = tcg_temp_new();

    switch(funct6) {
        //  Common for vi and vx
        //  zero-extended immediate
        case RISC_V_FUNCT_NSRL:
        case RISC_V_FUNCT_NSRA:
        case RISC_V_FUNCT_NCLIPU:
        case RISC_V_FUNCT_NCLIP:
        case RISC_V_FUNCT_SLIDEUP:
        case RISC_V_FUNCT_SLIDEDOWN:
        case RISC_V_FUNCT_RGATHER:
        case RISC_V_FUNCT_SLL:
        case RISC_V_FUNCT_SRL:
        case RISC_V_FUNCT_SRA:
        case RISC_V_FUNCT_SSRL:
        case RISC_V_FUNCT_SSRA:
            tcg_gen_movi_tl(t_simm5, simm5);
            gen_v_opivt(dc, funct6, vd, vs2, t_simm5, vm);
            break;
        //  sign-extended immediate
        case RISC_V_FUNCT_ADD:
        case RISC_V_FUNCT_RSUB:
        case RISC_V_FUNCT_AND:
        case RISC_V_FUNCT_OR:
        case RISC_V_FUNCT_XOR:
        case RISC_V_FUNCT_ADC:
        case RISC_V_FUNCT_MADC:
        case RISC_V_FUNCT_MERGE_MV:
        case RISC_V_FUNCT_MSEQ:
        case RISC_V_FUNCT_MSNE:
        case RISC_V_FUNCT_MSLEU:
        case RISC_V_FUNCT_MSLE:
        case RISC_V_FUNCT_MSGTU:
        case RISC_V_FUNCT_MSGT:
        case RISC_V_FUNCT_SADDU:
        case RISC_V_FUNCT_SADD:
            //  Reserved for vx
            simm5 = rs1 >= 0x10 ? (0xffffffffffffffe0) | rs1 : rs1;
            tcg_gen_movi_tl(t_simm5, simm5);
            gen_v_opivt(dc, funct6, vd, vs2, t_simm5, vm);
            break;
        //  Conflicting
        case RISC_V_FUNCT_MV_NF_R:
            t_vd = tcg_temp_new_i32();
            t_vs2 = tcg_temp_new_i32();
            tcg_gen_movi_i32(t_vd, vd);
            tcg_gen_movi_i32(t_vs2, vs2);

            switch(rs1) {
                case 0:
                    gen_helper_vmv1r_v(cpu_env, t_vd, t_vs2);
                    break;
                case 1:
                    gen_helper_vmv2r_v(cpu_env, t_vd, t_vs2);
                    break;
                case 3:
                    gen_helper_vmv4r_v(cpu_env, t_vd, t_vs2);
                    break;
                case 7:
                    gen_helper_vmv8r_v(cpu_env, t_vd, t_vs2);
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            tcg_temp_free_i32(t_vd);
            tcg_temp_free_i32(t_vs2);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free(t_simm5);
}

static void gen_v_opivx(DisasContext *dc, uint8_t funct6, int vd, int rs1, int vs2, uint8_t vm)
{
    generate_vill_check(dc);
    TCGv_i32 t_vd, t_vs2;
    TCGv t_tl;
    t_tl = tcg_temp_new();
    gen_get_gpr(t_tl, rs1);

    switch(funct6) {
        //  Common for vi and vx
        case RISC_V_FUNCT_ADD:
        case RISC_V_FUNCT_RSUB:
        case RISC_V_FUNCT_AND:
        case RISC_V_FUNCT_OR:
        case RISC_V_FUNCT_XOR:
        case RISC_V_FUNCT_RGATHER:
        case RISC_V_FUNCT_SLIDEUP:
        case RISC_V_FUNCT_SLIDEDOWN:
        case RISC_V_FUNCT_ADC:
        case RISC_V_FUNCT_MADC:
        case RISC_V_FUNCT_MERGE_MV:
        case RISC_V_FUNCT_MSEQ:
        case RISC_V_FUNCT_MSNE:
        case RISC_V_FUNCT_MSLEU:
        case RISC_V_FUNCT_MSLE:
        case RISC_V_FUNCT_MSGTU:
        case RISC_V_FUNCT_MSGT:
        case RISC_V_FUNCT_SADDU:
        case RISC_V_FUNCT_SADD:
        case RISC_V_FUNCT_SLL:
        case RISC_V_FUNCT_SRL:
        case RISC_V_FUNCT_SRA:
        case RISC_V_FUNCT_SSRL:
        case RISC_V_FUNCT_SSRA:
        case RISC_V_FUNCT_NSRL:
        case RISC_V_FUNCT_NSRA:
        case RISC_V_FUNCT_NCLIPU:
        case RISC_V_FUNCT_NCLIP:
        //  Reserved for vi
        case RISC_V_FUNCT_SUB:
        case RISC_V_FUNCT_MINU:
        case RISC_V_FUNCT_MIN:
        case RISC_V_FUNCT_MAXU:
        case RISC_V_FUNCT_MAX:
        case RISC_V_FUNCT_SBC:
        case RISC_V_FUNCT_MSBC:
        case RISC_V_FUNCT_MSLTU:
        case RISC_V_FUNCT_MSLT:
        case RISC_V_FUNCT_SSUBU:
        case RISC_V_FUNCT_SSUB:
            gen_v_opivt(dc, funct6, vd, vs2, t_tl, vm);
            break;
        //  Conflicting
        case RISC_V_FUNCT_SMUL:
            t_vd = tcg_temp_new_i32();
            t_vs2 = tcg_temp_new_i32();
            tcg_gen_movi_i32(t_vd, vd);
            tcg_gen_movi_i32(t_vs2, vs2);
            if(vm) {
                gen_helper_vsmul_ivx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vsmul_ivx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            tcg_temp_free_i32(t_vd);
            tcg_temp_free_i32(t_vs2);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free(t_tl);
}

static void gen_v_opmvv(DisasContext *dc, uint8_t funct6, int vd, int vs1, int vs2, uint8_t vm)
{
    generate_vill_check(dc);
    TCGv_i32 t_vd, t_vs1, t_vs2;
    TCGv t_tl;
    t_vd = tcg_temp_new_i32();
    t_vs1 = tcg_temp_new_i32();
    t_vs2 = tcg_temp_new_i32();
    t_tl = tcg_temp_new();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_vs1, vs1);
    tcg_gen_movi_i32(t_vs2, vs2);

    switch(funct6) {
        case RISC_V_FUNCT_REDSUM:
            if(vm) {
                gen_helper_vredsum_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredsum_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDAND:
            if(vm) {
                gen_helper_vredand_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredand_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDOR:
            if(vm) {
                gen_helper_vredor_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredor_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDXOR:
            if(vm) {
                gen_helper_vredxor_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredxor_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDMINU:
            if(vm) {
                gen_helper_vredminu_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredminu_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDMIN:
            if(vm) {
                gen_helper_vredmin_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredmin_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDMAXU:
            if(vm) {
                gen_helper_vredmaxu_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredmaxu_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REDMAX:
            if(vm) {
                gen_helper_vredmax_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vredmax_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_AADDU:
            if(vm) {
                gen_helper_vaaddu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vaaddu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_AADD:
            if(vm) {
                gen_helper_vaadd_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vaadd_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_ASUBU:
            if(vm) {
                gen_helper_vasubu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vasubu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_ASUB:
            if(vm) {
                gen_helper_vasub_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vasub_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WXUNARY0:
            switch(vs1) {
                case 0x0:
                    if(vm) {
                        gen_helper_vmv_xs(t_tl, cpu_env, t_vs2);
                        gen_set_gpr(vd, t_tl);
                    } else {
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    }
                    break;
                case 0x10:
                    if(vm) {
                        gen_helper_vpopc(t_tl, cpu_env, t_vs2);
                    } else {
                        gen_helper_vpopc_m(t_tl, cpu_env, t_vs2);
                    }
                    gen_set_gpr(vd, t_tl);
                    break;
                case 0x11:
                    if(vm) {
                        gen_helper_vfirst(t_tl, cpu_env, t_vs2);
                    } else {
                        gen_helper_vfirst_m(t_tl, cpu_env, t_vs2);
                    }
                    gen_set_gpr(vd, t_tl);
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case RISC_V_FUNCT_XUNARY0:
            switch(vs1) {
                case 2:
                    if(vm) {
                        gen_helper_vzext_vf8(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vzext_vf8_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 3:
                    if(vm) {
                        gen_helper_vsext_vf8(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vsext_vf8_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 4:
                    if(vm) {
                        gen_helper_vzext_vf4(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vzext_vf4_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 5:
                    if(vm) {
                        gen_helper_vsext_vf4(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vsext_vf4_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 6:
                    if(vm) {
                        gen_helper_vzext_vf2(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vzext_vf2_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 7:
                    if(vm) {
                        gen_helper_vsext_vf2(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vsext_vf2_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case RISC_V_FUNCT_MUNARY0:
            switch(vs1) {
                case 0x1:
                    if(vm) {
                        gen_helper_vmsbf(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vmsbf_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x2:
                    if(vm) {
                        gen_helper_vmsof(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vmsof_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x3:
                    if(vm) {
                        gen_helper_vmsif(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vmsif_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x10:
                    if(vm) {
                        gen_helper_viota(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_viota_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x11:
                    if(vs2) {
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    } else if(vm) {
                        gen_helper_vid(cpu_env, t_vd);
                    } else {
                        gen_helper_vid_m(cpu_env, t_vd);
                    }
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case RISC_V_FUNCT_COMPRESS:
            if(vm) {
                gen_helper_vcompress_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MANDNOT:
            if(vm) {
                gen_helper_vmandnot_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MAND:
            if(vm) {
                gen_helper_vmand_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MOR:
            if(vm) {
                gen_helper_vmor_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MXOR:
            if(vm) {
                gen_helper_vmxor_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MORNOT:
            if(vm) {
                gen_helper_vmornot_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MNAND:
            if(vm) {
                gen_helper_vmnand_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MNOR:
            if(vm) {
                gen_helper_vmnor_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_MXNOR:
            if(vm) {
                gen_helper_vmxnor_mm(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_DIVU:
            if(vm) {
                gen_helper_vdivu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vdivu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_DIV:
            if(vm) {
                gen_helper_vdiv_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vdiv_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REMU:
            if(vm) {
                gen_helper_vremu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vremu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_REM:
            if(vm) {
                gen_helper_vrem_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vrem_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MULHU:
            gen_helper_check_is_vmulh_valid(cpu_env);
            if(vm) {
                gen_helper_vmulhu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmulhu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MUL:
            if(vm) {
                gen_helper_vmul_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmul_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MULHSU:
            gen_helper_check_is_vmulh_valid(cpu_env);
            if(vm) {
                gen_helper_vmulhsu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmulhsu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MULH:
            gen_helper_check_is_vmulh_valid(cpu_env);
            if(vm) {
                gen_helper_vmulh_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmulh_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MADD:
            if(vm) {
                gen_helper_vmadd_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmadd_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_NMSUB:
            if(vm) {
                gen_helper_vnmsub_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vnmsub_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MACC:
            if(vm) {
                gen_helper_vmacc_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vmacc_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_NMSAC:
            if(vm) {
                gen_helper_vnmsac_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vnmsac_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WADDU:
            if(vm) {
                gen_helper_vwaddu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwaddu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WADD:
            if(vm) {
                gen_helper_vwadd_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwadd_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WSUBU:
            if(vm) {
                gen_helper_vwsubu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwsubu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WSUB:
            if(vm) {
                gen_helper_vwsub_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwsub_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WADDUW:
            if(vm) {
                gen_helper_vwaddu_mwv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwaddu_mwv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WADDW:
            if(vm) {
                gen_helper_vwadd_mwv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwadd_mwv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WSUBUW:
            if(vm) {
                gen_helper_vwsubu_mwv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwsubu_mwv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WSUBW:
            if(vm) {
                gen_helper_vwsub_mwv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwsub_mwv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WMULU:
            if(vm) {
                gen_helper_vwmulu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwmulu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WMULSU:
            if(vm) {
                gen_helper_vwmulsu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwmulsu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WMUL:
            if(vm) {
                gen_helper_vwmul_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwmul_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WMACCU:
            if(vm) {
                gen_helper_vwmaccu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwmaccu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WMACC:
            if(vm) {
                gen_helper_vwmacc_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwmacc_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WMACCSU:
            if(vm) {
                gen_helper_vwmaccsu_mvv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vwmaccsu_mvv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free(t_tl);
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_vs1);
    tcg_temp_free_i32(t_vs2);
}

static void gen_v_opmvx(DisasContext *dc, uint8_t funct6, int vd, int rs1, int vs2, uint8_t vm)
{
    generate_vill_check(dc);
    TCGv_i32 t_vd, t_vs2;
    TCGv t_tl;
    t_vd = tcg_temp_new_i32();
    t_vs2 = tcg_temp_new_i32();
    t_tl = tcg_temp_new();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_vs2, vs2);
    gen_get_gpr(t_tl, rs1);

    switch(funct6) {
        case RISC_V_FUNCT_AADDU:
            if(vm) {
                gen_helper_vaaddu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vaaddu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_AADD:
            if(vm) {
                gen_helper_vaadd_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vaadd_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_ASUBU:
            if(vm) {
                gen_helper_vasubu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vasubu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_ASUB:
            if(vm) {
                gen_helper_vasub_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vasub_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_SLIDE1UP:
            if(vm) {
                gen_helper_vslide1up(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vslide1up_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_SLIDE1DOWN:
            if(vm) {
                gen_helper_vslide1down(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vslide1down_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_RXUNARY0:
            if(vs2 == 0x0 && vm) {
                gen_helper_vmv_sx(cpu_env, t_vd, t_tl);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_DIVU:
            if(vm) {
                gen_helper_vdivu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vdivu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_DIV:
            if(vm) {
                gen_helper_vdiv_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vdiv_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_REMU:
            if(vm) {
                gen_helper_vremu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vremu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_REM:
            if(vm) {
                gen_helper_vrem_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vrem_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_MULHU:
            gen_helper_check_is_vmulh_valid(cpu_env);
            if(vm) {
                gen_helper_vmulhu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vmulhu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_MUL:
            if(vm) {
                gen_helper_vmul_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vmul_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_MULHSU:
            gen_helper_check_is_vmulh_valid(cpu_env);
            if(vm) {
                gen_helper_vmulhsu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vmulhsu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_MULH:
            gen_helper_check_is_vmulh_valid(cpu_env);
            if(vm) {
                gen_helper_vmulh_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vmulh_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_MADD:
            if(vm) {
                gen_helper_vmadd_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vmadd_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_NMSUB:
            if(vm) {
                gen_helper_vnmsub_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vnmsub_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_MACC:
            if(vm) {
                gen_helper_vmacc_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vmacc_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_NMSAC:
            if(vm) {
                gen_helper_vnmsac_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vnmsac_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WADDU:
            if(vm) {
                gen_helper_vwaddu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwaddu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WADD:
            if(vm) {
                gen_helper_vwadd_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwadd_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WSUBU:
            if(vm) {
                gen_helper_vwsubu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwsubu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WSUB:
            if(vm) {
                gen_helper_vwsub_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwsub_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WADDUW:
            if(vm) {
                gen_helper_vwaddu_mwx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwaddu_mwx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WADDW:
            if(vm) {
                gen_helper_vwadd_mwx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwadd_mwx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WSUBUW:
            if(vm) {
                gen_helper_vwsubu_mwx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwsubu_mwx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WSUBW:
            if(vm) {
                gen_helper_vwsub_mwx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwsub_mwx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMULU:
            if(vm) {
                gen_helper_vwmulu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmulu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMULSU:
            if(vm) {
                gen_helper_vwmulsu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmulsu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMUL:
            if(vm) {
                gen_helper_vwmul_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmul_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMACCU:
            if(vm) {
                gen_helper_vwmaccu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmaccu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMACC:
            if(vm) {
                gen_helper_vwmacc_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmacc_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMACCUS:
            if(vm) {
                gen_helper_vwmaccus_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmaccus_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        case RISC_V_FUNCT_WMACCSU:
            if(vm) {
                gen_helper_vwmaccsu_mvx(cpu_env, t_vd, t_vs2, t_tl);
            } else {
                gen_helper_vwmaccsu_mvx_m(cpu_env, t_vd, t_vs2, t_tl);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free(t_tl);
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_vs2);
}

static void gen_v_opfvv(DisasContext *dc, uint8_t funct6, int vd, int vs1, int vs2, uint8_t vm)
{
    generate_vill_check(dc);
    TCGv_i32 t_vd, t_vs2, t_vs1;
    t_vd = tcg_temp_new_i32();
    t_vs2 = tcg_temp_new_i32();
    t_vs1 = tcg_temp_new_i32();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_vs2, vs2);
    tcg_gen_movi_i32(t_vs1, vs1);

    switch(funct6) {
        case RISC_V_FUNCT_FADD:
            if(vm) {
                gen_helper_vfadd_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfadd_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FREDSUM:
            if(vm) {
                gen_helper_vfredusum_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfredusum_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FSUB:
            if(vm) {
                gen_helper_vfsub_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfsub_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FREDOSUM:
            if(vm) {
                gen_helper_vfredosum_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfredosum_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMIN:
            if(vm) {
                gen_helper_vfmin_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmin_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FREDMIN:
            if(vm) {
                gen_helper_vfredmin_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfredmin_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMAX:
            if(vm) {
                gen_helper_vfmax_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmax_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FREDMAX:
            if(vm) {
                gen_helper_vfredmax_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfredmax_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FSGNJ:
            if(vm) {
                gen_helper_vfsgnj_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfsgnj_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FSGNJN:
            if(vm) {
                gen_helper_vfsgnjn_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfsgnjn_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FSGNJX:
            if(vm) {
                gen_helper_vfsgnjx_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfsgnjx_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_WFUNARY0:
            if(vm && vs1 == 0) {
                gen_helper_vfmv_fs(cpu_env, t_vd, t_vs2);
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case RISC_V_FUNCT_FUNARY0:
            switch(vs1) {
                case 0x0:
                    if(vm) {
                        gen_helper_vfcvt_xuf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfcvt_xuf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x1:
                    if(vm) {
                        gen_helper_vfcvt_xf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfcvt_xf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x2:
                    if(vm) {
                        gen_helper_vfcvt_fxu_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfcvt_fxu_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x3:
                    if(vm) {
                        gen_helper_vfcvt_fx_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfcvt_fx_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x6:
                    if(vm) {
                        gen_helper_vfcvt_rtz_xuf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfcvt_rtz_xuf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x7:
                    if(vm) {
                        gen_helper_vfcvt_rtz_xf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfcvt_rtz_xf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x8:
                    if(vm) {
                        gen_helper_vfwcvt_xuf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_xuf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x9:
                    if(vm) {
                        gen_helper_vfwcvt_xf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_xf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0xa:
                    if(vm) {
                        gen_helper_vfwcvt_fxu_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_fxu_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0xb:
                    if(vm) {
                        gen_helper_vfwcvt_fx_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_fx_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0xc:
                    if(vm) {
                        gen_helper_vfwcvt_ff_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_ff_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0xe:
                    if(vm) {
                        gen_helper_vfwcvt_rtz_xuf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_rtz_xuf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0xf:
                    if(vm) {
                        gen_helper_vfwcvt_rtz_xf_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfwcvt_rtz_xf_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x10:
                    if(vm) {
                        gen_helper_vfncvt_xuf_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_xuf_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x11:
                    if(vm) {
                        gen_helper_vfncvt_xf_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_xf_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x12:
                    if(vm) {
                        gen_helper_vfncvt_fxu_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_fxu_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x13:
                    if(vm) {
                        gen_helper_vfncvt_fx_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_fx_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x14:
                    if(vm) {
                        gen_helper_vfncvt_ff_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_ff_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x15:
                    if(vm) {
                        gen_helper_vfncvt_rod_ff_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_rod_ff_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x16:
                    if(vm) {
                        gen_helper_vfncvt_rtz_xuf_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_rtz_xuf_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x17:
                    if(vm) {
                        gen_helper_vfncvt_rtz_xf_w(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfncvt_rtz_xf_w_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case RISC_V_FUNCT_FUNARY1:
            switch(vs1) {
                case 0x0:
                    if(vm) {
                        gen_helper_vfsqrt_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfsqrt_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x4:
                    if(vm) {
                        gen_helper_vfrsqrt7_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfrsqrt7_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x5:
                    if(vm) {
                        gen_helper_vfrec7_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfrec7_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                case 0x10:
                    if(vm) {
                        gen_helper_vfclass_v(cpu_env, t_vd, t_vs2);
                    } else {
                        gen_helper_vfclass_v_m(cpu_env, t_vd, t_vs2);
                    }
                    break;
                default:
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
            break;
        case RISC_V_FUNCT_MFEQ:
            if(vm) {
                gen_helper_vfeq_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfeq_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MFLE:
            if(vm) {
                gen_helper_vfle_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfle_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MFLT:
            if(vm) {
                gen_helper_vflt_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vflt_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_MFNE:
            if(vm) {
                gen_helper_vfne_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfne_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FDIV:
            if(vm) {
                gen_helper_vfdiv_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfdiv_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMUL:
            if(vm) {
                gen_helper_vfmul_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmul_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMADD:
            if(vm) {
                gen_helper_vfmadd_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmadd_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FNMADD:
            if(vm) {
                gen_helper_vfnmadd_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfnmadd_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMSUB:
            if(vm) {
                gen_helper_vfmsub_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmsub_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FNMSUB:
            if(vm) {
                gen_helper_vfnmsub_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfnmsub_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMACC:
            if(vm) {
                gen_helper_vfmacc_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmacc_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FNMACC:
            if(vm) {
                gen_helper_vfnmacc_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfnmacc_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FMSAC:
            if(vm) {
                gen_helper_vfmsac_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfmsac_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FNMSAC:
            if(vm) {
                gen_helper_vfnmsac_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfnmsac_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWADD:
            if(vm) {
                gen_helper_vfwadd_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwadd_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWREDSUM:
            if(vm) {
                gen_helper_vfwredusum_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwredusum_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWSUB:
            if(vm) {
                gen_helper_vfwsub_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwsub_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWREDOSUM:
            if(vm) {
                gen_helper_vfwredosum_vs(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwredosum_vs_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWADDW:
            if(vm) {
                gen_helper_vfwadd_wv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwadd_wv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWSUBW:
            if(vm) {
                gen_helper_vfwsub_wv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwsub_wv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWMUL:
            if(vm) {
                gen_helper_vfwmul_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwmul_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWMACC:
            if(vm) {
                gen_helper_vfwmacc_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwmacc_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWNMACC:
            if(vm) {
                gen_helper_vfwnmacc_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwnmacc_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWMSAC:
            if(vm) {
                gen_helper_vfwmsac_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwmsac_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        case RISC_V_FUNCT_FWNMSAC:
            if(vm) {
                gen_helper_vfwnmsac_vv(cpu_env, t_vd, t_vs2, t_vs1);
            } else {
                gen_helper_vfwnmsac_vv_m(cpu_env, t_vd, t_vs2, t_vs1);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_vs2);
    tcg_temp_free_i32(t_vs1);
}

static void gen_v_opfvf(DisasContext *dc, uint8_t funct6, int vd, int rs1, int vs2, uint8_t vm)
{
    generate_vill_check(dc);
    TCGv t_vd, t_vs2;
    t_vd = tcg_temp_new_i32();
    t_vs2 = tcg_temp_new_i32();
    tcg_gen_movi_i32(t_vd, vd);
    tcg_gen_movi_i32(t_vs2, vs2);

    switch(funct6) {
        case RISC_V_FUNCT_FADD:
            if(vm) {
                gen_helper_vfadd_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfadd_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FSUB:
            if(vm) {
                gen_helper_vfsub_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfsub_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMIN:
            if(vm) {
                gen_helper_vfmin_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmin_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMAX:
            if(vm) {
                gen_helper_vfmax_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmax_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FSGNJ:
            if(vm) {
                gen_helper_vfsgnj_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfsgnj_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FSGNJN:
            if(vm) {
                gen_helper_vfsgnjn_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfsgnjn_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FSGNJX:
            if(vm) {
                gen_helper_vfsgnjx_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfsgnjx_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FSLIDE1UP:
            if(vm) {
                gen_helper_vfslide1up(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfslide1up_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FSLIDE1DOWN:
            if(vm) {
                gen_helper_vfslide1down(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfslide1down_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_RFUNARY0:
            if(vm && vs2 == 0) {
                gen_get_fpr(t_vs2, vs2);
                gen_helper_vfmv_sf(cpu_env, t_vd, cpu_fpr[rs1]);
                break;
            }
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
        case RISC_V_FUNCT_FMERGE_FMV:
            if(vm) {
                if(vs2) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                } else {
                    gen_helper_vfmv_vf(cpu_env, t_vd, cpu_fpr[rs1]);
                }
            } else {
                gen_helper_vfmerge_vfm(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_MFEQ:
            if(vm) {
                gen_helper_vfeq_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfeq_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_MFLE:
            if(vm) {
                gen_helper_vfle_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfle_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_MFLT:
            if(vm) {
                gen_helper_vflt_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vflt_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_MFNE:
            if(vm) {
                gen_helper_vfne_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfne_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_MFGT:
            if(vm) {
                gen_helper_vfgt_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfgt_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_MFGE:
            if(vm) {
                gen_helper_vfge_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfge_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FDIV:
            if(vm) {
                gen_helper_vfdiv_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfdiv_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FRDIV:
            if(vm) {
                gen_helper_vfrdiv_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfrdiv_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMUL:
            if(vm) {
                gen_helper_vfmul_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmul_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FRSUB:
            if(vm) {
                gen_helper_vfrsub_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfrsub_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMADD:
            if(vm) {
                gen_helper_vfmadd_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmadd_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FNMADD:
            if(vm) {
                gen_helper_vfnmadd_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfnmadd_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMSUB:
            if(vm) {
                gen_helper_vfmsub_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmsub_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FNMSUB:
            if(vm) {
                gen_helper_vfnmsub_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfnmsub_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMACC:
            if(vm) {
                gen_helper_vfmacc_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmacc_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FNMACC:
            if(vm) {
                gen_helper_vfnmacc_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfnmacc_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FMSAC:
            if(vm) {
                gen_helper_vfmsac_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfmsac_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FNMSAC:
            if(vm) {
                gen_helper_vfnmsac_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfnmsac_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWADD:
            if(vm) {
                gen_helper_vfwadd_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwadd_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWSUB:
            if(vm) {
                gen_helper_vfwsub_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwsub_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWADDW:
            if(vm) {
                gen_helper_vfwadd_wf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwadd_wf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWSUBW:
            if(vm) {
                gen_helper_vfwsub_wf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwsub_wf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWMUL:
            if(vm) {
                gen_helper_vfwmul_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwmul_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWMACC:
            if(vm) {
                gen_helper_vfwmacc_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwmacc_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWNMACC:
            if(vm) {
                gen_helper_vfwnmacc_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwnmacc_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWMSAC:
            if(vm) {
                gen_helper_vfwmsac_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwmsac_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        case RISC_V_FUNCT_FWNMSAC:
            if(vm) {
                gen_helper_vfwnmsac_vf(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            } else {
                gen_helper_vfwnmsac_vf_m(cpu_env, t_vd, t_vs2, cpu_fpr[rs1]);
            }
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_temp_free_i32(t_vd);
    tcg_temp_free_i32(t_vs2);
}
#endif  //  HOST_LONG_BITS != 32

static void gen_v(DisasContext *dc, uint32_t opc, int rd, int rs1, int rs2, int imm)
{
//  Vector helpers require 128-bit ints which aren't supported on 32-bit hosts.
#if HOST_LONG_BITS == 32
    tlib_abort("Vector extension isn't available on 32-bit hosts.");
#else
    uint8_t funct6 = extract32(dc->opcode, 26, 6);
    uint8_t vm = extract32(dc->opcode, 25, 1);

    switch(opc) {
        case OPC_RISC_V_IVV:
            gen_v_opivv(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_FVV:
            gen_v_opfvv(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_MVV:
            gen_v_opmvv(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_IVI:
            gen_v_opivi(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_IVX:
            gen_v_opivx(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_FVF:
            gen_v_opfvf(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_MVX:
            gen_v_opmvx(dc, funct6, rd, rs1, rs2, vm);
            break;
        case OPC_RISC_V_CFG:
            gen_v_cfg(dc, MASK_OP_V_CFG(dc->opcode), rd, rs1, rs2, imm);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
    tcg_gen_movi_tl(cpu_vstart, 0);
#endif  //  HOST_LONG_BITS != 32
}

static void decode_RV32_64C0(DisasContext *dc)
{
    uint8_t funct3 = extract32(dc->opcode, 13, 3);
    uint8_t rd_rs2 = GET_C_RS2S(dc->opcode);
    uint8_t rs1s = GET_C_RS1S(dc->opcode);

    switch(funct3) {
        case 0:
            /* illegal */
            if(dc->opcode == 0) {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                /* C.ADDI4SPN -> addi rd', x2, zimm[9:2]*/
                target_ulong imm = GET_C_ADDI4SPN_IMM(dc->opcode);
                if(imm == 0) {
                    /* C.ADDI4SPN with nzuimm == 0 is reserved. */
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                } else {
                    gen_arith_imm(dc, OPC_RISC_ADDI, rd_rs2, 2, imm);
                }
            }
            break;
        case 1:
            /* C.FLD -> fld rd', offset[7:3](rs1')*/
            gen_fp_load(dc, OPC_RISC_FLD, rd_rs2, rs1s, GET_C_LD_IMM(dc->opcode));
            /* C.LQ(RV128) */
            break;
        case 2:
            /* C.LW -> lw rd', offset[6:2](rs1') */
            gen_load(dc, OPC_RISC_LW, rd_rs2, rs1s, GET_C_LW_IMM(dc->opcode));
            break;
        case 3:
#if defined(TARGET_RISCV64)
            /* C.LD(RV64/128) -> ld rd', offset[7:3](rs1')*/
            gen_load(dc, OPC_RISC_LD, rd_rs2, rs1s, GET_C_LD_IMM(dc->opcode));
#else
            /* C.FLW (RV32) -> flw rd', offset[6:2](rs1')*/
            gen_fp_load(dc, OPC_RISC_FLW, rd_rs2, rs1s, GET_C_LW_IMM(dc->opcode));
#endif
            break;
        case 4:
            /* Zcb extension instructions */
            if(!ensure_additional_extension(dc, RISCV_FEATURE_ZCB)) {
                break;
            }
            uint8_t funct5_high = extract32(dc->opcode, 10, 2);
            if(funct5_high == 0) {
                /* C.LBU -> lbu rd', uimm[1:0](rs1') */
                gen_load(dc, OPC_RISC_LBU, rd_rs2, rs1s, GET_C_LBU_IMM(dc->opcode));
            } else if(funct5_high == 1) {
                /* Differentiate between C.LH and C.LHU based on bit 6 */
                if(extract32(dc->opcode, 6, 1) == 0) {
                    /* C.LHU -> lhu rd', uimm[1](rs1') */
                    gen_load(dc, OPC_RISC_LHU, rd_rs2, rs1s, GET_C_LHU_IMM(dc->opcode));
                } else {
                    /* C.LH -> lh rd', uimm[1](rs1') */
                    gen_load(dc, OPC_RISC_LH, rd_rs2, rs1s, GET_C_LH_IMM(dc->opcode));
                }
            } else if(funct5_high == 2) {
                /* C.SB -> sb rs2', uimm[1:0](rs1') */
                gen_store(dc, OPC_RISC_SB, rs1s, rd_rs2, GET_C_SB_IMM(dc->opcode));
            } else if(funct5_high == 3) {
                /* C.SH -> sh rs2', uimm[1](rs1') */
                gen_store(dc, OPC_RISC_SH, rs1s, rd_rs2, GET_C_SH_IMM(dc->opcode));
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        case 5:
            /* C.FSD(RV32/64) -> fsd rs2', offset[7:3](rs1') */
            gen_fp_store(dc, OPC_RISC_FSD, rs1s, rd_rs2, GET_C_LD_IMM(dc->opcode));
            /* C.SQ (RV128) */
            break;
        case 6:
            /* C.SW -> sw rs2', offset[6:2](rs1')*/
            gen_store(dc, OPC_RISC_SW, rs1s, rd_rs2, GET_C_LW_IMM(dc->opcode));
            break;
        case 7:
#if defined(TARGET_RISCV64)
            /* C.SD (RV64/128) -> sd rs2', offset[7:3](rs1')*/
            gen_store(dc, OPC_RISC_SD, rs1s, rd_rs2, GET_C_LD_IMM(dc->opcode));
#else
            /* C.FSW (RV32) -> fsw rs2', offset[6:2](rs1')*/
            gen_fp_store(dc, OPC_RISC_FSW, rs1s, rd_rs2, GET_C_LW_IMM(dc->opcode));
#endif
            break;
    }
}

static void decode_RV32_64C1(CPUState *env, DisasContext *dc)
{
    uint8_t funct3 = extract32(dc->opcode, 13, 3);
    uint8_t rd_rs1 = GET_C_RS1(dc->opcode);
    uint8_t rs1s, rs2s;
    uint8_t funct2;
    target_long imm;

    switch(funct3) {
        case 0:
            /* C.ADDI -> addi rd, rd, nzimm[5:0] */
            gen_arith_imm(dc, OPC_RISC_ADDI, rd_rs1, rd_rs1, GET_C_IMM(dc->opcode));
            break;
        case 1:
#if defined(TARGET_RISCV64)
            /* C.ADDIW (RV64/128) -> addiw rd, rd, imm[5:0]*/
            if(!rd_rs1) { /* ISA V20191213: Reserved when rd == 0 */
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                gen_arith_imm(dc, OPC_RISC_ADDIW, rd_rs1, rd_rs1, GET_C_IMM(dc->opcode));
            }
#else
            /* C.JAL(RV32) -> jal x1, offset[11:1] */
            gen_jal(env, dc, 1, GET_C_J_IMM(dc->opcode));
#endif
            break;
        case 2:
            /* C.LI -> addi rd, x0, imm[5:0]*/
            gen_arith_imm(dc, OPC_RISC_ADDI, rd_rs1, 0, GET_C_IMM(dc->opcode));
            break;
        case 3:
            if(rd_rs1 == 2) {
                imm = GET_C_ADDI16SP_IMM(dc->opcode);
                /* C.ADDI16SP -> addi x2, x2, nzimm[9:4]*/
                if(!imm) { /* ISA V20191213: Reserved when nzimm == 0 */
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                } else {
                    gen_arith_imm(dc, OPC_RISC_ADDI, 2, 2, imm);
                }
            } else if(rd_rs1 != 0) {
                imm = GET_C_IMM(dc->opcode);
                /* C.LUI (rs1/rd =/= {0,2}) -> lui rd, nzimm[17:12]*/
                if(!imm) { /* ISA V20191213: Reserved when nzimm == 0 */
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                } else {
                    get_set_gpr_imm(rd_rs1, imm << 12);
                }
            }
            break;
        case 4:
            funct2 = extract32(dc->opcode, 10, 2);
            rs1s = GET_C_RS1S(dc->opcode);
            switch(funct2) {
                case 0: /* C.SRLI(RV32) -> srli rd', rd', shamt[5:0] */
                    gen_arith_imm(dc, OPC_RISC_SHIFT_RIGHT_I, rs1s, rs1s, GET_C_ZIMM(dc->opcode));
                    /* C.SRLI64(RV128) */
                    break;
                case 1:
                    /* C.SRAI -> srai rd', rd', shamt[5:0]*/
                    gen_arith_imm(dc, OPC_RISC_SHIFT_RIGHT_I, rs1s, rs1s, GET_C_ZIMM(dc->opcode) | 0x400);
                    /* C.SRAI64(RV128) */
                    break;
                case 2:
                    /* C.ANDI -> andi rd', rd', imm[5:0]*/
                    gen_arith_imm(dc, OPC_RISC_ANDI, rs1s, rs1s, GET_C_IMM(dc->opcode));
                    break;
                case 3:
                    funct2 = extract32(dc->opcode, 5, 2);
                    rs2s = GET_C_RS2S(dc->opcode);
                    //  Bit 12 = 0: C.AND, C.OR, C.XOR, C.SUB
                    if(extract32(dc->opcode, 12, 1) == 0) {
                        switch(funct2) {
                            case 0:
                                /* C.SUB -> sub rd', rd', rs2' */
                                gen_arith(dc, OPC_RISC_SUB, rs1s, rs1s, rs2s);
                                break;
                            case 1:
                                /* C.XOR -> xor rs1', rs1', rs2' */
                                gen_arith(dc, OPC_RISC_XOR, rs1s, rs1s, rs2s);
                                break;
                            case 2:
                                /* C.OR -> or rs1', rs1', rs2' */
                                gen_arith(dc, OPC_RISC_OR, rs1s, rs1s, rs2s);
                                break;
                            case 3:
                                /* C.AND -> and rs1', rs1', rs2' */
                                gen_arith(dc, OPC_RISC_AND, rs1s, rs1s, rs2s);
                                break;
                            default:
                                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                                break;
                        }
                    } else {  //  Bit 12 = 1: C.SUBW, C.ADDW and Zcb instructions
                        switch(funct2) {
#if defined(TARGET_RISCV64)
                            case 0:
                                /* C.SUBW (RV64/128) */
                                gen_arith(dc, OPC_RISC_SUBW, rs1s, rs1s, rs2s);
                                break;
                            case 1:
                                /* C.ADDW (RV64/128) */
                                gen_arith(dc, OPC_RISC_ADDW, rs1s, rs1s, rs2s);
                                break;
#endif
                            case 2:  //  C.MUL
                                if(!ensure_additional_extension(dc, RISCV_FEATURE_ZCB)) {
                                    break;
                                }
                                /* C.MUL -> mul rd', rd', rs2' */
                                gen_arith(dc, OPC_RISC_MUL, rs1s, rs1s, rs2s);
                                break;
                            case 3:
                                if(!ensure_additional_extension(dc, RISCV_FEATURE_ZCB)) {
                                    break;
                                }
                                /* C.ZEXT.B, C.SEXT.B, C.ZEXT.H, C.SEXT.H, C.ZEXT.W, C.NOT */
                                uint8_t c_op = extract32(dc->opcode, 2, 3);

                                switch(c_op) {
                                    case 0:
                                        /* C.ZEXT.B -> andi rd', rd', 255 */
                                        gen_arith_imm(dc, OPC_RISC_ANDI, rs1s, rs1s, 0xFF);
                                        break;
                                    case 1:
                                        /* C.SEXT.B -> sign extend byte (Zcb) */
                                        {
                                            TCGv t0 = tcg_temp_new();
                                            gen_get_gpr(t0, rs1s);
                                            tcg_gen_ext8s_tl(t0, t0);
                                            gen_set_gpr(rs1s, t0);
                                            tcg_temp_free(t0);
                                        }
                                        break;
                                    case 2:
                                        /* C.ZEXT.H -> zero extend halfword (Zcb) */
                                        {
                                            TCGv t0 = tcg_temp_new();
                                            gen_get_gpr(t0, rs1s);
                                            tcg_gen_ext16u_tl(t0, t0);
                                            gen_set_gpr(rs1s, t0);
                                            tcg_temp_free(t0);
                                        }
                                        break;
                                    case 3:
                                        /* C.SEXT.H -> sign extend halfword (Zcb) */
                                        {
                                            TCGv t0 = tcg_temp_new();
                                            gen_get_gpr(t0, rs1s);
                                            tcg_gen_ext16s_tl(t0, t0);
                                            gen_set_gpr(rs1s, t0);
                                            tcg_temp_free(t0);
                                        }
                                        break;
#if defined(TARGET_RISCV64)
                                    case 4:
                                        /* C.ZEXT.W -> add.uw rd', rd', x0 (RV64 only) */
                                        gen_arith(dc, OPC_RISC_ADD_UW, rs1s, rs1s, 0);
                                        break;
#endif
                                    case 5:
                                        /* C.NOT -> xori rd', rd', -1 */
                                        gen_arith_imm(dc, OPC_RISC_XORI, rs1s, rs1s, -1);
                                        break;
                                    default:
                                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                                        break;
                                }
                                break;
                            default:
                                break;
                        }
                    }
                    break;
            }
            break;
        case 5:
            /* C.J -> jal x0, offset[11:1]*/
            gen_jal(env, dc, 0, GET_C_J_IMM(dc->opcode));
            break;
        case 6:
            /* C.BEQZ -> beq rs1', x0, offset[8:1]*/
            rs1s = GET_C_RS1S(dc->opcode);
            gen_branch(env, dc, OPC_RISC_BEQ, rs1s, 0, GET_C_B_IMM(dc->opcode));
            break;
        case 7:
            /* C.BNEZ -> bne rs1', x0, offset[8:1]*/
            rs1s = GET_C_RS1S(dc->opcode);
            gen_branch(env, dc, OPC_RISC_BNE, rs1s, 0, GET_C_B_IMM(dc->opcode));
            break;
    }
}

static void decode_RV32_64C2(CPUState *env, DisasContext *dc)
{
    uint8_t rd, rs2;
    uint8_t funct3 = extract32(dc->opcode, 13, 3);

    rd = GET_RD(dc->opcode);

    switch(funct3) {
        case 0: /* C.SLLI -> slli rd, rd, shamt[5:0]
                   C.SLLI64 -> */
            gen_arith_imm(dc, OPC_RISC_SLLI, rd, rd, GET_C_ZIMM(dc->opcode));
            break;
        case 1: /* C.FLDSP(RV32/64DC) -> fld rd, offset[8:3](x2) */
            gen_fp_load(dc, OPC_RISC_FLD, rd, 2, GET_C_LDSP_IMM(dc->opcode));
            break;
        case 2:       /* C.LWSP -> lw rd, offset[7:2](x2) */
            if(!rd) { /* ISA V20191213: Reserved when rd == 0 */
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                gen_load(dc, OPC_RISC_LW, rd, 2, GET_C_LWSP_IMM(dc->opcode));
            }
            break;
        case 3:
#if defined(TARGET_RISCV64)
            /* C.LDSP(RVC64) -> ld rd, offset[8:3](x2) */
            if(!rd) { /* ISA V20191213: Reserved when rd == 0 */
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                break;
            }
            gen_load(dc, OPC_RISC_LD, rd, 2, GET_C_LDSP_IMM(dc->opcode));
#else
            /* C.FLWSP(RV32FC) -> flw rd, offset[7:2](x2) */
            gen_fp_load(dc, OPC_RISC_FLW, rd, 2, GET_C_LWSP_IMM(dc->opcode));
#endif
            break;
        case 4:
            rs2 = GET_C_RS2(dc->opcode);

            if(extract32(dc->opcode, 12, 1) == 0) {
                if(rs2 == 0) {
                    /* C.JR -> jalr x0, rs1, 0*/
                    if(!rd) { /* ISA V20191213: Reserved when rd == 0 */
                        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    } else {
                        gen_jalr(env, dc, OPC_RISC_JALR, 0, rd, 0);
                    }
                } else {
                    /* C.MV -> add rd, x0, rs2 */
                    gen_arith(dc, OPC_RISC_ADD, rd, 0, rs2);
                }
            } else {
                if(rd == 0) {
                    /* C.EBREAK -> ebreak*/
                    gen_system(dc, OPC_RISC_ECALL, 0, 0, 0x1, 0);
                } else {
                    if(rs2 == 0) {
                        /* C.JALR -> jalr x1, rs1, 0*/
                        gen_jalr(env, dc, OPC_RISC_JALR, 1, rd, 0);
                    } else {
                        /* C.ADD -> add rd, rd, rs2 */
                        gen_arith(dc, OPC_RISC_ADD, rd, rd, rs2);
                    }
                }
            }
            break;
        case 5: {
            if(!riscv_has_additional_ext(cpu, RISCV_FEATURE_ZCMT)) {
                /* C.FSDSP -> fsd rs2, offset[8:3](x2)*/
                gen_fp_store(dc, OPC_RISC_FSD, 2, GET_C_RS2(dc->opcode), GET_C_SDSP_IMM(dc->opcode));
                /* C.SQSP */
                break;
            }
            uint8_t funct5 = extract32(dc->opcode, 8, 5);
            uint8_t funct5_high = extract32(dc->opcode, 10, 3);
            uint8_t funct2 = extract32(dc->opcode, 5, 2);

            //  Zcmp C.PUSH, C.POP, C.POPRET and C.POPRETZ
            if(funct5 == 0x18 || funct5 == 0x1A || funct5 == 0x1E || funct5 == 0x1C) {
                uint8_t rlist = GET_C_PUSHPOP_RLIST(dc->opcode);
                target_ulong spimm = GET_C_PUSHPOP_SPIMM(dc->opcode);

                //  rlist values 0 to 3 are reserved for a future EABI variant called cm.push.e
                if(rlist < 4) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                }

                //  Check for RISC-V Zcmp extension instructions
                if(funct5 == 0x18) {  //  C.PUSH - save registers to stack
                    gen_push(dc, rlist, spimm);
                } else if(funct5 == 0x1A) {  //  C.POP - restore registers from stack
                    gen_pop(dc, rlist, spimm);
                } else if(funct5 == 0x1E) {  //  C.POPRET - restore registers and return
                    gen_popret(dc, rlist, spimm);
                } else if(funct5 == 0x1C) {  //  C.POPRETZ - restore registers, set a0=0, and return
                    gen_popretz(dc, rlist, spimm);
                }
            } else if(funct5_high == 0x3  //  Zcmp C.MVSA01/C.MVA01S
                      && (funct2 == 0x1 || funct2 == 0x3)) {
                uint8_t r1sc = GET_C_MVSA01_R1S(dc->opcode);
                uint8_t r2sc = GET_C_MVSA01_R2S(dc->opcode);

                //  Check illegal conditions:
                //  - CM.MVSA01: r1s', r2s' must be different
                //  - RV32E constraint: only s0,s1 exist
                if((funct2 == 0x1 && r1sc == r2sc) || (riscv_has_ext(env, RISCV_FEATURE_RVE) && (r1sc > 1 || r2sc > 1))) {
                    kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
                    break;
                }

                //  Map s-register selectors to architectural x-register numbers
                uint8_t xreg1 = sreg_selector_to_xreg(r1sc);
                uint8_t xreg2 = sreg_selector_to_xreg(r2sc);

                //  Execute moves based on funct2
                if(funct2 == 0x1) {                             /* CM.MVSA01: copy a0 → r1s' and a1 → r2s' */
                    gen_arith(dc, OPC_RISC_ADD, xreg1, 10, 0);  //  mv r1s', a0
                    gen_arith(dc, OPC_RISC_ADD, xreg2, 11, 0);  //  mv r2s', a1
                } else {                                        /* CM.MVA01S: copy r1s' → a0 and r2s' → a1 */
                    gen_arith(dc, OPC_RISC_ADD, 10, xreg1, 0);  //  mv a0, r1s'
                    gen_arith(dc, OPC_RISC_ADD, 11, xreg2, 0);  //  mv a1, r2s'
                }
            } else if(funct5_high == 0x0) {  //  Zcmt: C.JT, C.JALT
                uint8_t index = GET_C_JT_INDEX(dc->opcode);
                //  JT: index<32, JALT: index>=32
                gen_jt(dc, index, (index >= 32));
            } else {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            }
            break;
        }
        case 6: /* C.SWSP -> sw rs2, offset[7:2](x2)*/
            gen_store(dc, OPC_RISC_SW, 2, GET_C_RS2(dc->opcode), GET_C_SWSP_IMM(dc->opcode));
            break;
        case 7:
#if defined(TARGET_RISCV64)
            /* C.SDSP(Rv64/128) -> sd rs2, offset[8:3](x2)*/
            gen_store(dc, OPC_RISC_SD, 2, GET_C_RS2(dc->opcode), GET_C_SDSP_IMM(dc->opcode));
#else
            /* C.FSWSP(RV32) -> fsw rs2, offset[7:2](x2) */
            gen_fp_store(dc, OPC_RISC_FSW, 2, GET_C_RS2(dc->opcode), GET_C_SWSP_IMM(dc->opcode));
#endif
            break;
    }
}

static void decode_RV32_64C(CPUState *env, DisasContext *dc)
{
    uint8_t op = extract32(dc->opcode, 0, 2);

    switch(op) {
        case 0:
            decode_RV32_64C0(dc);
            break;
        case 1:
            decode_RV32_64C1(env, dc);
            break;
        case 2:
            decode_RV32_64C2(env, dc);
            break;
    }
}

static void decode_RV32_64G(CPUState *env, DisasContext *dc)
{
    int rs1;
    int rs2;
    int rd;
    uint32_t rm;
    uint32_t op;
    target_long imm;

    /* We do not do misaligned address check here: the address should never be
     * misaligned at this point. Instructions that set PC must do the check,
     * since epc must be the address of the instruction that caused us to
     * perform the misaligned instruction fetch */

    op = MASK_OP_MAJOR(dc->opcode);
    rs1 = GET_RS1(dc->opcode);
    rs2 = GET_RS2(dc->opcode);
    rd = GET_RD(dc->opcode);
    imm = GET_IMM(dc->opcode);
    rm = GET_RM(dc->opcode);

    switch(op) {
        case OPC_RISC_LUI:
            if(rd == 0) {
                break; /* NOP */
            }
            get_set_gpr_imm(rd, sextract64(dc->opcode, 12, 20) << 12);
            break;
        case OPC_RISC_AUIPC:
            if(rd == 0) {
                break; /* NOP */
            }
            get_set_gpr_imm(rd, (sextract64(dc->opcode, 12, 20) << 12) + dc->base.pc);
            break;
        case OPC_RISC_JAL:
            imm = GET_JAL_IMM(dc->opcode);
            gen_jal(env, dc, rd, imm);
            break;
        case OPC_RISC_JALR:
            gen_jalr(env, dc, MASK_OP_JALR(dc->opcode), rd, rs1, imm);
            break;
        case OPC_RISC_BRANCH:
            gen_branch(env, dc, MASK_OP_BRANCH(dc->opcode), rs1, rs2, GET_B_IMM(dc->opcode));
            break;
        case OPC_RISC_LOAD:
            /* Illegal, RV128I is not supported yet */
            if(MASK_OP_LOAD(dc->opcode) == OPC_RISC_LDU) {
                kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            } else {
                gen_load(dc, MASK_OP_LOAD(dc->opcode), rd, rs1, imm);
            }
            break;
        case OPC_RISC_STORE:
            gen_store(dc, MASK_OP_STORE(dc->opcode), rs1, rs2, GET_STORE_IMM(dc->opcode));
            break;
        case OPC_RISC_ARITH_IMM:
#if defined(TARGET_RISCV64)
        case OPC_RISC_ARITH_IMM_W:
#endif
            if(rd == 0) {
                break; /* NOP */
            }
            gen_arith_imm(dc, MASK_OP_ARITH_IMM(dc->opcode), rd, rs1, imm);
            break;
        case OPC_RISC_ARITH:
#if defined(TARGET_RISCV64)
        case OPC_RISC_ARITH_W:
#endif
            if(rd == 0) {
                break; /* NOP */
            }
            gen_arith(dc, MASK_OP_ARITH(dc->opcode), rd, rs1, rs2);
            break;
        case OPC_RISC_FP_LOAD:
            if(rm - 1 < 4) {
                gen_fp_load(dc, MASK_OP_FP_LOAD(dc->opcode), rd, rs1, imm);
            } else {
                gen_v_load(dc, MASK_OP_V_LOAD(dc->opcode), imm >> 5, rd, rs1, rs2, rm);
            }
            break;
        case OPC_RISC_FP_STORE:
            if(rm - 1 < 4) {
                gen_fp_store(dc, MASK_OP_FP_STORE(dc->opcode), rs1, rs2, GET_STORE_IMM(dc->opcode));
            } else {
                gen_v_store(dc, MASK_OP_V_STORE(dc->opcode), imm >> 5, rd, rs1, rs2, rm);
            }
            break;
        case OPC_RISC_ATOMIC:
            gen_atomic(env, dc, MASK_OP_ATOMIC(dc->opcode), rd, rs1, rs2);
            break;
        case OPC_RISC_FMADD:
            gen_fp_fmadd(dc, MASK_OP_FP_FMADD(dc->opcode), rd, rs1, rs2, GET_RS3(dc->opcode), GET_RM(dc->opcode));
            break;
        case OPC_RISC_FMSUB:
            gen_fp_fmsub(dc, MASK_OP_FP_FMSUB(dc->opcode), rd, rs1, rs2, GET_RS3(dc->opcode), GET_RM(dc->opcode));
            break;
        case OPC_RISC_FNMSUB:
            gen_fp_fnmsub(dc, MASK_OP_FP_FNMSUB(dc->opcode), rd, rs1, rs2, GET_RS3(dc->opcode), GET_RM(dc->opcode));
            break;
        case OPC_RISC_FNMADD:
            gen_fp_fnmadd(dc, MASK_OP_FP_FNMADD(dc->opcode), rd, rs1, rs2, GET_RS3(dc->opcode), GET_RM(dc->opcode));
            break;
        case OPC_RISC_FP_ARITH:
            gen_fp_arith(dc, MASK_OP_FP_ARITH(dc->opcode), rd, rs1, rs2, GET_RM(dc->opcode));
            break;
        case OPC_RISC_SYNCH:
            gen_synch(dc, MASK_OP_FENCE(dc->opcode));
            break;
        case OPC_RISC_SYSTEM:
            gen_system(dc, MASK_OP_SYSTEM(dc->opcode), rd, rs1, rs2, GET_FUNCT12(dc->opcode));
            break;
        case OPC_RISC_V:
            gen_v(dc, MASK_OP_V(dc->opcode), rd, rs1, rs2, imm);
            break;
        default:
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

static void log_unhandled_instruction_length(DisasContext *dc, uint32_t instruction_length)
{
    tlib_printf(LOG_LEVEL_ERROR, "Unsupported instruction length: %d bits. PC: 0x%llx, opcode: 0x%x", 8 * instruction_length,
                dc->base.pc, dc->opcode);
}

static int disas_insn(CPUState *env, DisasContext *dc)
{
    uint16_t first_word_of_opcode = lduw_code(dc->base.pc);

    /* Instructions containg all zeros are illegal in RISC-V.
       We don't need to check the length because the first
       word 0x0 would be identified as 16-bit anyways. */
    if(!first_word_of_opcode) {
        dc->opcode = first_word_of_opcode;
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
        return 0;
    }

    /* Instructions containg all ones are also illegal in RISC-V.
       We don't need to check the length because the first half word
       0xFFFF would be identified as >=192-bit which is not supported. */
    if(first_word_of_opcode == 0xFFFF) {
        dc->opcode = ldl_code(dc->base.pc);
        kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
        return 0;
    }

    int instruction_length = decode_instruction_length(first_word_of_opcode);
    int is_compressed = instruction_length == 2;

    switch(instruction_length) {
        case 2:
            dc->opcode = first_word_of_opcode;
            break;
        case 4:
            dc->opcode = ldl_code(dc->base.pc);
            break;
        case 6:
        case 8:
            dc->opcode = ldq_code(dc->base.pc);
            break;
        default:
            //  Load 32 bits (ILEN) of an instruction for storing in mtval and logging
            dc->opcode = ldl_code(dc->base.pc);
            log_unhandled_instruction_length(dc, instruction_length);
            kill_unknown(dc, RISCV_EXCP_ILLEGAL_INST);
            return 0;
    }

    /* handle custom instructions */
    int i;
    for(i = 0; i < env->custom_instructions_count; i++) {
        custom_instruction_descriptor_t *ci = &env->custom_instructions[i];

        if((dc->opcode & ci->mask) == ci->pattern) {
            dc->npc = dc->base.pc + ci->length;

            if(env->count_opcodes) {
                generate_opcode_count_increment(env, dc->opcode);
            }

            TCGv_i64 id = tcg_const_i64(ci->id);
            TCGv_i64 opcode = tcg_const_i64(dc->opcode & ((1ULL << (8 * ci->length)) - 1));
            TCGv_i32 pc_modified = tcg_temp_new_i32();

            gen_sync_pc(dc);
            gen_helper_handle_custom_instruction(pc_modified, id, opcode);

            int exit_tb_label = gen_new_label();
            tcg_gen_brcondi_i64(TCG_COND_EQ, pc_modified, 1, exit_tb_label);

            //  this is executed conditionally - only if `handle_custom_instruction` returns 0
            //  otherwise `cpu_pc` points to a proper value and should not be overwritten by `dc->base.pc`
            dc->base.pc = dc->npc;
            gen_sync_pc(dc);

            gen_set_label(exit_tb_label);
            gen_exit_tb_no_chaining(dc->base.tb);
            dc->base.is_jmp = DISAS_BRANCH;

            tcg_temp_free_i64(id);
            tcg_temp_free_i64(opcode);
            tcg_temp_free_i64(pc_modified);

            return ci->length;
        }
    }

    if(is_compressed && !ensure_extension(dc, RISCV_FEATURE_RVC)) {
        return 0;
    }

    //  Clear upper bits, leaves only the instruction to be decoded.
    dc->opcode = extract64(dc->opcode, 0, instruction_length * 8);

    /* check for compressed insn */
    dc->npc = dc->base.pc + instruction_length;

    if(env->count_opcodes) {
        generate_opcode_count_increment(env, dc->opcode);
    }

    //  Here opcode already has a valid value and it can be synced together with pc.
    //  Syncing opcode allows to fill mtval with opcode value when it caused exception.
    gen_sync_pc(dc);

    if(unlikely(env->are_pre_opcode_execution_hooks_enabled)) {
        generate_pre_opcode_execution_hook(env, dc->base.pc, dc->opcode);
    }

    if(is_compressed) {
        decode_RV32_64C(env, dc);
    } else {
        decode_RV32_64G(env, dc);
    }

    if(unlikely(env->are_post_opcode_execution_hooks_enabled)) {
        generate_post_opcode_execution_hook(env, dc->base.pc, dc->opcode);
    }

    dc->base.pc = dc->npc;

    if(unlikely(env->guest_profiler_enabled)) {
        int end_label = gen_new_label();
#ifdef TARGET_RISCV64
        tcg_gen_brcond_i64(TCG_COND_EQ, cpu_gpr[SP_64], cpu_prev_sp, end_label);
        gen_helper_announce_stack_pointer_change(cpu_pc, cpu_prev_sp, cpu_gpr[SP_64]);
        tcg_gen_mov_i64(cpu_prev_sp, cpu_gpr[SP_64]);
#else
        tcg_gen_brcond_i32(TCG_COND_EQ, cpu_gpr[SP_32], cpu_prev_sp, end_label);
        gen_helper_announce_stack_pointer_change(cpu_pc, cpu_prev_sp, cpu_gpr[SP_32]);
        tcg_gen_mov_i32(cpu_prev_sp, cpu_gpr[SP_32]);
#endif
        gen_set_label(end_label);
    }

    return instruction_length;
}

void setup_disas_context(DisasContextBase *dc, CPUState *env)
{
    dc->mem_idx = cpu_mmu_index(env);
}

int gen_breakpoint(DisasContextBase *base, CPUBreakpoint *bp)
{
    DisasContext *dc = (DisasContext *)base;
    generate_exception(dc, EXCP_DEBUG);
    /* Advance PC so that clearing the breakpoint will
       invalidate this TB.  */
    dc->base.pc += 4;
    return 1;
}

int gen_intermediate_code(CPUState *env, DisasContextBase *base)
{
    tcg_gen_insn_start(base->pc);

    base->tb->size += disas_insn(env, (DisasContext *)base);

    if((base->pc - (base->tb->pc & TARGET_PAGE_MASK)) >= TARGET_PAGE_SIZE) {
        return 0;
    }

    return 1;
}

uint32_t gen_intermediate_code_epilogue(CPUState *env, DisasContextBase *base)
{
    DisasContext *dc = (DisasContext *)base;
    switch(dc->base.is_jmp) {
        case DISAS_NONE: /* handle end of page - DO NOT CHAIN. See gen_goto_tb. */
            gen_sync_pc(dc);
            gen_exit_tb_no_chaining(dc->base.tb);
            break;
        case DISAS_NEXT:
        case DISAS_STOP:
            gen_goto_tb(dc, 0, dc->base.pc);
            break;
        case DISAS_BRANCH: /* ops using DISAS_BRANCH generate own exit seq */
            break;
    }
    return 0;
}

void restore_state_to_opc(CPUState *env, TranslationBlock *tb, target_ulong *data)
{
    env->pc = data[0];
}

void cpu_set_nmi(CPUState *env, int number, target_ulong mcause)
{
    if(number < 0 || number >= env->nmi_length) {
        tlib_abortf("NMI index %d not valid in cpu with nmi_length = %d", number, env->nmi_length);
    } else {
        env->nmi_pending |= (1 << number);
        env->nmi_mcause[number] = mcause;
        set_interrupt_pending(env, CPU_INTERRUPT_HARD);
    }
}

void cpu_reset_nmi(CPUState *env, int number)
{
    env->nmi_pending &= ~(1 << number);
}

int process_interrupt(int interrupt_request, CPUState *env)
{
    /*According to the debug spec draft, the debug mode implies all interrupts are masked (even NMI)
       / and the WFI acts as NOP. */
    if(tlib_is_in_debug_mode()) {
        return 0;
    }
    if(interrupt_request & (CPU_INTERRUPT_HARD | RISCV_CPU_INTERRUPT_CLIC | RISCV_CPU_INTERRUPT_CUSTOM)) {
        int interruptno = riscv_cpu_hw_interrupts_pending(env);
        if(env->nmi_pending > NMI_NONE) {
            do_interrupt(env);
            return 1;
        } else if(interruptno != EXCP_NONE) {
            env->exception_index = RISCV_EXCP_INT_FLAG | interruptno;
            do_interrupt(env);
            return 1;
        }
    }
    return 0;
}

//  TODO: These empty implementations are required due to problems with weak attribute.
//  Remove this after #7035.
void cpu_exec_epilogue(CPUState *env) { }

void cpu_exec_prologue(CPUState *env) { }
