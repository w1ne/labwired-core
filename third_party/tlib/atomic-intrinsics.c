#include "cpu.h"
#include "tb-helper.h"
#include "address-translation.h"
#include "tcg-op-atomic.h"

#if __llvm__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wunused-function"
#else
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-function"
#endif

typedef union TCGv_unknown {
    TCGv_i32 size32;
    TCGv_i64 size64;
    TCGv_i128 size128;
} TCGv_unknown;

#if TCG_TARGET_HAS_INTRINSIC_ATOMICS
/*
 * Branches to `unalignedLabel` if the given `guestAddress` spans two pages.
 */
static void tcg_gen_brcond_page_spanning_check(TCGv_ptr guestAddress, uint16_t dataSize, int unalignedLabel)
{
    TCGv negatedTargetPageMask = tcg_temp_new();
    TCGv maskedAddress = tcg_temp_new();

    //  if ((addr & ~TARGET_PAGE_MASK) + dataSize - 1) >= TARGET_PAGE_SIZE then goto unalignedLabel
    tcg_gen_movi_tl(negatedTargetPageMask, ~TARGET_PAGE_MASK);
    tcg_gen_and_i64(maskedAddress, guestAddress, negatedTargetPageMask);
    tcg_gen_addi_i64(maskedAddress, maskedAddress, dataSize - 1);
    tcg_gen_brcondi_i64(TCG_COND_GEU, maskedAddress, TARGET_PAGE_SIZE, unalignedLabel);

    tcg_temp_free(negatedTargetPageMask);
    tcg_temp_free(maskedAddress);
}

/*
 * Branches to `fallbackLabel` if the given `guestAddress` cannot be accessed by native atomics.
 * Writes the translated address to `hostAddress`.
 */
static inline void tcg_gen_translate_address_and_fallback_guard(TCGv_hostptr hostAddress, TCGv_ptr guestAddress,
                                                                uint32_t memIndex, int fallbackLabel, uint8_t size)
{
    tlib_assert(size == 128 || size == 64 || size == 32);

    //  If the address spans two pages, it can't be implemented by a single host intrinsic,
    //  will have to fall back on the global memory lock.
    tcg_gen_brcond_page_spanning_check(guestAddress, size, fallbackLabel);

    /*
     * If address is page-aligned:
     */
    TCGv_i32 memIndexVar = tcg_const_i32(memIndex);
    TCGv_i32 accessType = tcg_const_i32(WRITE);
    if(size == 128) {
        gen_helper_translate_page_aligned_address_and_fill_tlb_u128(hostAddress, guestAddress, accessType, memIndexVar);
    } else if(size == 64) {
        gen_helper_translate_page_aligned_address_and_fill_tlb_u64(hostAddress, guestAddress, accessType, memIndexVar);
    } else {
        gen_helper_translate_page_aligned_address_and_fill_tlb_u32(hostAddress, guestAddress, accessType, memIndexVar);
    }
    tcg_temp_free(memIndexVar);
    tcg_temp_free_i32(accessType);

    //  If it's an MMIO address, it will be returned unchanged.
    //  Since we can't handle that case, we'll have to jump to the fallback.
    tcg_gen_brcond_i64(TCG_COND_EQ, hostAddress, guestAddress, fallbackLabel);
}
#endif

#if (TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i32 == 1) || (TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i64 == 1)
static inline void tcg_try_gen_atomic_fetch_add_intrinsic(TCGv_unknown result, TCGv_ptr guestAddress, TCGv_unknown toAdd,
                                                          uint32_t memIndex, int fallbackLabel, uint8_t size)
{
    tlib_assert(size == 64 || size == 32);

    /*
     * Jumps to fallback if address is not accessible atomically.
     */
    TCGv_hostptr hostAddress = tcg_temp_local_new_hostptr();
    tcg_gen_translate_address_and_fallback_guard(hostAddress, guestAddress, memIndex, fallbackLabel, size);

    /*
     * If address is both page-aligned and _not_ MMIO:
     */
    if(size == 64) {
        tcg_gen_atomic_fetch_add_intrinsic_i64(result.size64, hostAddress, toAdd.size64);
    } else {
        tcg_gen_atomic_fetch_add_intrinsic_i32(result.size32, hostAddress, toAdd.size32);
    }
    tcg_temp_free_hostptr(hostAddress);
}
#endif

#if TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i32
/*
 * Attempts to generate an atomic fetch and add, possibly failing and needing a fallback.
 *
 * The `fallback` label is jumped to if it is not feasible to atomically operate on it.
 */
void tcg_try_gen_atomic_fetch_add_intrinsic_i32(TCGv_i32 result, TCGv_ptr guestAddress, TCGv_i32 toAdd, uint32_t memIndex,
                                                int fallbackLabel)
{
    TCGv_unknown retUnknown, toAddUnknown;
    retUnknown.size32 = result;
    toAddUnknown.size32 = toAdd;
    tcg_try_gen_atomic_fetch_add_intrinsic(retUnknown, guestAddress, toAddUnknown, memIndex, fallbackLabel, 32);
}
#else
/*
 * Always use the fallback, since the target doesn't have the intrinsic implemented.
 */
void tcg_try_gen_atomic_fetch_add_intrinsic_i32(TCGv_i32 result, TCGv_ptr guestAddress, TCGv_i32 toAdd, uint32_t memIndex,
                                                int fallbackLabel)
{
    tcg_gen_br(fallbackLabel);
}
#endif

#if TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i64
/*
 * Attempts to generate an atomic fetch and add, possibly failing and needing a fallback.
 *
 * The `fallback` label is jumped to if it is not feasible to atomically operate on it.
 */
void tcg_try_gen_atomic_fetch_add_intrinsic_i64(TCGv_i64 result, TCGv_ptr guestAddress, TCGv_i64 toAdd, uint32_t memIndex,
                                                int fallbackLabel)
{
    TCGv_unknown retUnknown, toAddUnknown;
    retUnknown.size64 = result;
    toAddUnknown.size64 = toAdd;
    tcg_try_gen_atomic_fetch_add_intrinsic(retUnknown, guestAddress, toAddUnknown, memIndex, fallbackLabel, 64);
}
#else
/*
 * Always use the fallback, since the target doesn't have the intrinsic implemented.
 */
void tcg_try_gen_atomic_fetch_add_intrinsic_i64(TCGv_i64 result, TCGv_ptr guestAddress, TCGv_i64 toAdd, uint32_t memIndex,
                                                int fallbackLabel)
{
    tcg_gen_br(fallbackLabel);
}
#endif

#if (TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i32 == 1) || \
    (TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i64 == 1) || (TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i128 == 1)
static inline void tcg_gen_atomic_compare_and_swap_intrinsic_host(TCGv_unknown actual, TCGv_unknown expected,
                                                                  TCGv_hostptr hostAddress, TCGv_unknown newValue, uint8_t size)
{
    tlib_assert(size == 128 || size == 64 || size == 32);

    if(size == 128) {
        tcg_gen_atomic_compare_and_swap_intrinsic_i128(actual.size128, expected.size128, hostAddress, newValue.size128);
    } else if(size == 64) {
        tcg_gen_atomic_compare_and_swap_intrinsic_i64(actual.size64, expected.size64, hostAddress, newValue.size64);
    } else {
        tcg_gen_atomic_compare_and_swap_intrinsic_i32(actual.size32, expected.size32, hostAddress, newValue.size32);
    }
}

static inline void tcg_try_gen_atomic_compare_and_swap_intrinsic(TCGv_unknown actual, TCGv_unknown expected,
                                                                 TCGv_ptr guestAddress, TCGv_unknown newValue, uint32_t memIndex,
                                                                 int fallbackLabel, uint8_t size)
{
    /*
     * Jump to fallback if address is not accessible atomically.
     */
    TCGv_hostptr hostAddress = tcg_temp_local_new_hostptr();
    tcg_gen_translate_address_and_fallback_guard(hostAddress, guestAddress, memIndex, fallbackLabel, size);

    /*
     * If address is atomically accessible:
     */
    tcg_gen_atomic_compare_and_swap_intrinsic_host(actual, expected, hostAddress, newValue, size);
    tcg_temp_free_hostptr(hostAddress);
}
#endif

#if TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i32
void tcg_try_gen_atomic_compare_and_swap_intrinsic_i32(TCGv_i32 actual, TCGv_i32 expected, TCGv_ptr guestAddress,
                                                       TCGv_i32 newValue, uint32_t memIndex, int fallbackLabel)
{
    TCGv_unknown actualUnknown, expectedUnknown, newValueUnknown;
    actualUnknown.size32 = actual;
    expectedUnknown.size32 = expected;
    newValueUnknown.size32 = newValue;
    tcg_try_gen_atomic_compare_and_swap_intrinsic(actualUnknown, expectedUnknown, guestAddress, newValueUnknown, memIndex,
                                                  fallbackLabel, 32);
}

void tcg_gen_atomic_compare_and_swap_host_intrinsic_i32(TCGv_i32 actual, TCGv_i32 expected, TCGv_hostptr hostAddress,
                                                        TCGv_i32 newValue)
{
    TCGv_unknown actualUnknown, expectedUnknown, newValueUnknown;
    actualUnknown.size32 = actual;
    expectedUnknown.size32 = expected;
    newValueUnknown.size32 = newValue;
    tcg_gen_atomic_compare_and_swap_intrinsic_host(actualUnknown, expectedUnknown, hostAddress, newValueUnknown, 32);
}
#else
#error "32-bit atomic compare and swap instrinsic is nessesary for tcg to function properly"
#endif

#if TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i64
void tcg_try_gen_atomic_compare_and_swap_intrinsic_i64(TCGv_i64 actual, TCGv_i64 expected, TCGv_ptr guestAddress,
                                                       TCGv_i64 newValue, uint64_t memIndex, int fallbackLabel)
{
    TCGv_unknown actualUnknown, expectedUnknown, newValueUnknown;
    actualUnknown.size64 = actual;
    expectedUnknown.size64 = expected;
    newValueUnknown.size64 = newValue;
    tcg_try_gen_atomic_compare_and_swap_intrinsic(actualUnknown, expectedUnknown, guestAddress, newValueUnknown, memIndex,
                                                  fallbackLabel, 64);
}
#else
/*
 * Always use the fallback, since the target doesn't have the intrinsic implemented.
 */
void tcg_try_gen_atomic_compare_and_swap_intrinsic_i64(TCGv_i64 actual, TCGv_i64 expected, TCGv_ptr guestAddress,
                                                       TCGv_i64 newValue, uint64_t memIndex, int fallbackLabel)
{
    tcg_gen_br(fallbackLabel);
}
#endif

#if TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i128
void tcg_try_gen_atomic_compare_and_swap_intrinsic_i128(TCGv_i128 actual, TCGv_i128 expected, TCGv_ptr guestAddress,
                                                        TCGv_i128 newValue, uint64_t memIndex, int fallbackLabel)
{
    TCGv_unknown actualUnknown, expectedUnknown, newValueUnknown;
    actualUnknown.size128 = actual;
    expectedUnknown.size128 = expected;
    newValueUnknown.size128 = newValue;
    tcg_try_gen_atomic_compare_and_swap_intrinsic(actualUnknown, expectedUnknown, guestAddress, newValueUnknown, memIndex,
                                                  fallbackLabel, 128);
}
#else
/*
 * Always use the fallback, since the target doesn't have the intrinsic implemented.
 */
void tcg_try_gen_atomic_compare_and_swap_intrinsic_i128(TCGv_i128 actual, TCGv_i128 expected, TCGv_ptr guestAddress,
                                                        TCGv_i128 newValue, uint64_t memIndex, int fallbackLabel)
{
    tcg_gen_br(fallbackLabel);
}
#endif

#ifdef __llvm__
#pragma clang diagnostic pop
#else
#pragma GCC diagnostic pop
#endif
