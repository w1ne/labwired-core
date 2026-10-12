/*
 * Tiny Code Generator for QEMU
 *
 * Copyright (c) 2008 Fabrice Bellard
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
 * THE SOFTWARE.
 */

/* define it to use liveness analysis (better code) */
#define USE_LIVENESS_ANALYSIS
#define USE_TCG_OPTIMIZATIONS

#include "additional.h"
#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <inttypes.h>
#ifdef _WIN32
#include <malloc.h>
#endif

#include "host-utils.h"

#include "tcg-op.h"
#include "../include/tlib-alloc.h"

#define R_386_PC32 2
#define R_386_PC8  23

#if defined(CONFIG_USE_GUEST_BASE) && !defined(TCG_TARGET_HAS_GUEST_BASE)
#error GUEST_BASE not supported on this host.
#endif

/* Forward declarations for functions declared in tcg-target.c and used here. */
static void tcg_target_init(TCGContext *s);
static void tcg_target_qemu_prologue(TCGContext *s);
static void patch_reloc(uint8_t *code_ptr, int type, tcg_target_long value, tcg_target_long addend);

/* Forward declarations for functions declared and used in tcg-target.c. */
static int target_parse_constraint(TCGArgConstraint *ct, const char **pct_str);
static void tcg_out_ld(TCGContext *s, TCGType type, TCGReg ret, TCGReg arg1, tcg_target_long arg2);
static void tcg_out_mov(TCGContext *s, TCGType type, TCGReg ret, TCGReg arg);
static void tcg_out_movi(TCGContext *s, TCGType type, TCGReg ret, tcg_target_long arg);
static void tcg_out_op(TCGContext *s, TCGOpcode opc, const TCGArg *args, const int *const_args);
static void tcg_out_st(TCGContext *s, TCGType type, TCGReg arg, TCGReg arg1, tcg_target_long arg2);
static int tcg_target_const_match(tcg_target_long val, const TCGArgConstraint *arg_ct);
static int tcg_target_get_call_iarg_regs_count(int flags);

TCGOpDef tcg_op_defs[] = {
#define DEF(s, oargs, iargs, cargs, flags) { #s, oargs, iargs, cargs, iargs + oargs + cargs, flags },
#include "tcg-opc.h"
#undef DEF
};
const size_t tcg_op_defs_max = ARRAY_SIZE(tcg_op_defs);

static TCGRegSet tcg_target_available_regs[2];
static TCGRegSet tcg_target_call_clobber_regs;

/* XXX: move that inside the context */
TCGOpcodeEntry *gen_opc_ptr;
TCGArg *gen_opparam_ptr;

static inline void tcg_print_opcode_backtrace(TCGContext *s)
{
#ifdef TCG_OPCODE_BACKTRACE
    TCGOpcodeEntry *entry = s->current_code;
    if(entry == NULL) {
        tlib_printf(LOG_LEVEL_WARNING, "%s: Current opcode not set", __func__);
        return;
    }

    TCGBacktrace backtrace = entry->backtrace;
    if(backtrace.address_count == 0) {
        tlib_printf(LOG_LEVEL_WARNING, "%s: Opcode %x has no backtrace", __func__, entry->opcode);
        return;
    }

    char **backtrace_functions = backtrace_symbols(backtrace.return_addresses, backtrace.address_count);
    if(unlikely(backtrace_functions == NULL)) {
        tlib_printf(LOG_LEVEL_ERROR, "%s: failed to identify symbol names", __func__);
        return;
    }

    tlib_printf(LOG_LEVEL_ERROR, "Failed when processing opcode %x", entry->opcode);
    for(int i = 0; i < backtrace.address_count; i++) {
        tlib_printf(LOG_LEVEL_ERROR, "At %s", backtrace_functions[i]);
    }

    free(backtrace_functions);
#else
    tlib_printf(LOG_LEVEL_WARNING, "TCG opcode backtrace not enabled. Recompile with TCG_OPCODE_BACKTRACE to enable");
#endif
}

static inline void tcg_out8(TCGContext *s, uint8_t v)
{
    *s->code_ptr++ = v;
}

static inline void tcg_out16(TCGContext *s, uint16_t v)
{
    *(uint16_t *)s->code_ptr = v;
    s->code_ptr += 2;
}

static inline void tcg_out32(TCGContext *s, uint32_t v)
{
    *(uint32_t *)s->code_ptr = v;
    s->code_ptr += 4;
}

/* label relocation processing */

static void tcg_out_reloc(TCGContext *s, uint8_t *code_ptr, int type, int label_index, uintptr_t addend)
{
    TCGLabel *l;
    TCGRelocation *r;

    tcg_debug_assert(label_index >= 0);
    tcg_debug_assert(label_index < TCG_MAX_LABELS);
    l = &s->labels[label_index];
    //  tcg_out_reloc should only be called once on each label
    tcg_debug_assert(!l->has_value);
    /* add a new relocation entry */
    r = tcg_malloc(sizeof(TCGRelocation));
    r->type = type;
    r->ptr = code_ptr;
    r->addend = addend;
    r->next = l->u.first_reloc;
    l->u.first_reloc = r;
}

static void tcg_out_label(TCGContext *s, int label_index, tcg_target_long value)
{
    TCGLabel *l;
    TCGRelocation *r;

    tcg_debug_assert(label_index >= 0);
    tcg_debug_assert(label_index < TCG_MAX_LABELS);
    l = &s->labels[label_index];
    if(l->has_value) {
        tcg_abort();
    }
    r = l->u.first_reloc;
    while(r != NULL) {
        patch_reloc(r->ptr, r->type, value, r->addend);
        r = r->next;
    }
    l->has_value = 1;
    //  Convert the address to the rx address space
    l->u.value = (tcg_target_long)rw_ptr_to_rx((void *)value);
}

int gen_new_label(void)
{
    TCGContext *s = tcg->ctx;
    int idx;
    TCGLabel *l;

    if(s->nb_labels >= TCG_MAX_LABELS) {
        tcg_abort();
    }
    idx = s->nb_labels++;
    l = &s->labels[idx];
    l->has_value = 0;
    l->u.first_reloc = NULL;
    return idx;
}

#include "tcg-target.c"

/* pool based memory allocation */
void *tcg_malloc_internal(TCGContext *s, int size)
{
    TCGPool *p;
    int pool_size;

    if(size > TCG_POOL_CHUNK_SIZE) {
        /* big malloc: insert a new pool (XXX: could optimize) */
        p = TCG_malloc(sizeof(TCGPool) + size);
        p->size = size;
        if(s->pool_current) {
            s->pool_current->next = p;
        } else {
            s->pool_first = p;
        }
        p->next = s->pool_current;
    } else {
        p = s->pool_current;
        if(!p) {
            p = s->pool_first;
            if(!p) {
                goto new_pool;
            }
        } else {
            if(!p->next) {
            new_pool:
                pool_size = TCG_POOL_CHUNK_SIZE;
                p = TCG_malloc(sizeof(TCGPool) + pool_size);
                p->size = pool_size;
                p->next = NULL;
                if(s->pool_current) {
                    s->pool_current->next = p;
                } else {
                    s->pool_first = p;
                }
            } else {
                p = p->next;
            }
        }
    }
    s->pool_current = p;
    s->pool_cur = p->data + size;
    s->pool_end = p->data + p->size;
    return p->data;
}

void tcg_pool_reset(TCGContext *s)
{
    s->pool_cur = s->pool_end = NULL;
    s->pool_current = NULL;
}

static void tcg_pool_free_inner(TCGPool *pool)
{
    if(pool->next) {
        tcg_pool_free_inner(pool->next);
    }
    TCG_free(pool);
}

static void tcg_pool_free(TCGContext *s)
{
    if(s->pool_first) {
        tcg_pool_free_inner(s->pool_first);
    }
}

static TCGContext ctx;
static TCGArg gen_opparam_buf[OPPARAM_BUF_SIZE];
static TCGOpcodeEntry gen_opc_buf[OPC_BUF_SIZE];

static uint16_t gen_insn_end_off[TCG_MAX_INSNS];
static target_ulong gen_insn_data[TCG_MAX_INSNS][TARGET_INSN_START_WORDS];

void tcg_attach(tcg_t *c)
{
    tcg = c;
    tcg->ctx = &ctx;
    //  code_gen_prologue get set by code_gen_alloc
    //  since that has not happened yet, set it to null in the meantime
    tcg->code_gen_prologue = NULL;
    tcg->gen_opparam_buf = gen_opparam_buf;
    tcg->gen_opc_buf = gen_opc_buf;
    tcg->gen_insn_end_off = gen_insn_end_off;
    tcg->gen_insn_data = gen_insn_data;
}

void tcg_context_attach_number_of_registered_cpus(uint32_t *pointer)
{
    tcg->ctx->number_of_registered_cpus = pointer;
}

void tcg_context_init()
{
    TCGContext *s = tcg->ctx;
    int op, total_args, n;
    TCGOpDef *def;
    TCGArgConstraint *args_ct;
    int *sorted_args;

    memset(s, 0, sizeof(*s));
    s->temps = s->static_temps;
    s->nb_globals = 0;
    tcg_context_use_tlb(1);

    /* Count total number of arguments and allocate the corresponding
       space */
    total_args = 0;
    for(op = 0; op < NB_OPS; op++) {
        def = &tcg_op_defs[op];
        n = def->nb_iargs + def->nb_oargs;
        total_args += n;
    }

    args_ct = TCG_malloc(sizeof(TCGArgConstraint) * total_args);
    sorted_args = TCG_malloc(sizeof(int) * total_args);

    for(op = 0; op < NB_OPS; op++) {
        def = &tcg_op_defs[op];
        def->args_ct = args_ct;
        def->sorted_args = sorted_args;
        n = def->nb_iargs + def->nb_oargs;
        sorted_args += n;
        args_ct += n;
    }
    tcg_target_init(s);
}

void tcg_context_use_tlb(int value)
{
    tcg->ctx->use_tlb = !!value;
}

void tcg_dispose()
{
    TCG_free(tcg_op_defs[0].args_ct);
    TCG_free(tcg_op_defs[0].sorted_args);
    tcg_pool_free(tcg->ctx);
    TCG_free(tcg->ctx->helpers);
}

void tcg_prologue_init()
{
    /* init global prologue and epilogue */
    tcg->ctx->code_buf = tcg->code_gen_prologue;
    tcg->ctx->code_ptr = tcg->ctx->code_buf;
    tcg_target_qemu_prologue(tcg->ctx);
    flush_icache_range((uintptr_t)rw_ptr_to_rx(tcg->ctx->code_buf), (uintptr_t)rw_ptr_to_rx(tcg->ctx->code_ptr));
}

void tcg_set_frame(TCGContext *s, int reg, tcg_target_long start, tcg_target_long size)
{
    s->frame_start = start;
    s->frame_end = start + size;
    s->frame_reg = reg;
}

void tcg_func_start(TCGContext *s)
{
    int i;
    tcg_pool_reset(s);
    s->nb_temps = s->nb_globals;
    for(i = 0; i < (TCG_TYPE_COUNT * 2); i++) {
        s->first_free_temp[i] = -1;
    }
    s->labels = tcg_malloc(sizeof(TCGLabel) * TCG_MAX_LABELS);
    s->nb_labels = 0;
    s->current_frame_offset = s->frame_start;

    gen_opc_ptr = tcg->gen_opc_buf;
    gen_opparam_ptr = tcg->gen_opparam_buf;
}

static inline void tcg_temp_alloc(TCGContext *s, int n)
{
    if(n > TCG_MAX_TEMPS) {
        tcg_abort();
    }
}

static inline int tcg_global_reg_new_internal(TCGType type, int reg, const char *name)
{
    TCGContext *s = tcg->ctx;
    TCGTemp *ts;
    int idx;

#if TCG_TARGET_REG_BITS == 32
    if(type != TCG_TYPE_I32) {
        tcg_abort();
    }
#endif
    if(tcg_regset_test_reg(s->reserved_regs, reg)) {
        tcg_abort();
    }
    idx = s->nb_globals;
    tcg_temp_alloc(s, s->nb_globals + 1);
    ts = &s->temps[s->nb_globals];
    ts->base_type = type;
    ts->type = type;
    ts->fixed_reg = 1;
    ts->reg = reg;
    ts->name = name;
    s->nb_globals++;
    tcg_regset_set_reg(s->reserved_regs, reg);
    return idx;
}

TCGv_i32 tcg_global_reg_new_i32(int reg, const char *name)
{
    int idx;

    idx = tcg_global_reg_new_internal(TCG_TYPE_I32, reg, name);
    return MAKE_TCGV_I32(idx);
}

TCGv_i64 tcg_global_reg_new_i64(int reg, const char *name)
{
    int idx;

    idx = tcg_global_reg_new_internal(TCG_TYPE_I64, reg, name);
    return MAKE_TCGV_I64(idx);
}

static inline int tcg_global_mem_new_internal(TCGType type, int reg, tcg_target_long offset, const char *name)
{
    TCGContext *s = tcg->ctx;
    TCGTemp *ts;
    int idx;

    idx = s->nb_globals;
#if TCG_TARGET_REG_BITS == 32
    if(type == TCG_TYPE_I64) {
        char buf[64];
        tcg_temp_alloc(s, s->nb_globals + 2);
        ts = &s->temps[s->nb_globals];
        ts->base_type = type;
        ts->type = TCG_TYPE_I32;
        ts->fixed_reg = 0;
        ts->mem_allocated = 1;
        ts->mem_reg = reg;
#ifdef TCG_TARGET_WORDS_BIGENDIAN
        ts->mem_offset = offset + 4;
#else
        ts->mem_offset = offset;
#endif
        TCG_pstrcpy(buf, sizeof(buf), name);
        TCG_pstrcat(buf, sizeof(buf), "_0");
        ts->name = strdup(buf);
        ts++;

        ts->base_type = type;
        ts->type = TCG_TYPE_I32;
        ts->fixed_reg = 0;
        ts->mem_allocated = 1;
        ts->mem_reg = reg;
#ifdef TCG_TARGET_WORDS_BIGENDIAN
        ts->mem_offset = offset;
#else
        ts->mem_offset = offset + 4;
#endif
        TCG_pstrcpy(buf, sizeof(buf), name);
        TCG_pstrcat(buf, sizeof(buf), "_1");
        ts->name = strdup(buf);

        s->nb_globals += 2;
    } else
#endif
    {
        tcg_temp_alloc(s, s->nb_globals + 1);
        ts = &s->temps[s->nb_globals];
        ts->base_type = type;
        ts->type = type;
        ts->fixed_reg = 0;
        ts->mem_allocated = 1;
        ts->mem_reg = reg;
        ts->mem_offset = offset;
        ts->name = name;
        s->nb_globals++;
    }
    return idx;
}

TCGv_i32 tcg_global_mem_new_i32(int reg, tcg_target_long offset, const char *name)
{
    int idx;

    idx = tcg_global_mem_new_internal(TCG_TYPE_I32, reg, offset, name);
    return MAKE_TCGV_I32(idx);
}

TCGv_i64 tcg_global_mem_new_i64(int reg, tcg_target_long offset, const char *name)
{
    int idx;

    idx = tcg_global_mem_new_internal(TCG_TYPE_I64, reg, offset, name);
    return MAKE_TCGV_I64(idx);
}

static inline int tcg_temp_new_internal(TCGType type, int temp_local)
{
    TCGContext *s = tcg->ctx;
    TCGTemp *ts;
    int idx, k;

    k = type;
    if(temp_local) {
        k += TCG_TYPE_COUNT;
    }
    idx = s->first_free_temp[k];
    if(idx != -1) {
        /* There is already an available temp with the
           right type */
        ts = &s->temps[idx];
        s->first_free_temp[k] = ts->next_free_temp;
        ts->temp_allocated = 1;
        assert(ts->temp_local == temp_local);
    } else {
        idx = s->nb_temps;
#if TCG_TARGET_REG_BITS == 32
        if(type == TCG_TYPE_I64) {
            tcg_temp_alloc(s, s->nb_temps + 2);
            ts = &s->temps[s->nb_temps];
            ts->base_type = type;
            ts->type = TCG_TYPE_I32;
            ts->temp_allocated = 1;
            ts->temp_local = temp_local;
            ts->name = NULL;
            ts++;
            ts->base_type = TCG_TYPE_I32;
            ts->type = TCG_TYPE_I32;
            ts->temp_allocated = 1;
            ts->temp_local = temp_local;
            ts->name = NULL;
            s->nb_temps += 2;
        } else
#endif
        {
            tcg_temp_alloc(s, s->nb_temps + 1);
            ts = &s->temps[s->nb_temps];
            ts->base_type = type;
            ts->type = type;
            ts->temp_allocated = 1;
            ts->temp_local = temp_local;
            ts->name = NULL;
            s->nb_temps++;
        }
    }

    return idx;
}

TCGv_i32 tcg_temp_new_internal_i32(int temp_local)
{
    int idx;

    idx = tcg_temp_new_internal(TCG_TYPE_I32, temp_local);
    return MAKE_TCGV_I32(idx);
}

TCGv_i64 tcg_temp_new_internal_i64(int temp_local)
{
    int idx;

    idx = tcg_temp_new_internal(TCG_TYPE_I64, temp_local);
    return MAKE_TCGV_I64(idx);
}

static inline void tcg_temp_free_internal(int idx)
{
    TCGContext *s = tcg->ctx;
    TCGTemp *ts;
    int k;

    assert(idx >= s->nb_globals && idx < s->nb_temps);
    ts = &s->temps[idx];
    assert(ts->temp_allocated != 0);
    ts->temp_allocated = 0;
    k = ts->base_type;
    if(ts->temp_local) {
        k += TCG_TYPE_COUNT;
    }
    ts->next_free_temp = s->first_free_temp[k];
    s->first_free_temp[k] = idx;
}

void tcg_temp_free_i32(TCGv_i32 arg)
{
    tcg_temp_free_internal(GET_TCGV_I32(arg));
}

void tcg_temp_free_i64(TCGv_i64 arg)
{
    tcg_temp_free_internal(GET_TCGV_I64(arg));
}

void tcg_temp_free_i128(TCGv_i128 arg)
{
    tcg_temp_free_i64(arg.low);
    tcg_temp_free_i64(arg.high);
}

TCGv_i32 tcg_const_i32(int32_t val)
{
    TCGv_i32 t0;
    t0 = tcg_temp_new_i32();
    tcg_gen_movi_i32(t0, val);
    return t0;
}

TCGv_i64 tcg_const_i64(int64_t val)
{
    TCGv_i64 t0;
    t0 = tcg_temp_new_i64();
    tcg_gen_movi_i64(t0, val);
    return t0;
}

TCGv_i32 tcg_const_local_i32(int32_t val)
{
    TCGv_i32 t0;
    t0 = tcg_temp_local_new_i32();
    tcg_gen_movi_i32(t0, val);
    return t0;
}

TCGv_i64 tcg_const_local_i64(int64_t val)
{
    TCGv_i64 t0;
    t0 = tcg_temp_local_new_i64();
    tcg_gen_movi_i64(t0, val);
    return t0;
}

void tcg_register_helper(void *func, const char *name)
{
    TCGContext *s = tcg->ctx;
    int n;
    if((s->nb_helpers + 1) > s->allocated_helpers) {
        n = s->allocated_helpers;
        if(n == 0) {
            n = 4;
        } else {
            n *= 2;
        }
        s->helpers = TCG_realloc(s->helpers, n * sizeof(TCGHelperInfo));
        s->allocated_helpers = n;
    }
    s->helpers[s->nb_helpers].func = (tcg_target_ulong)func;
    s->helpers[s->nb_helpers].name = name;
    s->nb_helpers++;
}

/* Note: we convert the 64 bit args to 32 bit and do some alignment
   and endian swap. Maybe it would be better to do the alignment
   and endian swap in tcg_reg_alloc_call(). */
void tcg_gen_callN(TCGContext *s, TCGv_ptr func, unsigned int flags, int sizemask, TCGArg ret, int nargs, TCGArg *args)
{
#if defined(TCG_TARGET_I386) && TCG_TARGET_REG_BITS < 64
    int call_type;
#endif
    int i;
    int real_args;
    int nb_rets;
    TCGArg *nparam;

#if defined(TCG_TARGET_EXTEND_ARGS) && TCG_TARGET_REG_BITS == 64
    for(i = 0; i < nargs; ++i) {
        int is_64bit = sizemask & (1 << (i + 1) * 2);
        int is_signed = sizemask & (2 << (i + 1) * 2);
        if(!is_64bit) {
            TCGv_i64 temp = tcg_temp_new_i64();
            TCGv_i64 orig = MAKE_TCGV_I64(args[i]);
            if(is_signed) {
                tcg_gen_ext32s_i64(temp, orig);
            } else {
                tcg_gen_ext32u_i64(temp, orig);
            }
            args[i] = GET_TCGV_I64(temp);
        }
    }
#endif /* TCG_TARGET_EXTEND_ARGS */

    *gen_opc_ptr++ = tcg_create_opcode_entry(INDEX_op_call);
    nparam = gen_opparam_ptr++;
#if defined(TCG_TARGET_I386) && TCG_TARGET_REG_BITS < 64
    call_type = (flags & TCG_CALL_TYPE_MASK);
#endif
    if(ret != TCG_CALL_DUMMY_ARG) {
#if TCG_TARGET_REG_BITS < 64
        if(sizemask & 1) {
#ifdef TCG_TARGET_WORDS_BIGENDIAN
            *gen_opparam_ptr++ = ret + 1;
            *gen_opparam_ptr++ = ret;
#else
            *gen_opparam_ptr++ = ret;
            *gen_opparam_ptr++ = ret + 1;
#endif
            nb_rets = 2;
        } else
#endif
        {
            *gen_opparam_ptr++ = ret;
            nb_rets = 1;
        }
    } else {
        nb_rets = 0;
    }
    real_args = 0;
    for(i = 0; i < nargs; i++) {
#if TCG_TARGET_REG_BITS < 64
        int is_64bit = sizemask & (1 << (i + 1) * 2);
        if(is_64bit) {
#ifdef TCG_TARGET_I386
            /* REGPARM case: if the third parameter is 64 bit, it is
               allocated on the stack */
            if(i == 2 && call_type == TCG_CALL_TYPE_REGPARM) {
                call_type = TCG_CALL_TYPE_REGPARM_2;
                flags = (flags & ~TCG_CALL_TYPE_MASK) | call_type;
            }
#endif
#ifdef TCG_TARGET_CALL_ALIGN_ARGS
            /* some targets want aligned 64 bit args */
            if(real_args & 1) {
                *gen_opparam_ptr++ = TCG_CALL_DUMMY_ARG;
                real_args++;
            }
#endif
            /* If stack grows up, then we will be placing successive
               arguments at lower addresses, which means we need to
               reverse the order compared to how we would normally
               treat either big or little-endian.  For those arguments
               that will wind up in registers, this still works for
               HPPA (the only current STACK_GROWSUP target) since the
               argument registers are *also* allocated in decreasing
               order.  If another such target is added, this logic may
               have to get more complicated to differentiate between
               stack arguments and register arguments.  */
#if defined(TCG_TARGET_WORDS_BIGENDIAN) != defined(TCG_TARGET_STACK_GROWSUP)
            *gen_opparam_ptr++ = args[i] + 1;
            *gen_opparam_ptr++ = args[i];
#else
            *gen_opparam_ptr++ = args[i];
            *gen_opparam_ptr++ = args[i] + 1;
#endif
            real_args += 2;
            continue;
        }
#endif /* TCG_TARGET_REG_BITS < 64 */

        *gen_opparam_ptr++ = args[i];
        real_args++;
    }
    *gen_opparam_ptr++ = GET_TCGV_PTR(func);

    *gen_opparam_ptr++ = flags;

    *nparam = (nb_rets << 16) | (real_args + 1);

    /* total parameters, needed to go backward in the instruction stream */
    *gen_opparam_ptr++ = 1 + nb_rets + real_args + 3;

#if defined(TCG_TARGET_EXTEND_ARGS) && TCG_TARGET_REG_BITS == 64
    for(i = 0; i < nargs; ++i) {
        int is_64bit = sizemask & (1 << (i + 1) * 2);
        if(!is_64bit) {
            TCGv_i64 temp = MAKE_TCGV_I64(args[i]);
            tcg_temp_free_i64(temp);
        }
    }
#endif /* TCG_TARGET_EXTEND_ARGS */
}

#if TCG_TARGET_REG_BITS == 32
void tcg_gen_shifti_i64(TCGv_i64 ret, TCGv_i64 arg1, int c, int right, int arith)
{
    if(c == 0) {
        tcg_gen_mov_i32(TCGV_LOW(ret), TCGV_LOW(arg1));
        tcg_gen_mov_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1));
    } else if(c >= 32) {
        c -= 32;
        if(right) {
            if(arith) {
                tcg_gen_sari_i32(TCGV_LOW(ret), TCGV_HIGH(arg1), c);
                tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), 31);
            } else {
                tcg_gen_shri_i32(TCGV_LOW(ret), TCGV_HIGH(arg1), c);
                tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
            }
        } else {
            tcg_gen_shli_i32(TCGV_HIGH(ret), TCGV_LOW(arg1), c);
            tcg_gen_movi_i32(TCGV_LOW(ret), 0);
        }
    } else {
        TCGv_i32 t0, t1;

        t0 = tcg_temp_new_i32();
        t1 = tcg_temp_new_i32();
        if(right) {
            tcg_gen_shli_i32(t0, TCGV_HIGH(arg1), 32 - c);
            if(arith) {
                tcg_gen_sari_i32(t1, TCGV_HIGH(arg1), c);
            } else {
                tcg_gen_shri_i32(t1, TCGV_HIGH(arg1), c);
            }
            tcg_gen_shri_i32(TCGV_LOW(ret), TCGV_LOW(arg1), c);
            tcg_gen_or_i32(TCGV_LOW(ret), TCGV_LOW(ret), t0);
            tcg_gen_mov_i32(TCGV_HIGH(ret), t1);
        } else {
            tcg_gen_shri_i32(t0, TCGV_LOW(arg1), 32 - c);
            /* Note: ret can be the same as arg1, so we use t1 */
            tcg_gen_shli_i32(t1, TCGV_LOW(arg1), c);
            tcg_gen_shli_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), c);
            tcg_gen_or_i32(TCGV_HIGH(ret), TCGV_HIGH(ret), t0);
            tcg_gen_mov_i32(TCGV_LOW(ret), t1);
        }
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}
#endif

static void tcg_reg_alloc_start(TCGContext *s)
{
    int i;
    TCGTemp *ts;
    for(i = 0; i < s->nb_globals; i++) {
        ts = &s->temps[i];
        if(ts->fixed_reg) {
            ts->val_type = TEMP_VAL_REG;
        } else {
            ts->val_type = TEMP_VAL_MEM;
        }
    }
    for(i = s->nb_globals; i < s->nb_temps; i++) {
        ts = &s->temps[i];
        ts->val_type = TEMP_VAL_DEAD;
        ts->mem_allocated = 0;
        ts->fixed_reg = 0;
    }
    for(i = 0; i < TCG_TARGET_NB_REGS; i++) {
        s->reg_to_temp[i] = -1;
    }
}

static char *tcg_get_arg_str_idx(TCGContext *s, char *buf, int buf_size, int idx)
{
    TCGTemp *ts;

    assert(idx >= 0 && idx < s->nb_temps);
    ts = &s->temps[idx];
    assert(ts);
    if(idx < s->nb_globals) {
        TCG_pstrcpy(buf, buf_size, ts->name);
    } else {
        if(ts->temp_local) {
            snprintf(buf, buf_size, "loc%d", idx - s->nb_globals);
        } else {
            snprintf(buf, buf_size, "tmp%d", idx - s->nb_globals);
        }
    }
    return buf;
}

char *tcg_get_arg_str_i32(TCGContext *s, char *buf, int buf_size, TCGv_i32 arg)
{
    return tcg_get_arg_str_idx(s, buf, buf_size, GET_TCGV_I32(arg));
}

char *tcg_get_arg_str_i64(TCGContext *s, char *buf, int buf_size, TCGv_i64 arg)
{
    return tcg_get_arg_str_idx(s, buf, buf_size, GET_TCGV_I64(arg));
}

static int helper_cmp(const void *p1, const void *p2)
{
    const TCGHelperInfo *th1 = p1;
    const TCGHelperInfo *th2 = p2;
    if(th1->func < th2->func) {
        return -1;
    } else if(th1->func == th2->func) {
        return 0;
    } else {
        return 1;
    }
}

/* find helper definition (Note: A hash table would be better) */
static TCGHelperInfo *tcg_find_helper(TCGContext *s, tcg_target_ulong val)
{
    int m, m_min, m_max;
    TCGHelperInfo *th;
    tcg_target_ulong v;

    if(unlikely(!s->helpers_sorted)) {
        qsort(s->helpers, s->nb_helpers, sizeof(TCGHelperInfo), helper_cmp);
        s->helpers_sorted = 1;
    }

    /* binary search */
    m_min = 0;
    m_max = s->nb_helpers - 1;
    while(m_min <= m_max) {
        m = (m_min + m_max) >> 1;
        th = &s->helpers[m];
        v = th->func;
        if(v == val) {
            return th;
        } else if(val < v) {
            m_max = m - 1;
        } else {
            m_min = m + 1;
        }
    }
    return NULL;
}

/* we give more priority to constraints with less registers */
static int get_constraint_priority(const TCGOpDef *def, int k)
{
    const TCGArgConstraint *arg_ct;

    int i, n;
    arg_ct = &def->args_ct[k];
    if(arg_ct->ct & TCG_CT_ALIAS) {
        /* an alias is equivalent to a single register */
        n = 1;
    } else {
        if(!(arg_ct->ct & TCG_CT_REG)) {
            return 0;
        }
        n = 0;
        for(i = 0; i < TCG_TARGET_NB_REGS; i++) {
            if(tcg_regset_test_reg(arg_ct->u.regs, i)) {
                n++;
            }
        }
    }
    return TCG_TARGET_NB_REGS - n + 1;
}

/* sort from highest priority to lowest */
static void sort_constraints(TCGOpDef *def, int start, int n)
{
    int i, j, p1, p2, tmp;

    for(i = 0; i < n; i++) {
        def->sorted_args[start + i] = start + i;
    }
    if(n <= 1) {
        return;
    }
    for(i = 0; i < n - 1; i++) {
        for(j = i + 1; j < n; j++) {
            p1 = get_constraint_priority(def, def->sorted_args[start + i]);
            p2 = get_constraint_priority(def, def->sorted_args[start + j]);
            if(p1 < p2) {
                tmp = def->sorted_args[start + i];
                def->sorted_args[start + i] = def->sorted_args[start + j];
                def->sorted_args[start + j] = tmp;
            }
        }
    }
}

void tcg_add_target_add_op_defs(const TCGTargetOpDef *tdefs)
{
    TCGOpcode op;
    TCGOpDef *def;
    const char *ct_str;
    int i, nb_args;

    for(;;) {
        if(tdefs->op == (TCGOpcode)-1) {
            break;
        }
        op = tdefs->op;
        assert((unsigned)op < NB_OPS);
        def = &tcg_op_defs[op];
        nb_args = def->nb_iargs + def->nb_oargs;
        for(i = 0; i < nb_args; i++) {
            ct_str = tdefs->args_ct_str[i];
            /* Incomplete TCGTargetOpDef entry? */
            assert(ct_str != NULL);
            tcg_regset_clear(def->args_ct[i].u.regs);
            def->args_ct[i].ct = 0;
            if(ct_str[0] >= '0' && ct_str[0] <= '9') {
                int oarg;
                oarg = ct_str[0] - '0';
                assert(oarg < def->nb_oargs);
                assert(def->args_ct[oarg].ct & TCG_CT_REG);
                /* TCG_CT_ALIAS is for the output arguments. The input
                   argument is tagged with TCG_CT_IALIAS. */
                def->args_ct[i] = def->args_ct[oarg];
                def->args_ct[oarg].ct = TCG_CT_ALIAS;
                def->args_ct[oarg].alias_index = i;
                def->args_ct[i].ct |= TCG_CT_IALIAS;
                def->args_ct[i].alias_index = oarg;
            } else {
                for(;;) {
                    if(*ct_str == '\0') {
                        break;
                    }
                    switch(*ct_str) {
                        case 'i':
                            def->args_ct[i].ct |= TCG_CT_CONST;
                            ct_str++;
                            break;
                        default:
                            if(target_parse_constraint(&def->args_ct[i], &ct_str) < 0) {
                                tcg_abortf("Invalid constraint '%s' for arg %d of operation '%s'\n", ct_str, i, def->name);
                                tlib_assert_not_reached();
                            }
                    }
                }
            }
        }

        /* TCGTargetOpDef entry with too much information? */
        assert(i == TCG_MAX_OP_ARGS || tdefs->args_ct_str[i] == NULL);

        /* sort the constraints (XXX: this is just an heuristic) */
        sort_constraints(def, 0, def->nb_oargs);
        sort_constraints(def, def->nb_oargs, def->nb_iargs);

        tdefs++;
    }
}

#ifdef USE_LIVENESS_ANALYSIS

/* set a nop for an operation using 'nb_args' */
static inline void tcg_set_nop(TCGContext *s, TCGOpcodeEntry *opc_ptr, TCGArg *args, int nb_args)
{
    if(nb_args == 0) {
        *opc_ptr = tcg_create_opcode_entry(INDEX_op_nop);
    } else {
        *opc_ptr = tcg_create_opcode_entry(INDEX_op_nopn);
        args[0] = nb_args;
        args[nb_args - 1] = nb_args;
    }
}

/* liveness analysis: end of function: globals are live, temps are
   dead. */
/* XXX: at this stage, not used as there would be little gains because
   most TBs end with a conditional jump. */
static inline void tcg_la_func_end(TCGContext *s, uint8_t *dead_temps)
{
    memset(dead_temps, 0, s->nb_globals);
    memset(dead_temps + s->nb_globals, 1, s->nb_temps - s->nb_globals);
}

/* liveness analysis: end of basic block: globals are live, temps are
   dead, local temps are live. */
static inline void tcg_la_bb_end(TCGContext *s, uint8_t *dead_temps)
{
    int i;
    TCGTemp *ts;

    memset(dead_temps, 0, s->nb_globals);
    ts = &s->temps[s->nb_globals];
    for(i = s->nb_globals; i < s->nb_temps; i++) {
        if(ts->temp_local) {
            dead_temps[i] = 0;
        } else {
            dead_temps[i] = 1;
        }
        ts++;
    }
}

/* Liveness analysis : update the opc_dead_args array to tell if a
   given input arguments is dead. Instructions updating dead
   temporaries are removed. */
static void tcg_liveness_analysis(TCGContext *s)
{
    int i, op_index, nb_args, nb_iargs, nb_oargs, arg, nb_ops;
    TCGOpcode op;
    TCGArg *args;
    const TCGOpDef *def;
    uint8_t *dead_temps;
    unsigned int dead_args;

    gen_opc_ptr++; /* skip end */

    nb_ops = gen_opc_ptr - tcg->gen_opc_buf;

    s->op_dead_args = tcg_malloc(nb_ops * sizeof(uint16_t));

    dead_temps = tcg_malloc(s->nb_temps);
    memset(dead_temps, 1, s->nb_temps);

    args = gen_opparam_ptr;
    op_index = nb_ops - 1;
    while(op_index >= 0) {
        op = tcg->gen_opc_buf[op_index].opcode;
        def = &tcg_op_defs[op];
        switch(op) {
            case INDEX_op_call: {
                int call_flags;

                nb_args = args[-1];
                args -= nb_args;
                nb_iargs = args[0] & 0xffff;
                nb_oargs = args[0] >> 16;
                args++;
                call_flags = args[nb_oargs + nb_iargs];

                /* pure functions can be removed if their result is not
                   used */
                if(call_flags & TCG_CALL_PURE) {
                    for(i = 0; i < nb_oargs; i++) {
                        arg = args[i];
                        if(!dead_temps[arg]) {
                            goto do_not_remove_call;
                        }
                    }
                    tcg_set_nop(s, tcg->gen_opc_buf + op_index, args - 1, nb_args);
                } else {
                do_not_remove_call:

                    /* output args are dead */
                    dead_args = 0;
                    for(i = 0; i < nb_oargs; i++) {
                        arg = args[i];
                        if(dead_temps[arg]) {
                            dead_args |= (1 << i);
                        }
                        dead_temps[arg] = 1;
                    }

                    if(!(call_flags & TCG_CALL_CONST)) {
                        /* globals are live (they may be used by the call) */
                        memset(dead_temps, 0, s->nb_globals);
                    }

                    /* input args are live */
                    for(i = nb_oargs; i < nb_iargs + nb_oargs; i++) {
                        arg = args[i];
                        if(arg != TCG_CALL_DUMMY_ARG) {
                            if(dead_temps[arg]) {
                                dead_args |= (1 << i);
                            }
                            dead_temps[arg] = 0;
                        }
                    }
                    s->op_dead_args[op_index] = dead_args;
                }
                args--;
            } break;
            case INDEX_op_insn_start:
#if TARGET_LONG_BITS > TCG_TARGET_REG_BITS
                args -= 2 * TARGET_INSN_START_WORDS;
#else
                args -= TARGET_INSN_START_WORDS;
#endif
                break;
            case INDEX_op_set_label:
                args--;
                /* mark end of basic block */
                tcg_la_bb_end(s, dead_temps);
                break;
            case INDEX_op_nopn:
                nb_args = args[-1];
                args -= nb_args;
                break;
            case INDEX_op_discard:
                args--;
                /* mark the temporary as dead */
                dead_temps[args[0]] = 1;
                break;
            case INDEX_op_end:
                break;
            /* XXX: optimize by hardcoding common cases (e.g. triadic ops) */
            default:
                args -= def->nb_args;
                nb_iargs = def->nb_iargs;
                nb_oargs = def->nb_oargs;

                /* Test if the operation can be removed because all
                   its outputs are dead. We assume that nb_oargs == 0
                   implies side effects */
                if(!(def->flags & TCG_OPF_SIDE_EFFECTS) && nb_oargs != 0) {
                    for(i = 0; i < nb_oargs; i++) {
                        arg = args[i];
                        if(!dead_temps[arg]) {
                            goto do_not_remove;
                        }
                    }
                    tcg_set_nop(s, tcg->gen_opc_buf + op_index, args, def->nb_args);
                } else {
                do_not_remove:

                    /* output args are dead */
                    dead_args = 0;
                    for(i = 0; i < nb_oargs; i++) {
                        arg = args[i];
                        if(dead_temps[arg]) {
                            dead_args |= (1 << i);
                        }
                        dead_temps[arg] = 1;
                    }

                    /* if end of basic block, update */
                    if(def->flags & TCG_OPF_BB_END) {
                        tcg_la_bb_end(s, dead_temps);
                    } else if(def->flags & TCG_OPF_CALL_CLOBBER) {
                        /* globals are live */
                        memset(dead_temps, 0, s->nb_globals);
                    }

                    /* input args are live */
                    for(i = nb_oargs; i < nb_oargs + nb_iargs; i++) {
                        arg = args[i];
                        if(dead_temps[arg]) {
                            dead_args |= (1 << i);
                        }
                        dead_temps[arg] = 0;
                    }
                    s->op_dead_args[op_index] = dead_args;
                }
                break;
        }
        op_index--;
    }

    if(args != tcg->gen_opparam_buf) {
        tlib_abortf("%s: %p should be %p. Missing %i args", __func__, args, tcg->gen_opparam_buf, args - tcg->gen_opparam_buf);
    }
}
#else
/* dummy liveness analysis */
static void tcg_liveness_analysis(TCGContext *s)
{
    int nb_ops;
    nb_ops = gen_opc_ptr - tcg->gen_opc_buf;

    s->op_dead_args = tcg_malloc(nb_ops * sizeof(uint16_t));
    memset(s->op_dead_args, 0, nb_ops * sizeof(uint16_t));
}
#endif

static void temp_allocate_frame(TCGContext *s, int temp)
{
    TCGTemp *ts;
    ts = &s->temps[temp];
#ifndef __sparc_v9__ /* Sparc64 stack is accessed with offset of 2047 */
    s->current_frame_offset = (s->current_frame_offset + (tcg_target_long)sizeof(tcg_target_long) - 1) &
                              ~(sizeof(tcg_target_long) - 1);
#endif
    if(s->current_frame_offset + (tcg_target_long)sizeof(tcg_target_long) > s->frame_end) {
        tcg_abort();
    }
    ts->mem_offset = s->current_frame_offset;
    ts->mem_reg = s->frame_reg;
    ts->mem_allocated = 1;
    s->current_frame_offset += (tcg_target_long)sizeof(tcg_target_long);
}

/* free register 'reg' by spilling the corresponding temporary if necessary */
static void tcg_reg_free(TCGContext *s, int reg)
{
    TCGTemp *ts;
    int temp;

    temp = s->reg_to_temp[reg];
    if(temp != -1) {
        ts = &s->temps[temp];
        assert(ts->val_type == TEMP_VAL_REG);
        if(!ts->mem_coherent) {
            if(!ts->mem_allocated) {
                temp_allocate_frame(s, temp);
            }
            tcg_out_st(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
        }
        ts->val_type = TEMP_VAL_MEM;
        s->reg_to_temp[reg] = -1;
    }
}

/* Allocate a register belonging to reg1 & ~reg2 */
static int tcg_reg_alloc(TCGContext *s, TCGRegSet reg1, TCGRegSet reg2)
{
    int i, reg;
    TCGRegSet reg_ct;

    tcg_regset_andnot(reg_ct, reg1, reg2);

    /* first try free registers */
    for(i = 0; i < ARRAY_SIZE(tcg_target_reg_alloc_order); i++) {
        reg = tcg_target_reg_alloc_order[i];
        if(tcg_regset_test_reg(reg_ct, reg) && s->reg_to_temp[reg] == -1) {
            return reg;
        }
    }

    /* XXX: do better spill choice */
    for(i = 0; i < ARRAY_SIZE(tcg_target_reg_alloc_order); i++) {
        reg = tcg_target_reg_alloc_order[i];
        if(tcg_regset_test_reg(reg_ct, reg)) {
            tcg_reg_free(s, reg);
            return reg;
        }
    }

    tcg_abort();
    /* Never reached */
    return -1;
}

/* save a temporary to memory. 'allocated_regs' is used in case a
   temporary registers needs to be allocated to store a constant. */
static void temp_save(TCGContext *s, int temp, TCGRegSet allocated_regs)
{
    TCGTemp *ts;
    int reg;

    ts = &s->temps[temp];
    if(!ts->fixed_reg) {
        switch(ts->val_type) {
            case TEMP_VAL_REG:
                tcg_reg_free(s, ts->reg);
                break;
            case TEMP_VAL_DEAD:
                ts->val_type = TEMP_VAL_MEM;
                break;
            case TEMP_VAL_CONST:
                reg = tcg_reg_alloc(s, tcg_target_available_regs[ts->type], allocated_regs);
                if(!ts->mem_allocated) {
                    temp_allocate_frame(s, temp);
                }
                tcg_out_movi(s, ts->type, reg, ts->val);
                tcg_out_st(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
                ts->val_type = TEMP_VAL_MEM;
                break;
            case TEMP_VAL_MEM:
                break;
            default:
                tcg_abort();
        }
    }
}

/* save globals to their cannonical location and assume they can be
   modified be the following code. 'allocated_regs' is used in case a
   temporary registers needs to be allocated to store a constant. */
static void save_globals(TCGContext *s, TCGRegSet allocated_regs)
{
    int i;

    for(i = 0; i < s->nb_globals; i++) {
        temp_save(s, i, allocated_regs);
    }
}

/* at the end of a basic block, we assume all temporaries are dead and
   all globals are stored at their canonical location. */
static void tcg_reg_alloc_bb_end(TCGContext *s, TCGRegSet allocated_regs)
{
    TCGTemp *ts;
    int i;

    for(i = s->nb_globals; i < s->nb_temps; i++) {
        ts = &s->temps[i];
        if(ts->temp_local) {
            temp_save(s, i, allocated_regs);
        } else {
            if(ts->val_type == TEMP_VAL_REG) {
                s->reg_to_temp[ts->reg] = -1;
            }
            ts->val_type = TEMP_VAL_DEAD;
        }
    }

    save_globals(s, allocated_regs);
}

#define IS_DEAD_ARG(n) ((dead_args >> (n)) & 1)

static void tcg_reg_alloc_movi(TCGContext *s, const TCGArg *args)
{
    TCGTemp *ots;
    tcg_target_ulong val;

    ots = &s->temps[args[0]];
    val = args[1];

    if(ots->fixed_reg) {
        /* for fixed registers, we do not do any constant
           propagation */
        tcg_out_movi(s, ots->type, ots->reg, val);
    } else {
        /* The movi is not explicitly generated here */
        if(ots->val_type == TEMP_VAL_REG) {
            s->reg_to_temp[ots->reg] = -1;
        }
        ots->val_type = TEMP_VAL_CONST;
        ots->val = val;
    }
}

static void tcg_reg_alloc_mov(TCGContext *s, const TCGOpDef *def, const TCGArg *args, unsigned int dead_args)
{
    TCGTemp *ts, *ots;
    int reg;
    const TCGArgConstraint *arg_ct;

    ots = &s->temps[args[0]];
    ts = &s->temps[args[1]];
    arg_ct = &def->args_ct[0];

    /* XXX: always mark arg dead if IS_DEAD_ARG(1) */
    if(ts->val_type == TEMP_VAL_REG) {
        if(IS_DEAD_ARG(1) && !ts->fixed_reg && !ots->fixed_reg) {
            /* the mov can be suppressed */
            if(ots->val_type == TEMP_VAL_REG) {
                s->reg_to_temp[ots->reg] = -1;
            }
            reg = ts->reg;
            s->reg_to_temp[reg] = -1;
            ts->val_type = TEMP_VAL_DEAD;
        } else {
            if(ots->val_type == TEMP_VAL_REG) {
                reg = ots->reg;
            } else {
                reg = tcg_reg_alloc(s, arg_ct->u.regs, s->reserved_regs);
            }
            if(ts->reg != reg) {
                tcg_out_mov(s, ots->type, reg, ts->reg);
            }
        }
    } else if(ts->val_type == TEMP_VAL_MEM) {
        if(ots->val_type == TEMP_VAL_REG) {
            reg = ots->reg;
        } else {
            reg = tcg_reg_alloc(s, arg_ct->u.regs, s->reserved_regs);
        }
        tcg_out_ld(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
    } else if(ts->val_type == TEMP_VAL_CONST) {
        if(ots->fixed_reg) {
            reg = ots->reg;
            tcg_out_movi(s, ots->type, reg, ts->val);
        } else {
            /* propagate constant */
            if(ots->val_type == TEMP_VAL_REG) {
                s->reg_to_temp[ots->reg] = -1;
            }
            ots->val_type = TEMP_VAL_CONST;
            ots->val = ts->val;
            return;
        }
    } else if(ts->val_type == TEMP_VAL_DEAD) {
        tcg_print_opcode_backtrace(s);
        tlib_abortf("%s: "
                    "unexpected val_type `TEMP_VAL_DEAD` for TCGv '%s' def '%s'",
                    __func__, ts->name ? ts->name : "unnamed", def->name);
    } else {
        tcg_print_opcode_backtrace(s);
        tlib_abortf("%s: unexpected val_type `%d` for TCGv '%s' def '%s'", __func__, ts->val_type,
                    ts->name ? ts->name : "unnamed", def->name);
    }
    s->reg_to_temp[reg] = args[0];
    ots->reg = reg;
    ots->val_type = TEMP_VAL_REG;
    ots->mem_coherent = 0;
}

static void tcg_reg_alloc_op(TCGContext *s, const TCGOpDef *def, TCGOpcode opc, const TCGArg *args, unsigned int dead_args)
{
    TCGRegSet allocated_regs;
    int i, k, nb_iargs, nb_oargs, reg;
    TCGArg arg;
    const TCGArgConstraint *arg_ct;
    TCGTemp *ts;
    TCGArg new_args[TCG_MAX_OP_ARGS];
    int const_args[TCG_MAX_OP_ARGS];

    nb_oargs = def->nb_oargs;
    nb_iargs = def->nb_iargs;

    /* copy constants */
    memcpy(new_args + nb_oargs + nb_iargs, args + nb_oargs + nb_iargs, sizeof(TCGArg) * def->nb_cargs);

    /* satisfy input constraints */
    tcg_regset_set(allocated_regs, s->reserved_regs);
    for(k = 0; k < nb_iargs; k++) {
        i = def->sorted_args[nb_oargs + k];
        arg = args[i];
        arg_ct = &def->args_ct[i];
        ts = &s->temps[arg];
        if(ts->val_type == TEMP_VAL_MEM) {
            reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
            tcg_out_ld(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
            ts->val_type = TEMP_VAL_REG;
            ts->reg = reg;
            ts->mem_coherent = 1;
            s->reg_to_temp[reg] = arg;
        } else if(ts->val_type == TEMP_VAL_CONST) {
            if(tcg_target_const_match(ts->val, arg_ct)) {
                /* constant is OK for instruction */
                const_args[i] = 1;
                new_args[i] = ts->val;
                goto iarg_end;
            } else {
                /* need to move to a register */
                reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
                tcg_out_movi(s, ts->type, reg, ts->val);
                ts->val_type = TEMP_VAL_REG;
                ts->reg = reg;
                ts->mem_coherent = 0;
                s->reg_to_temp[reg] = arg;
            }
        }
        if(ts->val_type != TEMP_VAL_REG) {
            tcg_print_opcode_backtrace(s);
            tlib_abortf("%s: TCGv '%s' must be in register at this point when allocating arg %d of `%s`", __func__,
                        ts->name ? ts->name : "unnamed", k, def->name);
        }
        assert(ts->val_type == TEMP_VAL_REG);
        if(arg_ct->ct & TCG_CT_IALIAS) {
            if(ts->fixed_reg) {
                /* if fixed register, we must allocate a new register
                   if the alias is not the same register */
                if(arg != args[arg_ct->alias_index]) {
                    goto allocate_in_reg;
                }
            } else {
                /* if the input is aliased to an output and if it is
                   not dead after the instruction, we must allocate
                   a new register and move it */
                if(!IS_DEAD_ARG(i)) {
                    goto allocate_in_reg;
                }
            }
        }
        reg = ts->reg;
        if(tcg_regset_test_reg(arg_ct->u.regs, reg)) {
            /* nothing to do : the constraint is satisfied */
        } else {
        allocate_in_reg:
            /* allocate a new register matching the constraint
               and move the temporary register into it */
            reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
            tcg_out_mov(s, ts->type, reg, ts->reg);
        }
        new_args[i] = reg;
        const_args[i] = 0;
        tcg_regset_set_reg(allocated_regs, reg);
    iarg_end:;
    }

    if(def->flags & TCG_OPF_BB_END) {
        tcg_reg_alloc_bb_end(s, allocated_regs);
    } else {
        /* mark dead temporaries and free the associated registers */
        for(i = nb_oargs; i < nb_oargs + nb_iargs; i++) {
            arg = args[i];
            if(IS_DEAD_ARG(i)) {
                ts = &s->temps[arg];
                if(!ts->fixed_reg) {
                    if(ts->val_type == TEMP_VAL_REG) {
                        s->reg_to_temp[ts->reg] = -1;
                    }
                    ts->val_type = TEMP_VAL_DEAD;
                }
            }
        }

        if(def->flags & TCG_OPF_CALL_CLOBBER) {
            /* XXX: permit generic clobber register list ? */
            for(reg = 0; reg < TCG_TARGET_NB_REGS; reg++) {
                if(tcg_regset_test_reg(tcg_target_call_clobber_regs, reg)) {
                    tcg_reg_free(s, reg);
                }
            }
            /* XXX: for load/store we could do that only for the slow path
               (i.e. when a memory callback is called) */

            /* store globals and free associated registers (we assume the insn
               can modify any global. */
            save_globals(s, allocated_regs);
        }

        /* satisfy the output constraints */
        tcg_regset_set(allocated_regs, s->reserved_regs);
        for(k = 0; k < nb_oargs; k++) {
            i = def->sorted_args[k];
            arg = args[i];
            arg_ct = &def->args_ct[i];
            ts = &s->temps[arg];
            if(arg_ct->ct & TCG_CT_ALIAS) {
                reg = new_args[arg_ct->alias_index];
            } else {
                /* if fixed register, we try to use it */
                reg = ts->reg;
                if(ts->fixed_reg && tcg_regset_test_reg(arg_ct->u.regs, reg)) {
                    goto oarg_end;
                }
                reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
            }
            tcg_regset_set_reg(allocated_regs, reg);
            /* if a fixed register is used, then a move will be done afterwards */
            if(!ts->fixed_reg) {
                if(ts->val_type == TEMP_VAL_REG) {
                    s->reg_to_temp[ts->reg] = -1;
                }
                if(IS_DEAD_ARG(i)) {
                    ts->val_type = TEMP_VAL_DEAD;
                } else {
                    ts->val_type = TEMP_VAL_REG;
                    ts->reg = reg;
                    /* temp value is modified, so the value kept in memory is
                       potentially not the same */
                    ts->mem_coherent = 0;
                    s->reg_to_temp[reg] = arg;
                }
            }
        oarg_end:
            new_args[i] = reg;
        }
    }

    /* emit instruction */
    tcg_out_op(s, opc, new_args, const_args);

    /* move the outputs in the correct register if needed */
    for(i = 0; i < nb_oargs; i++) {
        ts = &s->temps[args[i]];
        reg = new_args[i];
        if(ts->fixed_reg && ts->reg != reg) {
            tcg_out_mov(s, ts->type, ts->reg, reg);
        }
    }
}

#ifdef TCG_TARGET_STACK_GROWSUP
#define STACK_DIR(x) (-(x))
#else
#define STACK_DIR(x) (x)
#endif

static int tcg_reg_alloc_call(TCGContext *s, const TCGOpDef *def, TCGOpcode opc, const TCGArg *args, unsigned int dead_args)
{
    int nb_iargs, nb_oargs, flags, nb_regs, i, reg, nb_params;
    TCGArg arg, func_arg;
    TCGTemp *ts;
    tcg_target_long stack_offset, call_stack_size, func_addr;
    int const_func_arg, allocate_args;
    TCGRegSet allocated_regs;
    const TCGArgConstraint *arg_ct;

    arg = *args++;

    nb_oargs = arg >> 16;
    nb_iargs = arg & 0xffff;
    nb_params = nb_iargs - 1;

    flags = args[nb_oargs + nb_iargs];

    nb_regs = tcg_target_get_call_iarg_regs_count(flags);
    if(nb_regs > nb_params) {
        nb_regs = nb_params;
    }

    /* assign stack slots first */
    call_stack_size = (nb_params - nb_regs) * sizeof(tcg_target_long);
    call_stack_size = (call_stack_size + TCG_TARGET_STACK_ALIGN - 1) & ~(TCG_TARGET_STACK_ALIGN - 1);
    allocate_args = (call_stack_size > TCG_STATIC_CALL_ARGS_SIZE);
    if(allocate_args) {
        /* XXX: if more than TCG_STATIC_CALL_ARGS_SIZE is needed,
           preallocate call stack */
        tlib_abortf("%s: call stack size %i too large (>%i)", __func__, call_stack_size, TCG_STATIC_CALL_ARGS_SIZE);
    }

    stack_offset = TCG_TARGET_CALL_STACK_OFFSET;
    for(i = nb_regs; i < nb_params; i++) {
        arg = args[nb_oargs + i];
#ifdef TCG_TARGET_STACK_GROWSUP
        stack_offset -= sizeof(tcg_target_long);
#endif
        if(arg != TCG_CALL_DUMMY_ARG) {
            ts = &s->temps[arg];
            if(ts->val_type == TEMP_VAL_REG) {
                tcg_out_st(s, ts->type, ts->reg, TCG_REG_CALL_STACK, stack_offset);
            } else if(ts->val_type == TEMP_VAL_MEM) {
                reg = tcg_reg_alloc(s, tcg_target_available_regs[ts->type], s->reserved_regs);
                /* XXX: not correct if reading values from the stack */
                tcg_out_ld(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
                tcg_out_st(s, ts->type, reg, TCG_REG_CALL_STACK, stack_offset);
            } else if(ts->val_type == TEMP_VAL_CONST) {
                reg = tcg_reg_alloc(s, tcg_target_available_regs[ts->type], s->reserved_regs);
                /* XXX: sign extend may be needed on some targets */
                tcg_out_movi(s, ts->type, reg, ts->val);
                tcg_out_st(s, ts->type, reg, TCG_REG_CALL_STACK, stack_offset);
            } else {
                tlib_abortf("%s: unexpected val_type `%d` for TCGv '%s' def '%s'", __func__, ts->val_type,
                            ts->name ? ts->name : "unnamed", def->name);
            }
        }
#ifndef TCG_TARGET_STACK_GROWSUP
        stack_offset += sizeof(tcg_target_long);
#endif
    }

    /* assign input registers */
    tcg_regset_set(allocated_regs, s->reserved_regs);
    for(i = 0; i < nb_regs; i++) {
        arg = args[nb_oargs + i];
        if(arg != TCG_CALL_DUMMY_ARG) {
            ts = &s->temps[arg];
            reg = tcg_target_call_iarg_regs[i];
            tcg_reg_free(s, reg);
            if(ts->val_type == TEMP_VAL_REG) {
                if(ts->reg != reg) {
                    tcg_out_mov(s, ts->type, reg, ts->reg);
                }
            } else if(ts->val_type == TEMP_VAL_MEM) {
                tcg_out_ld(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
            } else if(ts->val_type == TEMP_VAL_CONST) {
                /* XXX: sign extend ? */
                tcg_out_movi(s, ts->type, reg, ts->val);
            } else {
                tcg_print_opcode_backtrace(s);
                tcg_abort();
            }
            tcg_regset_set_reg(allocated_regs, reg);
        }
    }

    /* assign function address */
    func_arg = args[nb_oargs + nb_iargs - 1];
    arg_ct = &def->args_ct[0];
    ts = &s->temps[func_arg];
    func_addr = ts->val;
    const_func_arg = 0;
    if(ts->val_type == TEMP_VAL_MEM) {
        reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
        tcg_out_ld(s, ts->type, reg, ts->mem_reg, ts->mem_offset);
        func_arg = reg;
        tcg_regset_set_reg(allocated_regs, reg);
    } else if(ts->val_type == TEMP_VAL_REG) {
        reg = ts->reg;
        if(!tcg_regset_test_reg(arg_ct->u.regs, reg)) {
            reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
            tcg_out_mov(s, ts->type, reg, ts->reg);
        }
        func_arg = reg;
        tcg_regset_set_reg(allocated_regs, reg);
    } else if(ts->val_type == TEMP_VAL_CONST) {
        if(tcg_target_const_match(func_addr, arg_ct)) {
            const_func_arg = 1;
            func_arg = func_addr;
        } else {
            reg = tcg_reg_alloc(s, arg_ct->u.regs, allocated_regs);
            tcg_out_movi(s, ts->type, reg, func_addr);
            func_arg = reg;
            tcg_regset_set_reg(allocated_regs, reg);
        }
    } else {
        tcg_abort();
    }

    /* mark dead temporaries and free the associated registers */
    for(i = nb_oargs; i < nb_iargs + nb_oargs; i++) {
        arg = args[i];
        if(IS_DEAD_ARG(i)) {
            ts = &s->temps[arg];
            if(!ts->fixed_reg) {
                if(ts->val_type == TEMP_VAL_REG) {
                    s->reg_to_temp[ts->reg] = -1;
                }
                ts->val_type = TEMP_VAL_DEAD;
            }
        }
    }

    /* clobber call registers */
    for(reg = 0; reg < TCG_TARGET_NB_REGS; reg++) {
        if(tcg_regset_test_reg(tcg_target_call_clobber_regs, reg)) {
            tcg_reg_free(s, reg);
        }
    }

    /* store globals and free associated registers (we assume the call
       can modify any global. */
    if(!(flags & TCG_CALL_CONST)) {
        save_globals(s, allocated_regs);
    }

    tcg_out_op(s, opc, &func_arg, &const_func_arg);

    /* assign output registers and emit moves if needed */
    for(i = 0; i < nb_oargs; i++) {
        arg = args[i];
        ts = &s->temps[arg];
        reg = tcg_target_call_oarg_regs[i];
        assert(s->reg_to_temp[reg] == -1);
        if(ts->fixed_reg) {
            if(ts->reg != reg) {
                tcg_out_mov(s, ts->type, ts->reg, reg);
            }
        } else {
            if(ts->val_type == TEMP_VAL_REG) {
                s->reg_to_temp[ts->reg] = -1;
            }
            if(IS_DEAD_ARG(i)) {
                ts->val_type = TEMP_VAL_DEAD;
            } else {
                ts->val_type = TEMP_VAL_REG;
                ts->reg = reg;
                ts->mem_coherent = 0;
                s->reg_to_temp[reg] = arg;
            }
        }
    }

    return nb_iargs + nb_oargs + def->nb_cargs + 1;
}

static inline int tcg_gen_code_common(TCGContext *s, uint8_t *gen_code_buf)
{
    TCGOpcode opc;
    int i, op_index, num_insns;
    const TCGOpDef *def;
    unsigned int dead_args;
    const TCGArg *args;

#ifdef USE_TCG_OPTIMIZATIONS
    gen_opparam_ptr = tcg_optimize(s, gen_opc_ptr, tcg->gen_opparam_buf, tcg_op_defs);
#endif

    tcg_liveness_analysis(s);

    tcg_reg_alloc_start(s);
    s->code_buf = gen_code_buf;
    s->code_ptr = gen_code_buf;

    args = tcg->gen_opparam_buf;
    op_index = 0;
    num_insns = -1;

    for(;;) {
        s->current_code = &tcg->gen_opc_buf[op_index];
        opc = s->current_code->opcode;
        def = &tcg_op_defs[opc];
        switch(opc) {
            case INDEX_op_mov_i32:
#if TCG_TARGET_REG_BITS == 64
            case INDEX_op_mov_i64:
#endif
                dead_args = s->op_dead_args[op_index];
                tcg_reg_alloc_mov(s, def, args, dead_args);
                break;
            case INDEX_op_movi_i32:
#if TCG_TARGET_REG_BITS == 64
            case INDEX_op_movi_i64:
#endif
                tcg_reg_alloc_movi(s, args);
                break;
            case INDEX_op_insn_start:
                if(num_insns >= 0) {
                    tcg->gen_insn_end_off[num_insns] = tcg_current_code_size(s);
                }
                num_insns++;
                for(i = 0; i < TARGET_INSN_START_WORDS; ++i) {
                    target_ulong a;
#if TARGET_LONG_BITS > TCG_TARGET_REG_BITS
                    a = ((target_ulong)args[i * 2 + 1] << 32) | args[i * 2];
#else
                    a = args[i];
#endif
                    tcg->gen_insn_data[num_insns][i] = a;
                }
                break;
            case INDEX_op_nop:
            case INDEX_op_nop1:
            case INDEX_op_nop2:
            case INDEX_op_nop3:
                break;
            case INDEX_op_nopn:
                args += args[0];
                goto next;
            case INDEX_op_discard: {
                TCGTemp *ts;
                ts = &s->temps[args[0]];
                /* mark the temporary as dead */
                if(!ts->fixed_reg) {
                    if(ts->val_type == TEMP_VAL_REG) {
                        s->reg_to_temp[ts->reg] = -1;
                    }
                    ts->val_type = TEMP_VAL_DEAD;
                }
            } break;
            case INDEX_op_set_label:
                tcg_reg_alloc_bb_end(s, s->reserved_regs);
                tcg_out_label(s, args[0], (uintptr_t)s->code_ptr);
                break;
            case INDEX_op_call:
                dead_args = s->op_dead_args[op_index];
                args += tcg_reg_alloc_call(s, def, opc, args, dead_args);
                goto next;
            case INDEX_op_end:
                goto the_end;
            default:
                /* Sanity check that we've not introduced any unhandled opcodes. */
                if(def->flags & TCG_OPF_NOT_PRESENT) {
                    tcg_abort();
                }
                /* Note: in order to speed up the code, it would be much
                   faster to have specialized register allocator functions for
                   some common argument patterns */
                dead_args = s->op_dead_args[op_index];
                tcg_reg_alloc_op(s, def, opc, args, dead_args);
                break;
        }
        args += def->nb_args;
    next:
        op_index++;
    }
the_end:
    if(num_insns >= 0) {
        tcg->gen_insn_end_off[num_insns] = tcg_current_code_size(s);
    }
    return -1;
}

int tcg_gen_code(TCGContext *s, uint8_t *gen_code_buf)
{
    tcg_gen_code_common(s, gen_code_buf);

    /* flush instruction cache */
    flush_icache_range((uintptr_t)rw_ptr_to_rx(gen_code_buf), (uintptr_t)rw_ptr_to_rx(s->code_ptr));
    return s->code_ptr - gen_code_buf;
}

#if !TCG_TARGET_MAYBE_vec
void tcg_expand_vec_op(TCGOpcode o, TCGType t, unsigned e, TCGArg a0, ...)
{
    tlib_assert_not_reached();
}
#endif
