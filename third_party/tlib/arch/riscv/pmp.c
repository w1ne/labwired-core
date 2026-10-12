/*
 * Physical Memory Protection
 *
 * Author: Daire McNamara, daire.mcnamara@emdalo.com
 *         Ivan Griffin, ivan.griffin@emdalo.com
 *
 * This provides a RISC-V Physical Memory Protection implementation
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
 * THE SOFTWARE.
 */

#include "cpu.h"
#include "arch_callbacks.h"

#ifdef DEBUG_PMP
#define PMP_DEBUG(fmt, ...)                                       \
    do {                                                          \
        tlib_printf(LOG_LEVEL_DEBUG, "pmp: " fmt, ##__VA_ARGS__); \
    } while(0)
#else
#define PMP_DEBUG(fmt, ...) \
    do {                    \
    } while(0)
#endif

static void pmp_write_cfg(CPUState *env, uint32_t addr_index, uint8_t val);
static uint8_t pmp_read_cfg(CPUState *env, uint32_t addr_index);
static void pmp_update_rule(CPUState *env, uint32_t pmp_index);

/*
 * Accessor method to extract address matching type 'a field' from cfg reg
 */
static inline uint8_t pmp_get_a_field(uint8_t cfg)
{
    uint8_t a = cfg >> 3;
    return a & 0x3;
}

/*
 * Check whether a PMP is locked or not.
 */
static inline int pmp_is_locked(CPUState *env, uint32_t pmp_index)
{
    if(env->pmp_state.pmp[pmp_index].cfg_reg & PMP_LOCK) {
        return 1;
    }

    /* Top PMP has no 'next' to check */
    if((pmp_index + 1u) >= env->pmp_entry_count) {
        return 0;
    }

    /* In TOR mode, need to check the lock bit of the next pmp
     * (if there is a next)
     */
    const uint8_t a_field = pmp_get_a_field(env->pmp_state.pmp[pmp_index + 1].cfg_reg);
    if((env->pmp_state.pmp[pmp_index + 1u].cfg_reg & PMP_LOCK) && (PMP_AMATCH_TOR == a_field)) {
        return 1;
    }

    return 0;
}

/*
 * Checks whether a PMP configuration is valid (e.g. doesn't contain reserved access bit combinations)
 */
static inline bool pmp_validate_configuration(CPUState *env, uint32_t pmp_index, uint8_t *val)
{
    bool rlb = (env->mseccfg & MSECCFG_RLB);
    bool mml = (env->mseccfg & MSECCFG_MML);
    bool read = (*val & PMP_READ) == PMP_READ;
    bool write = (*val & PMP_WRITE) == PMP_WRITE;
    bool exec = (*val & PMP_EXEC) == PMP_EXEC;
    bool locked = (*val & PMP_LOCK) == PMP_LOCK;
    bool shared = (!read && write);
    bool is_locked_shared_data_region = locked && read && write && exec;

    //  If mseccfg.MML is not set, the combination of pmpcfg.RW=01 remains reserved for future standard use.
    if(!mml && env->privilege_architecture >= RISCV_PRIV1_11 && shared) {
        PMP_DEBUG("Reserved permission bit combination (R=0, W=1) during pmpcfg write - clearing W bit");
        *val &= ~PMP_WRITE;
    }
    /* Adding a rule with executable privileges that either is M-mode-only or a locked Shared-Region
     * is not possible and such pmpcfg writes are ignored, leaving pmpcfg unchanged.
     * This restriction can be lifted by setting mseccfg.RLB */
    if(!rlb && (mml && locked && (exec || shared)) && !is_locked_shared_data_region) {
        return false;
    }
    return true;
}

/*
 * Count the number of active rules.
 */
static inline uint32_t pmp_get_num_rules(CPUState *env)
{
    return env->pmp_state.num_rules;
}

/*
 * Accessor to get the cfg reg for a specific PMP/HART
 */
static inline uint8_t pmp_read_cfg(CPUState *env, uint32_t pmp_index)
{
    tlib_assert(pmp_index < env->pmp_entry_count);
    return env->pmp_state.pmp[pmp_index].cfg_reg;
}

/*
 * Accessor to set the cfg reg for a specific PMP/HART
 * Bounds checks and relevant lock bit.
 */
static void pmp_write_cfg(CPUState *env, uint32_t pmp_index, uint8_t val)
{
    tlib_assert(pmp_index < env->pmp_entry_count);
    if(pmp_is_locked(env, pmp_index) && !(env->mseccfg & MSECCFG_RLB)) {
        PMP_DEBUG("Ignoring pmpcfg write - locked");
        return;
    }

    if(!pmp_validate_configuration(env, pmp_index, &val)) {
        PMP_DEBUG("Ignoring pmpcfg write - invalid configuration");
        return;
    }

    env->pmp_state.pmp[pmp_index].cfg_reg = val;
    pmp_update_rule(env, pmp_index);
}

static void pmp_decode_napot(target_ulong addr, int napot_grain, target_ulong *start_addr, target_ulong *end_addr)
{
    /* bits          meaning                       napot_grain
       aaaa...aaa0   8-byte NAPOT range            0
       aaaa...aa01   16-byte NAPOT range           1
       aaaa...a011   32-byte NAPOT range           2
       ...
       aaa0...1111   2^(XLEN-1)-byte NAPOT range   XLEN - 4
       aa01...1111   2^XLEN-byte NAPOT range       XLEN - 3
       a011...1111   2^(XLEN+1)-byte NAPOT range   XLEN - 2
       0111...1111   2^(XLEN+2)-byte NAPOT range   XLEN - 1
       1111...1111   2^(XLEN+3)-byte NAPOT range   XLEN
     */
    //  Avoid shifting UB, this will always cover the entire address space anyway
    if(unlikely(napot_grain == TARGET_LONG_BITS)) {
        PMP_DEBUG("Allowing ulong-sized napot");
        *start_addr = 0;
        *end_addr = -1;
        return;
    }
    target_ulong base = addr & ((target_ulong)-2 << napot_grain);
    //  We don't implement memory beyond XLEN-bit (viz. 34-bit memory), so if the top two
    //  address bits are set, this entry is referring to memory that doesn't exist
    if(unlikely((base & ((target_ulong)3 << (TARGET_LONG_BITS - 2))) != 0)) {
        PMP_DEBUG("Ignoring address %d (grain %d) for beyond XLEN-bit", addr, napot_grain);
        //  Nothing will match this range
        *start_addr = 1;
        *end_addr = 0;
        return;
    }
    base <<= 2;
    //  Grains `XLEN - 3` and above will underflow here, which means this covers
    //  the entire address space
    target_ulong range = ((target_ulong)8 << napot_grain) - 1;
    *start_addr = (target_ulong)base;
    *end_addr = (target_ulong)(base + range);
}

/* Convert cfg/addr reg values here into simple 'sa' --> start address and 'ea'
 *   end address values.
 *   This function is called relatively infrequently whereas the check that
 *   an address is within a pmp rule is called often, so optimise that one
 */
static void pmp_update_rule(CPUState *env, uint32_t pmp_index)
{
    int i;

    env->pmp_state.num_rules = 0;

    uint8_t this_cfg = env->pmp_state.pmp[pmp_index].cfg_reg;
    target_ulong this_addr = env->pmp_state.pmp[pmp_index].addr_reg;
    target_ulong prev_addr = 0u;
    target_ulong napot = 0u;
    target_ulong sa = 0u;
    target_ulong ea = 0u;

    if(pmp_index >= 1u) {
        prev_addr = env->pmp_state.pmp[pmp_index - 1].addr_reg;
    }

    switch(pmp_get_a_field(this_cfg)) {
        case PMP_AMATCH_OFF:
            sa = 0u;
            ea = -1;
            break;

        case PMP_AMATCH_TOR:
            sa = prev_addr << 2; /* shift up from [xx:0] to [xx+2:2] */
            ea = (this_addr << 2) - 1u;
            break;

        case PMP_AMATCH_NA4:
            sa = this_addr << 2; /* shift up from [xx:0] to [xx+2:2] */
            ea = (sa + 4u) - 1u;
            break;

        case PMP_AMATCH_NAPOT:
            /*  Since priv-1.11 PMP grain must be the same across all PMP regions */
            napot = ctz_tl(~this_addr);
            if(env->privilege_architecture >= RISCV_PRIV1_11) {
                if(cpu->pmp_napot_grain > napot) {
                    tlib_log(LOG_LEVEL_ERROR, "Tried to set NAPOT region size smaller than the platform defined grain. This "
                                              "region will be enlarged to grain size");
                    napot = cpu->pmp_napot_grain;
                }
            }
            pmp_decode_napot(this_addr, napot, &sa, &ea);
            break;

        default:
            sa = 0u;
            ea = 0u;
            break;
    }

    env->pmp_state.addr[pmp_index].sa = sa & ((cpu->pmp_addr_mask << 2) | 0x3);
    env->pmp_state.addr[pmp_index].ea = ea & ((cpu->pmp_addr_mask << 2) | 0x3);

    for(i = 0; i < env->pmp_entry_count; i++) {
        const uint8_t a_field = pmp_get_a_field(env->pmp_state.pmp[i].cfg_reg);
        if(PMP_AMATCH_OFF != a_field) {
            env->pmp_state.num_rules++;
        }
    }

    tlb_flush(env, 1, true);
}

static int pmp_is_in_range(CPUState *env, int pmp_index, target_ulong addr)
{
    int result = 0;
    addr &= ((cpu->pmp_addr_mask << 2) | 0x3);

    if((addr >= env->pmp_state.addr[pmp_index].sa) && (addr <= env->pmp_state.addr[pmp_index].ea)) {
        result = 1;
    } else {
        result = 0;
    }

    return result;
}

/*
 * Public Interface
 */

int pmp_find_overlapping(CPUState *env, target_ulong addr, target_ulong size, int starting_index)
{
    int i;
    target_ulong pmp_sa;
    target_ulong pmp_ea;
    addr &= ((cpu->pmp_addr_mask << 2) | 0x3);
    uint8_t a_field;

    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_find_overlapping(addr, size, starting_index);
    }

    for(i = starting_index; i < env->pmp_entry_count; i++) {
        a_field = pmp_get_a_field(env->pmp_state.pmp[i].cfg_reg);
        if(a_field == PMP_AMATCH_OFF) {
            continue;
        }
        pmp_sa = env->pmp_state.addr[i].sa;
        pmp_ea = env->pmp_state.addr[i].ea;

        if(pmp_sa < addr) {
            if(pmp_ea >= addr) {
                return i;
            }
        } else if(pmp_sa <= addr + size - 1) {
            return i;
        }
    }

    return -1;
}

/* Normal PMP rules behavior, without Smepmp
 * or with Machine Mode Lockdown (MSECCFG_MML) disabled */
static inline pmp_priv_t pmp_get_privs_normal(int pmp_index, target_ulong priv)
{
    tlib_assert(!(env->mseccfg & MSECCFG_MML));

    pmp_priv_t allowed_privs = PMP_READ | PMP_WRITE | PMP_EXEC;

    if((priv != PRV_M) || pmp_is_locked(env, pmp_index)) {
        allowed_privs &= env->pmp_state.pmp[pmp_index].cfg_reg;
    }
    return allowed_privs;
}

/* For Machine Mode Lockdown look at: Chapter 6. "Smepmp" Extension
 * of RISC-V Privileged Architecture 1.12 */
static inline pmp_priv_t pmp_get_privs_mml(int pmp_index, target_ulong priv)
{
    //  Should not reach this function if Machine Mode Lockdown is not active
    tlib_assert(env->mseccfg & MSECCFG_MML);

    uint8_t rule_privs = env->pmp_state.pmp[pmp_index].cfg_reg;
    bool is_read = (rule_privs & PMP_READ);
    bool is_write = (rule_privs & PMP_WRITE);
    bool is_exec = (rule_privs & PMP_EXEC);
    bool is_locked = (rule_privs & PMP_LOCK);

    /* Shared memory regions
     * they use previously reserved PMP encodings W=1 R=0
     * special case if RWXL = 0b1111 which is Read-only for M/S/U modes */
    if(is_read && is_write && is_exec && is_locked) {
        return PMP_READ;
    }

    pmp_priv_t allowed_privs = 0;

    //  Shared memory regions - cont.
    if(!is_read && is_write) {
        if(!is_locked) {  //  Shared data region
            allowed_privs |= PMP_READ;
            /* Machine has RW by default
             * User/Supervisor has R, but gains W if X=1 */
            if(priv == PRV_M || is_exec) {
                allowed_privs |= PMP_WRITE;
            }
        } else {  //  Shared code region
            //  M/S/U modes have executable access by default
            allowed_privs |= PMP_EXEC;
            if(priv == PRV_M && is_exec) {
                //  Machine can gain R if X=1
                allowed_privs |= PMP_READ;
            }
        }
    } else {
        /* PMP_LOCK changes behavior - if set it enforces the rule for Machine Mode
         * otherwise if unset it enforces the rule for Supervisor/User mode
         * the other mode is denied by default */
        if((is_locked && priv != PRV_M) || (!is_locked && priv == PRV_M)) {
            allowed_privs = 0;
        } else {
            allowed_privs = rule_privs & (PMP_READ | PMP_WRITE | PMP_EXEC);
        }
    }

    return allowed_privs;
}

/*
 * Find and return PMP configuration matching memory address
 */
int pmp_get_access(CPUState *env, target_ulong addr, target_ulong size, int access_type)
{
    int i = 0;
    int ret = -1;
    target_ulong s = 0;
    target_ulong e = 0;
    addr &= ((cpu->pmp_addr_mask << 2) | 0x3);

    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_get_access(addr, size, access_type);
    }

    /*
     * According to the RISC-V Privileged Architecture Specification (ch. 3.6),
     * to calculate the effective accessing mode during loads and stores,
     * we have to account for the value of the mstatus.MPRV field.
     * If mstatus.MPRV = 1, then the effective mode is dictated by the mstatus.MPP value.
     * Take that into the account when determining the PMP configuration for a given address.
     */
    target_ulong priv = env->priv;
    if(get_field(env->mstatus, MSTATUS_MPRV) && (access_type == ACCESS_DATA_LOAD || access_type == ACCESS_DATA_STORE)) {
        priv = get_field(env->mstatus, MSTATUS_MPP);
    }

    /* Short cut if no rules */
    if(0 == pmp_get_num_rules(env)) {
        if(priv == PRV_M) {
            return (env->mseccfg & MSECCFG_MMWP) ? 0 : PMP_READ | PMP_WRITE | PMP_EXEC;
        } else {
            return riscv_has_additional_ext(env, RISCV_FEATURE_SMEPMP) ? 0 : PMP_READ | PMP_WRITE | PMP_EXEC;
        }
    }

    /* 1.10 draft priv spec states there is an implicit order
         from low to high */
    for(i = 0; i < env->pmp_entry_count; i++) {
        s = pmp_is_in_range(env, i, addr);
        e = pmp_is_in_range(env, i, addr + size - 1);
        const uint8_t a_field = pmp_get_a_field(env->pmp_state.pmp[i].cfg_reg);
        if(a_field == PMP_AMATCH_OFF) {
            continue;
        }

        /* partially inside */
        if((s + e) == 1) {
            PMP_DEBUG("pmp violation - access is partially in inside");
            ret = 0;
            break;
        }

        /* fully inside */
        if((s + e) == 2) {
            ret = (env->mseccfg & MSECCFG_MML) ? pmp_get_privs_mml(i, priv) : pmp_get_privs_normal(i, priv);
            break;
        }
    }

    /* No rule matched */
    if(ret == -1) {
        if(priv == PRV_M) {
            uint8_t allowed = PMP_READ | PMP_WRITE | PMP_EXEC;
            /* Executing code with Machine mode privileges is only possible from
             * memory regions with a matching M-mode-only rule or a locked
             * Shared-Region rule with executable privileges.
             * Executing code from a region without a matching rule
             * or with a matching S/U-mode-only rule is denied. */
            if(env->mseccfg & MSECCFG_MML) {
                allowed &= ~PMP_EXEC;
            }
            /* Privileged spec v1.10 states if no PMP entry matches an
             * M-Mode access, the access succeeds
             * unless MMWP is set, which inverts this logic */
            ret = env->mseccfg & MSECCFG_MMWP ? 0 : allowed;
        } else {
            ret = 0; /* Other modes are not allowed to succeed if they don't
                      * match a rule, but there are rules.  We've checked for
                      * no rule earlier in this function. */
        }
    }

    return ret;
}

/*
 * Handle a write to a pmpcfg CSP
 */
void pmpcfg_csr_write(CPUState *env, uint32_t reg_index, target_ulong val)
{
    int i;
    uint8_t cfg_val;
    uint32_t base_offset;

    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_cfg_csr_write(reg_index, val);
    }

    PMP_DEBUG("hart " TARGET_FMT_ld " writes: reg%d, val: 0x" TARGET_FMT_lx, env->mhartid, reg_index, val);

#if defined(TARGET_RISCV64)
    //  for RV64 only even pmpcfg registers are used:
    //  pmpcfg0 = [pmp0cfg, pmp1cfg, ..., pmp7cfg]
    //  there is NO pmpcfg1
    //  pmpcfg2 = [pmp8cfg, pmp9cfg, ..., pmp15cfg]
    //  so we obtain the effective index by dividing by 2
    if(unlikely(reg_index % 2 != 0)) {
        tlib_printf(LOG_LEVEL_WARNING, "Attempted illegal write to pmpcfg register with odd-numbered index (%d)", reg_index);
        helper_raise_illegal_instruction(env);
    }
    reg_index /= 2;
#endif
    base_offset = reg_index * sizeof(target_ulong);
    //  PMP entry counts are multiples of 16, so if the first entry exists (at `base_offset`),
    //  the last entry will exist too (at `base_offset + sizeof(target_ulong)`)
    if(unlikely(base_offset >= env->pmp_entry_count)) {
        tlib_printf(LOG_LEVEL_WARNING, "Attempted illegal write to pmpcfg entry beyond entry count (%d, entry count is %d)",
                    base_offset, env->pmp_entry_count);
        helper_raise_illegal_instruction(env);
    }

    for(i = 0; i < sizeof(target_ulong); i++) {
        //  Bits 5 and 6 are WARL since Priviledged ISA 1.11
        //  The soft should ignore them either way in older spec
        cfg_val = (val >> 8 * i) & 0x9f;
        pmp_write_cfg(env, base_offset + i, cfg_val);
    }
}

/*
 * Handle a read from a pmpcfg CSP
 */
target_ulong pmpcfg_csr_read(CPUState *env, uint32_t reg_index)
{
    int i;
    target_ulong cfg_val = 0;
    uint8_t val = 0;
    uint32_t base_offset;

    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_cfg_csr_read(reg_index);
    }

#if defined(TARGET_RISCV64)
    //  for RV64 only even pmpcfg registers are used
    //  see a comment in pmpcfg_csr_write for details
    if(unlikely(reg_index % 2 != 0)) {
        tlib_printf(LOG_LEVEL_WARNING, "Attempted illegal read from pmpcfg register with odd-numbered index (%d)", reg_index);
        helper_raise_illegal_instruction(env);
    }
    reg_index /= 2;
#endif
    base_offset = reg_index * sizeof(target_ulong);
    //  Same logic as in pmpcfg_csr_write
    if(unlikely(base_offset >= env->pmp_entry_count)) {
        tlib_printf(LOG_LEVEL_WARNING, "Attempted illegal read from pmpcfg entry beyond entry count (%d, entry count is %d)",
                    base_offset, env->pmp_entry_count);
        helper_raise_illegal_instruction(env);
    }

    for(i = 0; i < sizeof(target_ulong); i++) {
        val = pmp_read_cfg(env, base_offset + i);
        cfg_val |= (target_ulong)val << (i * 8);
    }

    PMP_DEBUG("hart " TARGET_FMT_ld "  reads: reg%d, val: 0x" TARGET_FMT_lx, env->mhartid, reg_index, cfg_val);

    return cfg_val;
}

/*
 * Handle a write to a pmpaddr CSP
 */
void pmpaddr_csr_write(CPUState *env, uint32_t addr_index, target_ulong val)
{
    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_address_csr_write(addr_index, val);
    }

    PMP_DEBUG("hart " TARGET_FMT_ld " writes: addr%d, val: 0x" TARGET_FMT_lx, env->mhartid, addr_index, val);

    if(addr_index >= env->pmp_entry_count) {
        tlib_printf(LOG_LEVEL_WARNING, "Attempted illegal write to pmpaddr register beyond entry count (%d, entry count is %d)",
                    addr_index, env->pmp_entry_count);
        helper_raise_illegal_instruction(env);
    }

    if(pmp_is_locked(env, addr_index) && !(env->mseccfg & MSECCFG_RLB)) {
        PMP_DEBUG("ignoring pmpaddr write - locked");
        return;
    }
    //  No bit shift, since this is a raw write into the register
    env->pmp_state.pmp[addr_index].addr_reg = val & cpu->pmp_addr_mask;
    pmp_update_rule(env, addr_index);
}

/*
 * Handle a read from a pmpaddr CSP
 */
target_ulong pmpaddr_csr_read(CPUState *env, uint32_t addr_index)
{
    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_address_csr_read(addr_index);
    }

    PMP_DEBUG("hart " TARGET_FMT_ld "  reads: addr%d, val: 0x" TARGET_FMT_lx, env->mhartid, addr_index,
              env->pmp_state.pmp[addr_index].addr_reg);

    if(addr_index >= env->pmp_entry_count) {
        tlib_printf(LOG_LEVEL_WARNING, "Attempted illegal read from pmpaddr register beyond entry count (%d, entry count is %d)",
                    addr_index, env->pmp_entry_count);
        helper_raise_illegal_instruction(env);
    }

    return env->pmp_state.pmp[addr_index].addr_reg;
}

bool pmp_is_any_region_locked(CPUState *env)
{
    if(unlikely(env->use_external_pmp)) {
        return tlib_extpmp_is_any_region_locked();
    }

    for(int i = 0; i < env->pmp_entry_count; i++) {
        if(pmp_is_locked(env, i)) {
            return true;
        }
    }
    return false;
}
