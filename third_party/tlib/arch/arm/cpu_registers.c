/*
 *  ARM registers interface.
 *
 *  Copyright (c) Antmicro
 *  Copyright (c) Realtime Embedded
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
#include "cpu_registers.h"
#include "unwind.h"

#ifdef TARGET_ARM64
uint64_t *get_reg_pointer_64(int reg)
{
    switch(reg) {
        //  64-bit regs:
        case X_0_64 ... X_31_64:
            return &(cpu->xregs[reg - X_0_64]);
        case PC_64:
            return &(cpu->pc);
        default:
            return NULL;
    }
}

CPU_REGISTER_ACCESSOR(64)
#endif
#if defined(TARGET_ARM32) || defined(TARGET_ARM64)
#ifdef TARGET_PROTO_ARM_M
void warn_about_fp_lsp(const char *reg_name, int reg_number)
{
    if(need_fp_lazy_state_preservation(env)) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Writing to %s%d register when lazy state preservation flag is set; register might be overwritten on the "
                    "execution of the next floating point instruction",
                    reg_name, reg_number);
    }
}

void warn_about_fp_context(const char *reg_name)
{
    if(need_fp_context(env) || need_fp_lazy_state_preservation(env)) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Writing to %s register without floating point context; register might be reset on the execution of the next "
                    "floating point instruction",
                    reg_name);
    }
}
#endif

uint32_t *get_reg_pointer_32_with_security(int reg, bool is_secure);

uint32_t *get_reg_pointer_32(int reg)
{
    return get_reg_pointer_32_with_security(reg, cpu->secure);
}

uint32_t *get_reg_pointer_32_with_security(int reg, bool is_secure)
{
    switch(reg) {
        case R_0_32 ... R_15_32:
            return &(cpu->regs[reg]);
        case CPSR_32:
            return &(cpu->uncached_cpsr);
#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
        case Control_32:
            return &(cpu->v7m.control[is_secure]);
        case BasePri_32:
            return &(cpu->v7m.basepri[is_secure]);
        case VecBase_32:
            return &(cpu->v7m.vecbase[is_secure]);
        case OtherSP_32:
            return &(cpu->v7m.other_sp);
        case FPCAR_32:
            return &(cpu->v7m.fpcar[is_secure]);
        case FPDSCR_32:
            return &(cpu->v7m.fpdscr[is_secure]);
        case CPACR_32:
            return &(cpu->v7m.cpacr[is_secure]);
        case NSACR_32:
            return &(cpu->v7m.nsacr);
        case PRIMASK_32:
            return &(cpu->v7m.primask[is_secure]);
        case FAULTMASK_32:
            return &(cpu->v7m.faultmask[is_secure]);
        case FPSCR_32:
            return &(cpu->vfp.xregs[ARM_VFP_FPSCR]);
        case VPR_32:
            return &(cpu->v7m.vpr);
        case S_0_32 ... S_31_32: {
            int tmp_reg = reg - S_0_32;
            CPU_DoubleU *d_reg_ref = (CPU_DoubleU *)&(cpu->vfp.regs[tmp_reg >> 1]);  //  S0...S31 are contained within D0...D15
            if(tmp_reg & 1) {  //  We are using relative register number to ensure parity is correct. ie. we want reg & 1 == 0 for
                               //  s0, s2, s4 etc.  which is not the case when S_0_32 is odd.
                return &(d_reg_ref->l.upper);
            } else {
                return &(d_reg_ref->l.lower);
            }
        }

#endif
        default:
            return NULL;
    }
}

uint32_t tlib_get_register_value_32_with_security(int reg_number, bool is_secure)
{
    if(reg_number == CPSR_32) {
#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
        return xpsr_read(cpu);
#else
        return cpsr_read(cpu);
#endif
    }
#ifdef TARGET_PROTO_ARM_M
    else if(reg_number == FPCCR_32) {
        return fpccr_read(env, is_secure);
    } else if(reg_number == PRIMASK_32) {
        //  PRIMASK: b0: IRQ mask enabled/disabled, b1-b31: reserved.
        return cpu->v7m.primask[is_secure] & PRIMASK_EN ? 1 : 0;
    } else if(reg_number == FPSCR_32) {
        return vfp_get_fpscr(cpu);
    }
#endif

    uint32_t *ptr = get_reg_pointer_32_with_security(reg_number, is_secure);
    if(ptr == NULL) {
        tlib_abortf("Read from undefined CPU register number %d detected", reg_number);
    }

#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
    /* CONTROL is a special case, since in TrustZone we hold the Non-banked bits
     * in the common (Secure) bank. So we need to remember to OR the values to get
     * the real contents of the register (with SFPA only accessible from Secure state) */
    if(reg_number == Control_32) {
        const uint32_t unbanked_bits = ARM_CONTROL_FPCA_MASK | (is_secure << ARM_CONTROL_SFPA);
        return (*ptr) | (env->v7m.control[M_REG_COMMON] & unbanked_bits);
    }
#endif

    return *ptr;
}

#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
uint32_t tlib_get_register_value_32_non_secure(int reg_number)
{
    tlib_assert(cpu->v7m.has_trustzone);
    return tlib_get_register_value_32_with_security(reg_number, false);
}
EXC_INT_1(uint32_t, tlib_get_register_value_32_non_secure, int, reg_number)
#endif

uint32_t tlib_get_register_value_32(int reg_number)
{
    return tlib_get_register_value_32_with_security(reg_number, cpu->secure);
}

EXC_INT_1(uint32_t, tlib_get_register_value_32, int, reg_number)

void tlib_set_register_value_32_with_security(int reg_number, uint32_t value, bool is_secure)
{
    if(reg_number == CPSR_32) {
#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
        xpsr_write(cpu, value, 0xffffffff);
#else
        cpsr_write(cpu, value, 0xffffffff);
#endif
        return;
    }
#ifdef TARGET_PROTO_ARM_M
    else if(reg_number == FPCCR_32) {
        return fpccr_write(env, value, is_secure);
    } else if(reg_number == BasePri_32) {
        cpu->v7m.basepri[is_secure] = value & 0xff;
        tlib_nvic_write_basepri(value & 0xff, is_secure);
        return;
    } else if(reg_number == PRIMASK_32) {
        //  PRIMASK: b0: IRQ mask enabled/disabled, b1-b31: reserved.
        cpu->v7m.primask[is_secure] = value & PRIMASK_EN;
        tlib_nvic_find_pending_irq();
        return;
    } else if(reg_number == FAULTMASK_32) {
        cpu->v7m.faultmask[is_secure] = value & 1;
        tlib_nvic_find_pending_irq();
        return;
    } else if(reg_number == FPSCR_32) {
        warn_about_fp_context("FPSCR");
        vfp_set_fpscr(cpu, value);
    } else if(reg_number == VPR_32) {
        warn_about_fp_context("VPR");
    } else if(S_0_32 <= reg_number && reg_number <= S_31_32) {
        warn_about_fp_lsp("S", reg_number - S_0_32);
    }
#endif

    uint32_t *ptr = get_reg_pointer_32_with_security(reg_number, is_secure);
    if(ptr == NULL) {
        tlib_abortf("Write to undefined CPU register number %d detected", reg_number);
    }

#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
    if(reg_number == Control_32) {
        /* Not banked bits are always stored in the Secure CONTROL bank.
         * SFPA bit is not writable from the Non-secure state
         * FPCA is writable from the Non-secure state only if NSACR.CP10 field is set.
         */

        if(!is_secure) {
            if(env->v7m.nsacr & ARM_NSACR_CP10_MASK) {
                env->v7m.control[M_REG_COMMON] &= ARM_CONTROL_FPCA_MASK;
                env->v7m.control[M_REG_COMMON] |= value & ARM_CONTROL_FPCA_MASK;
            }
            value &= ~(ARM_CONTROL_SFPA_MASK | ARM_CONTROL_FPCA_MASK);
        }
    } else if(reg_number == SP_32 || reg_number == OtherSP_32) {
        //  bits [1:0] of SP are WI or SBZP
        value &= 0xFFFFFFFC;
    }
#endif

    *ptr = value;
}

#if defined(TARGET_ARM32) && defined(TARGET_PROTO_ARM_M)
void tlib_set_register_value_32_non_secure(int reg_number, uint32_t value)
{
    tlib_assert(cpu->v7m.has_trustzone);
    tlib_set_register_value_32_with_security(reg_number, value, false);
}
EXC_VOID_2(tlib_set_register_value_32_non_secure, int, reg_number, uint32_t, value)
#endif

void tlib_set_register_value_32(int reg_number, uint32_t value)
{
    tlib_set_register_value_32_with_security(reg_number, value, cpu->secure);
}

EXC_VOID_2(tlib_set_register_value_32, int, reg_number, uint32_t, value)

//  64 bit support for VFP (Floating-Point coprocessor)
#if defined(TARGET_PROTO_ARM_M)
uint64_t *get_reg_pointer_64(int reg)
{
    switch(reg) {
        case D_0_64 ... D_31_64:
            return &(cpu->vfp.regs[reg - D_0_64]);
        default:
            return NULL;
    }
}

uint64_t tlib_get_register_value_64(int reg_number)
{
    uint64_t *ptr = get_reg_pointer_64(reg_number);
    if(ptr == NULL) {
        tlib_abortf("Read from undefined CPU register number %d detected", reg_number);
    }

    return *ptr;
}

EXC_INT_1(uint64_t, tlib_get_register_value_64, int, reg_number)

void tlib_set_register_value_64(int reg_number, uint64_t value)
{
    if(D_0_64 <= reg_number && reg_number <= D_31_64) {
        warn_about_fp_lsp("D", reg_number - D_0_64);
    }

    uint64_t *ptr = get_reg_pointer_64(reg_number);
    if(ptr == NULL) {
        tlib_abortf("Write to undefined CPU register number %d detected", reg_number);
    }

    *ptr = value;
}

EXC_VOID_2(tlib_set_register_value_64, int, reg_number, uint64_t, value)

#endif

#endif
