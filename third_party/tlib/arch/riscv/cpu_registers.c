/*
 *  RISCV registers interface
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
#include "cpu_registers.h"

//  REMARK: here we use #ifdef/#endif,#ifdef/#endif notation just to be consistent with header file; in header it is required by
//  our parser
#ifdef TARGET_RISCV64
uint64_t *get_reg_pointer_64(int reg)
{
    switch(reg) {
        case X_0_64 ... X_31_64:
            return &(cpu->gpr[reg - X_0_64]);
        case F_0_64 ... F_31_64:
            return &(cpu->fpr[reg - F_0_64]);
        case PC_64:
            return &(cpu->pc);
        case PRIV_64:
            return &(cpu->priv);
        case SSTATUS_64:
            return &(cpu->mstatus);
        case SIE_64:
            return &(cpu->mie);
        case STVEC_64:
            return &(cpu->stvec);
        case SSCRATCH_64:
            return &(cpu->sscratch);
        case SEPC_64:
            return &(cpu->sepc);
        case SCAUSE_64:
            return &(cpu->scause);
        case STVAL_64:
            return &(cpu->stval);
        case SIP_64:
            return &(cpu->mip);
        case SPTBR_64:  //  same index as SATP_64
            return (cpu->privilege_architecture >= RISCV_PRIV1_10) ? &(cpu->satp) : &(cpu->sptbr);
            return &(cpu->sptbr);
        case MSTATUS_64:
            return &(cpu->mstatus);
        case MISA_64:
            return &(cpu->misa);
        case MEDELEG_64:
            return &(cpu->medeleg);
        case MIDELEG_64:
            return &(cpu->mideleg);
        case MIE_64:
            return &(cpu->mie);
        case MTVEC_64:
            return &(cpu->mtvec);
        case MSCRATCH_64:
            return &(cpu->mscratch);
        case MEPC_64:
            return &(cpu->mepc);
        case MCAUSE_64:
            return &(cpu->mcause);
        case MTVAL_64:
            return &(cpu->mtval);
        case MIP_64:
            return &(cpu->mip);
        case VSTART_64:
            return &(cpu->vstart);
        case VXSAT_64:
            return &(cpu->vxsat);
        case VXRM_64:
            return &(cpu->vxrm);
        case VCSR_64:
            return &(cpu->vcsr);
        case VL_64:
            return &(cpu->vl);
        case VTYPE_64:
            return &(cpu->vtype);
        case VLENB_64:
            return &(cpu->vlenb);
        case MSECCFG_64:
            return &(cpu->mseccfg);
        case JVT_64:
            return &(cpu->jvt);
        default:
            break;
    }
    return NULL;
}

CPU_REGISTER_ACCESSOR(64)

uint32_t *get_reg_pointer_32(int reg)
{
    switch(reg) {
        case FFLAGS_32:
            return &(cpu->fflags);
        case FRM_32:
            return &(cpu->frm);
        default:
            break;
    }
    return NULL;
}

CPU_REGISTER_ACCESSOR(32)
#endif
#ifdef TARGET_RISCV32
uint32_t *get_reg_pointer_32(int reg)
{
    switch(reg) {
        case X_0_32 ... X_31_32:
            return &(cpu->gpr[reg - X_0_32]);
        case PC_32:
            return &(cpu->pc);
        case PRIV_32:
            return &(cpu->priv);
        case SSTATUS_32:
            return &(cpu->mstatus);
        case SIE_32:
            return &(cpu->mie);
        case STVEC_32:
            return &(cpu->stvec);
        case SSCRATCH_32:
            return &(cpu->sscratch);
        case SEPC_32:
            return &(cpu->sepc);
        case SCAUSE_32:
            return &(cpu->scause);
        case STVAL_32:
            return &(cpu->stval);
        case SIP_32:
            return &(cpu->mip);
        case SPTBR_32:  //  same index as SATP_32
            return (cpu->privilege_architecture >= RISCV_PRIV1_10) ? &(cpu->satp) : &(cpu->sptbr);
        case MSTATUS_32:
            return &(cpu->mstatus);
        case MISA_32:
            return &(cpu->misa);
        case MEDELEG_32:
            return &(cpu->medeleg);
        case MIDELEG_32:
            return &(cpu->mideleg);
        case MIE_32:
            return &(cpu->mie);
        case MTVEC_32:
            return &(cpu->mtvec);
        case MSCRATCH_32:
            return &(cpu->mscratch);
        case MEPC_32:
            return &(cpu->mepc);
        case MCAUSE_32:
            return &(cpu->mcause);
        case MTVAL_32:
            return &(cpu->mtval);
        case MIP_32:
            return &(cpu->mip);
        case FFLAGS_32:
            return &(cpu->fflags);
        case FRM_32:
            return &(cpu->frm);
        case VSTART_32:
            return &(cpu->vstart);
        case VXSAT_32:
            return &(cpu->vxsat);
        case VXRM_32:
            return &(cpu->vxrm);
        case VCSR_32:
            return &(cpu->vcsr);
        case VL_32:
            return &(cpu->vl);
        case VTYPE_32:
            return &(cpu->vtype);
        case VLENB_32:
            return &(cpu->vlenb);
        case MSECCFG_32:
            return &(cpu->mseccfg);
        case JVT_32:
            return &(cpu->jvt);
        default:
            break;
    }
    return NULL;
}

CPU_REGISTER_ACCESSOR(32)

uint64_t *get_reg_pointer_64(int reg)
{
    switch(reg) {
        case F_0_64 ... F_31_64:
            return &(cpu->fpr[reg - F_0_64]);
        default:
            break;
    }
    return NULL;
}

CPU_REGISTER_ACCESSOR(64)
#endif
