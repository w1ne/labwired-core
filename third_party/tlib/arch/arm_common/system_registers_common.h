/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include "cpu.h"

#include "helper_common.h"
#define GEN_HELPER 1
#include "helper_common.h"

//  Types of ARMCPRegInfo.
//  Each bit is a different type.
#define ARM_CP_NOP       (1 << 0)
#define ARM_CP_CURRENTEL (1 << 1)
//  Special regs
#define ARM_CP_NZCV    (1 << 2)
#define ARM_CP_DC_ZVA  (1 << 3)
#define ARM_CP_DC_GVA  (1 << 4)
#define ARM_CP_DC_GZVA (1 << 5)
#define ARM_CP_WFI     (1 << 6)
#define ARM_CP_BARRIER (1 << 7)
//  Mask used on above type - remember to update it when adding more special types!
#define ARM_CP_SPECIAL_MASK 0x00FF

#define ARM_CP_64BIT           (1 << 8)  //  Register is 64 bits wide
#define ARM_CP_CONST           (1 << 9)
#define ARM_CP_FPU             (1 << 10)
#define ARM_CP_IO              (1 << 11)
#define ARM_CP_RAISES_EXC      (1 << 12)
#define ARM_CP_RO              (1 << 13)  //  Read-only
#define ARM_CP_SME             (1 << 14)
#define ARM_CP_SUPPRESS_TB_END (1 << 15)
#define ARM_CP_SVE             (1 << 16)
#define ARM_CP_WO              (1 << 17)  //  Write-Only
#define ARM_CP_TLB_FLUSH       (1 << 18)  //  TLB will be flushed after writing such a register
//  TODO: Implement gen_helper_rebuild_hflags_a32_newel() for handling ARM_CP_NEWEL
#define ARM_CP_NEWEL        (1 << 19)  //  Write can change EL
#define ARM_CP_FORCE_TB_END (1 << 20)  //  Force end of TB, even if the register is only read from
#define ARM_CP_INSTRUCTION  (1 << 21)  //  For system instructions
#define ARM_CP_GIC          (1 << 22)  //  GIC register
#define ARM_CP_GTIMER       (1 << 23)  //  Generic Timer register
#define ARM_CP_AARCH64      (1 << 24)  //  Register is only accessible in AArch64 state, else only in AArch32 state

//  Minimum EL access
#define ARM_CP_EL_SHIFT 28
#define ARM_CP_EL_MASK  (3 << ARM_CP_EL_SHIFT)
#define ARM_CP_EL_0     (0 << ARM_CP_EL_SHIFT)
#define ARM_CP_EL_1     (1 << ARM_CP_EL_SHIFT)
#define ARM_CP_EL_2     (2 << ARM_CP_EL_SHIFT)
#define ARM_CP_EL_3     (3 << ARM_CP_EL_SHIFT)

#define ARM_CP_READABLE(ri_type)   (!(ri_type & ARM_CP_WO))
#define ARM_CP_WRITABLE(ri_type)   (!(ri_type & ARM_CP_RO))
#define ARM_CP_GET_MIN_EL(ri_type) ((ri_type & ARM_CP_EL_MASK) >> ARM_CP_EL_SHIFT)

typedef enum {
    CP_ACCESS_EL0,
    CP_ACCESS_EL1,
    CP_ACCESS_EL2,
    CP_ACCESS_EL3,
    CP_ACCESS_OK = 0x10,
    CP_ACCESS_TRAP_EL2 = 0x20,
    CP_ACCESS_TRAP_UNCATEGORIZED = 0x30,
    CP_ACCESS_TRAP = 0x40,
} CPAccessResult;
#define CP_ACCESS_EL_MASK 3

typedef struct ARMCPRegInfo ARMCPRegInfo;
typedef CPAccessResult AccessFn(CPUState *, const ARMCPRegInfo *, bool isread);
typedef uint64_t ReadFn(CPUState *, const ARMCPRegInfo *);
typedef void WriteFn(CPUState *, const ARMCPRegInfo *, uint64_t);

struct ARMCPRegInfo {
    const char *name;
    //  Register coprocessor, in AArch64 always CP_REG_ARM64_SYSREG_CP
    uint32_t cp;
    //  type of register, if require special handling
    uint32_t type;

    //  from C5.1.2, only 2 lower bits used
    uint8_t op0;
    //  from C5.1.1, only 3 lower bits used
    uint8_t op1;
    //  from C5.1.3, only 4 lower bits used
    uint8_t crn;
    //  from C5.1.3, only 4 lower bits used
    uint8_t crm;
    //  from C5.1.3, only 4 lower bits used
    uint8_t op2;
    //  offset from CPUState struct when there is no readfn/writefn
    uint32_t fieldoffset;
    //  resetvalue of the register
    uint64_t resetvalue;
    //  fuction that checks if access to the register should be granted
    AccessFn *accessfn;
    //  read function (required when fieldoffset and type is missing)
    ReadFn *readfn;
    //  write function (required when fieldoffset and type is missing)
    WriteFn *writefn;

    //  Is the entry dynamically allocated
    bool dynamic;
};

//  Only EL and RO/WO are checked here. Traps etc. are checked in the 'access_check_cp_reg' helper.
static inline bool cp_access_ok(int current_el, const ARMCPRegInfo *reg_info, bool isread)
{
    uint32_t ri_type = reg_info->type;

    //  The prints need to be emitted at runtime, so use helpers
    if(current_el < ARM_CP_GET_MIN_EL(ri_type)) {
        TCGv_ptr reg_info_const = tcg_const_ptr((tcg_target_long)reg_info);
        TCGv_i32 current_el_const = tcg_const_i32(current_el);

        gen_helper_warn_cp_invalid_el(cpu_env, reg_info_const, current_el_const);

        tcg_temp_free_ptr(reg_info_const);
        tcg_temp_free_i32(current_el_const);
        return false;
    }

    //  Rule IWCXDT
    if((isread && !ARM_CP_READABLE(ri_type)) || (!isread && !ARM_CP_WRITABLE(ri_type))) {
        TCGv_ptr reg_info_const = tcg_const_ptr((tcg_target_long)reg_info);
        TCGv_i32 isread_const = tcg_const_i32(isread);

        gen_helper_warn_cp_invalid_access(cpu_env, reg_info_const, isread_const);

        tcg_temp_free_ptr(reg_info_const);
        tcg_temp_free_i32(isread_const);
        return false;
    }
    return true;
}

//  Always pass an actual array directly; otherwise 'sizeof(array)' will be a pointer size.
#define ARM_CP_ARRAY_COUNT(array) (sizeof(array) / sizeof(ARMCPRegInfo))

/* Macros creating ARMCPRegInfo entries. */

//  The parameters have to start with an underscore because preprocessing replaces '.PARAMETER' too.
//  The 'extra_type' parameter is any type besides 'ARM_CP_64BIT' and 'ARM_CP_EL*' since those are set automatically.
#define ARM_CP_REG_DEFINE(_name, _cp, _op0, _op1, _crn, _crm, _op2, width, el, extra_type, ...) \
    { .name = #_name,                                                                           \
      .cp = _cp,                                                                                \
      .op0 = _op0,                                                                              \
      .op1 = _op1,                                                                              \
      .crn = _crn,                                                                              \
      .crm = _crm,                                                                              \
      .op2 = _op2,                                                                              \
      .type = (extra_type | (el << ARM_CP_EL_SHIFT) | (width == 64 ? ARM_CP_64BIT : 0x0)),      \
      __VA_ARGS__ },

//  All ARM64 (AArch64) registers use the same CP value.
#define ARM64_CP_REG_DEFINE(name, op0, op1, crn, crm, op2, el, extra_type, ...) \
    ARM_CP_REG_DEFINE(name, CP_REG_ARM64_SYSREG_CP, op0, op1, crn, crm, op2, 64, el, ARM_CP_AARCH64 | extra_type, __VA_ARGS__)

#define ARM32_CP_REG_DEFINE(name, cp, op1, crn, crm, op2, el, extra_type, ...) \
    ARM_CP_REG_DEFINE(name, cp, 0, op1, crn, crm, op2, 32, el, extra_type, __VA_ARGS__)

#define ARM32_CP_64BIT_REG_DEFINE(name, cp, op1, crm, el, extra_type, ...) \
    ARM_CP_REG_DEFINE(name, cp, 0, op1, 0, crm, 0, 64, el, extra_type, __VA_ARGS__)

/* Macros for the most common types used in 'extra_type'.
 *
 * Reading/writing the register specified as WO/RO (respectively) will trigger the 'Undefined instruction' exception.
 * Therefore CONST can be used with RO if the instruction to write the given register doesn't exist.
 * Writes to a CONST register are simply ignored unless RO is used too.
 *
 * CONST and GIC have to be used as the last ones in 'extra_type', e.g., 'RW | CONST(0xCAFE)'.
 * IGNORED silences the unhandled warning.
 */

#define CONST(resetvalue) ARM_CP_CONST, RESETVALUE(resetvalue)
#define GIC               ARM_CP_GIC, ACCESSFN(interrupt_cpu_interface)
#define GTIMER            ARM_CP_GTIMER
#define IGNORED           ARM_CP_NOP
#define INSTRUCTION       ARM_CP_INSTRUCTION
#define RO                ARM_CP_RO
#define RW                0x0
#define WO                ARM_CP_WO

//  Optional macro arguments.
#define ACCESSFN(name)         .accessfn = access_##name
#define FIELD(cpu_state_field) .fieldoffset = offsetof(CPUState, cpu_state_field)
#define READFN(name)           .readfn = read_##name
#define RESETVALUE(value)      .resetvalue = value
#define RW_FNS(name)           READFN(name), WRITEFN(name)
#define WRITEFN(name)          .writefn = write_##name

/* Read/write functions. */

#define READ_FUNCTION(width, mnemonic, value)                                \
    uint##width##_t read_##mnemonic(CPUState *env, const ARMCPRegInfo *info) \
    {                                                                        \
        return value;                                                        \
    }

#define WRITE_FUNCTION(width, mnemonic, write_statement)                                  \
    void write_##mnemonic(CPUState *env, const ARMCPRegInfo *info, uint##width##_t value) \
    {                                                                                     \
        write_statement;                                                                  \
    }

#define RW_FUNCTIONS(width, mnemonic, read_value, write_statement) \
    READ_FUNCTION(width, mnemonic, read_value)                     \
    WRITE_FUNCTION(width, mnemonic, write_statement)

#define RW_FUNCTIONS_PTR(width, mnemonic, pointer) RW_FUNCTIONS(width, mnemonic, *(pointer), *(pointer) = value)

#define ACCESS_FUNCTION(mnemonic, expr)                                                    \
    CPAccessResult access_##mnemonic(CPUState *env, const ARMCPRegInfo *info, bool isread) \
    {                                                                                      \
        return (expr);                                                                     \
    }

void cp_reg_add(CPUState *env, ARMCPRegInfo *reg_info);

//  These need to be marked as unused, so the compiler doesn't complain when including the header outside `system_registers`
__attribute__((unused)) static void cp_regs_add(CPUState *env, ARMCPRegInfo *reg_info_array, uint32_t array_count)
{
    for(int i = 0; i < array_count; i++) {
        ARMCPRegInfo *reg_info = &reg_info_array[i];
        cp_reg_add(env, reg_info);
    }
}

__attribute__((unused)) static void cp_reg_add_with_key(CPUState *env, TTable *cp_regs, uint32_t *key, ARMCPRegInfo *reg_info)
{
    if(!ttable_insert_check(cp_regs, key, reg_info)) {
        tlib_printf(LOG_LEVEL_ERROR,
                    "Duplicated system_register definition!: name: %s, cp: %d, crn: %d, op1: %d, crm: %d, op2: %d, op0: %d",
                    reg_info->name, reg_info->cp, reg_info->crn, reg_info->op1, reg_info->crm, reg_info->op2, reg_info->op0);

        const char *const name = reg_info->name;
        reg_info = ttable_lookup_value_eq(cp_regs, key);
        tlib_printf(LOG_LEVEL_ERROR, "Previously defined as!: name: %s, cp: %d, crn: %d, op1: %d, crm: %d, op2: %d, op0: %d",
                    reg_info->name, reg_info->cp, reg_info->crn, reg_info->op1, reg_info->crm, reg_info->op2, reg_info->op0);
        tlib_abortf("Redefinition of register %s by %s", name, reg_info->name);
    }
}

static inline bool is_gic_register(const ARMCPRegInfo *reg_info)
{
    tlib_assert(reg_info != NULL);
    return reg_info->type & ARM_CP_GIC;
}

static inline bool is_generic_timer_register(const ARMCPRegInfo *reg_info)
{
    tlib_assert(reg_info != NULL);
    return reg_info->type & ARM_CP_GTIMER;
}

static inline bool is_system_register(ARMCPRegInfo *reg_info)
{
    tlib_assert(reg_info != NULL);
    bool is_system_instruction = reg_info->type & ARM_CP_INSTRUCTION;
    return !is_system_instruction;
}

static inline void log_unhandled_sysreg_access(const char *sysreg_name, bool is_write)
{
    //  %-6s is used to have sysreg names aligned for both reads and writes.
    tlib_printf(LOG_LEVEL_WARNING, "Unhandled system instruction or register %-6s %s; %s",
                is_write ? "write:" : "read:", sysreg_name, is_write ? "write ignored" : "returning 0");
}

static inline void log_unhandled_sysreg_read(const char *sysreg_name)
{
    log_unhandled_sysreg_access(sysreg_name, false);
}

static inline void log_unhandled_sysreg_write(const char *sysreg_name)
{
    log_unhandled_sysreg_access(sysreg_name, true);
}

static inline bool try_set_array_entry_to_system_register(void **array_entry, void *value)
{
    tlib_assert(value != NULL && array_entry != NULL);
    ARMCPRegInfo *reg_info = (ARMCPRegInfo *)value;

    //  The value can be a system instruction and should be excluded.
    if(is_system_register(reg_info)) {
        *array_entry = value;
        return true;
    }
    return false;
}

//  ARM Architecture Reference Manual ARMv7A and ARMv7-R (A8.6.92)
#define CP_REG_ARM32_32BIT_SYSREG_CRM_SHIFT 0
#define CP_REG_ARM32_32BIT_SYSREG_OP2_SHIFT (CP_REG_ARM32_32BIT_SYSREG_CRM_SHIFT + 5)
#define CP_REG_ARM32_32BIT_SYSREG_CRN_SHIFT (CP_REG_ARM32_32BIT_SYSREG_OP2_SHIFT + 11)
#define CP_REG_ARM32_32BIT_SYSREG_OP1_SHIFT (CP_REG_ARM32_32BIT_SYSREG_CRN_SHIFT + 5)

//  ARM Architecture Reference Manual ARMv7A and ARMv7-R (A8.6.93)
#define CP_REG_ARM32_64BIT_SYSREG_CRM_SHIFT 0
#define CP_REG_ARM32_64BIT_SYSREG_OP1_SHIFT (CP_REG_ARM32_64BIT_SYSREG_CRM_SHIFT + 4)

static inline uint32_t encode_as_aarch32_64bit_register(const ARMCPRegInfo *info)
{
    return (info->op1 << CP_REG_ARM32_64BIT_SYSREG_OP1_SHIFT) | (info->crm << CP_REG_ARM32_64BIT_SYSREG_CRM_SHIFT);
}

static inline uint32_t encode_as_aarch32_32bit_register(const ARMCPRegInfo *info)
{
    return (info->op1 << CP_REG_ARM32_32BIT_SYSREG_OP1_SHIFT) | (info->crn << CP_REG_ARM32_32BIT_SYSREG_CRN_SHIFT) |
           (info->op2 << CP_REG_ARM32_32BIT_SYSREG_OP2_SHIFT) | (info->crm << CP_REG_ARM32_32BIT_SYSREG_CRM_SHIFT);
}

/* Functions for accessing system registers by their names. */

static inline uint64_t *sysreg_field_ptr(CPUState *env, const ARMCPRegInfo *ri)
{
    //  Fieldoffset is in bytes hence 'env' is cast to 'uint8_t' before the addition.
    return (uint64_t *)(((uint8_t *)env) + ri->fieldoffset);
}

static inline int ttable_compare_sysreg_name(TTable_entry entry, const void *sysreg_name)
{
    ARMCPRegInfo *ri = (ARMCPRegInfo *)entry.value;
    return strcasecmp(ri->name, sysreg_name);
}

const char *sysreg_patch_lookup_name(const char *name);
static const ARMCPRegInfo *sysreg_find_by_name(CPUState *env, const char *name)
{
    const char *lookup_name = sysreg_patch_lookup_name(name);
    TTable_entry *entry = ttable_lookup_custom(env->cp_regs, ttable_compare_sysreg_name, lookup_name);
    if(entry != NULL) {
        return (ARMCPRegInfo *)entry->value;
    }
    return NULL;
}

static inline uint64_t sysreg_get_by_name(CPUState *env, const char *name, bool log_unhandled_access)
{
    const ARMCPRegInfo *ri = sysreg_find_by_name(env, name);
    if(ri == NULL) {
        tlib_printf(LOG_LEVEL_WARNING, "Reading from system register failure. No such register: %s", name);
        return 0x0;
    }

    uint64_t result;
    if(ri->type & ARM_CP_CONST) {
        result = ri->resetvalue;
    } else if(ri->readfn) {
        result = ri->readfn(env, ri);
    } else if(ri->fieldoffset != 0) {
        if(ri->type & ARM_CP_64BIT) {
            result = *sysreg_field_ptr(env, ri);
        } else {
            result = *(uint32_t *)sysreg_field_ptr(env, ri);
        }
    } else {
        if(log_unhandled_access) {
            log_unhandled_sysreg_read(ri->name);
        }
        return 0x0;
    }

    if(ri->type & ARM_CP_64BIT) {
        return result;
    } else {
        return (uint32_t)result;
    }
}

static inline void sysreg_set_by_name(CPUState *env, const char *name, uint64_t value, bool log_unhandled_access)
{
    const ARMCPRegInfo *ri = sysreg_find_by_name(env, name);
    if(ri == NULL) {
        tlib_printf(LOG_LEVEL_WARNING, "Writing to system register failure. No such register: %s", name);
        return;
    }

    //  Truncate values written to 32-bit registers.
    if(!(ri->type & ARM_CP_64BIT)) {
        value = (uint32_t)value;
    }

    if(ri->writefn) {
        ri->writefn(env, ri, value);
    } else if(ri->fieldoffset != 0) {
        if(ri->type & ARM_CP_64BIT) {
            *sysreg_field_ptr(env, ri) = value;
        } else {
            *(uint32_t *)sysreg_field_ptr(env, ri) = value;
        }
    } else if(log_unhandled_access) {
        log_unhandled_sysreg_write(ri->name);
    }
}
