#include "cpu.h"
#include "cpu_names.h"
#include "helper.h"
#include "syndrome.h"

/* All these helpers have been based on tlib's 'arm/helper.c'. */

int bank_number(int mode)
{
    switch(mode) {
        case ARM_CPU_MODE_USR:
        case ARM_CPU_MODE_SYS:
            return BANK_USRSYS;
        case ARM_CPU_MODE_SVC:
            return 1;
        case ARM_CPU_MODE_ABT:
            return 2;
        case ARM_CPU_MODE_UND:
            return 3;
        case ARM_CPU_MODE_IRQ:
            return 4;
        case ARM_CPU_MODE_FIQ:
            return 5;
        case ARM_CPU_MODE_HYP:
            return 6;
        case ARM_CPU_MODE_MON:
            return 7;
    }
    cpu_abort(cpu, "Bad mode %x\n", mode);
    __builtin_unreachable();
}

int r14_bank_number(int mode)
{
    //  Arm A-profile manual: "User mode, System mode, and Hyp mode share the same LR."
    return mode == ARM_CPU_MODE_HYP ? BANK_USRSYS : bank_number(mode);
}

void switch_mode(CPUState *env, int mode)
{
    int old_mode;
    int i;

    old_mode = env->uncached_cpsr & CPSR_M;
    if(mode == old_mode) {
        return;
    }

    if(old_mode == ARM_CPU_MODE_FIQ) {
        memcpy(env->fiq_regs, env->regs + 8, 5 * sizeof(uint32_t));
        memcpy(env->regs + 8, env->usr_regs, 5 * sizeof(uint32_t));
    } else if(mode == ARM_CPU_MODE_FIQ) {
        memcpy(env->usr_regs, env->regs + 8, 5 * sizeof(uint32_t));
        memcpy(env->regs + 8, env->fiq_regs, 5 * sizeof(uint32_t));
    }

    i = bank_number(old_mode);
    env->banked_r13[i] = env->regs[13];
    env->banked_r14[r14_bank_number(old_mode)] = env->regs[14];
    env->banked_spsr[i] = env->spsr;

    i = bank_number(mode);
    env->regs[13] = env->banked_r13[i];
    env->regs[14] = env->banked_r14[r14_bank_number(mode)];
    env->spsr = env->banked_spsr[i];

    tlib_on_execution_mode_changed(arm_cpu_mode_to_el(env, mode), arm_is_secure(env));
}

static bool is_target_mode_valid(CPUState *env, uint32_t current_mode, uint32_t target_mode, CPSRWriteType write_type)
{
    //  trivial case which is always true
    if(target_mode == current_mode) {
        return true;
    }

    uint32_t target_el = arm_cpu_mode_to_el(env, target_mode);
    if(target_el == -1) {
        return false;
    }

    if(write_type == CPSRWriteByInstr) {

        //  change to/from a hyp mode is not allowed by instruction
        if(current_mode == ARM_CPU_MODE_HYP || target_mode == ARM_CPU_MODE_HYP) {
            return false;
        }

        //  change to a higher exception level is not allowed by instruction
        uint32_t current_el = arm_current_el(env);
        if(target_el > current_el) {
            return false;
        }
    }

    return true;
}

void cpsr_write(CPUState *env, uint32_t val, uint32_t mask, CPSRWriteType write_type)
{
    if(mask & CPSR_NZCV) {
        env->ZF = (~val) & CPSR_Z;
        env->NF = val;
        env->CF = (val >> 29) & 1;
        env->VF = (val << 3) & 0x80000000;
    }
    if(mask & CPSR_Q) {
        env->QF = ((val & CPSR_Q) != 0);
    }
    if(mask & CPSR_T) {
        env->thumb = ((val & CPSR_T) != 0);
    }
    if(mask & CPSR_IT_0_1) {
        env->condexec_bits &= ~3;
        env->condexec_bits |= (val >> 25) & 3;
    }
    if(mask & CPSR_IT_2_7) {
        env->condexec_bits &= 3;
        env->condexec_bits |= (val >> 8) & 0xfc;
    }
    if(mask & CPSR_GE) {
        env->GE = (val >> 16) & 0xf;
    }

    //  always update AIF flags
    uint64_t daif_mask = CPSR_AIF & mask;
    env->daif = (env->daif & ~daif_mask) | (val & daif_mask);

    //  Write to CPSR during normal execution may change the mode
    //  and bank the appropriate registers. The CPSRWriteRaw write type
    //  is used to prevent these additional effects.

    uint64_t mode_mask = CPSR_M & mask;
    bool change_mode = (env->uncached_cpsr ^ val) & mode_mask;
    bool normal_exec = write_type != CPSRWriteRaw;

    if(change_mode) {
        uint32_t target_mode = val & mode_mask;

        if(normal_exec) {
            uint32_t current_mode = env->uncached_cpsr & mode_mask;

            //  if target mode is invalid do not change the mode and set CPSR_IL
            if(!is_target_mode_valid(env, current_mode, target_mode, write_type)) {
                mask = (mask & ~CPSR_M) | CPSR_IL;
                val |= CPSR_IL;
            }

            switch_mode(env, target_mode);
        } else {
            //  Trust the received target_mode, and don't verify it with `is_target_mode_valid`
            tlib_on_execution_mode_changed(arm_cpu_mode_to_el(env, target_mode), arm_is_secure(env));
        }
    }

    mask &= ~CACHED_CPSR_BITS;
    env->uncached_cpsr = (env->uncached_cpsr & ~mask) | (val & mask);

    if(normal_exec) {
        arm_rebuild_hflags(env);
    }

    find_pending_irq_if_primask_unset(env);
}

uint32_t cpsr_read(CPUARMState *env)
{
    int ZF;
    ZF = (env->ZF == 0);
    return env->uncached_cpsr | (env->NF & 0x80000000) | (ZF << 30) | (env->CF << 29) | ((env->VF & 0x80000000) >> 3) |
           (env->QF << 27) | (env->thumb << 5) | ((env->condexec_bits & 3) << 25) | ((env->condexec_bits & 0xfc) << 8) |
           (env->GE << 16) | (env->daif & CPSR_AIF);
}

//  Copied from arch/arm/helper.c:do_interrupt()
void do_interrupt_a32(CPUState *env)
{
    uint32_t addr;
    uint32_t mask;
    int new_mode;
    uint32_t offset;
    uint32_t dbgdscr_moe;

    uint32_t target_el = env->exception.target_el;
    addr = env->cp15.vbar_el[target_el];

#ifdef TARGET_PROTO_ARM_M
    do_interrupt_v7m(env);
    return;
#endif
    switch(syn_get_ec(env->exception.syndrome)) {
        case SYN_EC_BREAKPOINT_LOWER_EL:
        case SYN_EC_BREAKPOINT_SAME_EL:
            dbgdscr_moe = 0b0001;
            break;
        case SYN_EC_WATCHPOINT_LOWER_EL:
        case SYN_EC_WATCHPOINT_SAME_EL:
            dbgdscr_moe = 0b0010;
            break;
        case SYN_EC_AA32_BKPT:
            dbgdscr_moe = 0b0011;
            break;
        case SYN_EC_AA32_VECTOR_CATCH:
            dbgdscr_moe = 0b0101;
            break;
        default:
            dbgdscr_moe = 0;
            break;
    }
    if(dbgdscr_moe) {
        env->cp15.mdscr_el1 = deposit64(env->cp15.mdscr_el1, 2, 4, dbgdscr_moe);
    }
    /* TODO: Vectored interrupt controller.  */
    switch(env->exception_index) {
        case EXCP_UDEF:
            new_mode = ARM_CPU_MODE_UND;
            addr += 0x04;
            mask = CPSR_I;
            if(env->thumb) {
                offset = 2;
            } else {
                offset = 4;
            }
            if(target_el == 3) {
                cpu_abort(env, "EXCP_UDEF not available in Monitor mode");
            }
            break;
        case EXCP_SMC:
            if(target_el != 2) {
                cpu_abort(env, "EXCP_SMC available only in Monitor mode");
            }
            goto case_EXCP_SWI_SVC;
        case EXCP_HVC:
            if(target_el != 2) {
                cpu_abort(env, "EXCP_HVC available only in Hypervisor mode");
            }
            goto case_EXCP_SWI_SVC;
        case EXCP_SWI_SVC:
        case_EXCP_SWI_SVC:
            new_mode = ARM_CPU_MODE_SVC;
            addr += 0x08;
            mask = CPSR_I;
            /* The PC already points to the next instruction.  */
            offset = 0;
            break;
        case EXCP_BKPT:
        case EXCP_PREFETCH_ABORT:
            new_mode = ARM_CPU_MODE_ABT;
            addr += 0x0c;
            mask = CPSR_A | CPSR_I;
            offset = 4;
            break;
        case EXCP_DATA_ABORT:
            new_mode = ARM_CPU_MODE_ABT;
            addr += 0x10;
            mask = CPSR_A | CPSR_I;
            offset = 8;
            break;
        case EXCP_IRQ:
        case EXCP_VIRQ:
            new_mode = ARM_CPU_MODE_IRQ;
            addr += 0x18;
            /* Disable IRQ and imprecise data aborts.  */
            mask = CPSR_A | CPSR_I;
            offset = 4;
            break;
        case EXCP_FIQ:
        case EXCP_VFIQ:
            new_mode = ARM_CPU_MODE_FIQ;
            addr += 0x1c;
            /* Disable FIQ, IRQ and imprecise data aborts.  */
            mask = CPSR_A | CPSR_I | CPSR_F;
            offset = 4;
            break;
        case EXCP_HYP_TRAP: {
            new_mode = ARM_CPU_MODE_HYP;
            addr += 0x14;
            mask = CPSR_I;
            offset = 4;
            uint32_t ec = env->exception.syndrome >> SYN_EC_SHIFT;
            //  For Data Aborts, mark the syndrome as invalid if it came from Thumb mode
            if(ec == SYN_EC_DATA_ABORT_LOWER_EL || ec == SYN_EC_DATA_ABORT_SAME_EL) {
                //  At this point, PC points to the instruction that attempted the transfer
                //  (it was restored by restore_state_to_opc)
                uint32_t prev_pc = env->regs[15];
                //  Check whether the CPU is currently in thumb mode or whether the previous PC has LSB set,
                //  which indicates we used to be in Thumb mode
                if(env->thumb || (prev_pc & 1)) {
                    tlib_printf(LOG_LEVEL_WARNING, "CPU in Thumb mode. The instruction syndrome will not be set");
                    //  Clear the ISV bit to say that the ISS is invalid
                    env->exception.syndrome &= ~(SYN_DATA_ABORT_ISV);
                    break;
                }
            }
            break;
        }
        default:
            cpu_abort(env, "Unhandled exception 0x%x\n", env->exception_index);
            return; /* Never happens.  Keep compiler happy.  */
    }
    if(target_el == 2) {
        new_mode = ARM_CPU_MODE_HYP;
        offset = 0;
        if(arm_feature(env, ARM_FEATURE_EL3)) {
            mask = 0;
            if(!(env->cp15.scr_el3 & SCR_EA)) {
                mask |= CPSR_A;
            }
            if(!(env->cp15.scr_el3 & SCR_IRQ)) {
                mask |= CPSR_I;
            }
            if(!(env->cp15.scr_el3 & SCR_FIQ)) {
                mask |= CPSR_F;
            }
        }
    }
    if(env->exception_index != EXCP_IRQ && env->exception_index != EXCP_FIQ &&
       //  The [di]far/[di]fsr regiesters are set to proper values, they are kept in union with AA64 esr_el and this would
       //  overwrite them
       env->exception_index != EXCP_DATA_ABORT && env->exception_index != EXCP_PREFETCH_ABORT) {
        env->cp15.esr_el[target_el] = env->exception.syndrome;
    }
    /* High vectors.  */
    if(env->cp15.sctlr_ns & (1 << 13)) {
        addr += 0xffff0000;
    }
    switch_mode(env, new_mode);
    env->spsr = cpsr_read(env);
    /* Clear IT bits.  */
    env->condexec_bits = 0;
    /* Switch to the new mode, and to the correct instruction set.  */
    env->uncached_cpsr = (env->uncached_cpsr & ~CPSR_M) | new_mode;
    env->daif |= (mask & CPSR_AIF);

    find_pending_irq_if_primask_unset(env);

    /* this is a lie, as the was no c1_sys on V4T/V5, but who cares
     * and we should just guard the thumb mode on V4 */
    if(arm_feature(env, ARM_FEATURE_V4T)) {
        env->thumb = (env->cp15.sctlr_ns & (1 << 30)) != 0;
    }
    if(target_el == 2) {
        env->elr_el[2] = env->regs[15];
    } else {
        env->regs[14] = env->regs[15] + offset;
    }
    env->regs[15] = addr;
    set_interrupt_pending(env, CPU_INTERRUPT_EXITTB);

    arm_rebuild_hflags(env);

    //  arm_announce_stack_change();
}

//  Exracted from cpu_reset.
void cpu_reset_vfp(CPUState *env)
{
    set_flush_to_zero(1, &env->vfp.standard_fp_status);
    set_flush_inputs_to_zero(1, &env->vfp.standard_fp_status);
    set_default_nan_mode(1, &env->vfp.standard_fp_status);
    set_float_detect_tininess(float_tininess_before_rounding, &env->vfp.fp_status);
    set_float_detect_tininess(float_tininess_before_rounding, &env->vfp.standard_fp_status);

    env->vfp.xregs[ARM_VFP_FPEXC] = 0x700;  //  Bits 8-10 are RES1 for A53, A75 and R52 where it's accessible.
    env->vfp.xregs[ARM_VFP_FPSID] = env->arm_core_config.reset_fpsid;
}

/* return 0 if not found */
uint32_t cpu_arm_find_by_name(const char *name)
{
    int i;
    uint32_t id;

    id = 0;
    for(i = 0; arm_cpu_names[i].name; i++) {
        if(strcmp(name, arm_cpu_names[i].name) == 0) {
            id = arm_cpu_names[i].id;
            break;
        }
    }
    return id;
}

int cpu_init(const char *cpu_model)
{
    uint32_t id;

    id = cpu_arm_find_by_name(cpu_model);
    if(id == ARM_CPUID_NOT_FOUND) {
        tlib_printf(LOG_LEVEL_ERROR, "Unknown CPU model: %s", cpu_model);
        return -1;
    }
    env->cp15.c0_cpuid = id;

    cpu_init_v8(cpu, id);
    cpu_reset(cpu);
    return 0;
}

void set_mmu_fault_registers(int access_type, target_ulong address, int fault_type)
{
    if(access_type == ACCESS_INST_FETCH) {
        env->cp15.ifsr_ns = fault_type;
        env->cp15.ifar_ns = address;
        env->exception_index = EXCP_PREFETCH_ABORT;
    } else {
        uint32_t isWriteBit = (access_type == ACCESS_DATA_STORE ? 1u : 0u) << 11;
        env->cp15.dfsr_ns = fault_type | isWriteBit;
        env->cp15.dfar_ns = address;
        env->exception_index = EXCP_DATA_ABORT;
    }
}

#define PMSA_ATTRIBUTE_ONLY_SAME_LEVEL(setting) ((setting & 0b1) == 0)
#define PMSA_ATTRIBUTE_IS_READONLY(setting)     ((setting & 0b10) != 0)

inline uint32_t pmsav8_number_of_el1_regions(CPUState *env)
{
    return extract32(env->arm_core_config.mpuir, 8, 8);
}

inline uint32_t pmsav8_number_of_el2_regions(CPUState *env)
{
    return extract32(env->arm_core_config.hmpuir, 0, 8);
}

void set_pmsav8_regions_count(CPUState *env, uint32_t el1_regionscount, uint32_t el2_regions_count)
{
    env->arm_core_config.mpuir = deposit32(env->arm_core_config.mpuir, 8, 8, el1_regionscount);
    env->arm_core_config.hmpuir = deposit32(env->arm_core_config.hmpuir, 0, 8, el2_regions_count);
}

static inline int get_default_memory_map_access(uint32_t current_el, target_ulong address)
{
    int prot = 0;
    switch(current_el) {
        case 0 ... 2:
            switch(address) {
                case 0x00000000 ... 0x7FFFFFFF:
                    prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
                    break;
                //  Devices
                case 0x80000000 ... 0xFFFFFFFF:
                    prot = PAGE_READ | PAGE_WRITE;
                    break;
                default:
                    tlib_abortf("Address out of range. This should never happen");
            }
            break;
        default:
            tlib_abortf("The EL == %d is not supported yet", current_el);
            break;
    }
    /* This should take the access type under consideration as well, but it would influence only the cacheability and sherability.
       Neither of this have any influence on our simulation - the memory is always treated in the same way. */
    return prot;
}

static inline int find_first_matching_region_for_addr_masked(pmsav8_region *regions, target_ulong address, int start_index,
                                                             int regions_count, uint64_t mask)
{
    pmsav8_region region;
    int index = start_index;
    mask = mask >> start_index;
    while(mask && (index < regions_count)) {
        if(mask & 0x1) {
            region = regions[index];
            if(region.enabled && (address >= region.address_start) && (address <= region.address_limit)) {
                return index;
            }
        }
        mask = mask >> 1;
        index++;
    }

    return -1;
}

static inline int find_first_matching_region_for_addr(pmsav8_region *regions, target_ulong address, int regions_count)
{
    return find_first_matching_region_for_addr_masked(regions, address, 0, regions_count, UINT64_MAX);
}

static inline int get_region_prot(pmsav8_region *region, int current_el, bool is_el2_translation)
{
    int prot = 0;
    if(region == NULL) {
        return prot;
    }
    uint8_t access_permission_bits = region->access_permission_bits;
    prot |= (region->execute_never ? 0 : PAGE_EXEC);
    if(!PMSA_ATTRIBUTE_ONLY_SAME_LEVEL(access_permission_bits) || current_el == (is_el2_translation ? 2 : 1)) {
        prot |= PAGE_READ;
        if(!PMSA_ATTRIBUTE_IS_READONLY(access_permission_bits)) {
            prot |= PAGE_WRITE;
        }
    }
    return prot;
}

enum mpu_result {
    FOUND,
    NOT_FOUND,
    OVERLAP,
};

static enum mpu_result pmsav8_mpu_find_matching_region(pmsav8_region *regions, target_ulong address, int num_regions,
                                                       pmsav8_region *found_region)
{
    int found_region_index = find_first_matching_region_for_addr(regions, address, num_regions);
    if(found_region_index != -1) {
        *found_region = regions[found_region_index];
        if(unlikely(found_region->overlapping_regions_mask)) {
            //  Only need to check regions that follow that one
            if(find_first_matching_region_for_addr_masked(regions, address, found_region_index + 1, num_regions,
                                                          found_region->overlapping_regions_mask) != -1) {
                return OVERLAP;
            }
        }
    } else {
        return NOT_FOUND;
    }
    return FOUND;
}

//  Two-stage translation modeled after figures C1-2 and C1-3 from ARM ID110520, as well as the pseudocode from section H1.2.4
//  NOTE: We don't use the convention the pseudocode uses (first and second stage).
//        Instead, we structure the code in a similar fashion to figures C1-2 and C1-3,
//        i.e. we define an EL1 stage and EL2 stage, both of which are optional,
//        though at least one always takes place, and EL1 always goes before EL2.
static bool pmsav8_translate_stage(CPUState *env, target_ulong address, uint32_t current_el, uint32_t access_type,
                                   int *out_fault_type, int *out_prot, bool use_el2_mpu)
{
    int mpu_el = use_el2_mpu ? 2 : 1;
    tlib_assert(current_el <= mpu_el);
    uint64_t sctlr = arm_sctlr_eff(env, mpu_el);
    //  The logic is broken for HCTLR.M == 0 and HSCTLR.BR == 0 in Figure C1-3 from Armv8 Manual Supplement for Armv8-R AArch32.
    //  It's contrary to section C1.3:
    //  "Each PMSAv8-32 MPU has an associated default memory map which is used when the MPU is not enabled.".
    //  The translation should fail with HSCTLR.BR == 0 only when the MPU is enabled and no MPU region was hit for the address.
    if((sctlr & SCTLR_M) == 0) {
        goto default_mapping;
    }

    pmsav8_region *mpu_regions = use_el2_mpu ? env->pmsav8.hregions : env->pmsav8.regions;
    int num_regions = use_el2_mpu ? pmsav8_number_of_el2_regions(env) : pmsav8_number_of_el1_regions(env);
    pmsav8_region region;
    enum mpu_result result = pmsav8_mpu_find_matching_region(mpu_regions, address, num_regions, &region);

    switch(result) {
        case FOUND:
            *out_prot = get_region_prot(&region, current_el, use_el2_mpu);
            goto check_permissions;
        case OVERLAP:
            *out_fault_type = TRANSLATION_FAULT;
            return false;
        case NOT_FOUND:
            if((sctlr & SCTLR_BR) == 0) {
                *out_fault_type = BACKGROUND_FAULT;
                return false;
            }
            break;
            /* Fall-through to default_mapping */
    }

default_mapping:
    if(current_el == mpu_el) {
        *out_prot = pmsav8_default_cacheability_enabled(env) ? PAGE_READ | PAGE_WRITE | PAGE_EXEC
                                                             : get_default_memory_map_access(current_el, address);
    } else {
        *out_fault_type = TRANSLATION_FAULT;
        return false;
    }

    /* Fall-through to check_permissions */

check_permissions:
    if(!is_page_access_valid(*out_prot, access_type)) {
        *out_fault_type = PERMISSION_FAULT;
        return false;
    }
    return true;
}

//  All addresses are flat mapped -> (virtual address == physical address), all we do is figure out the access permissions and
//  memory attributes. There is no distinction between reads from data/instruction fetch paths, hence the execute_never attribute.
//  There is no need to respect the cacheability and shareability settings - we handle it all as shareable and cacheable anyway,
//  with all the necessary restrictions and precautions.
int get_phys_addr_pmsav8(CPUState *env, target_ulong address, int access_type, uint32_t current_el, uintptr_t return_address,
                         bool suppress_faults, target_ulong *phys_ptr, int *prot, target_ulong *page_size, int access_width)
{
    tlib_assert(current_el <= 2);
    int fault_type = TRANSLATION_FAULT;
    bool stage_result;

    //  Fixed for now to the minimum size to avoid adding to tlb
    *page_size = 0x40;
    *phys_ptr = address;

    if((access_type == ACCESS_INST_FETCH) && ((address & 0x1) != 0)) {
        if(!suppress_faults) {
            env->exception.target_el = current_el == 2 ? 2 : 1;
            set_mmu_fault_registers(access_type, address, ALIGNMENT_FAULT);
        }
        return TRANSLATE_FAIL;
    }

    uint64_t hcr = arm_hcr_el2_eff(env);
    bool tge_set = (hcr & HCR_TGE) != 0;
    bool vm_set = (hcr & HCR_VM) != 0;

    bool has_el1_stage = current_el < 2 && !tge_set;
    bool has_el2_stage = !has_el1_stage || vm_set;

    int el1_stage_prot = -1;
    if(has_el1_stage) {
        stage_result = pmsav8_translate_stage(env, address, current_el, access_type, &fault_type, &el1_stage_prot, false);
        if(!stage_result) {
            if(!suppress_faults) {
                env->exception.target_el = 1;
                set_mmu_fault_registers(access_type, address, fault_type);
            }
            return TRANSLATE_FAIL;
        }
    } else if(unlikely(current_el == 1)) {
        //  Being in EL1 when HCR.TGE is set is an illegal state,
        //  meaning we should never be in EL1 with only EL2 stage.
        tlib_assert_not_reached();
    }

    int el2_stage_prot = -1;
    if(has_el2_stage) {
        stage_result = pmsav8_translate_stage(env, address, current_el, access_type, &fault_type, &el2_stage_prot, true);
    }

    *prot = el1_stage_prot & el2_stage_prot;
    if(stage_result) {
        return TRANSLATE_SUCCESS;
    }

    if(!suppress_faults) {
        env->exception.target_el = 2;
        if(current_el == 2) {
            set_mmu_fault_registers(access_type, address, fault_type);
        } else {
            env->exception_index = EXCP_HYP_TRAP;
            if(access_type == ACCESS_INST_FETCH) {
                env->cp15.ifar_s = address;
                env->exception.syndrome = syn_instruction_abort(/* same_el */ false, /* s1ptw */ false, /* ifsc */ 0);
            } else {
                env->cp15.dfar_s = address;
                env->exception.syndrome = syn_data_abort_with_iss(
                    /* same_el */ false,
                    /* access_size */ 0,            /* will be set by insn_start data */
                    /* sign_extend */ false,        /* will be set by insn_start data */
                    /* insn_rt */ 0,                /* will be set by insn_start data */
                    /* is_64bit_gpr_ldst */ false,  /* will be set by insn_start data */
                    /* acquire_or_release */ false, /* will be set by insn_start data */
                    /* set */ 0,
                    /* cm */ false,
                    /* s1ptw */ false,
                    /* wnr */ access_type == ACCESS_DATA_STORE,
                    /* dfsc */ fault_type,
                    /* is_16bit */ true); /* will be set by insn_start data, here true is the neutral value (results in 0) */
                env->exception.dabt_syndrome_partial = true;
            }
        }
    }
    return TRANSLATE_FAIL;
}

inline int get_phys_addr(CPUState *env, target_ulong address, int access_type, int mmu_idx, uintptr_t return_address,
                         bool suppress_faults, target_ulong *phys_ptr, int *prot, target_ulong *page_size, int access_width)
{
    if(!arm_feature(env, ARM_FEATURE_V8)) {
        return get_phys_addr_v7(env, address, access_type, mmu_idx, phys_ptr, prot, page_size, suppress_faults);
    }

    ARMMMUIdx arm_mmu_idx = core_to_aa64_mmu_idx(mmu_idx);
    uint32_t el = arm_mmu_idx_to_el(arm_mmu_idx);

    //  get_phys_addr_pmsav8 handles disabled MPU itself
    if(arm_feature(env, ARM_FEATURE_PMSA) && arm_feature(env, ARM_FEATURE_V8)) {
        return get_phys_addr_pmsav8(env, address, access_type, el, return_address, suppress_faults, phys_ptr, prot, page_size,
                                    access_width);
    }

    if((arm_sctlr_eff(env, el) & SCTLR_M) == 0) {
        /* MMU/MPU disabled.  */
        *phys_ptr = address;
        *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
        *page_size = TARGET_PAGE_SIZE;
        return TRANSLATE_SUCCESS;
    }

    int result =
        get_phys_addr_v8(env, address, access_type, mmu_idx, return_address, suppress_faults, phys_ptr, prot, page_size, false);
    //  'get_phys_addr_v8' shouldn't return in case of a fault unless fault handling is suppressed.
    tlib_assert(result == TRANSLATE_SUCCESS || suppress_faults);
    return result;
}

target_phys_addr_t cpu_get_phys_page_debug(CPUState *env, target_ulong addr)
{
    target_ulong phys_addr = 0;
    target_ulong page_size = 0;
    int prot = 0;

    int access_type = ACCESS_DATA_LOAD;
    int mmu_idx = cpu_mmu_index(env);
    uintptr_t return_address = 0;
    bool suppress_faults = true;

    int result =
        get_phys_addr(env, addr, access_type, mmu_idx, return_address, suppress_faults, &phys_addr, &prot, &page_size, 1);
    if(result != TRANSLATE_SUCCESS) {
        return -1;
    }

    return phys_addr & TARGET_PAGE_MASK;
}

/* Transaction filtering by state is not yet implemented for this architecture.
 * This placeholder function is here to make it clear that more CPUs are expected to support this in the future. */
uint64_t cpu_get_state_for_memory_transaction(CPUState *env, target_ulong addr, int access_type)
{
    return 0;
}

//  The name of the function is a little misleading. It doesn't handle MMU faults as much as TLB misses.
int cpu_handle_mmu_fault(CPUState *env, target_ulong address, int access_type, int mmu_idx, uintptr_t return_address,
                         bool suppress_faults, int access_width, target_phys_addr_t *paddr)
{
    target_ulong phys_addr = 0;
    target_ulong page_size = 0;
    int prot = 0;
    int ret;

    ret = get_phys_addr(env, address, access_type, mmu_idx, return_address, suppress_faults, &phys_addr, &prot, &page_size,
                        access_width);
    if(ret == TRANSLATE_SUCCESS) {
        *paddr = phys_addr;
        /* Map a single [sub]page.  */
        phys_addr &= TARGET_PAGE_MASK;
        address &= TARGET_PAGE_MASK;
        tlb_set_page(env, address, phys_addr, prot, mmu_idx, page_size);
    }
    return ret;
}

void arch_raise_mmu_fault_exception(CPUState *env, int errcode, int access_type, target_ulong address, void *retaddr)
{
    //  access_type == CODE ACCESS - do not fire block_end hooks!
    cpu_loop_exit_restore(env, (uintptr_t)retaddr, access_type != ACCESS_INST_FETCH);
}

/* try to fill the TLB and return an exception if error. If retaddr is
   NULL, it means that the function was called in C code (i.e. not
   from generated code or from helper.c) */
/* XXX: fix it to restore all registers */
int arch_tlb_fill(CPUState *env1, target_ulong addr, int access_type, int mmu_idx, void *retaddr, int no_page_fault,
                  int access_width, target_phys_addr_t *paddr)
{
    CPUState *saved_env;
    int ret;

    saved_env = env;
    env = env1;
    ret = cpu_handle_mmu_fault(env, addr, access_type, mmu_idx, (uintptr_t)retaddr, no_page_fault, access_width, paddr);

    //  Unless fault handling is suppressed with 'no_page_fault', we will never get back here
    //  in case of a fault with MMU (only). Faults are handled directly in that function.
    //  The arch_raise_mmu_fault_exception call in tlb_fill (exec.c) handles MPU and external MMU faults.

    env = saved_env;
    return ret;
}

/* Sign/zero extend */
uint32_t HELPER(sxtb16)(uint32_t x)
{
    uint32_t res;
    res = (uint16_t)(int8_t)x;
    res |= (uint32_t)(int8_t)(x >> 16) << 16;
    return res;
}

uint32_t HELPER(uxtb16)(uint32_t x)
{
    uint32_t res;
    res = (uint16_t)(uint8_t)x;
    res |= (uint32_t)(uint8_t)(x >> 16) << 16;
    return res;
}

//  TODO: 'cpu_env' is the first argument now upstream. Why?
int32_t HELPER(sdiv)(int32_t num, int32_t den)
{
    if(den == 0) {
        return 0;
    }
    if(num == INT_MIN && den == -1) {
        return INT_MIN;
    }
    return num / den;
}

//  TODO: 'cpu_env' is the first argument now upstream. Why?
uint32_t HELPER(udiv)(uint32_t num, uint32_t den)
{
    if(den == 0) {
        return 0;
    }
    return num / den;
}

uint32_t HELPER(rbit)(uint32_t x)
{
    x = ((x & 0xff000000) >> 24) | ((x & 0x00ff0000) >> 8) | ((x & 0x0000ff00) << 8) | ((x & 0x000000ff) << 24);
    x = ((x & 0xf0f0f0f0) >> 4) | ((x & 0x0f0f0f0f) << 4);
    x = ((x & 0x88888888) >> 3) | ((x & 0x44444444) >> 1) | ((x & 0x22222222) << 1) | ((x & 0x11111111) << 3);
    return x;
}

static inline uint8_t do_usad(uint8_t a, uint8_t b)
{
    if(a > b) {
        return a - b;
    } else {
        return b - a;
    }
}

/* Unsigned sum of absolute byte differences.  */
uint32_t HELPER(usad8)(uint32_t a, uint32_t b)
{
    uint32_t sum;
    sum = do_usad(a, b);
    sum += do_usad(a >> 8, b >> 8);
    sum += do_usad(a >> 16, b >> 16);
    sum += do_usad(a >> 24, b >> 24);
    return sum;
}

/* For ARMv6 SEL instruction.  */
uint32_t HELPER(sel_flags)(uint32_t flags, uint32_t a, uint32_t b)
{
    uint32_t mask;

    mask = 0;
    if(flags & 1) {
        mask |= 0xff;
    }
    if(flags & 2) {
        mask |= 0xff00;
    }
    if(flags & 4) {
        mask |= 0xff0000;
    }
    if(flags & 8) {
        mask |= 0xff000000;
    }
    return (a & mask) | (b & ~mask);
}

/* Note that signed overflow is undefined in C.  The following routines are
   careful to use unsigned types where modulo arithmetic is required.
   Failure to do so _will_ break on newer gcc.  */

/* Signed saturating arithmetic.  */

/* Perform 16-bit signed saturating addition.  */
#define PFX_Q
#include "op_addsub.h"
#undef PFX_Q

#define PFX_UQ
#include "op_addsub.h"
#undef PFX_UQ

/* Signed modulo arithmetic.  */
#define PFX_S
#define ARITH_GE
#include "op_addsub.h"
#undef ARITH_GE
#undef PFX_S

#define PFX_U
#define ARITH_GE
#include "op_addsub.h"
#undef ARITH_GE
#undef PFX_U

#define PFX_SH
#include "op_addsub.h"
#undef PFX_SH

#define PFX_UH
#include "op_addsub.h"
#undef PFX_UH
