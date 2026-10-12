/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>
#include <stdbool.h>

struct CPUState;
enum arm_cpu_mode;

//  PMU (Performance Monitoring Unit)

#define PMU_MAX_PROGRAMMABLE_COUNTERS 31
#define PMU_EVENTS_NUMBER             3

//  Supported event types
#define PMU_EVENT_SOFTWARE_INCREMENT    0x00
#define PMU_EVENT_INSTRUCTIONS_EXECUTED 0x08
#define PMU_EVENT_CYCLES                0x11

#define PMU_CYCLE_COUNTER_ID 31

#define PMCR_E  (1 << 0)
#define PMCR_P  (1 << 1)
#define PMCR_C  (1 << 2)
#define PMCR_D  (1 << 3)
#define PMCR_X  (1 << 4)
#define PMCR_DP (1 << 5)

#define PMCR_IMPL               (0xFF << 24)
#define PMCR_NO_COUNTERS_OFFSET 11
#define PMCR_NO_COUNTERS_MASK   0x1F

#define PMUSERENR_EN (1 << 0)

#define PMXEVTYPER_EVT 0xFF
#define PMXEVTYPER_P   (1u << 31)
#define PMXEVTYPER_U   (1 << 30)

//  Implemented PMU events
typedef struct pmu_event {
    char *name;
    uint32_t identifier;
} pmu_event;

typedef struct pmu_counter {
    //  Counter id, which should be the same as its position in pmu.counters array
    int id;
    bool enabled;
    bool enabled_overflow_interrupt;
    //  ID of measured event (`pm_event`)
    int measured_event_id;

    /* These are PMUv2 specific. Note, that setting to 0/false means that they will count. */
    bool ignore_count_at_pl0;
    bool ignore_count_at_pl1;
    //  Analogously would be counts in Secure/Non-Secure mode, which we don't support at this moment

    //  === These fields should not be directly modified ===
    //  Instead `helper_pmu_update_event_counters` (recommended) or `pmu_update_counter` should be called
    uint32_t val;
    //  Counter value is not always updated per-tick - we need to explicitly calculate it's value somehow
    //  Currently only for cycles and instructions, this could be extended to be more generic in the future
    uint64_t snapshot;  //  Snapshot taken of external event source - used to compute return val lazily
    //  Note that snapshots can only be taken when the counter is capable of counting events (both PMU and the counter are
    //  enabled) If the counter changes state in any way (is disabled or changes event) the value MUST be calculated and snapshot
    //  reset
    bool has_snapshot;

    bool overflowed;
} pmu_counter;

//  Is the counter a `Cycle` or `Executed instructions` counter
static inline bool pmu_is_insns_or_cycles_counter(const pmu_counter *const counter)
{
    return counter->measured_event_id == PMU_EVENT_INSTRUCTIONS_EXECUTED || counter->measured_event_id == PMU_EVENT_CYCLES;
}

void pmu_recalculate_cycles_instruction_limit_custom_mode(enum arm_cpu_mode mode);
void pmu_recalculate_cycles_instruction_limit();
void pmu_update_cycles_instruction_limit(const pmu_counter *const cnt);
uint32_t pmu_get_insn_cycle_value(const pmu_counter *const counter);
void pmu_init_reset(struct CPUState *env);
void pmu_switch_mode_user(enum arm_cpu_mode new_mode);

void set_c9_pmcntenset(struct CPUState *env, uint64_t val);
void set_c9_pmcntenclr(struct CPUState *env, uint64_t val);
void set_c9_pmcr(struct CPUState *env, uint64_t val);
void set_c9_pmovsr(struct CPUState *env, uint64_t val);
void set_c9_pmuserenr(struct CPUState *env, uint64_t val);
void set_c9_pmintenset(struct CPUState *env, uint64_t val);
void set_c9_pmintenclr(struct CPUState *env, uint64_t val);
uint64_t get_c9_pmccntr(struct CPUState *env);
void set_c9_pmselr(struct CPUState *env, uint64_t val);
void set_c9_pmxevtyper(struct CPUState *env, uint64_t val);
uint32_t get_c9_pmxevcntr(struct CPUState *env);
void set_c9_pmxevcntr(struct CPUState *env, uint64_t val);
void set_c9_pmswinc(struct CPUState *env, uint64_t val);

void helper_pmu_count_instructions_cycles(uint32_t icount);

void pmu_update_counter(struct CPUState *env, pmu_counter *const counter, uint64_t amount);
void helper_pmu_update_event_counters(struct CPUState *env, int event_id, uint32_t amount);

void pmu_recalculate_all_lazy(void);
void pmu_take_all_snapshots(void);
