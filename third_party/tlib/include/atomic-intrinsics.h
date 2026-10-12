#pragma once

void tcg_try_gen_atomic_fetch_add_intrinsic_i32(TCGv_i32 result, TCGv_ptr guestAddress, TCGv_i32 toAdd, uint32_t memIndex,
                                                int fallbackLabel);

void tcg_try_gen_atomic_fetch_add_intrinsic_i64(TCGv_i64 result, TCGv_ptr guestAddress, TCGv_i64 toAdd, uint32_t memIndex,
                                                int fallbackLabel);

void tcg_try_gen_atomic_compare_and_swap_intrinsic_i32(TCGv_i32 actual, TCGv_i32 expected, TCGv_ptr guestAddress,
                                                       TCGv_i32 newValue, uint32_t memIndex, int fallbackLabel);

void tcg_gen_atomic_compare_and_swap_host_intrinsic_i32(TCGv_i32 actual, TCGv_i32 expected, TCGv_hostptr hostAddress,
                                                        TCGv_i32 newValue);

void tcg_try_gen_atomic_compare_and_swap_intrinsic_i64(TCGv_i64 actual, TCGv_i64 expected, TCGv_ptr guestAddress,
                                                       TCGv_i64 newValue, uint32_t memIndex, int fallbackLabel);

void tcg_try_gen_atomic_compare_and_swap_intrinsic_i128(TCGv_i128 actual, TCGv_i128 expected, TCGv_ptr guestAddress,
                                                        TCGv_i128 newValue, uint64_t memIndex, int fallbackLabel);
