#include "def-helper.h"

DEF_HELPER_1(prepare_block_for_execution, i32, ptr)
DEF_HELPER_0(block_begin_event, i32)
DEF_HELPER_2(block_finished_event, void, tl, i32)
DEF_HELPER_1(try_exit_cpu_loop, void, env)
DEF_HELPER_2(log, void, i32, i64)
DEF_HELPER_1(var_log, void, tl)
DEF_HELPER_0(abort, void)
DEF_HELPER_2(announce_stack_change, void, tl, i32)
DEF_HELPER_3(announce_stack_pointer_change, void, tl, tl, tl)
DEF_HELPER_1(on_interrupt_end_event, void, i64)

DEF_HELPER_1(invalidate_dirty_addresses_shared, void, env)
DEF_HELPER_4(mark_tbs_as_dirty, void, env, tl, i32, i32)

DEF_HELPER_1(count_opcode_inner, void, i32)
DEF_HELPER_1(tlb_flush, void, env)

DEF_HELPER_1(acquire_global_memory_lock, void, env)
DEF_HELPER_1(release_global_memory_lock, void, env)
DEF_HELPER_3(reserve_address, void, env, uintptr, i32)
DEF_HELPER_2(check_address_reservation, tl, env, uintptr)
DEF_HELPER_2(register_address_access, void, env, uintptr)
DEF_HELPER_1(cancel_reservation, void, env)

DEF_HELPER_3(translate_page_aligned_address_and_fill_tlb_u32, uintptr, tl, i32, i32)
DEF_HELPER_3(translate_page_aligned_address_and_fill_tlb_u64, uintptr, tl, i32, i32)
DEF_HELPER_3(translate_page_aligned_address_and_fill_tlb_u128, uintptr, tl, i32, i32)

DEF_HELPER_3(handle_pre_opcode_execution_hook, void, i32, i64, i64)
DEF_HELPER_3(handle_post_opcode_execution_hook, void, i32, i64, i64)

DEF_HELPER_1(abort_message, void, ptr)

#include "def-helper.h"
