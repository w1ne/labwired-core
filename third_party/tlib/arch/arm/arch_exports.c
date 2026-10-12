/*
 *  ARM interface functions.
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
#include "helper.h"
#include "system_registers.h"
#include "unwind.h"
#include "tightly_coupled_memory.h"
#include "bit_helper.h"
#include "host-utils.h"

//  This is an id representing processor's model (not registration id, and neither SMP id)
uint32_t tlib_get_cpu_model_id()
{
    return cpu->cp15.c0_cpuid;
}

EXC_INT_0(uint32_t, tlib_get_cpu_model_id)

uint32_t tlib_get_it_state()
{
    return cpu->condexec_bits;
}

EXC_INT_0(uint32_t, tlib_get_it_state)

uint32_t tlib_evaluate_condition_code(uint32_t condition)
{
    uint8_t ZF = (env->ZF == 0);
    uint8_t NF = (env->NF & 0x80000000) > 0;
    uint8_t CF = (env->CF == 1);
    uint8_t VF = (env->VF & 0x80000000) > 0;
    switch(condition) {
        case 0b0000:  //  EQ
            return ZF == 1;
        case 0b0001:  //  NE
            return ZF == 0;
        case 0b0010:  //  CS
            return CF == 1;
        case 0b0011:  //  CC
            return CF == 0;
        case 0b0100:  //  MI
            return NF == 1;
        case 0b0101:  //  PL
            return NF == 0;
        case 0b0110:  //  VS
            return VF == 1;
        case 0b0111:  //  VC
            return VF == 0;
        case 0b1000:  //  HI
            return CF == 1 && ZF == 0;
        case 0b1001:  //  LS
            return CF == 0 || ZF == 1;
        case 0b1010:  //  GE
            return NF == VF;
        case 0b1011:  //  LT
            return NF != VF;
        case 0b1100:  //  GT
            return ZF == 0 && NF == VF;
        case 0b1101:  //  LE
            return ZF == 1 || NF != VF;
        case 0b1110:  //  AL
            return 1;
        case 0b1111:  //  NV
            return 0;
        default:
            tlib_printf(LOG_LEVEL_ERROR, "trying to evaluate incorrect condition code (0x%x)", condition);
            return 0;
    }
}

EXC_INT_1(uint32_t, tlib_evaluate_condition_code, uint32_t, condition)

void tlib_set_cpu_model_id(uint32_t value)
{
    cpu->cp15.c0_cpuid = value;
}

EXC_VOID_1(tlib_set_cpu_model_id, uint32_t, value)

void tlib_toggle_fpu(int32_t enabled)
{
    if(enabled) {
        cpu->vfp.xregs[ARM_VFP_FPEXC] |= ARM_VFP_FPEXC_FPUEN_MASK;
    } else {
        cpu->vfp.xregs[ARM_VFP_FPEXC] &= ~ARM_VFP_FPEXC_FPUEN_MASK;
    }
}

EXC_VOID_1(tlib_toggle_fpu, int32_t, enabled)

void tlib_set_sev_on_pending(int32_t value)
{
    cpu->sev_on_pending = !!value;
}

EXC_VOID_1(tlib_set_sev_on_pending, int32_t, value)

void tlib_set_event_flag(void)
{
    cpu->event_register = 1;
}

EXC_VOID_0(tlib_set_event_flag)

void tlib_set_thumb(int value)
{
    cpu->thumb = value != 0;
}

EXC_VOID_1(tlib_set_thumb, int, value)

void tlib_set_number_of_mpu_regions(uint32_t value)
{
    if(value >= MAX_MPU_REGIONS) {
        tlib_abortf("Failed to set number of unified MPU regions to %u, maximal supported value is %u", value, MAX_MPU_REGIONS);
    }
    cpu->number_of_mpu_regions = value;
}

EXC_VOID_1(tlib_set_number_of_mpu_regions, uint32_t, value)

uint32_t tlib_get_number_of_mpu_regions()
{
    return cpu->number_of_mpu_regions;
}

EXC_INT_0(uint32_t, tlib_get_number_of_mpu_regions)

void tlib_register_tcm_region(uint32_t address, uint64_t size, uint64_t index)
{
    //  interface index is the opc2 value when addressing region register via MRC/MCR
    //  region index is the selection register value
    uint32_t interface_index = index >> 32;
    uint32_t region_index = index;
    if(interface_index >= 2) {
        tlib_abortf("Attempted to register TCM region for interface #%u. Only 2 TCM interfaces are supported", interface_index);
    }
    if(size == 0) {
        cpu->cp15.c9_tcmregion[interface_index][region_index] = 0;
        return;
    }

    validate_tcm_region(address, size, region_index, TARGET_PAGE_SIZE);

    cpu->cp15.c9_tcmregion[interface_index][region_index] = address | (ctz64(size / TCM_UNIT_SIZE) << 2) | 1;  //  always enable
}

EXC_VOID_3(tlib_register_tcm_region, uint32_t, address, uint64_t, size, uint64_t, index)

void tlib_update_pmu_counters(int event_id, uint32_t amount)
{
    helper_pmu_update_event_counters(cpu, event_id, amount);
}

EXC_VOID_2(tlib_update_pmu_counters, int, event_id, uint32_t, amount);

void tlib_pmu_set_debug(uint32_t debug)
{
    env->pmu.extra_logs_enabled = debug != 0;
}

EXC_VOID_1(tlib_pmu_set_debug, uint32_t, debug)

uint32_t tlib_get_exception_vector_address(void)
{
    return cpu->cp15.c12_vbar;
}
EXC_INT_0(uint32_t, tlib_get_exception_vector_address)

void tlib_set_exception_vector_address(uint32_t address)
{
    cpu->cp15.c12_vbar = address;
}
EXC_VOID_1(tlib_set_exception_vector_address, uint32_t, address)

uint32_t tlib_get_arm_feature(int32_t feature)
{
    return arm_feature(cpu, feature);
}

EXC_INT_1(uint32_t, tlib_get_arm_feature, int32_t, feature)

#ifdef TARGET_PROTO_ARM_M

void tlib_set_security_state(uint32_t state)
{
    /* Update information about TrustZone support here
     * this is required to turn TZ support on in the core and is a bit hackish
     * since we don't have this information available during first cpu_init
     */
    cpu->v7m.has_trustzone = tlib_has_enabled_trustzone() > 0;

    if(!arm_feature(env, ARM_FEATURE_V8)) {
        if(env->v7m.has_trustzone) {
            cpu_abort(env, "TrustZone enabled for M-Architecture different than V8 is not supported");
        }
    }
    if(!cpu->v7m.has_trustzone) {
        tlib_abort("Changing Security State for CPU with disabled TrustZone");
    }
    cpu->secure = state;
}
EXC_VOID_1(tlib_set_security_state, uint32_t, state)

uint32_t tlib_get_security_state()
{
    if(!cpu->v7m.has_trustzone) {
        tlib_printf(LOG_LEVEL_WARNING, "This CPU has TrustZone disabled, so its security state is bogus");
    }
    return cpu->secure;
}
EXC_INT_0(uint32_t, tlib_get_security_state)

void tlib_set_sleep_on_exception_exit(int32_t value)
{
    cpu->sleep_on_exception_exit = !!value;
}

EXC_VOID_1(tlib_set_sleep_on_exception_exit, int32_t, value)

void tlib_set_interrupt_vector_base(uint32_t address, bool secure)
{
    cpu->v7m.vecbase[secure] = address;
}

EXC_VOID_2(tlib_set_interrupt_vector_base, uint32_t, address, bool, secure)

uint32_t tlib_get_interrupt_vector_base(bool secure)
{
    return cpu->v7m.vecbase[secure];
}

EXC_INT_1(uint32_t, tlib_get_interrupt_vector_base, bool, secure)

uint32_t tlib_get_ccr(bool secure)
{
    return ccr_read(env, secure);
}

EXC_INT_1(uint32_t, tlib_get_ccr, bool, secure)

void tlib_set_ccr(uint32_t value, bool secure)
{
    ccr_write(env, value, secure);
}

EXC_VOID_2(tlib_set_ccr, uint32_t, value, bool, secure)

uint32_t tlib_get_xpsr()
{
    return xpsr_read(cpu);
}

EXC_INT_0(uint32_t, tlib_get_xpsr)

void tlib_do_lazy_floating_point_state_preservation(bool createContext)
{
    if(createContext) {
        helper_fp_lsp(env);
    } else {
        helper_fp_lsp_no_context(env);
    }
}

EXC_VOID_1(tlib_do_lazy_floating_point_state_preservation, bool, createContext)

uint32_t tlib_get_fault_status(bool secure)
{
    return (cpu->v7m.fault_status[secure] & ~BUS_FAULT_STATUS_MASK) | cpu->v7m.bus_fault_status;
}

EXC_INT_1(uint32_t, tlib_get_fault_status, bool, secure)

uint32_t tlib_get_primask(bool secure)
{
    return cpu->v7m.primask[secure];
}

EXC_INT_1(uint32_t, tlib_get_primask, bool, secure)

uint32_t tlib_get_faultmask(bool secure)
{
    return cpu->v7m.faultmask[secure];
}

EXC_INT_1(uint32_t, tlib_get_faultmask, bool, secure)

void tlib_set_fault_status(uint32_t value, bool secure)
{
    cpu->v7m.fault_status[secure] = value & ~BUS_FAULT_STATUS_MASK;
    cpu->v7m.bus_fault_status = value & BUS_FAULT_STATUS_MASK;
}

EXC_VOID_2(tlib_set_fault_status, uint32_t, value, bool, secure)

uint32_t tlib_get_bus_fault_address()
{
    return cpu->v7m.bus_fault_address;
}

EXC_INT_0(uint32_t, tlib_get_bus_fault_address)

void tlib_raise_precise_bus_fault(uint32_t address)
{
    if(cpu->v7m.exception_phase != V7M_EXCEPTION_PHASE_NONE) {
        /* Armv8-M ARM rules RGJJG, RCTKP, and RTCJR distinguish BusFaults
         * generated by exception sequencing from normal instruction data
         * accesses. The surrounding sequence consumes this recorded derived
         * exception after completing the architecturally required state
         * updates. */
        if(cpu->v7m.exception_phase_fault == 0) {
            cpu->v7m.exception_phase_fault = ARMV7M_EXCP_BUS;
        }

        if(cpu->v7m.exception_phase == V7M_EXCEPTION_PHASE_STACKING) {
            cpu->v7m.bus_fault_status |= BUS_FAULT_STKERR;
        } else if(cpu->v7m.exception_phase == V7M_EXCEPTION_PHASE_UNSTACKING) {
            cpu->v7m.bus_fault_status |= BUS_FAULT_UNSTKERR;
        } else if(cpu->v7m.exception_phase == V7M_EXCEPTION_PHASE_LAZY_FP) {
            /* PreserveFPState() uses AccType_LAZYFP. A failed bus access
             * therefore records BFSR.LSPERR, without PRECISERR or BFARVALID
             * (See also MemA_with_priv_security). */
            cpu->v7m.bus_fault_status |= BUS_FAULT_LSPERR;
        }
        return;
    }

    /* Armv8-M ARM, E2.1.294 MemA_with_priv_security, and rules RDDJJ and
     * RFLDT: a synchronous BusFault on a normal data access records BFAR,
     * BFSR.BFARVALID, and BFSR.PRECISERR before exception handling. */
    cpu->v7m.bus_fault_address = address;
    cpu->v7m.bus_fault_status |= BUS_FAULT_BFARVALID | BUS_FAULT_PRECISERR;

    /* CCR.BFHFNMIGN suppresses the exception if it can be ignored, but not the BFAR/BFSR updates. */
    bool requested_priority_is_negative = is_requested_exception_priority_negative(cpu, cpu->secure);
    if(requested_priority_is_negative && (cpu->v7m.ccr[M_REG_COMMON] & FIELD_MASK(V7M_CCR, BFHFNMIGN))) {
        return;
    }

    cpu->tb_interrupt_exception_from_callback = EXCP_BUS_FAULT;
    cpu->tb_interrupt_request_from_callback = TB_INTERRUPT_SYNCHRONOUS_EXCEPTION;
}

EXC_VOID_1(tlib_raise_precise_bus_fault, uint32_t, address)

uint32_t tlib_is_locked_up()
{
    return cpu->v7m.locked_up;
}

EXC_INT_0(uint32_t, tlib_is_locked_up)

void tlib_enter_reset_lockup()
{
    v7m_enter_reset_lockup(cpu);
}

EXC_VOID_0(tlib_enter_reset_lockup)

uint32_t tlib_get_memory_fault_address(bool secure)
{
    return cpu->v7m.memory_fault_address[secure];
}

EXC_INT_1(uint32_t, tlib_get_memory_fault_address, bool, secure)

uint32_t tlib_get_secure_fault_address()
{
    return cpu->v7m.secure_fault_address;
}

EXC_INT_0(uint32_t, tlib_get_secure_fault_address)

uint32_t tlib_get_secure_fault_status()
{
    return cpu->v7m.secure_fault_status;
}

EXC_INT_0(uint32_t, tlib_get_secure_fault_status)

void tlib_set_secure_fault_status(uint32_t value)
{
    cpu->v7m.secure_fault_status = value;
}

EXC_VOID_1(tlib_set_secure_fault_status, uint32_t, value)

uint32_t tlib_is_mpu_enabled()
{
    return cpu->cp15.c1_sys & 0x1;
}

EXC_INT_0(uint32_t, tlib_is_mpu_enabled)

void tlib_enable_mpu(int32_t enabled)
{
    if(!!enabled != (cpu->cp15.c1_sys & 1)) {
        cpu->cp15.c1_sys ^= 1;
        tlb_flush(cpu, 1, false);
    }
}

EXC_VOID_1(tlib_enable_mpu, int32_t, enabled)

void tlib_set_mpu_region_number(uint32_t value)
{
    if(value >= cpu->number_of_mpu_regions) {
        tlib_abortf("MPU: Trying to use non-existent MPU region. Number of regions: %d, faulting region number: %d",
                    cpu->number_of_mpu_regions, value);
    }
    cpu->cp15.c6_region_number = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_mpu_region_number, uint32_t, value)

//  This function mimics mpu configuration through the "Region Base Address" register
void tlib_set_mpu_region_base_address(uint32_t value)
{
    if(value & 0x10) {
        /* If VALID (0x10) bit is set, we change the region number to zero-extended value of youngest 4 bits */
        tlib_set_mpu_region_number(value & 0xF);
    }
    cpu->cp15.c6_base_address[cpu->cp15.c6_region_number] = value & 0xFFFFFFE0;
#if DEBUG
    tlib_printf(LOG_LEVEL_DEBUG, "MPU: Set base address 0x%x, for region %lld", value & 0xFFFFFFE0, cpu->cp15.c6_region_number);
#endif
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_mpu_region_base_address, uint32_t, value)

//  This function mimics mpu configuration through the "Region Attribute and Size" register
void tlib_set_mpu_region_size_and_enable(uint32_t value)
{
    uint32_t index = cpu->cp15.c6_region_number;
    cpu->cp15.c6_size_and_enable[index] = value & MPU_SIZE_AND_ENABLE_FIELD_MASK;
    cpu->cp15.c6_subregion_disable[index] = (value & MPU_SUBREGION_DISABLE_FIELD_MASK) >> MPU_SUBREGION_DISABLE_FIELD_OFFSET;
    cpu->cp15.c6_access_control[index] = value >> 16;
#if DEBUG
    tlib_printf(LOG_LEVEL_DEBUG, "MPU: Set access control 0x%x, permissions 0x%x, size 0x%x, enable 0x%x, for region %lld",
                value >> 16, ((value >> 16) & MPU_PERMISSION_FIELD_MASK) >> 8, (value & MPU_SIZE_FIELD_MASK) >> 1,
                value & MPU_REGION_ENABLED_BIT, index);
#endif
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_mpu_region_size_and_enable, uint32_t, value)

//  This function mimics mpu configuration through the "Region Base Address" register
uint32_t tlib_get_mpu_region_base_address()
{
    return cpu->cp15.c6_base_address[cpu->cp15.c6_region_number] | cpu->cp15.c6_region_number;
}

EXC_INT_0(uint32_t, tlib_get_mpu_region_base_address)

//  This function mimics mpu configuration through the "Region Attribute and Size" register
uint32_t tlib_get_mpu_region_size_and_enable()
{
    uint32_t index = cpu->cp15.c6_region_number;
    return (cpu->cp15.c6_access_control[index] << 16) | (cpu->cp15.c6_subregion_disable[index] << 8) |
           cpu->cp15.c6_size_and_enable[index];
}

EXC_INT_0(uint32_t, tlib_get_mpu_region_size_and_enable)

uint32_t tlib_get_mpu_region_number()
{
    return cpu->cp15.c6_region_number;
}

EXC_INT_0(uint32_t, tlib_get_mpu_region_number)

void tlib_set_number_of_sau_regions(uint32_t value)
{
    if(cpu->number_of_sau_regions == value) {
        return;
    }

    if(value >= MAX_SAU_REGIONS) {
        tlib_abortf("Failed to set number of SAU regions to %u, maximal supported value is %u", value, MAX_SAU_REGIONS);
    }

    if(!cpu->v7m.has_trustzone) {
        tlib_printf(LOG_LEVEL_WARNING, "Setting SAU regions to %u, but TrustZone is not enabled", value);
    }
    cpu->number_of_sau_regions = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_number_of_sau_regions, uint32_t, value)

uint32_t tlib_get_number_of_sau_regions()
{
    return cpu->number_of_sau_regions;
}

EXC_INT_0(uint32_t, tlib_get_number_of_sau_regions)

void tlib_set_sau_control(uint32_t value)
{
    if(cpu->sau.ctrl == value) {
        return;
    }

    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return;
    }
    cpu->sau.ctrl = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_sau_control, uint32_t, value)

void tlib_set_sau_region_number(uint32_t value)
{
    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return;
    }

    if(value >= cpu->number_of_sau_regions) {
        tlib_abortf("SAU: Trying to use non-existent SAU region. Number of regions: %d, faulting region number: %d",
                    cpu->number_of_sau_regions, value);
    }
    cpu->sau.rnr = value;
}

EXC_VOID_1(tlib_set_sau_region_number, uint32_t, value)

void tlib_set_sau_region_base_address(uint32_t value)
{
    uint32_t region = cpu->sau.rnr;
    if(cpu->sau.rbar[region] == value) {
        return;
    }

    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return;
    }
    cpu->sau.rbar[region] = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_sau_region_base_address, uint32_t, value)

void tlib_set_sau_region_limit_address(uint32_t value)
{
    uint32_t region = cpu->sau.rnr;
    if(cpu->sau.rlar[region] == value) {
        return;
    }

    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return;
    }
    cpu->sau.rlar[region] = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_sau_region_limit_address, uint32_t, value)

uint32_t tlib_get_sau_control()
{
    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return 0;
    }
    return cpu->sau.ctrl;
}

EXC_INT_0(uint32_t, tlib_get_sau_control)

uint32_t tlib_get_sau_region_number()
{
    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return 0;
    }
    return cpu->sau.rnr;
}

EXC_INT_0(uint32_t, tlib_get_sau_region_number)

uint32_t tlib_get_sau_region_base_address()
{
    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return 0;
    }
    uint32_t result = cpu->sau.rbar[cpu->sau.rnr];
    return result;
}

EXC_INT_0(uint32_t, tlib_get_sau_region_base_address)

uint32_t tlib_get_sau_region_limit_address()
{
    if(!cpu->secure) {
        //  These are RAZ/WI when accessed from Non-Secure state
        return 0;
    }
    uint32_t result = cpu->sau.rlar[cpu->sau.rnr];
    return result;
}

EXC_INT_0(uint32_t, tlib_get_sau_region_limit_address)

void tlib_set_number_of_idau_regions(uint32_t value)
{
    if(cpu->number_of_idau_regions == value) {
        return;
    }

    if(value >= MAX_IDAU_REGIONS) {
        tlib_abortf("Failed to set number of IDAU regions to %u, maximal supported value is %u", value, MAX_IDAU_REGIONS);
    }
    cpu->number_of_idau_regions = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_number_of_idau_regions, uint32_t, value)

uint32_t tlib_get_number_of_idau_regions()
{
    return cpu->number_of_idau_regions;
}

EXC_INT_0(uint32_t, tlib_get_number_of_idau_regions)

void tlib_set_idau_enabled(bool value)
{
    if(value == cpu->idau.enabled) {
        return;
    }
    cpu->idau.enabled = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_idau_enabled, bool, value)

void tlib_set_custom_idau_handler_enabled(bool value)
{
    if(value == cpu->idau.custom_handler_enabled) {
        return;
    }
    cpu->idau.custom_handler_enabled = value;
    tcg_context_use_tlb(!value);
    tb_flush(cpu);
    tlb_flush(cpu, 1, false);
}

EXC_VOID_1(tlib_set_custom_idau_handler_enabled, bool, value)

uint32_t tlib_get_idau_enabled()
{
    return cpu->idau.enabled;
}

EXC_INT_0(uint32_t, tlib_get_idau_enabled)

void tlib_set_idau_region_base_address_register(uint32_t index, uint32_t value)
{
    if(index >= cpu->number_of_idau_regions) {
        tlib_abortf("IDAU: Trying to use non-existent IDAU region. Number of regions: %u, faulting region number: %u",
                    cpu->number_of_idau_regions, index);
    }

    uint32_t flags = pmsav8_idau_sau_get_flags(value);
    if(flags) {
        tlib_printf(LOG_LEVEL_WARNING, "IDAU: Unsetting invalid RBAR flags used for region %u: 0x%02x; RBAR has no flags", index,
                    flags);
        value &= ~flags;
    }

    if(cpu->idau.rbar[index] == value) {
        return;
    }
    cpu->idau.rbar[index] = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_2(tlib_set_idau_region_base_address_register, uint32_t, index, uint32_t, value)

void tlib_set_idau_region_limit_address_register(uint32_t index, uint32_t value)
{
    if(index >= cpu->number_of_idau_regions) {
        tlib_abortf("IDAU: Trying to use non-existent IDAU region. Number of regions: %u, faulting region number: %u",
                    cpu->number_of_idau_regions, index);
    }

    uint32_t valid_flags = IDAU_SAU_RLAR_ENABLE | IDAU_SAU_RLAR_NSC;
    uint32_t values_invalid_flags = pmsav8_idau_sau_get_flags(value) & ~valid_flags;
    if(values_invalid_flags) {
        tlib_printf(LOG_LEVEL_WARNING, "IDAU: Unsetting invalid RLAR flags used for region %u: 0x%02x; valid flags are: 0x%02x",
                    index, values_invalid_flags, valid_flags);
        value &= ~values_invalid_flags;
    }

    if(cpu->idau.rlar[index] == value) {
        return;
    }
    cpu->idau.rlar[index] = value;
    tlb_flush(cpu, 1, false);
}

EXC_VOID_2(tlib_set_idau_region_limit_address_register, uint32_t, index, uint32_t, value)

uint32_t tlib_get_idau_region_base_address_register(uint32_t index)
{
    if(index >= cpu->number_of_idau_regions) {
        tlib_abortf("IDAU: Trying to use non-existent IDAU region. Number of regions: %u, faulting region number: %u",
                    cpu->number_of_idau_regions, index);
    }
    uint32_t result = cpu->idau.rbar[index];
    return result;
}

EXC_INT_1(uint32_t, tlib_get_idau_region_base_address_register, uint32_t, index)

uint32_t tlib_get_idau_region_limit_address_register(uint32_t index)
{
    if(index >= cpu->number_of_idau_regions) {
        tlib_abortf("IDAU: Trying to use non-existent IDAU region. Number of regions: %u, faulting region number: %u",
                    cpu->number_of_idau_regions, index);
    }
    uint32_t result = cpu->idau.rlar[index];
    return result;
}

EXC_INT_1(uint32_t, tlib_get_idau_region_limit_address_register, uint32_t, index)

bool tlib_get_process_sp()
{
    return env->v7m.process_sp;
}

EXC_INT_0(bool, tlib_get_process_sp)

void tlib_set_process_sp(bool use_process_sp)
{
    switch_v7m_sp(env, use_process_sp);
}

EXC_VOID_1(tlib_set_process_sp, bool, use_process_sp)

bool tlib_try_add_implementation_defined_exemption_region(uint32_t start, uint32_t end)
{
    if(cpu->impl_def_attr_exemptions.count + 1 >= MAX_IMPL_DEF_ATTRIBUTION_EXEMPTIONS) {
        tlib_printf(LOG_LEVEL_ERROR,
                    "Adding implementation-defined exemption region 0x%08" PRIx32 "-0x%08" PRIx32 " failed; "
                    "max number of implementation-defined exemption regions reached: %u",
                    start, end, MAX_IMPL_DEF_ATTRIBUTION_EXEMPTIONS);
        return false;
    }

    //  Check alignment.
    if(pmsav8_idau_sau_get_region_base(start) != start || pmsav8_idau_sau_get_region_limit(end) != end) {
        tlib_printf(LOG_LEVEL_ERROR,
                    "Adding implementation-defined exemption region 0x%08" PRIx32 "-0x%08" PRIx32 " failed; "
                    "region must be aligned to %uB granularity with end address being included, e.g. 0x0-0x1F is correct while "
                    "0x0-0x20 isn't",
                    start, end, PMSAV8_IDAU_SAU_REGION_GRANULARITY_B);
        return false;
    }
    uint32_t index = cpu->impl_def_attr_exemptions.count++;

    cpu->impl_def_attr_exemptions.start[index] = start;
    cpu->impl_def_attr_exemptions.end[index] = end;

    tlb_flush(cpu, 1, false);
    return true;
}

EXC_INT_2(bool, tlib_try_add_implementation_defined_exemption_region, uint32_t, start, uint32_t, end)

bool tlib_try_remove_implementation_defined_exemption_region(uint32_t start, uint32_t end)
{
    uint32_t region_index;
    uint32_t start_at = 0;
    bool region_found = false;
    while(start_at < cpu->impl_def_attr_exemptions.count) {
        if(!try_get_impl_def_attr_exemption_region(start, start_at, &region_index, /* applies_to_whole_page: */ NULL)) {
            break;
        }

        uint32_t region_start = cpu->impl_def_attr_exemptions.start[region_index];
        uint32_t region_end = cpu->impl_def_attr_exemptions.end[region_index];
        if(region_start == start && region_end == end) {
            region_found = true;
            break;
        } else {
            //  Regions can overlap so let's check regions past this one too.
            start_at = region_index + 1;
        }
    }

    if(!region_found) {
        tlib_printf(LOG_LEVEL_ERROR,
                    "Removing implementation-defined exemption region 0x%08" PRIx32 "-0x%08" PRIx32 " failed; region not found",
                    start, end);
        return false;
    }
    cpu->impl_def_attr_exemptions.count--;

    //  Let's move the last region, disabled by decreased regions count, in place of the removed one
    //  unless the last region or the only region is being removed.
    uint32_t last_index = cpu->impl_def_attr_exemptions.count;
    if(region_index != last_index) {
        cpu->impl_def_attr_exemptions.start[region_index] = cpu->impl_def_attr_exemptions.start[last_index];
        cpu->impl_def_attr_exemptions.end[region_index] = cpu->impl_def_attr_exemptions.end[last_index];
    }

    tlb_flush(cpu, 1, false);
    return true;
}

EXC_INT_2(bool, tlib_try_remove_implementation_defined_exemption_region, uint32_t, start, uint32_t, end)

/* See vfp_trigger_exception for irq_number value interpretation */
void tlib_set_fpu_interrupt_number(int32_t irq_number)
{
    cpu->vfp.fpu_interrupt_irq_number = irq_number;
}

EXC_VOID_1(tlib_set_fpu_interrupt_number, int32_t, irq_number)

uint32_t tlib_get_architecture_version()
{
    if(arm_feature(env, ARM_FEATURE_V8)) {
        return 8;
    } else if(arm_feature(env, ARM_FEATURE_V7)) {
        return 7;
    } else if(arm_feature(env, ARM_FEATURE_V6)) {
        return 6;
    } else if(arm_feature(env, ARM_FEATURE_V5)) {
        return 5;
    } else if(arm_feature(env, ARM_FEATURE_V4T)) {
        return 4;
    } else {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Getting architecture version below ARMv4T, returning 3, but the version might be different than that");
        return 3;
    }
}
EXC_INT_0(uint32_t, tlib_get_architecture_version)

/* PMSAv8 */

static void guard_pmsav8(bool is_write)
{
    if(!arm_feature(env, ARM_FEATURE_V8)) {
        tlib_abort("This feature is only supported on ARM v8-M architecture");
    }
    if(is_write) {
        tlb_flush(cpu, 1, false);
    }
}

void tlib_set_pmsav8_ctrl(uint32_t value, bool secure)
{
    guard_pmsav8(true);
    cpu->pmsav8[secure].ctrl = value;
}
EXC_VOID_2(tlib_set_pmsav8_ctrl, uint32_t, value, bool, secure)

void tlib_set_pmsav8_rnr(uint32_t value, bool secure)
{
    guard_pmsav8(true);
    if(value > MAX_MPU_REGIONS) {
        tlib_printf(LOG_LEVEL_ERROR, "Requested RNR value is greater than the maximum MPU regions");
        return;
    }
    cpu->pmsav8[secure].rnr = value;
}
EXC_VOID_2(tlib_set_pmsav8_rnr, uint32_t, value, bool, secure)

void tlib_set_pmsav8_rbar(uint32_t value, uint32_t region_offset, bool secure)
{
    guard_pmsav8(true);
    uint32_t index = cpu->pmsav8[secure].rnr;
    if(region_offset > 0) {
        index = (index << 2) + region_offset;
    }
    cpu->pmsav8[secure].rbar[index] = value;
}
EXC_VOID_3(tlib_set_pmsav8_rbar, uint32_t, value, uint32_t, region_offset, bool, secure)

void tlib_set_pmsav8_rlar(uint32_t value, uint32_t region_offset, bool secure)
{
    guard_pmsav8(true);
    uint32_t index = cpu->pmsav8[secure].rnr;
    if(region_offset > 0) {
        index = (index << 2) + region_offset;
    }

    //  XN is enforced in 0xE0000000-0xFFFFFFFF space; ARMv8-M Manual: Rules VCTC and KDJG.
    bool region_enabled = value & 0x1;
    if(region_enabled) {
        bool xn = extract32(cpu->pmsav8[secure].rbar[index], 4, 1);
        if(!xn && pmsav8_idau_sau_get_region_limit(value) >= 0xE0000000) {
            tlib_printf(LOG_LEVEL_WARNING,
                        "Enabled MPU region %u without Execute-Never bit set includes addresses from "
                        "0xE0000000-0xFFFFFFFF address space for which instruction fetch is"
                        " architecturally prohibited so it won't be possible nevertheless",
                        index);
        }
    }
    cpu->pmsav8[secure].rlar[index] = value;
}
EXC_VOID_3(tlib_set_pmsav8_rlar, uint32_t, value, uint32_t, region_offset, bool, secure)

void tlib_set_pmsav8_mair(uint32_t index, uint32_t value, bool secure)
{
    guard_pmsav8(true);
    if(index > 1) {
        tlib_printf(LOG_LEVEL_ERROR, "Only indexes {0,1} are supported by MAIR registers");
        return;
    }
    cpu->pmsav8[secure].mair[index] = value;
}
EXC_VOID_3(tlib_set_pmsav8_mair, uint32_t, index, uint32_t, value, bool, secure)

uint32_t tlib_get_pmsav8_ctrl(bool secure)
{
    guard_pmsav8(false);
    return cpu->pmsav8[secure].ctrl;
}
EXC_INT_1(uint32_t, tlib_get_pmsav8_ctrl, bool, secure)

uint32_t tlib_get_pmsav8_rnr(bool secure)
{
    guard_pmsav8(false);
    return cpu->pmsav8[secure].rnr;
}
EXC_INT_1(uint32_t, tlib_get_pmsav8_rnr, bool, secure)

uint32_t tlib_get_pmsav8_rbar(uint32_t region_offset, bool secure)
{
    guard_pmsav8(false);
    uint32_t index = cpu->pmsav8[secure].rnr;
    if(region_offset > 0) {
        index = (index << 2) + region_offset;
    }
    return cpu->pmsav8[secure].rbar[index];
}
EXC_INT_2(uint32_t, tlib_get_pmsav8_rbar, uint32_t, region_offset, bool, secure)

uint32_t tlib_get_pmsav8_rlar(uint32_t region_offset, bool secure)
{
    guard_pmsav8(false);
    uint32_t index = cpu->pmsav8[secure].rnr;
    if(region_offset > 0) {
        index = (index << 2) + region_offset;
    }
    return cpu->pmsav8[secure].rlar[index];
}
EXC_INT_2(uint32_t, tlib_get_pmsav8_rlar, uint32_t, region_offset, bool, secure)

uint32_t tlib_get_pmsav8_mair(uint32_t index, bool secure)
{
    guard_pmsav8(false);
    if(index > 1) {
        tlib_printf(LOG_LEVEL_ERROR, "Only indexes {0,1} are supported by MAIR registers");
        return 0;
    }
    return cpu->pmsav8[secure].mair[index];
}
EXC_INT_2(uint32_t, tlib_get_pmsav8_mair, uint32_t, index, bool, secure)

#endif
