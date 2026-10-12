#include "helper_common.h"
#include "system_registers_common.h"
#include "infrastructure.h"

#if TARGET_LONG_BITS == 64
#define PC_FMT PRIx64
#else
#define PC_FMT PRIx32
#endif

void HELPER(warn_cp_invalid_el)(CPUState *env, void *reg_info, uint32_t current_el)
{
    tlib_printf(LOG_LEVEL_DEBUG, "[PC=0x%" PC_FMT "] The '%s' register shouldn't be accessed on EL%d", CPU_PC(env),
                ((ARMCPRegInfo *)reg_info)->name, current_el);
}

void HELPER(warn_cp_invalid_access)(CPUState *env, void *reg_info, uint32_t isread)
{
    tlib_printf(LOG_LEVEL_DEBUG, "[PC=0x%" PC_FMT "] The '%s' register shouldn't be %s", CPU_PC(env),
                ((ARMCPRegInfo *)reg_info)->name, isread ? "read from" : "written to");
}
