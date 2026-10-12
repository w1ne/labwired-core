/*
 * Copyright (c) 2011, Max Filippov, Open Source and Linux Lab.
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the Open Source and Linux Lab nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

#include "cpu.h"
#include "osdep.h"
#include "tb-helper.h"
#include "ttable.h"

extern CPUState *env;

typedef int MMUAccessType;

#define MAX_TTABLE_SIZE 4096

static TTable *hash_opcode_translators(const XtensaOpcodeTranslators *t)
{
    unsigned i, j;
    TTable *translator = ttable_create(MAX_TTABLE_SIZE, NULL, ttable_compare_key_string);

    for(i = 0; i < t->num_opcodes; ++i) {
        if(t->opcode[i].op_flags & XTENSA_OP_NAME_ARRAY) {
            const char *const *name = t->opcode[i].name;

            for(j = 0; name[j]; ++j) {
                if(!ttable_insert_check(translator, (void *)name[j], (void *)(t->opcode + i))) {
                    tlib_abortf("Translators: Multiple definitions of '%s' opcode in a single table", name[j]);
                }
            }
        } else {
            if(!ttable_insert_check(translator, (void *)t->opcode[i].name, (void *)(t->opcode + i))) {
                tlib_abortf("Translators: Multiple definitions of '%s' opcode in a single table", t->opcode[i].name);
            }
        }
    }

    ttable_sort_by_keys(translator);
    return translator;
}

static TTable *translators = NULL;

static XtensaOpcodeOps *xtensa_find_opcode_ops(const XtensaOpcodeTranslators *t, const char *name)
{
    TTable *translator;

    if(translators == NULL) {
        translators = ttable_create(MAX_TTABLE_SIZE, NULL, ttable_compare_key_pointer);
    }
    translator = ttable_lookup_value_eq(translators, (void *)t);
    if(translator == NULL) {
        translator = hash_opcode_translators(t);

        ttable_insert(translators, (void *)t, translator);
    }
    return (XtensaOpcodeOps *)ttable_lookup_value_eq(translator, (void *)name);
}

static void init_libisa(XtensaConfig *config)
{
    unsigned i, j;
    unsigned opcodes;
    unsigned formats;
    unsigned regfiles;

    config->isa = xtensa_isa_init(config->isa_internal, NULL, NULL);
    assert(xtensa_isa_maxlength(config->isa) <= MAX_INSN_LENGTH);
    assert(xtensa_insnbuf_size(config->isa) <= MAX_INSNBUF_LENGTH);
    opcodes = xtensa_isa_num_opcodes(config->isa);
    formats = xtensa_isa_num_formats(config->isa);
    regfiles = xtensa_isa_num_regfiles(config->isa);
    config->opcode_ops = malloc(sizeof(XtensaOpcodeOps *) * opcodes);

    for(i = 0; i < formats; ++i) {
        assert(xtensa_format_num_slots(config->isa, i) <= MAX_INSN_SLOTS);
    }

    for(i = 0; i < opcodes; ++i) {
        const char *opc_name = xtensa_opcode_name(config->isa, i);
        XtensaOpcodeOps *ops = NULL;

        assert(xtensa_opcode_num_operands(config->isa, i) <= MAX_OPCODE_ARGS);
        if(!config->opcode_translators) {
            ops = xtensa_find_opcode_ops(&xtensa_core_opcodes, opc_name);
        } else {
            for(j = 0; !ops && config->opcode_translators[j]; ++j) {
                ops = xtensa_find_opcode_ops(config->opcode_translators[j], opc_name);
            }
        }
#ifdef DEBUG
        if(ops == NULL) {
            tlib_printf(LOG_LEVEL_WARNING, "opcode translator not found for %s's opcode '%s'\n", config->name, opc_name);
        }
#endif
        config->opcode_ops[i] = ops;
    }
    ttable_sort_by_keys(translators);

    config->a_regfile = xtensa_regfile_lookup(config->isa, "AR");

    config->regfile = malloc(sizeof(int *) * regfiles);
    for(i = 0; i < regfiles; ++i) {
        const char *name = xtensa_regfile_name(config->isa, i);
        int entries = xtensa_regfile_num_entries(config->isa, i);
        int bits = xtensa_regfile_num_bits(config->isa, i);

        config->regfile[i] = xtensa_get_regfile_by_name(name, entries, bits);
#ifdef DEBUG
        if(config->regfile[i] == NULL) {
            tlib_printf(LOG_LEVEL_WARNING, "regfile '%s' not found for %s\n", name, config->name);
        }
#endif
    }
    xtensa_collect_sr_names(config);
}

XtensaConfig *xtensa_finalize_config(const char *core_name)
{
    XtensaConfig *config = NULL;

#define TRY_SET_CONFIG(X)              \
    if(strcmp(core_name, X.name) == 0) \
        config = &X;
    TRY_SET_CONFIG(apollolake)
    else TRY_SET_CONFIG(baytrail) else TRY_SET_CONFIG(cannonlake) else TRY_SET_CONFIG(dc233c) else TRY_SET_CONFIG(de212) else TRY_SET_CONFIG(de233_fpu) else TRY_SET_CONFIG(dsp3400) else TRY_SET_CONFIG(esp32) else TRY_SET_CONFIG(esp32s2) else TRY_SET_CONFIG(
        esp32s3) else TRY_SET_CONFIG(haswell) else TRY_SET_CONFIG(icelake) else TRY_SET_CONFIG(imx8) else TRY_SET_CONFIG(imx8m) else TRY_SET_CONFIG(sample_controller) else TRY_SET_CONFIG(test_kc705_be) else TRY_SET_CONFIG(test_mmuhifi_c3) else TRY_SET_CONFIG(tigerlake) else
    {
        tlib_abortf("Invalid Xtensa core name: %s", core_name);
        __builtin_unreachable();
    }

    if(config && config->isa_internal) {
        init_libisa(config);
    }
    return config;
}

void do_unaligned_access(target_ulong addr, MMUAccessType access_type, int mmu_idx, uintptr_t retaddr)
{
    if(xtensa_option_enabled(env->config, XTENSA_OPTION_UNALIGNED_EXCEPTION) &&
       !xtensa_option_enabled(env->config, XTENSA_OPTION_HW_ALIGNMENT)) {
        cpu_restore_state(env, (void *)retaddr);
        HELPER(exception_cause_vaddr)(env, env->pc, LOAD_STORE_ALIGNMENT_CAUSE, addr);
    }
}

void arch_raise_mmu_fault_exception(CPUState *env, int errcode, int access_type, target_ulong address, void *retaddr)
{
    cpu_restore_state(env, (void *)retaddr);
    HELPER(exception_cause_vaddr)(env, env->pc, errcode, address);
}

int arch_tlb_fill(CPUState *env, target_ulong address, MMUAccessType access_type, int mmu_idx, void *retaddr, int no_page_fault,
                  int access_width, target_phys_addr_t *out_paddr)
{
    uint32_t paddr;
    uint32_t page_size;
    int access, ret;

    ret = get_physical_address(env, true, address, access_type, mmu_idx, &paddr, &page_size, &access);
#if DEBUG
    tlib_printf(LOG_LEVEL_DEBUG, "%s(%08x, %d, %d) -> %08x, ret = %d\n", __func__, address, access_type, mmu_idx, paddr, ret);
#endif

    if(ret == TRANSLATE_SUCCESS) {
        *out_paddr = paddr;
        tlb_set_page(env, address & TARGET_PAGE_MASK, paddr & TARGET_PAGE_MASK, access, mmu_idx, page_size);
        return TRANSLATE_SUCCESS;
    }
    return TRANSLATE_FAIL;
}

void tlib_arch_dispose()
{
    //  TODO: do we want to put sth here? Probably not - there are no mallocs in our code
}
