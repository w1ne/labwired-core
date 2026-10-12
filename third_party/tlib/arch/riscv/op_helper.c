/*
 *  RISC-V emulation helpers
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
#include "cpu.h"
#include "pmp.h"
#define ALIGNED_ONLY
#include "softmmu_exec.h"

//  Vector helpers require 128-bit ints which aren't supported on 32-bit hosts.
#if HOST_LONG_BITS != 32

//  Note that MASKED is not defined for the 2nd include
#define MASKED
#define SHIFT 0
#include "vector_helper_template.h"
#define SHIFT 0
#include "vector_helper_template.h"

#define MASKED
#define SHIFT 1
#include "vector_helper_template.h"
#define SHIFT 1
#include "vector_helper_template.h"

#define MASKED
#define SHIFT 2
#include "vector_helper_template.h"
#define SHIFT 2
#include "vector_helper_template.h"

#define MASKED
#define SHIFT 3
#include "vector_helper_template.h"
#define SHIFT 3
#include "vector_helper_template.h"

#endif  //  HOST_LONG_BITS != 32

#include "arch_callbacks.h"
#include "global_helper.h"

#if defined(TARGET_RISCV32)
static const char valid_vm_1_09[16] = {
    [VM_1_09_MBARE] = 1,
    [VM_1_09_SV32] = 1,
};
static const char valid_vm_1_10[16] = { [VM_1_10_MBARE] = 1, [VM_1_10_SV32] = 1 };
#elif defined(TARGET_RISCV64)
static const char valid_vm_1_09[16] = {
    [VM_1_09_MBARE] = 1,
    [VM_1_09_SV39] = 1,
    [VM_1_09_SV48] = 1,
};
static const char valid_vm_1_10[16] = { [VM_1_10_MBARE] = 1, [VM_1_10_SV39] = 1, [VM_1_10_SV48] = 1, [VM_1_10_SV57] = 1 };
#endif

static int validate_vm(CPUState *env, target_ulong vm)
{
    return (env->privilege_architecture >= RISCV_PRIV1_10) ? valid_vm_1_10[vm & 0xf] : valid_vm_1_09[vm & 0xf];
}

static target_ulong validate_mpp_setting(CPUState *env, target_ulong value_to_write)
{
    int mpp_to_write = get_field(value_to_write, MSTATUS_MPP);
    bool allowed;
    switch(mpp_to_write) {
        case PRV_U:
            allowed = riscv_has_ext(env, RISCV_FEATURE_RVU);
            break;
        case PRV_S:
            allowed = riscv_has_ext(env, RISCV_FEATURE_RVS);
            break;
        case PRV_M:
            allowed = true;
            break;
        default:
            allowed = false;
    }

    if(!allowed) {
        //  Keep the old value instead
        value_to_write &= ~MSTATUS_MPP;
        value_to_write |= env->mstatus & MSTATUS_MPP;
    }
    return value_to_write;
}

static inline target_ulong get_counter_enabled_mask(CPUState *env)
{
    if(env->priv == PRV_H) {
        tlib_abort("H-mode is currently not supported");
        return 0;
    }

    if(env->privilege_architecture >= RISCV_PRIV1_10) {
        switch(env->priv) {
            case PRV_M:
                return -1U;  //  All counters are available in M-mode
            case PRV_S:
                return env->mcounteren;
            case PRV_U:
                if(riscv_has_ext(env, RISCV_FEATURE_RVS)) {
                    return env->mcounteren & env->scounteren;
                }
                return env->mcounteren;
            default:
                tlib_abortf("Invalid privilege level: %d", env->priv);
                return 0;
        }
    } else {
        switch(env->priv) {
            case PRV_M:
                return -1U;  //  All counters are available in M-mode
            case PRV_S:
                return env->mscounteren;
            case PRV_U:
                return env->mucounteren;
            default:
                tlib_abortf("Invalid privilege level: %d", env->priv);
                return 0;
        }
    }
}

static inline bool is_counter_csr(target_ulong csrno)
{
    switch(csrno) {
        case CSR_CYCLE ... CSR_HPMCOUNTER31:
        case CSR_MCYCLE ... CSR_MHPMCOUNTER31:
#if defined(TARGET_RISCV32)
        case CSR_CYCLEH ... CSR_HPMCOUNTER31H:
        case CSR_MCYCLEH ... CSR_MHPMCOUNTER31H:
#endif
            return true;
        default:
            return false;
    }
}

static inline uint64_t cpu_riscv_read_instret(CPUState *env)
{
    uint64_t retval = env->instructions_count_total_value;
    return retval;
}

/* Exceptions processing helpers */
static inline void __attribute__((__noreturn__)) do_raise_exception_err(CPUState *env, uint32_t exception, uintptr_t pc,
                                                                        uint32_t call_hook)
{
    env->exception_index = exception;
    cpu_loop_exit_restore(env, pc, call_hook);
}

void helper_raise_exception(CPUState *env, uint32_t exception)
{
    do_raise_exception_err(env, exception, 0, 1);
}

void helper_raise_exception_debug(CPUState *env)
{
    do_raise_exception_err(env, EXCP_DEBUG, 0, 1);
}

void helper_raise_exception_mbadaddr(CPUState *env, uint32_t exception, target_ulong bad_pc)
{
    env->badaddr = bad_pc;
    do_raise_exception_err(env, exception, 0, 1);
}

void helper_raise_illegal_instruction(CPUState *env)
{
    env->badaddr = env->opcode;
    do_raise_exception_err(env, RISCV_EXCP_ILLEGAL_INST, 0, 1);
}

static inline void validate_vector_state(CPUState *env)
{
    if(get_field(env->mstatus, MSTATUS_VS) == MSTATUS_VS_OFF) {
        helper_raise_illegal_instruction(env);
    }
}

static inline uint64_t get_minstret_current(CPUState *env)
{
    if(env->privilege_architecture >= RISCV_PRIV1_11 && (env->mcountinhibit & MCOUNTINHIBIT_IR)) {
        return env->minstret_snapshot_offset > 0 ? env->minstret_snapshot_offset : 0;
    }
    return cpu_riscv_read_instret(env) - env->minstret_snapshot + env->minstret_snapshot_offset;
}

static inline uint64_t get_mcycles_current(CPUState *env)
{
    if(env->privilege_architecture >= RISCV_PRIV1_11 && (env->mcountinhibit & MCOUNTINHIBIT_CY)) {
        return env->mcycle_snapshot_offset > 0 ? env->mcycle_snapshot_offset : 0;
    }
    uint64_t instructions = cpu_riscv_read_instret(env) - env->mcycle_snapshot + env->mcycle_snapshot_offset;
    return instructions_to_cycles(env, instructions);
}

target_ulong priv_version_csr_filter(CPUState *env, target_ulong csrno)
{
    if(env->privilege_architecture == RISCV_PRIV1_11) {
        switch(csrno) {
            /* CSR_MUCOUNTEREN register is now used for CSR_MCOUNTINHIBIT */
            case CSR_MSCOUNTEREN:
                tlib_printf(LOG_LEVEL_WARNING, "The csr mscounteren is not present in privilege architecture version 1.11");
                return CSR_UNHANDLED;
        }
    } else if(env->privilege_architecture == RISCV_PRIV1_10) {
        switch(csrno) {
            case CSR_MUCOUNTEREN:
            case CSR_MSCOUNTEREN:
                tlib_printf(LOG_LEVEL_WARNING, "The csr ms/ucounteren is not present in privilege architecture version 1.10");
                return CSR_UNHANDLED;
        }
    } else if(env->privilege_architecture == RISCV_PRIV1_09) {
        switch(csrno) {
            case CSR_SCOUNTEREN:
            case CSR_MCOUNTEREN:
            case CSR_PMPCFG0 ... CSR_PMPCFG_LAST:
            case CSR_PMPADDR0 ... CSR_PMPADDR_LAST:
            case CSR_MHPMEVENT3 ... CSR_MHPMEVENT31:
                tlib_printf(LOG_LEVEL_WARNING, "CSR with id #%i is not available in privilege architecture version 1.09", csrno);
                return CSR_UNHANDLED;
        }
    }
    return csrno;
}

static target_ulong mtvec_stvec_write_handler(target_ulong val_to_write, char *register_name)
{
    target_ulong new_value = 0;

    bool is_before_clic = env->privilege_architecture < RISCV_PRIV_UNRATIFIED;
    if(((env->privilege_architecture >= RISCV_PRIV1_10 && is_before_clic) && (val_to_write & 0x2)) ||
       (env->privilege_architecture < RISCV_PRIV1_10 && (val_to_write & 0x3))) {
        tlib_printf(LOG_LEVEL_WARNING, "Trying to set unaligned %s: 0x%X, aligning to 4-byte boundary.", register_name,
                    val_to_write);
    }

    switch(cpu->interrupt_mode) {
        case INTERRUPT_MODE_AUTO:
            if(env->privilege_architecture >= RISCV_PRIV_UNRATIFIED) {
                new_value = val_to_write;
            } else if(env->privilege_architecture >= RISCV_PRIV1_10) {
                new_value = val_to_write & ~0x2;
            } else {
                new_value = val_to_write & ~MTVEC_MODE;
            }
            break;

        case INTERRUPT_MODE_DIRECT:
            new_value = val_to_write & ~MTVEC_MODE;
            break;

        case INTERRUPT_MODE_VECTORED:
            new_value = (val_to_write & ~MTVEC_MODE) | MTVEC_MODE_CLINT_VECTORED;
            break;

        default:
            tlib_abortf("Unexpected interrupt mode: %d", cpu->interrupt_mode);
    }

    if(new_value != val_to_write) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "%s value written to CSR corrected from 0x%x to 0x%x because of the selected interrupt mode and privilege "
                    "architecture",
                    register_name, val_to_write, new_value);
    }

    return new_value;
}

static target_ulong mnxti_read_handler(target_ulong src)
{
    uint32_t clic_level = env->clic_interrupt_level;
    if(env->clic_interrupt_pending != EXCP_NONE && env->clic_interrupt_priv == PRV_M &&
       clic_level > get_field(env->mcause, MCAUSE_MPIL) && clic_level > get_field(env->mintthresh, MINTTHRESH_TH) &&
       !env->clic_interrupt_vectored) {
        if(src) {
            csr_write_helper(env, set_field(env->mintstatus, MINTSTATUS_MIL, clic_level), CSR_MINTSTATUS);
            target_ulong mc = env->mcause;
            mc = set_field(mc, MCAUSE_EXCCODE, env->clic_interrupt_pending);
            mc |= MCAUSE_INTERRUPT;
            csr_write_helper(env, mc, CSR_MCAUSE);
            tlib_clic_clear_edge_interrupt();
        }
        return env->mtvt + env->clic_interrupt_pending * sizeof(target_ulong);
    }
    return 0;
}

static target_ulong snxti_read_handler(target_ulong src)
{
    uint32_t clic_level = env->clic_interrupt_level;
    if(env->clic_interrupt_pending != EXCP_NONE && env->clic_interrupt_priv == PRV_S &&
       clic_level > get_field(env->scause, SCAUSE_SPIL) && clic_level > get_field(env->sintthresh, SINTTHRESH_TH) &&
       !env->clic_interrupt_vectored) {
        if(src) {
            csr_write_helper(env, set_field(env->mintstatus, MINTSTATUS_SIL, clic_level), CSR_MINTSTATUS);
            target_ulong sc = env->scause;
            sc = set_field(sc, SCAUSE_EXCCODE, env->clic_interrupt_pending);
            sc |= SCAUSE_INTERRUPT;
            csr_write_helper(env, sc, CSR_SCAUSE);
            tlib_clic_clear_edge_interrupt();
        }
        return env->stvt + env->clic_interrupt_pending * sizeof(target_ulong);
    }
    return 0;
}

static inline void warn_nonexistent_csr_read(const int no)
{
    if(no == CSR_UNHANDLED) {
        tlib_printf(LOG_LEVEL_WARNING, "Trying to read from a non-exsistent CSR");
    } else {
        tlib_printf(LOG_LEVEL_WARNING, "Reading from CSR #%d that is not implemented.", no);
    }
}
static inline void warn_nonexistent_csr_write(const int no, const target_ulong val)
{
    if(no == CSR_UNHANDLED) {
        tlib_printf(LOG_LEVEL_WARNING, "Trying to write value 0x%X to a non-exsistent CSR", val);
    } else {
        tlib_printf(LOG_LEVEL_WARNING, "Writing value 0x%X to CSR #%d that is not implemented.", val, no);
    }
}

uint32_t has_custom_csr(CPUState *env, uint64_t id)
{
    if(id > MAX_CSR_ID) {
        return 0;
    }

    int slotId = id / CSRS_PER_SLOT;
    int slotOffset = id % CSRS_PER_SLOT;

    return !!(env->custom_csrs[slotId] & (1 << slotOffset));
}

/*
 * Handle writes to CSRs and any resulting special behavior
 *
 * Adapted from Spike's processor_t::set_csr
 */
inline void csr_write_helper(CPUState *env, target_ulong val_to_write, target_ulong csrno)
{
    uint64_t delegable_ints = IRQ_SS | IRQ_ST | IRQ_SE | ~((1 << 12) - 1);  //  all local interrupts are delegable as well
    uint64_t all_ints = delegable_ints | IRQ_MS | IRQ_MT | IRQ_ME;

    csrno = priv_version_csr_filter(env, csrno);

    //  testing for non-standard CSRs here (i.e., before the switch)
    //  allows us to override existing CSRs with our custom implementation;
    //  this is necessary for RI5CY core
    if(has_custom_csr(env, csrno) != 0) {
        tlib_write_csr(csrno, val_to_write);
        return;
    }

    switch(csrno) {
        case CSR_FFLAGS:
            if(riscv_mstatus_fs(env)) {
                env->fflags = val_to_write & (FSR_AEXC >> FSR_AEXC_SHIFT);
                mark_fs_dirty();
            } else {
                helper_raise_illegal_instruction(env);
            }
            break;
        case CSR_FRM:
            if(riscv_mstatus_fs(env)) {
                env->frm = val_to_write & (FSR_RD >> FSR_RD_SHIFT);
                mark_fs_dirty();
            } else {
                helper_raise_illegal_instruction(env);
            }
            break;
        case CSR_FCSR:
            if(riscv_mstatus_fs(env)) {
                env->fflags = (val_to_write & FSR_AEXC) >> FSR_AEXC_SHIFT;
                env->frm = (val_to_write & FSR_RD) >> FSR_RD_SHIFT;
                mark_fs_dirty();
            } else {
                helper_raise_illegal_instruction(env);
            }
            break;
        case CSR_JVT:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_ZCMT)) {
                tlib_printf(LOG_LEVEL_ERROR, "CSR_JVT can only be accessed when ZCMT extension is enabled");
                goto unhandled_csr_write;
            }
            //  Bits [5:0] are reserved and must be 0, jump table base address must be 64-byte aligned.
            env->jvt = val_to_write & ~((target_ulong)0x3F);
            break;
        case CSR_MSTATUS: {
            target_ulong mstatus = env->mstatus;
            target_ulong mask = MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_FS | MSTATUS_MPRV | MSTATUS_MPP | MSTATUS_MXR | MSTATUS_VS;
            if(riscv_has_ext(env, RISCV_FEATURE_RVS)) {
                mask |= MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SUM | MSTATUS_SPP;

                if(env->privilege_architecture >= RISCV_PRIV1_10) {
                    mask |= MSTATUS_TVM | MSTATUS_TSR;
                }
            }

            if(env->privilege_architecture < RISCV_PRIV1_10) {
                if((val_to_write ^ mstatus) & (MSTATUS_MXR | MSTATUS_MPP | MSTATUS_MPRV | MSTATUS_SUM | MSTATUS_VM)) {
                    tlb_flush(env, 1, true);
                }
                mask |= validate_vm(env, get_field(val_to_write, MSTATUS_VM)) ? MSTATUS_VM : 0;
            } else {
                if((val_to_write ^ mstatus) & (MSTATUS_MXR | MSTATUS_MPP | MSTATUS_MPRV | MSTATUS_SUM)) {
                    tlb_flush(env, 1, true);
                }
            }
#ifdef TARGET_RISCV64
            mask |= MSTATUS_UXL | MSTATUS_SXL;
#endif
            val_to_write = validate_mpp_setting(env, val_to_write);
            if(!riscv_has_ext(env, RISCV_FEATURE_RVU)) {
                /* MPRV is read-only 0 if U-mode is not supported, so disallow setting it */
                val_to_write = set_field(val_to_write, MSTATUS_MPRV, 0);
            }
            //  Big endian data access is not supported so warn if the guest trie
            //  to set any of the control bits for it
            target_ulong big_endian_mode_mask = MSTATUS_UBE;
#ifdef TARGET_RISCV64
            big_endian_mode_mask |= MSTATUS_MBE | MSTATUS_SBE;
#endif
            if(val_to_write & big_endian_mode_mask) {
                tlib_printf(LOG_LEVEL_WARNING,
                            "Big endian data access is not supported, writes to MSTATUS.MBE/SBE/UBE are ignored");
            }

            mstatus = (mstatus & ~mask) | (val_to_write & mask);

            int dirty = (mstatus & MSTATUS_FS) == MSTATUS_FS;
            dirty |= (mstatus & MSTATUS_XS) == MSTATUS_XS;
            mstatus = set_field(mstatus, MSTATUS_SD, dirty);
            env->mstatus = mstatus;
            if(cpu_in_clic_mode(env)) {
                /* See CSR_MCAUSE for explanation, why this happens (MPP and MPIE are aliased in MCAUSE/MSTATUS into each other)
                 */
                env->mcause = set_field(env->mcause, MCAUSE_MPP, get_field(env->mstatus, MSTATUS_MPP));
                env->mcause = set_field(env->mcause, MCAUSE_MPIE, get_field(env->mstatus, MSTATUS_MPIE));
            }
            break;
        }
        case CSR_MSTATUSH: {
#ifdef TARGET_RISCV32
            //  CSR was added in priv 1.12
            if(env->privilege_architecture < RISCV_PRIV1_12) {
                helper_raise_illegal_instruction(env);
            }
            //  All fields hardwired to zero in the current implementation
            if(val_to_write & (MSTATUSH_MBE | MSTATUSH_SBE)) {
                tlib_printf(LOG_LEVEL_WARNING, "Big endian data access is not supported, writes to MSTATUSH.MBE/SBE are ignored");
            }
#else
            //  This CSR only exists on RV32
            helper_raise_illegal_instruction(env);
#endif
            break;
        }
        case CSR_MIP: {
            /* MIP is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                break;
            }
            target_ulong custom_interrupts_mask = env->custom_interrupts & env->mip_triggered_custom_interrupts;
            target_ulong mask = IRQ_SS | IRQ_ST | IRQ_SE | custom_interrupts_mask;
            pthread_mutex_lock(&env->mip_lock);
            env->mip = (env->mip & ~mask) | (val_to_write & mask);
            pthread_mutex_unlock(&env->mip_lock);
            tlib_mip_changed(env->mip);
            if(env->mip != 0) {
                set_interrupt_pending(env, CPU_INTERRUPT_HARD);
            }
            break;
        }
        case CSR_MIE: {
            /* MIE is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                break;
            }
            env->mie = (env->mie & ~all_ints) | (val_to_write & all_ints);
            break;
        }
        case CSR_MIDELEG:
            /* MIDELEG is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                break;
            }
            env->mideleg = (env->mideleg & ~delegable_ints) | (val_to_write & delegable_ints);
            break;
        case CSR_MEDELEG: {
            target_ulong mask = 0;
            mask |= 1ULL << (RISCV_EXCP_INST_ADDR_MIS);
            mask |= 1ULL << (RISCV_EXCP_INST_ACCESS_FAULT);
            mask |= 1ULL << (RISCV_EXCP_ILLEGAL_INST);
            mask |= 1ULL << (RISCV_EXCP_BREAKPOINT);
            mask |= 1ULL << (RISCV_EXCP_LOAD_ADDR_MIS);
            mask |= 1ULL << (RISCV_EXCP_LOAD_ACCESS_FAULT);
            mask |= 1ULL << (RISCV_EXCP_STORE_AMO_ADDR_MIS);
            mask |= 1ULL << (RISCV_EXCP_STORE_AMO_ACCESS_FAULT);
            mask |= 1ULL << (RISCV_EXCP_U_ECALL);
            mask |= 1ULL << (RISCV_EXCP_S_ECALL);
            mask |= 1ULL << (RISCV_EXCP_H_ECALL);
            mask |= 1ULL << (RISCV_EXCP_M_ECALL);
            mask |= 1ULL << (RISCV_EXCP_INST_PAGE_FAULT);
            mask |= 1ULL << (RISCV_EXCP_LOAD_PAGE_FAULT);
            mask |= 1ULL << (RISCV_EXCP_STORE_PAGE_FAULT);
            env->medeleg = (env->medeleg & ~mask) | (val_to_write & mask);
            break;
        }
        case CSR_MCOUNTINHIBIT:
            /* There are different CSRs under this address in different privilege architecture versions:
             * - version 1.9.1: this address is used by mucounteren csr,
             * - version 1.10: this address is not used, and all calls are filtered by priv_version_csr_filter()
             * - since version 1.11: this address is used by mcountinhibit csr. */
            if(env->privilege_architecture == RISCV_PRIV1_09) {
                env->mucounteren = val_to_write;
            } else if(env->privilege_architecture >= RISCV_PRIV1_11) {
                uint64_t current_mcycle = get_mcycles_current(env);
                uint64_t current_minstret = get_minstret_current(env);
                uint64_t current_instret = cpu_riscv_read_instret(env);

                env->mcountinhibit = val_to_write;

                env->mcycle_snapshot_offset = current_mcycle;
                env->mcycle_snapshot = current_instret;
                env->minstret_snapshot_offset = current_minstret;
                env->minstret_snapshot = current_instret;
            }
            break;
        case CSR_MSCOUNTEREN:
            env->mscounteren = val_to_write;
            break;
        case CSR_MHPMEVENT3 ... CSR_MHPMEVENT31:
            break; /* Stubbed implementation */
#ifdef TARGET_RISCV32
        case CSR_MHPMEVENT3H ... CSR_MHPMEVENT31H:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SSCOFPMF)) {
                goto unhandled_csr_write;
            }
            break; /* Stubbed implementation */
#endif
        case CSR_SSTATUS: {
            target_ulong s = env->mstatus;
            target_ulong mask = SSTATUS_SIE | SSTATUS_SPIE | SSTATUS_UIE | SSTATUS_UPIE | SSTATUS_SPP | SSTATUS_FS | SSTATUS_XS |
                                SSTATUS_SUM | SSTATUS_MXR | SSTATUS_SD;
#ifdef TARGET_RISCV64
            mask |= SSTATUS_UXL;
#endif
            s = (s & ~mask) | (val_to_write & mask);
            csr_write_helper(env, s, CSR_MSTATUS);
            break;
        }
        case CSR_SIP: {
            /* SIP is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                break;
            }
            target_ulong deleg = env->mideleg;
            target_ulong s = env->mip;
            target_ulong custom_interrupts_mask = env->custom_interrupts & env->sip_triggered_custom_interrupts;
            target_ulong mask = IRQ_US | IRQ_SS | IRQ_UT | IRQ_ST | IRQ_UE | IRQ_SE | custom_interrupts_mask;
            pthread_mutex_lock(&env->mip_lock);
            env->mip = (s & ~mask) | ((val_to_write & deleg) & mask);
            pthread_mutex_unlock(&env->mip_lock);
            tlib_mip_changed(env->mip);
            if(env->mip != 0) {
                set_interrupt_pending(env, CPU_INTERRUPT_HARD);
            }
            break;
        }
        case CSR_SIE: {
            /* SIE is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                break;
            }
            target_ulong deleg = env->mideleg;
            target_ulong s = env->mie;
            target_ulong mask = IRQ_US | IRQ_SS | IRQ_UT | IRQ_ST | IRQ_UE | IRQ_SE | env->custom_interrupts;
            env->mie = (s & ~mask) | ((val_to_write & deleg) & mask);
            break;
        }
        case CSR_SATP: /* CSR_SPTBR */ {
            if((env->privilege_architecture < RISCV_PRIV1_10) && (val_to_write ^ env->sptbr)) {
                tlb_flush(env, 1, true);
                env->sptbr = val_to_write & (((target_ulong)1 << (TARGET_PHYS_ADDR_SPACE_BITS - PGSHIFT)) - 1);
                if(unlikely(env->guest_profiler_enabled)) {
                    tlib_announce_context_change(env->sptbr);
                }
            }
            if(env->privilege_architecture >= RISCV_PRIV1_10 && validate_vm(env, get_field(val_to_write, SATP_MODE)) &&
               ((val_to_write ^ env->satp) & (SATP_MODE | SATP_ASID | SATP_PPN))) {
                if(env->priv == PRV_S && env->mstatus & MSTATUS_TVM) {
                    helper_raise_illegal_instruction(env);
                }
                tlb_flush(env, 1, true);
                env->satp = val_to_write;
                if(unlikely(env->guest_profiler_enabled)) {
                    //  We use the entire content of the SATP register to identify a context, even though it contains multiple
                    //  fields
                    tlib_announce_context_change(env->satp);
                }
            }
            break;
        }
        case CSR_SEPC:
            env->sepc = val_to_write & ~((target_ulong)1);
            break;
        case CSR_STVEC:
            env->stvec = mtvec_stvec_write_handler(val_to_write, "STVEC");
            break;
        case CSR_SCOUNTEREN:
            env->scounteren = val_to_write;
            break;
        case CSR_STVT:
            env->stvt = val_to_write & ~MTVEC_MODE_SUBMODE;
            break;
        case CSR_SINTTHRESH:
            env->sintthresh = val_to_write;
            break;
        case CSR_SSCRATCH:
            env->sscratch = val_to_write;
            break;
        case CSR_SCAUSE:
            env->scause = val_to_write;
            break;
        case CSR_STVAL:
            env->stval = val_to_write;
            break;
        case CSR_MEPC:
            env->mepc = val_to_write & ~((target_ulong)1);
            break;
        case CSR_MTVEC:
            env->mtvec = mtvec_stvec_write_handler(val_to_write, "MTVEC");
            break;
        case CSR_MCOUNTEREN:
            env->mcounteren = val_to_write;
            break;
        case CSR_MTVT:
            env->mtvt = val_to_write & ~MTVEC_MODE_SUBMODE;
            break;
        case CSR_SINTSTATUS:
            env->mintstatus = set_field(env->mintstatus, SINTSTATUS_MASK, val_to_write);
            break;
        case CSR_MINTSTATUS:
            env->mintstatus = val_to_write;
            break;
        case CSR_MINTTHRESH:
            env->mintthresh = val_to_write;
            break;
        case CSR_MSCRATCH:
            env->mscratch = val_to_write;
            break;
        case CSR_MCAUSE:
            env->mcause = val_to_write;
            if(cpu_in_clic_mode(env)) {
                /* From CLIC manual (Version v0.9, 2024-06-28: Draft), section: "5.7.6. Changes to xcause CSRs":
                 * The mcause.mpp and mcause.mpie fields mirror the mstatus.mpp and mstatus.mpie fields, and are aliased
                 * into mcause to reduce context save/restore code. */
                /* Here, copy the aliased bits, and try to write to MSTATUS */
                target_ulong new_mstatus = env->mstatus;
                new_mstatus = set_field(new_mstatus, MSTATUS_MPP, get_field(env->mcause, MCAUSE_MPP));
                new_mstatus = set_field(new_mstatus, MSTATUS_MPIE, get_field(env->mcause, MCAUSE_MPIE));
                csr_write_helper(env, new_mstatus, CSR_MSTATUS);
                /* Now copy the aliased bits back from MSTATUS, so we are sure that the write succeeded
                 * as MSTATUS helper can not permit setting some bits (e.g. MPP if targets unsupported mode) */
                env->mcause = set_field(env->mcause, MCAUSE_MPP, get_field(env->mstatus, MSTATUS_MPP));
                env->mcause = set_field(env->mcause, MCAUSE_MPIE, get_field(env->mstatus, MSTATUS_MPIE));
            }
            break;
        case CSR_MTVAL:
            env->mtval = val_to_write;
            break;
        case CSR_MISA: {
            if(!(val_to_write & RISCV_FEATURE_RVF)) {
                val_to_write &= ~RISCV_FEATURE_RVD;
            }
            //  We don't allow to modify User or Supervisor mode with MISA
            //  as per "3.1.1 Machine ISA Register" Priviledged ISA version 1.12
            val_to_write |= (RISCV_FEATURE_RVS | RISCV_FEATURE_RVU) & env->misa_mask;
            env->misa = val_to_write & env->misa_mask;
            break;
        }
        case CSR_TSELECT:
            //  TSELECT is hardwired in this implementation
            break;
        case CSR_TDATA1:
            tlib_abort("CSR_TDATA1 write not implemented");
            break;
        case CSR_TDATA2:
            tlib_abort("CSR_TDATA2 write not implemented");
            break;
        case CSR_DCSR:
            tlib_abort("CSR_DCSR write not implemented");
            break;
        case CSR_MCYCLE:
#if defined(TARGET_RISCV32)
            env->mcycle_snapshot_offset = (get_mcycles_current(env) & 0xFFFFFFFF00000000) | val_to_write;
#else
            env->mcycle_snapshot_offset = get_mcycles_current(env);
#endif
            env->mcycle_snapshot = cpu_riscv_read_instret(env);
            break;
        case CSR_MCYCLEH:
#if defined(TARGET_RISCV32)
            env->mcycle_snapshot_offset = (get_mcycles_current(env) & 0x00000000FFFFFFFF) | ((uint64_t)val_to_write << 32);
            env->mcycle_snapshot = cpu_riscv_read_instret(env);
#endif
            break;
        case CSR_MINSTRET:
#if defined(TARGET_RISCV32)
            env->minstret_snapshot_offset = (get_minstret_current(env) & 0xFFFFFFFF00000000) | val_to_write;
#else
            env->minstret_snapshot_offset = get_minstret_current(env);
#endif
            env->minstret_snapshot = cpu_riscv_read_instret(env);
            break;
        case CSR_MINSTRETH:
#if defined(TARGET_RISCV32)
            env->minstret_snapshot_offset = (get_minstret_current(env) & 0x00000000FFFFFFFF) | ((uint64_t)val_to_write << 32);
            env->minstret_snapshot = cpu_riscv_read_instret(env);
#endif
            break;
        case CSR_PMPCFG0 ... CSR_PMPCFG_LAST:
            pmpcfg_csr_write(env, csrno - CSR_PMPCFG0, val_to_write);
            break;
        case CSR_PMPADDR0 ... CSR_PMPADDR_LAST:
            pmpaddr_csr_write(env, csrno - CSR_PMPADDR0, val_to_write);
            break;
        case CSR_VSTART:
            validate_vector_state(env);
            target_ulong vstart_mask = (env->vlenb * 8) - 1;
            env->vstart = val_to_write & vstart_mask;
            env->mstatus = set_field(env->mstatus, MSTATUS_VS, MSTATUS_VS_DIRTY);
            break;
        case CSR_VXSAT:
            validate_vector_state(env);
            env->vxsat = val_to_write & VCSR_VXSAT;
            env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat;
            env->mstatus = set_field(env->mstatus, MSTATUS_VS, MSTATUS_VS_DIRTY);
            break;
        case CSR_VXRM:
            validate_vector_state(env);
            env->vxrm = val_to_write & VXRM_MASK;
            env->vcsr = (env->vcsr & ~VCSR_VXRM) | ((env->vxrm & VXRM_MASK) << VXRM_VCSR_SHIFT);
            env->mstatus = set_field(env->mstatus, MSTATUS_VS, MSTATUS_VS_DIRTY);
            break;
        case CSR_VCSR:
            validate_vector_state(env);
            env->vcsr = val_to_write & VCSR_MASK;
            env->vxsat = env->vcsr & VCSR_VXSAT;
            env->vxrm = (env->vcsr >> 1) & VXRM_MASK;
            env->mstatus = set_field(env->mstatus, MSTATUS_VS, MSTATUS_VS_DIRTY);
            break;
        case CSR_MENVCFG:
            if(!riscv_has_ext(env, RISCV_FEATURE_RVU)) {
                goto unhandled_csr_write;
            }
            env->menvcfg = val_to_write & ~((target_ulong)MENVCFG_RO_ZERO_MASK);
            break;
        case CSR_MENVCFGH:
#if defined(TARGET_RISCV32)
            if(!riscv_has_ext(env, RISCV_FEATURE_RVU)) {
                goto unhandled_csr_write;
            }
            env->menvcfgh = val_to_write & ~((target_ulong)(MENVCFG_RO_ZERO_MASK >> 32));
            break;
#else
            goto unhandled_csr_write;
#endif
        case CSR_MSECCFG:
            //  Based on the SMEPMP documentation Version 1.0
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SMEPMP)) {
                tlib_printf(LOG_LEVEL_ERROR, "CSR_MSECCFG can only be accessed when SMEPMP extension is enabled");
                goto unhandled_csr_write;
            }
            if(env->priv != PRV_M) {
                tlib_printf(LOG_LEVEL_WARNING, "CSR_MSECCFG can only be accessed in Machine mode");
                goto unhandled_csr_write;
            }

            target_ulong mseccfg_mask = MSECCFG_MML | MSECCFG_MMWP | MSECCFG_RLB;
            if(!(env->mseccfg & MSECCFG_RLB) && pmp_is_any_region_locked(env)) {
                /* When mseccfg.RLB is 0 and pmpcfg.L is 1 in any rule or entry (including disabled entries), then
                   mseccfg.RLB remains 0 and any further modifications to mseccfg.RLB are ignored until a PMP reset */
                mseccfg_mask ^= MSECCFG_RLB;
            }
            //  MML and MMWP bits are sticky - after being set, the cannot be unset
            env->mseccfg = (val_to_write & mseccfg_mask) | (env->mseccfg & (MSECCFG_MML | MSECCFG_MMWP));

            //  This settings can change behavior of PMP, so flush existing TLBs
            tlb_flush(env, 1, true);
            break;
        case CSR_MSECCFGH:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SMEPMP)) {
                tlib_printf(LOG_LEVEL_ERROR, "CSR_MSECCFGH can only be accessed when SMEPMP extension is enabled");
                goto unhandled_csr_write;
            }
            if(env->priv != PRV_M) {
                tlib_printf(LOG_LEVEL_WARNING, "CSR_MSECCFGH can only be accessed in Machine mode");
                goto unhandled_csr_write;
            }
            env->mseccfgh = val_to_write;
            break;
        default:
        unhandled_csr_write:
            warn_nonexistent_csr_write(csrno, val_to_write);
            helper_raise_illegal_instruction(env);
    }
}

/*
 * Handle reads to CSRs and any resulting special behavior
 *
 * Adapted from Spike's processor_t::get_csr
 */
static inline target_ulong csr_read_helper(CPUState *env, target_ulong csrno)
{
    csrno = priv_version_csr_filter(env, csrno);

    if(is_counter_csr(csrno)) {
        target_ulong ctr_ok = (get_counter_enabled_mask(env) >> (csrno & 31)) & 1;
        if(!ctr_ok) {
            helper_raise_illegal_instruction(env);
        }
    }

    //  testing for non-standard CSRs here (i.e., before the switch)
    //  allows us to override existing CSRs with our custom implementation;
    //  this is necessary for RI5CY core
    if(has_custom_csr(env, csrno) != 0) {
        return tlib_read_csr(csrno);
    }

    switch(csrno) {
        case CSR_FFLAGS:
            if(riscv_mstatus_fs(env)) {
                return env->fflags;
            } else {
                helper_raise_illegal_instruction(env);
                break;
            }
        case CSR_FRM:
            if(riscv_mstatus_fs(env)) {
                return env->frm;
            } else {
                helper_raise_illegal_instruction(env);
                break;
            }
        case CSR_FCSR:
            if(riscv_mstatus_fs(env)) {
                return env->fflags << FSR_AEXC_SHIFT | env->frm << FSR_RD_SHIFT;
            } else {
                helper_raise_illegal_instruction(env);
                break;
            }
        case CSR_JVT:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_ZCMT)) {
                tlib_printf(LOG_LEVEL_ERROR, "CSR_JVT can only be accessed when ZCMT extension is enabled");
                goto unhandled_csr_read;
            }
            return env->jvt;
        case CSR_TIME:
            return tlib_get_cpu_time();
        case CSR_TIMEH:
#if defined(TARGET_RISCV32)
            return tlib_get_cpu_time() >> 32;
#endif
            break;
        case CSR_INSTRET:
        case CSR_MINSTRET:
            return get_minstret_current(env);
        case CSR_CYCLE:
        case CSR_MCYCLE:
            return get_mcycles_current(env);
        case CSR_INSTRETH:
        case CSR_MINSTRETH:
#if defined(TARGET_RISCV32)
            return get_minstret_current(env) >> 32;
#endif
            break;
        case CSR_CYCLEH:
        case CSR_MCYCLEH:
#if defined(TARGET_RISCV32)
            return get_mcycles_current(env) >> 32;
#endif
            break;
        case CSR_HPMCOUNTER3 ... CSR_HPMCOUNTER31:
        case CSR_MHPMCOUNTER3 ... CSR_MHPMCOUNTER31:
            return 0; /* Stubbed implementation */
        case CSR_HPMCOUNTER3H ... CSR_HPMCOUNTER31H:
        case CSR_MHPMCOUNTER3H ... CSR_MHPMCOUNTER31H:
#if defined(TARGET_RISCV32)
            return 0; /* Stubbed implementation */
#endif
            break;
        case CSR_MCOUNTINHIBIT:
            /* There are different CSRs under this address on different privilege architecture version:
             * - version 1.9.1: this address is used by mucounteren csr,
             * - version 1.10: this address is not used, and all calls are filtered by priv_version_csr_filter()
             * - since version 1.11: this address is used by mcountinhibit csr. */
            if(env->privilege_architecture == RISCV_PRIV1_09) {
                return env->mucounteren;
            } else if(env->privilege_architecture >= RISCV_PRIV1_11) {
                return env->mcountinhibit;
            }
            break;
        case CSR_MSCOUNTEREN:
            return env->mscounteren;
        case CSR_MHPMEVENT3 ... CSR_MHPMEVENT31:
            return 0; /* Stubbed implementation */
#ifdef TARGET_RISCV32
        case CSR_MHPMEVENT3H ... CSR_MHPMEVENT31H:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SSCOFPMF)) {
                goto unhandled_csr_read;
            }
            return 0; /* Stubbed implementation */
#endif
        case CSR_SSTATUS: {
            target_ulong mask = SSTATUS_SIE | SSTATUS_SPIE | SSTATUS_UIE | SSTATUS_UPIE | SSTATUS_SPP | SSTATUS_FS | SSTATUS_XS |
                                SSTATUS_SUM | SSTATUS_SD;
            if(env->privilege_architecture >= RISCV_PRIV1_10) {
                mask |= SSTATUS_MXR;
            }
#ifdef TARGET_RISCV64
            mask |= SSTATUS_UXL;
#endif
            return env->mstatus & mask;
        }
        case CSR_SIP: {
            /* SIP is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                return 0;
            }
            target_ulong mask = IRQ_US | IRQ_SS | IRQ_UT | IRQ_ST | IRQ_UE | IRQ_SE | env->custom_interrupts;
            return env->mip & env->mideleg & mask;
        }
        case CSR_SIE: {
            /* SIP is hardwired to zero in CLIC mode */
            if(cpu_in_clic_mode(env)) {
                return 0;
            }
            target_ulong mask = IRQ_US | IRQ_SS | IRQ_UT | IRQ_ST | IRQ_UE | IRQ_SE | env->custom_interrupts;
            return env->mie & env->mideleg & mask;
        }
        case CSR_SEPC:
            return env->sepc;
        case CSR_STVAL:
            return env->stval;
        case CSR_STVEC:
            return env->stvec | (cpu_in_clic_mode(env) ? MTVEC_MODE_CLIC : 0);
        case CSR_SCOUNTEREN:
            return env->scounteren;
        case CSR_STVT:
            return env->stvt;
        case CSR_SINTTHRESH:
            return env->sintthresh;
        case CSR_SCAUSE:
            return env->scause;
        case CSR_SATP: /* CSR_SPTBR */
            if(env->privilege_architecture >= RISCV_PRIV1_10) {
                if(env->priv == PRV_S && env->mstatus & MSTATUS_TVM) {
                    helper_raise_illegal_instruction(env);
                    break;
                }
                return env->satp;
            } else {
                return env->sptbr;
            }
        case CSR_SSCRATCH:
            return env->sscratch;
        case CSR_MSTATUS:
            return env->mstatus;
        case CSR_MSTATUSH:
#ifdef TARGET_RISCV32
            //  CSR was added in priv 1.12
            if(env->privilege_architecture < RISCV_PRIV1_12) {
                helper_raise_illegal_instruction(env);
            }
            //  Since mixed-endian mode is not supported in renode, both bits with meaning here are hardwired to 0
            return 0;
#else
            //  This CSR is only present on RV32
            helper_raise_illegal_instruction(env);
#endif
            break;
        case CSR_MIP:
            /* MIP is hardwired to zero in CLIC mode */
            return cpu_in_clic_mode(env) ? 0 : env->mip;
        case CSR_MIE:
            /* MIE is hardwired to zero in CLIC mode */
            return cpu_in_clic_mode(env) ? 0 : env->mie;
        case CSR_MEPC:
            return env->mepc;
        case CSR_MSCRATCH:
            return env->mscratch;
        case CSR_MCAUSE:
            return env->mcause;
        case CSR_MTVAL:
            return env->mtval;
        case CSR_MISA:
            return env->misa;
        case CSR_MARCHID:
            return 0; /* as spike does */
        case CSR_MIMPID:
            return 0; /* as spike does */
        case CSR_MVENDORID:
            return 0; /* as spike does */
        case CSR_MHARTID:
            return env->mhartid;
        case CSR_MTVEC:
            return env->mtvec;
        case CSR_MCOUNTEREN:
            return env->mcounteren;
        case CSR_MTVT:
            return env->mtvt;
        case CSR_MINTTHRESH:
            return env->mintthresh;
        case CSR_MEDELEG:
            return env->medeleg;
        case CSR_MIDELEG:
            /* MIDELEG is hardwired to zero in CLIC mode */
            return cpu_in_clic_mode(env) ? 0 : env->mideleg;
        case CSR_PMPCFG0 ... CSR_PMPCFG_LAST:
            return pmpcfg_csr_read(env, csrno - CSR_PMPCFG0);
        case CSR_PMPADDR0 ... CSR_PMPADDR_LAST:
            return pmpaddr_csr_read(env, csrno - CSR_PMPADDR0);
        case CSR_VSTART:
            validate_vector_state(env);
            return env->vstart;
        case CSR_VXSAT:
            validate_vector_state(env);
            return env->vxsat & VXSAT_MASK;
        case CSR_VXRM:
            validate_vector_state(env);
            return env->vxrm & VXRM_MASK;
        case CSR_VCSR:
            validate_vector_state(env);
            return env->vcsr;
        case CSR_VL:
            validate_vector_state(env);
            return env->vl;
        case CSR_VTYPE:
            validate_vector_state(env);
            return env->vtype;
        case CSR_VLENB:
            validate_vector_state(env);
            return env->vlenb;
        case CSR_SCOUNTOVF:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SSCOFPMF)) {
                goto unhandled_csr_read;
            }
            return 0; /* stubbed implementation */
        case CSR_SINTSTATUS:
            return env->mintstatus & SINTSTATUS_MASK;
        case CSR_MINTSTATUS:
            return env->mintstatus;
        case CSR_MENVCFG:
            return env->menvcfg;
        case CSR_MENVCFGH:
            return env->menvcfgh;
        case CSR_MSECCFG:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SMEPMP)) {
                tlib_printf(LOG_LEVEL_ERROR, "CSR_MSECCFG can only be accessed when SMEPMP extension is enabled");
                goto unhandled_csr_read;
            }
            if(env->priv != PRV_M) {
                tlib_printf(LOG_LEVEL_WARNING, "CSR_MSECCFG can only be accessed in Machine mode");
                goto unhandled_csr_read;
            }
            return env->mseccfg;
        case CSR_MSECCFGH:
            if(!riscv_has_additional_ext(env, RISCV_FEATURE_SMEPMP)) {
                tlib_printf(LOG_LEVEL_ERROR, "CSR_MSECCFGH can only be accessed when SMEPMP extension is enabled");
                goto unhandled_csr_read;
            }
            if(env->priv != PRV_M) {
                tlib_printf(LOG_LEVEL_WARNING, "CSR_MSECCFGH can only be accessed in Machine mode");
                goto unhandled_csr_read;
            }
            return env->mseccfgh;
        case CSR_TSELECT:
            return 0u;
        case CSR_TDATA1:
            return 0u;
        case CSR_TDATA2:
            return 0u;
        default:
        unhandled_csr_read:
            /* used by e.g. MTIME read */
            warn_nonexistent_csr_read(csrno);
            helper_raise_illegal_instruction(env);
    }
    return 0;
}

/*
 * Check that CSR access is allowed.
 *
 * Adapted from Spike's decode.h:validate_csr
 */
void validate_csr(CPUState *env, uint64_t which, uint64_t write)
{
    unsigned csr_priv = get_field((which), 0x300);
    unsigned csr_read_only = get_field((which), 0xC00) == 3;

    switch(env->csr_validation_level) {
        case CSR_VALIDATION_FULL:
            if(((write) && csr_read_only) || (env->priv < csr_priv)) {
                helper_raise_illegal_instruction(env);
            }
            break;

        case CSR_VALIDATION_PRIV:
            if(env->priv < csr_priv) {
                helper_raise_illegal_instruction(env);
            }
            break;

        case CSR_VALIDATION_NONE:
            break;

        default:
            tlib_abortf("Unexpected CSR validation level: %d", env->csr_validation_level);
            break;
    }
}

void helper_csrrw_no_read(CPUState *env, target_ulong src, target_ulong csr)
{
    validate_csr(env, csr, 1);
    csr_write_helper(env, src, csr);
}

target_ulong helper_csrrw(CPUState *env, target_ulong src, target_ulong csr)
{
    validate_csr(env, csr, 1);

    /* We implement these CSRs explicitly here because reading them might return `src` which is not available
       in csr_read_helper. Also, access to them with instructions other than csrrw is reserved. */
    switch(csr) {
        case CSR_MSCRATCHCSW:
        case CSR_SSCRATCHCSW: {
            target_ulong ppfield = csr == CSR_MSCRATCHCSW ? MSTATUS_MPP : MSTATUS_SPP;
            target_ulong prev_priv = get_field(env->mstatus, ppfield);
            /* If xpp == current priv, csr value and rd unchanged */
            if(env->priv == prev_priv) {
                return src;
            } else {
                target_ulong scratch;
                if(csr == CSR_MSCRATCHCSW) {
                    scratch = env->mscratch;
                    env->mscratch = src;
                } else {
                    scratch = env->sscratch;
                    env->sscratch = src;
                }
                return scratch;
            }
        } break;
        case CSR_MSCRATCHCSWL: {
            /* If neither mpil nor mil are in an interrupt, or both are, csr value and rd unchanged */
            if((get_field(env->mcause, MCAUSE_MPIL) == 0) == (get_field(env->mintstatus, MINTSTATUS_MIL) == 0)) {
                return src;
            } else {
                target_ulong mscratch = env->mscratch;
                env->mscratch = src;
                return mscratch;
            }
        } break;
        case CSR_SSCRATCHCSWL: {
            /* If neither mpil nor mil are in an interrupt, or both are, csr value and rd unchanged */
            if((get_field(env->scause, SCAUSE_SPIL) == 0) == (get_field(env->mintstatus, MINTSTATUS_SIL) == 0)) {
                return src;
            } else {
                target_ulong sscratch = env->sscratch;
                env->sscratch = src;
                return sscratch;
            }
        } break;
        default:
            /* No additional action for other CSRs. */
            break;
    }

    uint64_t csr_backup = csr_read_helper(env, csr);
    csr_write_helper(env, src, csr);
    return csr_backup;
}

target_ulong helper_csrrs(CPUState *env, target_ulong src, target_ulong csr, target_ulong rs1_pass)
{
    validate_csr(env, csr, rs1_pass != 0);

    /* We implement these CSRs explicitly here because the value used in the RMW (mstatus) is different
       from the result. */
    if(csr == CSR_MNXTI) {
        src &= 0x1f;
        target_ulong ms = env->mstatus;
        ms |= src;
        csr_write_helper(env, ms, CSR_MSTATUS);

        return mnxti_read_handler(src);
    } else if(csr == CSR_SNXTI) {
        src &= 0x1f;
        target_ulong ss = env->mstatus;
        ss |= src;
        csr_write_helper(env, ss, CSR_SSTATUS);

        return snxti_read_handler(src);
    }

    uint64_t csr_backup = csr_read_helper(env, csr);
    if(rs1_pass != 0) {
        csr_write_helper(env, src | csr_backup, csr);
    }
    return csr_backup;
}

void helper_sfence_vma(CPUState *env)
{
    /* SFENCE.VMA is privileged: U-mode must always trap */
    if(env->priv < PRV_S) {
        helper_raise_illegal_instruction(env);
    }

    /* In S-mode, TVM=1 also traps (priv spec >= 1.10) */
    if(env->privilege_architecture >= RISCV_PRIV1_10 && env->priv == PRV_S && get_field(env->mstatus, MSTATUS_TVM)) {
        helper_raise_illegal_instruction(env);
    }

    /* TODO: handle ASID specific fences */
    tlb_flush(env, 1, true);
}

target_ulong helper_csrrc(CPUState *env, target_ulong src, target_ulong csr, target_ulong rs1_pass)
{
    validate_csr(env, csr, rs1_pass != 0);

    if(csr == CSR_MNXTI) {
        src &= 0x1f;
        target_ulong ms = env->mstatus;
        ms &= ~src;
        csr_write_helper(env, ms, CSR_MSTATUS);

        return mnxti_read_handler(src);
    } else if(csr == CSR_SNXTI) {
        src &= 0x1f;
        target_ulong ss = env->mstatus;
        ss &= ~src;
        csr_write_helper(env, ss, CSR_SSTATUS);

        return snxti_read_handler(src);
    }

    uint64_t csr_backup = csr_read_helper(env, csr);
    if(rs1_pass != 0) {
        csr_write_helper(env, (~src) & csr_backup, csr);
    }
    return csr_backup;
}

void riscv_set_mode(CPUState *env, target_ulong newpriv)
{
    if(newpriv > PRV_M) {
        tlib_abort("invalid privilege level");
    }
    if(newpriv == PRV_H) {
        newpriv = PRV_U;
    }
    env->priv = newpriv;
}

target_ulong helper_sret(CPUState *env, target_ulong cpu_pc_deb)
{
    if(!riscv_has_ext(env, RISCV_FEATURE_RVS)) {
        helper_raise_illegal_instruction(env);
    }

    if(env->priv < PRV_S) {
        tlib_printf(LOG_LEVEL_ERROR, "Trying to execute Sret from privilege level %u", env->priv);
        helper_raise_illegal_instruction(env);
    }

    if(env->privilege_architecture >= RISCV_PRIV1_10 && env->priv == PRV_S && get_field(env->mstatus, MSTATUS_TSR)) {
        helper_raise_illegal_instruction(env);
    }

    target_ulong retpc = env->sepc;
    if(!riscv_has_ext(env, RISCV_FEATURE_RVC) && (retpc & 0x3)) {
        helper_raise_exception(env, RISCV_EXCP_INST_ADDR_MIS);
    }

    target_ulong sstatus = env->mstatus;
    target_ulong prev_priv = get_field(sstatus, SSTATUS_SPP);
    sstatus = set_field(sstatus, (env->privilege_architecture >= RISCV_PRIV1_10) ? SSTATUS_SIE : 1 << prev_priv,
                        get_field(sstatus, SSTATUS_SPIE));
    sstatus = set_field(sstatus, SSTATUS_SPIE, 1);
    sstatus = set_field(sstatus, SSTATUS_SPP, PRV_U);
    if(env->privilege_architecture >= RISCV_PRIV1_12) {
        sstatus = set_field(sstatus, MSTATUS_MPRV, 0);
    }

    riscv_set_mode(env, prev_priv);
    csr_write_helper(env, sstatus, CSR_SSTATUS);

    target_ulong scause = env->scause;
    scause = set_field(scause, SCAUSE_SPP, PRV_U);
    scause = set_field(scause, SCAUSE_SPIE, 1);
    //  SCAUSE_SPIL not affected by sret
    csr_write_helper(env, scause, CSR_SCAUSE);

    if(cpu_in_clic_mode(env)) {
        target_ulong sintstatus = env->mintstatus;
        sintstatus = set_field(sintstatus, MINTSTATUS_SIL, get_field(env->scause, SCAUSE_SPIL));
        csr_write_helper(env, sintstatus, CSR_SINTSTATUS);

        if(env->scause & SCAUSE_SINHV) {
            retpc = ldp_phys(retpc) & ~1ULL;
        }
    }

    //  Clear this HART's reservation.
    env->reserved_address = 0;

    if(env->interrupt_end_callback_enabled) {
        tlib_on_interrupt_end(env->exception_index);
    }

    return retpc;
}

target_ulong helper_mret(CPUState *env, target_ulong cpu_pc_deb)
{
    if(env->priv < PRV_M) {
        tlib_printf(LOG_LEVEL_ERROR, "Trying to execute Mret from privilege level %u", env->priv);
        helper_raise_illegal_instruction(env);
    }

    target_ulong retpc = env->mepc;
    if(!riscv_has_ext(env, RISCV_FEATURE_RVC) && (retpc & 0x3)) {
        helper_raise_exception(env, RISCV_EXCP_INST_ADDR_MIS);
    }

    target_ulong mstatus = env->mstatus;
    target_ulong prev_priv = get_field(mstatus, MSTATUS_MPP);
    mstatus = set_field(mstatus, env->privilege_architecture >= RISCV_PRIV1_10 ? MSTATUS_MIE : 1 << prev_priv,
                        get_field(mstatus, MSTATUS_MPIE));
    mstatus = set_field(mstatus, MSTATUS_MPIE, 1);

    int lowest_priv = riscv_has_ext(env, RISCV_FEATURE_RVU) ? PRV_U : PRV_M;

    mstatus = set_field(mstatus, MSTATUS_MPP, lowest_priv);
    if(env->privilege_architecture >= RISCV_PRIV1_12 && prev_priv != PRV_M) {
        mstatus = set_field(mstatus, MSTATUS_MPRV, 0);
    }
    riscv_set_mode(env, prev_priv);
    csr_write_helper(env, mstatus, CSR_MSTATUS);

    target_ulong mcause = env->mcause;
    mcause = set_field(mcause, MCAUSE_MPP, lowest_priv);
    mcause = set_field(mcause, MCAUSE_MPIE, 1);
    //  MCAUSE_MPIL not affected by mret
    csr_write_helper(env, mcause, CSR_MCAUSE);

    if(cpu_in_clic_mode(env)) {
        target_ulong mintstatus = env->mintstatus;
        mintstatus = set_field(mintstatus, MINTSTATUS_MIL, get_field(env->mcause, MCAUSE_MPIL));
        csr_write_helper(env, mintstatus, CSR_MINTSTATUS);

        if(env->mcause & MCAUSE_MINHV) {
            retpc = ldp_phys(retpc) & ~1ULL;
        }
    }

    //  Clear this HART's reservation.
    env->reserved_address = 0;

    if(env->interrupt_end_callback_enabled) {
        tlib_on_interrupt_end(env->exception_index);
    }

    return retpc;
}

void helper_wfi(CPUState *env)
{
    /* To treat wfi as nop (as described in the specification), additional facilities, outside of this translation library, are
     * required. */
    env->wfi = 1;
    env->exception_index = EXCP_WFI;
}

void helper_fence_i(CPUState *env)
{
    helper_invalidate_dirty_addresses_shared(env);
}

void do_unaligned_access(target_ulong addr, int access_type, int mmu_idx, void *retaddr)
{
    env->badaddr = addr;
    switch(access_type) {
        case ACCESS_DATA_LOAD:
            do_raise_exception_err(env, RISCV_EXCP_LOAD_ADDR_MIS, (uintptr_t)retaddr, 1);
            break;
        case ACCESS_DATA_STORE:
            do_raise_exception_err(env, RISCV_EXCP_STORE_AMO_ADDR_MIS, (uintptr_t)retaddr, 1);
            break;
        case ACCESS_INST_FETCH:
            //  we don't restore intructions count / fire block_end hooks on translation
            do_raise_exception_err(env, RISCV_EXCP_INST_ADDR_MIS, 0, 0);
            break;
        default:
            tlib_abort("Illegal memory access type!");
    }
}

void arch_raise_mmu_fault_exception(CPUState *env, int errcode, int access_type, target_ulong address, void *retaddr)
{
    //  access_type == 2 ==> CODE ACCESS - do not fire block_end hooks!
    do_raise_exception_err(env, env->exception_index, (uintptr_t)retaddr, access_type != 2);
}

/* called to fill tlb */
int arch_tlb_fill(CPUState *env, target_ulong addr, int access_type, int mmu_idx, void *retaddr, int no_page_fault,
                  int access_width, target_phys_addr_t *paddr)
{
    return cpu_handle_mmu_fault(env, addr, access_type, mmu_idx, access_width, no_page_fault, paddr);
}
