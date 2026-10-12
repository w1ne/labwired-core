#include "hash-table-store-test.h"

#include "cpu.h"
#include "infrastructure.h"
#include "tcg-op.h"
#include "tcg-op-atomic.h"
#include "atomic-intrinsics.h"

#include "debug.h"

static uint64_t hst_guest_address_mask = 0;

static_assert((sizeof(store_table_entry_t) & (sizeof(store_table_entry_t) - 1)) == 0, "HST entry size has to be a power of 2");

void calculate_hst_mask(const CPUState *env)
{
    //  *_bits means how many bits are used in the addresses, not how many bits
    //  large the contents is. I.e. if an entry is 64 bits large it uses 3 bits.
    const uint64_t content_bits = sizeof(uintptr_t) * 8 - (uint64_t)env->store_table_bits;

    const uint64_t table_size_bytes = 1ULL << content_bits;

    //  Table size must be a power of 2
    tlib_assert((table_size_bytes & (table_size_bytes - 1)) == 0);

#ifdef DEBUG
    uint64_t size_kib = table_size_bytes >> 10;
    uint64_t size_mib = table_size_bytes >> 20;
    uint64_t size_gib = table_size_bytes >> 30;
    if(size_kib == 0) {
        tlib_printf(LOG_LEVEL_DEBUG, "Store table is %u B (%u bits)", table_size_bytes, env->store_table_bits);
    } else if(size_mib == 0) {
        tlib_printf(LOG_LEVEL_DEBUG, "Store table is %u KiB (%u bits)", size_kib, env->store_table_bits);
    } else if(size_gib == 0) {
        tlib_printf(LOG_LEVEL_DEBUG, "Store table is %u MiB (%u bits)", size_mib, env->store_table_bits);
    } else {
        tlib_printf(LOG_LEVEL_DEBUG, "Store table is %u GiB (%u bits)", size_gib, env->store_table_bits);
    }
#endif

    //  `hst_guest_address_mask` will be ANDed with an arbitrary guest address,
    //  and the outcome of that AND must be an offset to an entry in the store
    //  table. `table_size_bytes` and `sizeof(store_table_entry_t)` are both
    //  powers of 2, so simply subtracting 1 will result in a sequence of
    //  `log2(n)` 1 bits

    const uint64_t table_mask = table_size_bytes - 1;

    const uint64_t entry_alignment_mask = ~(sizeof(store_table_entry_t) - 1);

    hst_guest_address_mask = table_mask & entry_alignment_mask;

    tlib_printf(LOG_LEVEL_DEBUG, "%s: store table is at 0x%016llx", __func__, env->store_table);
}

/* Get the address to the table entry (`entry_address`) for the given `guest_address` */
static void gen_get_table_entry(CPUState *env, TCGv_hostptr entry_address, TCGv_guestptr guest_address)
{
    tcg_gen_mov_tl(entry_address, guest_address);

    tcg_gen_andi_i64(entry_address, entry_address, hst_guest_address_mask);
    //  As explained above, `entry_address` is now an offset into an entry of the store table

    //  The HST table allocation is perfectly aligned, so OR is equivalent to ADD,
    //  since all the offset bits will be `0` on the table address
    uintptr_t store_table_address = (uintptr_t)env->store_table;
    tcg_gen_ori_i64(entry_address, entry_address, store_table_address);
}

store_table_entry_t *get_table_entry(CPUState *env, target_ulong guest_address)
{
    uintptr_t entry_address = guest_address;
    entry_address &= hst_guest_address_mask;
    entry_address |= (uintptr_t)env->store_table;
    return (store_table_entry_t *)entry_address;
}

//  Abstract away what the actual core id comes from.
uint32_t get_core_id(CPUState *env)
{
    //  Use the atomic_id as the "core id" required for HST.
    return env->atomic_id;
}

/*  Result is 1 if check succeeds, 0 otherwise. */
void gen_store_table_check(CPUState *env, TCGv result, TCGv_guestptr guest_address)
{
    //  Check will always succeed when there is only one core present, as there is no other core to access the reserved address.
    if(!are_multiple_cpus_registered()) {
        tcg_gen_movi_tl(result, 1);
        return;
    }

    TCGv_hostptr entry_address = tcg_temp_new_hostptr();
    gen_get_table_entry(env, entry_address, guest_address);

    //  Load core id from store table, to see which core last accessed the address.
    tcg_gen_ld32u_tl(result, entry_address, offsetof(store_table_entry_t, last_accessed_by_core_id));

    //  See if the current core is the one who last accessed the reserved address.
    //  returns 1 if condition succeeds, 0 otherwise.
    tcg_gen_setcondi_tl(TCG_COND_EQ, result, result, get_core_id(env));

    tcg_temp_free_hostptr(entry_address);
}

void gen_lock_not_owned_warning(TCGv_i64 holder_id, uint32_t current_core_id, TCGv_guestptr guest_address,
                                const char *function_name)
{
    generate_log(0, "%s: hash table entry lock for guest address:", function_name);
    generate_var_log(guest_address);
    generate_log(0, "is not held by current core (id %u), it is held by:", current_core_id);
    generate_var_log(holder_id);
}

/* Debug assert that ensures the current core owns the hash table entry lock
 * of the entry associated with the given `guest_address`.
 */
void gen_ensure_entry_locked(CPUState *env, TCGv_guestptr guest_address, const char *function_name)
{
#ifdef DEBUG
    TCGv_hostptr entry_address = tcg_temp_local_new_hostptr();
    gen_get_table_entry(env, entry_address, guest_address);

    TCGv_i32 holder_id = tcg_temp_local_new_i32();
    //  Load lock from store table, to see which core holds it.
    tcg_gen_ld32u_tl(holder_id, entry_address, offsetof(store_table_entry_t, lock));

    int done = gen_new_label();
    uint32_t current_core_id = get_core_id(env);
    //  Check if the lock is owned by the current core.
    tcg_gen_brcondi_i32(TCG_COND_EQ, holder_id, current_core_id, done);
    //  Lock isn't owned by the current core, abort.
    gen_lock_not_owned_warning(holder_id, current_core_id, guest_address, __func__);
    generate_backtrace_print();
    gen_helper_abort();

    gen_set_label(done);

    tcg_temp_free_i32(holder_id);
    tcg_temp_free_hostptr(entry_address);
#endif
}

void gen_store_table_set(CPUState *env, TCGv_guestptr guest_address)
{
    //  There's no need for updating the hash table when there's only a single core present, since it already tracks its own
    //  reservations.
    if(!are_multiple_cpus_registered()) {
        return;
    }

    //  In the normal RAM case this is called while the HST lock is still held.
    //  MMIO accesses may release dangling locks before the access completes to
    //  avoid deadlocks in managed callbacks; the reservation marker still has to
    //  be updated after the access so LR/SC observes the completed store.

    TCGv_hostptr entry_address = tcg_temp_local_new_hostptr();
    gen_get_table_entry(env, entry_address, guest_address);

    TCGv_i32 core_id = tcg_const_i32(get_core_id(env));

    //  The entry address now points to the table entry for the core id, so store it there.
    //  Note that the table entry update occurs atomically, with a single store.
    tcg_gen_st32_tl(core_id, entry_address, offsetof(store_table_entry_t, last_accessed_by_core_id));
    //  Memory barrier to ensure that this store doesn't get reordered with a store
    //  that will release the lock.
    tcg_gen_mb(TCG_MO_ST_ST);

    tcg_temp_free_hostptr(entry_address);
    tcg_temp_free_i32(core_id);
}

static void gen_store_table_lock_address(CPUState *env, TCGv_guestptr guest_address, tcg_target_long locked_address_offset)
{
    //  Skip the hash table locks when there is only one core present, since no synchronization is necessary.
    if(!are_multiple_cpus_registered()) {
        return;
    }

    TCGv_hostptr entry_address = tcg_temp_local_new_hostptr();
    gen_get_table_entry(env, entry_address, guest_address);

    //  Add the offset of the lock field, since we want to access the lock and not the core id.
    TCGv_hostptr lock_address = tcg_temp_local_new_hostptr();
    tcg_gen_addi_i64(lock_address, entry_address, offsetof(store_table_entry_t, lock));

    TCGv_i32 expected_lock = tcg_const_local_i32(HST_UNLOCKED);

    //  Acquiring the lock means storing this core's id.
    TCGv_i32 new_lock = tcg_const_local_i32(get_core_id(env));

    /*  We need to check two cases, hence the two branching instructions
     *  after the initial CAS. The table entry is either unlocked,
     *  locked by another thread, or locked by the current thread.
     *
     *                         │
     * ┌───────────────────────▼────────────────────────────┐
     * │result = CAS(expected_lock, lock_address, new_lock) |◄────────┐
     * └───────────────────────┬────────────────────────────┘         │
     *        true    ┌────────▼──────────┐                           │
     *         ┌──────┼ result == core_id │ "already locked by me?"   │
     *         ▼      └────────┬──────────┘                           │
     *       abort             │ false                                │
     *                ┌────────▼───────────────┐  true                │
     *      "locked?" │ result != HST_UNLOCKED ┼──────────────────────┘
     *                └────────┬───────────────┘
     *                         │ false
     *                         ▼
     *                   lock acquired!
     */
    int retry = gen_new_label();
    gen_set_label(retry);

    TCGv_i32 result = tcg_temp_local_new_i32();

    //  Optimistically try to atomically acquire the lock (only succeeds if it's currently unlocked).
    tcg_gen_atomic_compare_and_swap_host_intrinsic_i32(result, expected_lock, lock_address, new_lock);

    int start_retrying = gen_new_label();
    //  Locks are not reentrant, so it is an implementation bug
    //  if the lock is already taken by this core.
    tcg_gen_brcondi_i32(TCG_COND_NE, result, get_core_id(env), start_retrying);
    //  Abort if result == get_core_id(env) (reentrant lock attempt)
    TCGv_hostptr abort_message =
        tcg_const_hostptr((uintptr_t)"Attempted to acquire a store table lock that this CPU already holds");
    gen_helper_abort_message(abort_message);
    tcg_temp_free_hostptr(abort_message);

    gen_set_label(start_retrying);
    //  If result != HST_UNLOCKED, then the lock is taken, and we should keep retrying.
    tcg_gen_brcondi_i32(TCG_COND_NE, result, HST_UNLOCKED, retry);

    //  Lock is now owned by the current core.

    //  Update cpu's currently locked address.
    tcg_gen_st_tl(guest_address, cpu_env, locked_address_offset);

    tcg_temp_free_hostptr(entry_address);
    tcg_temp_free_hostptr(lock_address);
    tcg_temp_free_i32(expected_lock);
    tcg_temp_free_i32(new_lock);
    tcg_temp_free_i32(result);
}

void gen_store_table_lock(CPUState *env, TCGv_guestptr guest_address)
{
    gen_store_table_lock_address(env, guest_address, offsetof(CPUState, locked_address));
}

static void gen_store_table_unlock_address(CPUState *env, TCGv_guestptr guest_address, tcg_target_long locked_address_offset)
{
    //  There is no reason to unlock address when there is only one core present, as it was never locked to begin with.
    if(!are_multiple_cpus_registered()) {
        return;
    }

    TCGv_hostptr entry_address = tcg_temp_new_hostptr();
    gen_get_table_entry(env, entry_address, guest_address);

    //  Add the offset of the lock field, since we want to access the lock and not the core id.
    TCGv_hostptr lock_address = tcg_temp_new_hostptr();
    tcg_gen_addi_i64(lock_address, entry_address, offsetof(store_table_entry_t, lock));

    TCGv_i32 unlocked = tcg_const_i32(HST_UNLOCKED);

    uint32_t current_core_id_val = get_core_id(env);
    TCGv_i32 current_core_id = tcg_const_local_i32(current_core_id_val);
    TCGv_i32 holder_id = tcg_temp_local_new_i32();
    //  We must verify ownership over the lock in the table entry before releasing it.
    //  MMIO callbacks may call `unlock_dangling_locks` to prevent a deadlock
    //  while managed code blocks or pauses emulation.
    //  As a result, another core may acquire the lock, and we must not release it on its behalf.
    tcg_gen_atomic_compare_and_swap_host_intrinsic_i32(holder_id, current_core_id, lock_address, unlocked);
    //  Emit a barrier to ensure that the store is visible to other processors.
    tcg_gen_mb(TCG_MO_ST_ST);

    //  Update cpu's currently locked address.
    TCGv_guestptr null = tcg_const_tl(0);
    tcg_gen_st_tl(null, cpu_env, locked_address_offset);

    tcg_temp_free_hostptr(entry_address);
    tcg_temp_free_hostptr(lock_address);
    tcg_temp_free_i32(unlocked);
    tcg_temp_free_i32(current_core_id);
    tcg_temp_free_i32(holder_id);
    tcg_temp_free(null);
}

void gen_store_table_unlock(CPUState *env, TCGv_guestptr guest_address)
{
    gen_store_table_unlock_address(env, guest_address, offsetof(CPUState, locked_address));
}

static void gen_store_table_lock_high(CPUState *env, TCGv_guestptr guest_address)
{
    gen_store_table_lock_address(env, guest_address, offsetof(CPUState, locked_address_high));
}

static void gen_store_table_unlock_high(CPUState *env, TCGv_guestptr guest_address)
{
    gen_store_table_unlock_address(env, guest_address, offsetof(CPUState, locked_address_high));
}

void gen_store_table_lock_128(CPUState *env, TCGv_guestptr guest_addr_low, TCGv_guestptr guest_addr_high)
{
#ifdef DEBUG
    int addr_is_equal = gen_new_label();
    TCGv_guestptr comp_addr = tcg_temp_local_new_ptr();
    tcg_gen_addi_tl(comp_addr, guest_addr_low, sizeof(uint64_t));
    tcg_gen_brcond_tl(TCG_COND_EQ, comp_addr, guest_addr_high, addr_is_equal);

    /* If we didn't jump, the address pair is illegal */
    generate_log(0, "Illegal pair of guest addresses in %s", __func__);
    generate_log(0, "guest_addr_low:");
    generate_var_log(guest_addr_low);
    generate_log(0, "guest_addr_high:");
    generate_var_log(guest_addr_high);
    generate_log(0, "Should be: guest_addr_high == guest_addr_low+8bytes");
    generate_backtrace_print();
    gen_helper_abort();

    gen_set_label(addr_is_equal);
    tcg_temp_free_ptr(comp_addr);
#endif

    /* To avoid deadlocks, the order is important and should always lock the lowest 64 bits first */
    gen_store_table_lock(env, guest_addr_low);
    gen_store_table_lock_high(env, guest_addr_high);
}

void gen_store_table_unlock_128(CPUState *env, TCGv_guestptr guest_addr_low, TCGv_guestptr guest_addr_high)
{
    gen_store_table_unlock(env, guest_addr_low);
    gen_store_table_unlock_high(env, guest_addr_high);
}
