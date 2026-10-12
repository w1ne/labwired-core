#pragma once

#include <stdint.h>
#include <stdbool.h>

#if TARGET_LONG_BITS == 32
#define TCGv TCGv_i32
typedef uint32_t target_ulong __attribute__((aligned(TARGET_LONG_ALIGNMENT)));
#elif TARGET_LONG_BITS == 64
#define TCGv TCGv_i64
typedef uint64_t target_ulong __attribute__((aligned(TARGET_LONG_ALIGNMENT)));
#else
#error Unhandled TARGET_LONG_BITS value
#endif

#define TCGv_guestptr TCGv

#if HOST_LONG_BITS == 32
#define TCGv_hostptr TCGv_i32
#elif HOST_LONG_BITS == 64
#define TCGv_hostptr TCGv_i64
#else
#error Unhandled HOST_LONG_BITS value
#endif

#define HST_UNLOCKED 0xFFFFFFFF
#define HST_NO_CORE  0xFFFFFFFF

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

/* A single entry in the store table. */
typedef struct {
    /*  The ID of the core that last wrote to (or reserved)
     *  one of the addresses represented by this entry.
     */
    uint32_t last_accessed_by_core_id;

    /*  A fine-grained lock used to ensure mutual exclusion when modifying the above field. */
    uint32_t lock;
} store_table_entry_t;

typedef int TCGv_i32;
typedef int TCGv_i64;
typedef struct CPUState CPUState;

void calculate_hst_mask(const CPUState *env);

uint32_t get_core_id(CPUState *env);

/* Generates code to update the hash table entry corresponding to the given
 * `guest_address` with the current core's ID.
 */
void gen_store_table_set(CPUState *env, TCGv_guestptr guest_address);

/*
 * Generates code to check whether the hash table entry corresponding to the
 * given `guest_address` has been invalidated.
 * Places 1 in `result` if the address is reserved by the current core, 0 otherwise.
 */
void gen_store_table_check(CPUState *env, TCGv result, TCGv_guestptr guest_address);

/* Generates code to acquire the lock of the hash table entry
 * corresponding to the given `guest_address`.
 */
void gen_store_table_lock(CPUState *env, TCGv_guestptr guest_address);

/* Generates code to release the lock of the hash table entry
 * corresponding to the given `guest_address`.
 */
void gen_store_table_unlock(CPUState *env, TCGv_guestptr guest_address);

/* Computes which hash table entry address corresponds to the given `guest_address`. */
store_table_entry_t *get_table_entry(CPUState *cpu_env, target_ulong guest_address);

/* Generates code to lock a 128 bit region (two hash table entries).
 * The arguments must adhere to:
 *    * `guest_addr_low < guest_addr_high`
 *    * `guest_addr_low + sizeof(uint64_t) == guest_addr_high`
 */
void gen_store_table_lock_128(CPUState *env, TCGv_guestptr guest_addr_low, TCGv_guestptr guest_addr_high);

/* Generates code to unlock a 128 bit region (two hash table entries)
 * This should match the addresses used when calling `gen_store_table_lock_pair()`
 */
void gen_store_table_unlock_128(CPUState *env, TCGv_guestptr guest_addr_low, TCGv_guestptr guest_addr_high);
