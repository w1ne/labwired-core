#pragma once

#include <pthread.h>
#include <stdint.h>
#include <stdbool.h>
#include "targphys.h"

#define MAX_NUMBER_OF_CPUS 32

#define NO_CPU_ID      0xFFFFFFFF
#define NO_RESERVATION -1

struct CPUState;

typedef struct address_reservation_t {
    uint32_t locking_cpu_id;
    uint64_t address;
    uint8_t active_flag;
    uint8_t id;
    uint8_t manual_free;
} address_reservation_t;

//  Accessing peripherals invokes managed MMIO callbacks that may block, pause
//  emulation, or otherwise wait for other cores. Before such callbacks,
//  `unlock_dangling_locks()` releases memory locks to avoid deadlocks.
//
//  We assume that atomic instructions are not used to perform operations that
//  invoke managed MMIO callbacks. If such a callback is invoked, the atomicity of the
//  current instruction is no longer guaranteed.
//
//  Atomic instructions executed concurrently on other cores remain atomic
//  because lock release paths verify ownership before unlocking, preventing a
//  core from releasing locks held by another core.
typedef struct atomic_memory_state_t {
    uint8_t is_mutex_initialized;
    uint8_t are_reservations_valid;

    uint32_t number_of_registered_cpus;

    uint32_t locking_cpu_id;
    uint32_t entries_count;

    int reservations_count;
    //  Invariant:
    //  A CPU may access reservations table only if it currently
    //  owns the global memory lock
    int reservations_by_cpu[MAX_NUMBER_OF_CPUS];
    address_reservation_t reservations[MAX_NUMBER_OF_CPUS];

    pthread_mutex_t global_mutex;
    pthread_cond_t global_cond;

} atomic_memory_state_t;

bool are_multiple_cpus_registered();

int32_t register_in_atomic_memory_state(atomic_memory_state_t *sm, int32_t atomic_id);

void acquire_global_memory_lock(struct CPUState *env);
void release_global_memory_lock(struct CPUState *env);

void reserve_address(struct CPUState *env, target_phys_addr_t address, uint8_t manual_free);
uint32_t check_address_reservation(struct CPUState *env, target_phys_addr_t address);
void register_address_access(struct CPUState *env, target_phys_addr_t address);
void cancel_reservation(struct CPUState *env);

void unlock_dangling_locks(struct CPUState *env);
