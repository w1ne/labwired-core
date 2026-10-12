/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include <stdint.h>
#include "cpu.h"
#include "system_registers_common.h"

uint32_t tlib_check_system_register_access(const char *name, bool is_write)
{
    enum {
        REGISTER_NOT_FOUND = 1,
        ACCESSOR_NOT_FOUND = 2,
        ACCESS_VALID = 3,
    };

    const ARMCPRegInfo *ri = sysreg_find_by_name(env, name);
    if(ri == NULL) {
        return REGISTER_NOT_FOUND;
    }

    if(ri->fieldoffset) {
        return ACCESS_VALID;
    }

    if(is_write) {
        bool write_possible = ri->writefn != NULL;
        return write_possible ? ACCESS_VALID : ACCESSOR_NOT_FOUND;
    } else {
        bool read_possible = (ri->readfn != NULL) || (ri->type & ARM_CP_CONST);
        return read_possible ? ACCESS_VALID : ACCESSOR_NOT_FOUND;
    }
}
EXC_INT_2(uint32_t, tlib_check_system_register_access, const char *, name, bool, is_write)

uint32_t tlib_create_system_registers_array(char ***array)
{
    return ttable_create_values_array(cpu->cp_regs, (void ***)array, try_set_array_entry_to_system_register);
}
EXC_INT_1(uint32_t, tlib_create_system_registers_array, char ***, array)

uint64_t tlib_get_system_register(const char *name, bool log_unhandled_access)
{
    return sysreg_get_by_name(cpu, name, log_unhandled_access);
}
EXC_INT_2(uint64_t, tlib_get_system_register, const char *, name, bool, log_unhandled_access)

void tlib_set_system_register(const char *name, uint64_t value, bool log_unhandled_access)
{
    sysreg_set_by_name(cpu, name, value, log_unhandled_access);
}
EXC_VOID_3(tlib_set_system_register, const char *, name, uint64_t, value, bool, log_unhandled_access)
