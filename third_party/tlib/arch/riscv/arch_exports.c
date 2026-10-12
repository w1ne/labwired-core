/*
 *  RISC-V interface functions
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
#include <stdint.h>
#include "cpu.h"
#include "unwind.h"
#include "arch_exports.h"

void tlib_set_hart_id(uint32_t id)
{
    cpu->mhartid = id;
}

EXC_VOID_1(tlib_set_hart_id, uint32_t, id)

uint32_t tlib_get_hart_id()
{
    return cpu->mhartid;
}

EXC_INT_0(uint32_t, tlib_get_hart_id)

uint32_t tlib_get_current_priv()
{
    return cpu->priv;
}

EXC_INT_0(uint32_t, tlib_get_current_priv)

void tlib_set_mip_bit(uint32_t position, uint32_t value)
{
    pthread_mutex_lock(&cpu->mip_lock);
    //  here we might have a race
    if(value) {
        cpu->mip |= ((target_ulong)1 << position);
    } else {
        cpu->mip &= ~((target_ulong)1 << position);
    }
    pthread_mutex_unlock(&cpu->mip_lock);
}

void handle_interrupt(CPUState *env, int mask);
void tlib_raise_interrupt(uint32_t exception)
{
    tlib_set_mip_bit(exception, 1);
    env->exception_index = exception;
    if(cpu->custom_interrupts & (1UL << exception)) {
        cpu_interrupt(cpu, RISCV_CPU_INTERRUPT_CUSTOM);
    } else {
        cpu_interrupt(cpu, CPU_INTERRUPT_HARD);
    }
}
EXC_VOID_1(tlib_raise_interrupt, uint32_t, exception)

EXC_VOID_2(tlib_set_mip_bit, uint32_t, position, uint32_t, value)

void tlib_set_clic_interrupt_state(int32_t intno, uint32_t vectored, uint32_t level, uint32_t mode)
{
    env->clic_interrupt_pending = intno;
    env->clic_interrupt_vectored = vectored;
    env->clic_interrupt_level = level;
    env->clic_interrupt_priv = mode;

    if(intno != EXCP_NONE) {
        set_interrupt_pending(env, RISCV_CPU_INTERRUPT_CLIC);
    } else {
        clear_interrupt_pending(env, RISCV_CPU_INTERRUPT_CLIC);
    }
}

EXC_VOID_4(tlib_set_clic_interrupt_state, int32_t, intno, uint32_t, vectored, uint32_t, level, uint32_t, mode)

void tlib_allow_feature(uint32_t feature_bit)
{
#if HOST_LONG_BITS == 32
    if(feature_bit == 'V' - 'A') {
        tlib_printf(LOG_LEVEL_ERROR, "Vector extension can't be enabled on 32-bit hosts.");
        return;
    }
#endif

    cpu->misa_mask |= (1L << feature_bit);
    cpu->misa |= (1L << feature_bit);

    if(feature_bit == 'D' - 'A') {
        tlib_allow_feature('F' - 'A');
    }
    if(feature_bit == 'V' - 'A') {
        tlib_allow_additional_feature(RISCV_FEATURE_ZVE64D);
    }
    //  availability of F/D extensions
    //  is indicated by a bit in MSTATUS
    if(feature_bit == 'F' - 'A' || feature_bit == 'D' - 'A') {
        set_default_mstatus();
    }
}

EXC_VOID_1(tlib_allow_feature, uint32_t, feature_bit)

void tlib_allow_additional_feature(uint32_t feature)
{
    if(feature > RISCV_FEATURE_ONE_HIGHER_THAN_HIGHEST_ADDITIONAL - 1) {
        tlib_abort("Invalid architecture set extension.");
        return;
    }

    enum riscv_additional_feature extension = feature;
    switch(extension) {
        case RISCV_FEATURE_ZVE64D:
            tlib_allow_feature('D' - 'A');
            tlib_allow_additional_feature(RISCV_FEATURE_ZVE64F);
            break;
        case RISCV_FEATURE_ZVE64F:
            tlib_allow_additional_feature(RISCV_FEATURE_ZVE32F);
            tlib_allow_additional_feature(RISCV_FEATURE_ZVE64X);
            break;
        case RISCV_FEATURE_ZVE32F:
            tlib_allow_feature('F' - 'A');
            tlib_allow_additional_feature(RISCV_FEATURE_ZVE32X);
            break;
        case RISCV_FEATURE_ZVE64X:
            tlib_allow_additional_feature(RISCV_FEATURE_ZVE32X);
            break;
        case RISCV_FEATURE_ZVE32X:
            tlib_allow_additional_feature(RISCV_FEATURE_ZICSR);
#if HOST_LONG_BITS == 32
            tlib_abort("Vector extension can't be enabled on 32-bit hosts.");
            return;
#else
            break;
#endif
        case RISCV_FEATURE_ZFH:
            tlib_allow_feature('F' - 'A');
            break;
        case RISCV_FEATURE_ZVFH:
            tlib_allow_additional_feature(RISCV_FEATURE_ZFH);  //  It should depends precisely on Zfhmin that isn't supported.
            tlib_allow_additional_feature(RISCV_FEATURE_ZVE32F);
            break;
        case RISCV_FEATURE_SMEPMP:
        case RISCV_FEATURE_SSCOFPMF:
            tlib_allow_additional_feature(RISCV_FEATURE_ZICSR);
            break;
        case RISCV_FEATURE_ZBA:
        case RISCV_FEATURE_ZBB:
        case RISCV_FEATURE_ZBC:
        case RISCV_FEATURE_ZBS:
        case RISCV_FEATURE_ZBKB:
        case RISCV_FEATURE_ZICSR:
        case RISCV_FEATURE_ZIFENCEI:
        case RISCV_FEATURE_ZACAS:
        case RISCV_FEATURE_ZICOND:
            //  No dependencies
            break;
        case RISCV_FEATURE_ZCB:
        case RISCV_FEATURE_ZCMP:
        case RISCV_FEATURE_ZCMT:
            tlib_allow_feature('C' - 'A');  //  Depends on RVC
            break;
        case RISCV_FEATURE_ONE_HIGHER_THAN_HIGHEST_ADDITIONAL:
            //  We should never reach here, as we are gated by if check
            tlib_assert_not_reached();
            break;
    }
    cpu->additional_extensions |= 1U << extension;
}

EXC_VOID_1(tlib_allow_additional_feature, uint32_t, feature_bit)

void tlib_mark_feature_silent(uint32_t feature_bit, uint32_t value)
{
    if(value) {
        cpu->silenced_extensions |= (1L << feature_bit);
    } else {
        cpu->silenced_extensions &= ~(1L << feature_bit);
    }
}

EXC_VOID_2(tlib_mark_feature_silent, uint32_t, feature_bit, uint32_t, value)

uint32_t tlib_is_feature_enabled(uint32_t feature_bit)
{
    return (cpu->misa & (1L << feature_bit)) != 0;
}

EXC_INT_1(uint32_t, tlib_is_feature_enabled, uint32_t, feature_bit)

uint32_t tlib_is_feature_allowed(uint32_t feature_bit)
{
    return (cpu->misa_mask & (1L << feature_bit)) != 0;
}

EXC_INT_1(uint32_t, tlib_is_feature_allowed, uint32_t, feature_bit)

uint32_t tlib_is_additional_feature_enabled(uint32_t feature_bit)
{
    return (cpu->additional_extensions & (1L << feature_bit)) != 0;
}

EXC_INT_1(uint32_t, tlib_is_additional_feature_enabled, uint32_t, feature_bit)

void tlib_set_privilege_architecture(int32_t privilege_architecture)
{
    if(privilege_architecture > RISCV_PRIV1_12 && privilege_architecture != RISCV_PRIV_UNRATIFIED) {
        tlib_abort("Invalid privilege architecture set. Highest supported version is 1.12");
    }
    cpu->privilege_architecture = privilege_architecture;
}

EXC_VOID_1(tlib_set_privilege_architecture, int32_t, privilege_architecture)

uint64_t tlib_install_custom_instruction(uint64_t mask, uint64_t pattern, uint64_t length)
{
    if(cpu->custom_instructions_count == CPU_CUSTOM_INSTRUCTIONS_LIMIT) {
        //  no more empty slots
        return 0;
    }

    custom_instruction_descriptor_t *ci = &cpu->custom_instructions[cpu->custom_instructions_count];

    ci->id = ++cpu->custom_instructions_count;
    ci->mask = mask;
    ci->pattern = pattern;
    ci->length = length;

    return ci->id;
}

EXC_INT_3(uint64_t, tlib_install_custom_instruction, uint64_t, mask, uint64_t, pattern, uint64_t, length)

int32_t tlib_install_custom_csr(uint16_t id)
{
    if(id > MAX_CSR_ID) {
        return -1;
    }

    int slotId = id / CSRS_PER_SLOT;
    int slotOffset = id % CSRS_PER_SLOT;

    cpu->custom_csrs[slotId] |= (1 << slotOffset);

    return 0;
}

EXC_INT_1(int32_t, tlib_install_custom_csr, uint16_t, id)

void helper_wfi(CPUState *env);
void tlib_enter_wfi()
{
    helper_wfi(cpu);
}

EXC_VOID_0(tlib_enter_wfi)

int32_t tlib_install_custom_interrupt(uint8_t id, bool mip_trigger, bool sip_trigger)
{
#if defined(TARGET_RISCV64)
    tlib_assert(id < 64);
#else
    tlib_assert(id < 32);
#endif
    target_ulong bit = ((target_ulong)1) << id;

    target_ulong all_ints = IRQ_MS | IRQ_MT | IRQ_ME | IRQ_SS | IRQ_ST | IRQ_SE;
    if(all_ints & bit) {
        //  Don't install a custom interrupt with the same id as a standard one
        tlib_printf(LOG_LEVEL_ERROR, "Custom interrupt with id: %d has the same id as a standard one", id);
        return -1;
    }
    if(cpu->custom_interrupts & bit) {
        tlib_printf(LOG_LEVEL_WARNING, "Custom interrupt with id: %d is already registered", id);
    }

    cpu->custom_interrupts |= bit;
    if(mip_trigger) {
        cpu->mip_triggered_custom_interrupts |= bit;
    }
    if(sip_trigger) {
        cpu->sip_triggered_custom_interrupts |= bit;
    }

    return 0;
}

EXC_INT_3(int32_t, tlib_install_custom_interrupt, uint8_t, id, bool, m_trigger, bool, s_trigger)

void tlib_enable_pre_stack_access_hook(bool enable)
{
    if(cpu->is_pre_stack_access_hook_enabled != enable) {
        cpu->is_pre_stack_access_hook_enabled = enable;
        tb_flush(env);
    }
}

EXC_VOID_1(tlib_enable_pre_stack_access_hook, bool, enable)

void tlib_set_csr_validation_level(uint32_t value)
{
    switch(value) {
        case CSR_VALIDATION_FULL:
        case CSR_VALIDATION_PRIV:
        case CSR_VALIDATION_NONE:
            cpu->csr_validation_level = value;
            break;

        default:
            tlib_abortf("Unexpected CSR validation level: %d", value);
    }
}

EXC_VOID_1(tlib_set_csr_validation_level, uint32_t, value)

uint32_t tlib_get_csr_validation_level()
{
    return cpu->csr_validation_level;
}

EXC_INT_0(uint32_t, tlib_get_csr_validation_level)

void tlib_set_nmi_vector(uint64_t nmi_adress, uint32_t nmi_length)
{
    if(nmi_adress > (TARGET_ULONG_MAX - nmi_length)) {
        cpu_abort(cpu, "NMIVectorAddress or NMIVectorLength value invalid. "
                       "Vector defined with these parameters will not fit in memory address space.");
    } else {
        cpu->nmi_address = (target_ulong)nmi_adress;
    }
    if(nmi_length > 32) {
        cpu_abort(cpu, "NMIVectorLength %d too big, maximum length supported is 32", nmi_length);
    } else {
        cpu->nmi_length = nmi_length;
    }
}

EXC_VOID_2(tlib_set_nmi_vector, uint64_t, nmi_adress, uint32_t, nmi_length)

void tlib_set_nmi(int32_t nmi, int32_t state, uint64_t mcause)
{
    if(state) {
        cpu_set_nmi(cpu, nmi, (target_ulong)mcause);
    } else {
        cpu_reset_nmi(cpu, nmi);
    }
}

EXC_VOID_3(tlib_set_nmi, int32_t, nmi, int32_t, state, uint64_t, mcause)

void tlib_allow_unaligned_accesses(int32_t allowed)
{
    cpu->allow_unaligned_accesses = allowed;
}

EXC_VOID_1(tlib_allow_unaligned_accesses, int32_t, allowed)

void tlib_set_interrupt_mode(int32_t mode)
{
    target_ulong new_value;

    switch(mode) {
        case INTERRUPT_MODE_AUTO:
            break;

        case INTERRUPT_MODE_DIRECT:
            new_value = (cpu->mtvec & ~MTVEC_MODE);
            if(cpu->mtvec != new_value) {
                tlib_printf(LOG_LEVEL_WARNING, "Direct interrupt mode set - updating MTVEC from 0x%x to 0x%x", cpu->mtvec,
                            new_value);
                cpu->mtvec = new_value;
            }

            new_value = (cpu->stvec & ~MTVEC_MODE);
            if(cpu->stvec != new_value) {
                tlib_printf(LOG_LEVEL_WARNING, "Direct interrupt mode set - updating STVEC from 0x%x to 0x%x", cpu->stvec,
                            new_value);
                cpu->stvec = new_value;
            }
            break;

        case INTERRUPT_MODE_VECTORED:
            if(cpu->privilege_architecture < RISCV_PRIV1_10) {
                tlib_abortf("Vectored interrupt mode not supported in the selected privilege architecture");
            }

            new_value = (cpu->mtvec & ~MTVEC_MODE) | MTVEC_MODE_CLINT_VECTORED;
            if(cpu->mtvec != new_value) {
                tlib_printf(LOG_LEVEL_WARNING, "Vectored interrupt mode set - updating MTVEC from 0x%x to 0x%x", cpu->mtvec,
                            new_value);
                cpu->mtvec = new_value;
            }

            new_value = (cpu->stvec & ~MTVEC_MODE) | MTVEC_MODE_CLINT_VECTORED;
            if(cpu->stvec != new_value) {
                tlib_printf(LOG_LEVEL_WARNING, "Vectored interrupt mode set - updating STVEC from 0x%x to 0x%x", cpu->stvec,
                            new_value);
                cpu->stvec = new_value;
            }

            break;

        default:
            tlib_abortf("Unexpected interrupt mode: %d", mode);
            return;
    }

    cpu->interrupt_mode = mode;
}

EXC_VOID_1(tlib_set_interrupt_mode, int32_t, mode)

uint32_t tlib_set_vlen(uint32_t vlen)
{
    //  a power of 2 and not greater than VLEN_MAX
    if(!is_power_of_2(vlen) || vlen > VLEN_MAX || vlen < cpu->elen) {
        return 1;
    }
    cpu->vlenb = vlen / 8;
    return 0;
}

EXC_INT_1(uint32_t, tlib_set_vlen, uint32_t, vlen)

uint32_t tlib_set_elen(uint32_t elen)
{
    //  a power of 2 and greater or equal to 8
    //  current implementation puts upper bound of 64
    if(!is_power_of_2(elen) || elen < 8 || elen > 64 || elen > (env->vlenb << 3)) {
        return 1;
    }
    cpu->elen = elen;
    return 0;
}

EXC_INT_1(uint32_t, tlib_set_elen, uint32_t, elen)

void tlib_set_pmpaddr_bits(uint32_t number_of_bits)
{
    const uint32_t maximum_number_of_bits =
#if defined(TARGET_RISCV64)
        //  Top 10 bits are WARL
        54;
#else
        32;
#endif

    if(number_of_bits > maximum_number_of_bits || number_of_bits == 0) {
        tlib_abortf("Unsupported number of PMPADDR bits %" PRIu32 " expected between 1 and %" PRIu32 ", inclusive",
                    number_of_bits, maximum_number_of_bits);
    }
    cpu->pmp_addr_mask = ((uint64_t)1 << (uint64_t)number_of_bits) - 1;
}

EXC_VOID_1(tlib_set_pmpaddr_bits, uint32_t, number_of_bits)

void tlib_enable_external_pmp(bool value)
{
    if(value != cpu->use_external_pmp) {
        cpu->use_external_pmp = value;
        tlb_flush(cpu, /* flush_global: */ 1, /* from_generated_code: */ false);
    }
}

EXC_VOID_1(tlib_enable_external_pmp, bool, value)

static_assert(MAX_RISCV_PMPS >= 64, "Pre-defined PMP entry count must not be lesser than max settable count");
void tlib_set_pmp_entry_count(uint32_t entry_count)
{
    if(entry_count != 0 && entry_count != 16 && entry_count != 64) {
        tlib_abortf("Unexpected PMP entry count %d, only 0, 16, and 64 are allowed", entry_count);
        return;
    }
    cpu->pmp_entry_count = entry_count;
}

EXC_VOID_1(tlib_set_pmp_entry_count, uint32_t, max_entry_count)

void tlib_set_pmpaddr(uint32_t index, uint64_t start_address, uint64_t end_address)
{
    if(index < cpu->pmp_entry_count) {
        cpu->pmp_state.addr[index].sa = start_address & ((cpu->pmp_addr_mask << 2) | 0x3);
        cpu->pmp_state.addr[index].ea = end_address & ((cpu->pmp_addr_mask << 2) | 0x3);
    } else {
        tlib_printf(LOG_LEVEL_ERROR, "Tried to set the address of PMP entry %u but the maximum index is %u, write ignored", index,
                    cpu->pmp_entry_count - 1);
    }
}
EXC_VOID_3(tlib_set_pmpaddr, uint32_t, index, uint64_t, start_address, uint64_t, end_address)

static bool check_vector_register_number(uint32_t regn)
{
    if(regn >= 32) {
        tlib_printf(LOG_LEVEL_ERROR, "Vector register number out of bounds");
        return 1;
    }
    return 0;
}

static bool check_vector_access(uint32_t regn, uint32_t idx)
{
    if(check_vector_register_number(regn)) {
        return 1;
    }
    if(regn >= 32) {
        tlib_printf(LOG_LEVEL_ERROR, "Vector register number out of bounds");
        return 1;
    }
    if(V_IDX_INVALID(regn)) {
        tlib_printf(LOG_LEVEL_ERROR, "Invalid vector register number (not divisible by LMUL=%d)", cpu->vlmul);
        return 1;
    }
    if(idx >= cpu->vlmax) {
        tlib_printf(LOG_LEVEL_ERROR, "Vector element index out of bounds (VLMAX=%d)", cpu->vlmax);
        return 1;
    }
    return 0;
}

uint64_t tlib_get_vector(uint32_t regn, uint32_t idx)
{
    if(check_vector_access(regn, idx)) {
        return 0;
    }

    switch(cpu->vsew) {
        case 8:
            return ((uint8_t *)V(regn))[idx];
        case 16:
            return ((uint16_t *)V(regn))[idx];
        case 32:
            return ((uint32_t *)V(regn))[idx];
        case 64:
            return ((uint64_t *)V(regn))[idx];
        default:
            tlib_printf(LOG_LEVEL_ERROR, "Unsupported SEW (%d)", cpu->vsew);
            return 0;
    }
}

EXC_INT_2(uint64_t, tlib_get_vector, uint32_t, regn, uint32_t, idx)

void tlib_set_vector(uint32_t regn, uint32_t idx, uint64_t value)
{
    if(check_vector_access(regn, idx)) {
        return;
    }
    if(value >> cpu->vsew) {
        tlib_printf(LOG_LEVEL_ERROR, "`value` (0x%llx) won't fit in vector element with SEW=%d", value, cpu->vsew);
        return;
    }

    switch(cpu->vsew) {
        case 8:
            ((uint8_t *)V(regn))[idx] = value;
            break;
        case 16:
            ((uint16_t *)V(regn))[idx] = value;
            break;
        case 32:
            ((uint32_t *)V(regn))[idx] = value;
            break;
        case 64:
            ((uint64_t *)V(regn))[idx] = value;
            break;
        default:
            tlib_printf(LOG_LEVEL_ERROR, "Unsupported SEW (%d)", cpu->vsew);
    }
}

EXC_VOID_3(tlib_set_vector, uint32_t, regn, uint32_t, idx, uint64_t, value)

uint32_t tlib_get_whole_vector(uint32_t regn, uint8_t *bytes)
{
    if(check_vector_register_number(regn)) {
        return 1;
    } else {
        memcpy(bytes, V(regn), env->vlenb);
        return 0;
    }
}

EXC_INT_2(uint32_t, tlib_get_whole_vector, uint32_t, regn, uint8_t *, bytes)

uint32_t tlib_set_whole_vector(uint32_t regn, uint8_t *bytes)
{
    if(check_vector_register_number(regn)) {
        return 1;
    } else {
        memcpy(V(regn), bytes, env->vlenb);
        return 0;
    }
}

EXC_INT_2(uint32_t, tlib_set_whole_vector, uint32_t, regn, uint8_t *, bytes)

void tlib_enable_post_gpr_access_hooks(uint32_t value)
{
    env->are_post_gpr_access_hooks_enabled = !!value;
    tb_flush(env);
}

EXC_VOID_1(tlib_enable_post_gpr_access_hooks, uint32_t, value)

void tlib_enable_post_gpr_access_hook_on(uint32_t register_index, uint32_t value)
{
    if(register_index > 31) {
        tlib_abort("Unable to add GPR access hook on register with index higher than 31");
    }
    if(value) {
        env->post_gpr_access_hook_mask |= 1 << register_index;
    } else {
        env->post_gpr_access_hook_mask &= ~(1u << register_index);
    }
}

EXC_VOID_2(tlib_enable_post_gpr_access_hook_on, uint32_t, register_index, uint32_t, value)

void tlib_set_napot_grain(uint32_t minimal_napot_in_bytes)
{
    if(((minimal_napot_in_bytes & (minimal_napot_in_bytes - 1)) != 0) && (minimal_napot_in_bytes >= 8)) {
        tlib_abort("PMP NAPOT size must be a power of 2 and larger than 4");
    }
    int grain = minimal_napot_in_bytes >> 4;
    env->pmp_napot_grain = grain;
}

EXC_VOID_1(tlib_set_napot_grain, uint32_t, minimal_napot_in_bytes)
