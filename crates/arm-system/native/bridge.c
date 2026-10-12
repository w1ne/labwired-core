/* LabWired host bridge. No longjmp may cross a Rust callback frame. */
// clang-format off
#include <stdint.h>
#include <setjmp.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>
#include "unwind.h"
#include "cpu.h"
#include "exec-all.h"
#include "tcg.h"
#include "tlib-alloc.h"
#include <sys/mman.h>
// clang-format on
extern int tlib_init(char *);
extern void tlib_reset(void), tlib_dispose(void),
    tlib_invalidate_translation_cache(void);
extern uint64_t tlib_get_register_value(int),
    tlib_get_executed_instructions(void);
extern void tlib_set_register_value(int, uint64_t),
    tlib_register_access_flags_for_range(uint64_t, uint64_t, uint32_t),
    tlib_set_irq(int, int);
extern int tlib_execute_ex(uint32_t), tlib_is_wfi(void);
extern uint32_t tlib_get_page_size(void);
extern void tlib_set_block_finished_hook_present(uint32_t);
extern void tlib_set_block_begin_hook_present(uint32_t);
extern void tlib_set_interrupt_begin_hook_present(uint32_t);
extern void tlib_set_translation_cache_configuration(uint64_t, uint64_t);
extern int32_t tlib_atomic_memory_state_init(uintptr_t, int32_t);
extern void tlib_unwind(void), tlib_try_interrupt_translation_block(void);
typedef int (*callback)(void *, uint32_t, uint64_t, uint64_t, uint64_t,
                        uint64_t, uint64_t *);
static void *context;
static callback host;
static jmp_buf fatal_env;
static int guard_active;
static char error[1024];
static int trace_enabled, trace_valid;
static uint32_t trace_pc, trace_opcode;
static uint32_t exceptions_without_retirement;
static uint64_t translation_cache_size, translation_cache_allocations;
#define LW_CACHE_MIN (8ULL * 1024 * 1024)
#define LW_CACHE_MAX (16ULL * 1024 * 1024)
_Static_assert(LW_CACHE_MIN > TCG_MAX_CODE_SIZE,
               "ARM translation cache must exceed a maximum generated block");
static struct {
  uint32_t pc, ifar, ifsr, dfar, dfsr, sctlr, ttbr0, ttbr1, dacr;
  uint64_t index;
} first_exception;
void tlib_on_translation_cache_size_change(uint64_t size) {
  translation_cache_size = size;
  translation_cache_allocations++;
}
/* tlib's exclusive monitor storage is caller-owned. BSS is initially zeroed
 * and belongs to this CPU's unique private library, never to another CPU.
 * tlib initializes its pthread objects; we destroy them before dlclose frees
 * this storage. There is exactly one registered CPU in this memory domain. */
extern void tlib_abort(char *);
static atomic_memory_state_t atomic_state;
static void dispose_atomic_state(void) {
  if (atomic_state.is_mutex_initialized) {
    int cond_status = pthread_cond_destroy(&atomic_state.global_cond);
    int mutex_status = pthread_mutex_destroy(&atomic_state.global_mutex);
    atomic_state.is_mutex_initialized = 0;
    if (cond_status || mutex_status)
      tlib_abort("native atomic state synchronization disposal failed");
  }
}
void lw_bind(void *ctx, callback cb) {
  context = ctx;
  host = cb;
}
const char *lw_error(void) { return error; }
void tlib_abort(char *message) {
  strncpy(error, message ? message : "native fatal error", sizeof(error) - 1);
  error[sizeof(error) - 1] = 0;
  /* Every entry point, including init/dispose, is reached through lw_call. */
  if (guard_active)
    longjmp(fatal_env, 1);
}
/* The pinned upstream free_code_gen_buf unmaps rw and rx unconditionally.
 * On x86_64 they alias. After the first unmap, another CPU may allocate the
 * address; a second unmap would steal that CPU's JIT mapping. Linker wrapping
 * fixes ownership without modifying the vendored source or serializing CPUs. */
void __wrap_free_code_gen_buf(void) {
  uint8_t *rw = tcg_rw_buffer;
  uint8_t *rx = tcg_rx_buffer;
  size_t size = code_gen_buffer_size + TCG_PROLOGUE_SIZE;
  tcg_rw_buffer = NULL;
  tcg_rx_buffer = NULL;
  if (rw && munmap(rw, size))
    tlib_abort("native JIT writable mapping disposal failed");
  if (rx && rx != rw && munmap(rx, size))
    tlib_abort("native JIT executable mapping disposal failed");
}
/* These strong symbols bind native libc fatal paths locally (-Bsymbolic).
 * They are called only below an active lw_call C guard; no Rust frames are
 * skipped. They do not attempt to contain memory corruption or OS faults. */
_Noreturn void abort(void) {
  tlib_abort("native abort()");
  __builtin_unreachable();
}
_Noreturn void __assert_fail(const char *expression, const char *file,
                             unsigned int line, const char *function) {
  char message[1024];
  snprintf(message, sizeof(message), "native assertion %s at %s:%u (%s)",
           expression, file, line, function);
  tlib_abort(message);
  __builtin_unreachable();
}
static void reject_lpae(void) {
  if (cpu && cpu->cp15.c2_ttbcr_eae)
    tlib_abort("ARM LPAE translation is unsupported");
}
/* Invalid MMU vectors can repeatedly fault inside ONE tlib_execute call, so
 * a Rust-side zero-retirement retry bound cannot contain them. This hook is
 * reached only in C, after physical callbacks have returned. Count only
 * consecutive exception entries; actual retirement permits normal recurring
 * SVC/IRQ/fault handlers indefinitely. */
void tlib_on_interrupt_begin(uint64_t exception_index) {
  if (exceptions_without_retirement == 0) {
    first_exception.pc = CPU_PC(cpu);
    first_exception.ifar = cpu->cp15.c6_insn;
    first_exception.ifsr = cpu->cp15.c5_insn;
    first_exception.dfar = cpu->cp15.c6_data;
    first_exception.dfsr = cpu->cp15.c5_data;
    first_exception.sctlr = cpu->cp15.c1_sys;
    first_exception.ttbr0 = cpu->cp15.c2_base0;
    first_exception.ttbr1 = cpu->cp15.c2_base1;
    first_exception.dacr = cpu->cp15.c3;
    first_exception.index = exception_index;
  }
  if (exceptions_without_retirement >= 8) {
    char message[1024];
    snprintf(message, sizeof(message),
             "ARM exception entry repeated without retirement: "
             "pc=%#x exception=%llu ifar=%#x ifsr=%#x dfar=%#x dfsr=%#x "
             "sctlr=%#x ttbr0=%#x ttbr1=%#x dacr=%#x; "
             "first_pc=%#x first_exception=%llu first_ifar=%#x first_ifsr=%#x "
             "first_dfar=%#x first_dfsr=%#x first_sctlr=%#x first_ttbr0=%#x "
             "first_ttbr1=%#x first_dacr=%#x",
             (uint32_t)CPU_PC(cpu), (unsigned long long)exception_index,
             cpu->cp15.c6_insn, cpu->cp15.c5_insn, cpu->cp15.c6_data,
             cpu->cp15.c5_data, cpu->cp15.c1_sys, cpu->cp15.c2_base0,
             cpu->cp15.c2_base1, cpu->cp15.c3, first_exception.pc,
             (unsigned long long)first_exception.index, first_exception.ifar,
             first_exception.ifsr, first_exception.dfar, first_exception.dfsr,
             first_exception.sctlr, first_exception.ttbr0,
             first_exception.ttbr1, first_exception.dacr);
    tlib_abort(message);
  }
  exceptions_without_retirement++;
}
void tlib_on_block_finished(uint64_t pc, uint32_t count) {
  if (count > 0)
    exceptions_without_retirement = 0;
  /* Pinned translate.c ends a TB on CP15 register writes unless explicitly
   * suppressed. TTBCR has no suppression flag, so EAE is checked before the
   * next guest instruction/access. This C callback never enters Rust. */
  reject_lpae();
}
static uint32_t cp15_key(uint32_t encoded, int wide) {
  return encoded &
         (wide ? 0xffu : ((7u << 21) | (15u << 16) | (7u << 5) | 15u));
}
static uint64_t invoke(uint32_t kind, uint64_t a, uint64_t b, uint64_t c,
                       uint64_t d) {
  uint64_t value = 0;
  int status = host && context ? host(context, kind, a, b, c, d, &value) : -1;
  if (status) {
    strcpy(error, "physical bus callback failed");
    /* Rust has returned; only C frames are unwound. */
    tlib_unwind();
    tlib_try_interrupt_translation_block();
  }
  return value;
}
/* The native decoder has already resolved these physical code pages. Using
 * the current TB avoids VA-as-PA reads and the debug translation API's TLB
 * flush, which is forbidden while generated code is running. No Rust frame
 * is skipped on failure: invoke() unwinds only after its callback returns. */
static uint64_t trace_physical(uint64_t virtual) {
  TranslationBlock *tb = cpu->current_tb;
  unsigned page =
      (virtual & TARGET_PAGE_MASK) == (tb->pc & TARGET_PAGE_MASK) ? 0 : 1;
  if (tb->page_addr[page] == (tb_page_addr_t)-1)
    tlib_abort("native trace has no translated physical page");
  return tb->page_addr[page] | (virtual & ~TARGET_PAGE_MASK);
}
uint32_t tlib_on_block_begin(uint64_t pc, uint32_t count) {
  if (!trace_enabled)
    return 1;
  trace_pc = pc;
  if (tlib_get_register_value(25) & 32) {
    uint32_t low = invoke(0, trace_physical(pc), 2, 0, 0);
    uint32_t prefix = low >> 11;
    trace_opcode = low;
    if (prefix >= 0x1d) {
      uint32_t high = invoke(0, trace_physical(pc + 2), 2, 0, 0);
      trace_opcode |= high << 16;
    }
  } else {
    trace_opcode = invoke(0, trace_physical(pc), 4, 0, 0);
  }
  trace_valid = 1;
  return 1;
}
#define READ(n, w)                                                             \
  uint64_t tlib_read_##n(uint64_t a, uint64_t s) {                             \
    return invoke(0, a, w, s, 0);                                              \
  }
#define WRITE(n, w)                                                            \
  void tlib_write_##n(uint64_t a, uint64_t v, uint64_t s) {                    \
    invoke(1, a, w, s, v);                                                     \
  }
// clang-format off
READ(byte, 1)
READ(word, 2)
READ(double_word, 4)
READ(quad_word, 8)
WRITE(byte, 1)
WRITE(word, 2)
WRITE(double_word, 4)
WRITE(quad_word, 8)
// clang-format on
uint32_t tlib_read_cp15_32(uint32_t i) {
  return invoke(2, cp15_key(i, 0), 0, 0, 0);
}
uint64_t tlib_read_cp15_64(uint32_t i) {
  return invoke(2, cp15_key(i, 1), 1, 0, 0);
}
void tlib_write_cp15_32(uint32_t i, uint32_t v) {
  invoke(3, cp15_key(i, 0), 0, 0, v);
}
void tlib_write_cp15_64(uint32_t i, uint64_t v) {
  invoke(3, cp15_key(i, 1), 1, 0, v);
}
void *tlib_guest_offset_to_host_ptr(uint64_t offset) {
  tlib_abort("unexpected request for host RAM pointer");
  return NULL;
}
/* This C-only fatal guard also protects calls which have no tlib _ex wrapper.
 * Restoring unwind nesting is essential if fatal jumps out of execute_ex. */
int lw_call(uint32_t op, uint64_t a, uint64_t b, uint64_t *out,
            uint64_t *extra) {
  int previous_depth = unwind_state.env_idx;
  error[0] = 0;
  guard_active = 1;
  if (setjmp(fatal_env)) {
    unwind_state.env_idx = previous_depth;
    unwind_state.need_jump = 0;
    guard_active = 0;
    trace_enabled = 0;
    return -1;
  }
  switch (op) {
  case 0:
    /* The pinned default 1 MiB cache is smaller than the code/search reserve;
     * its unsigned high-water calculation underflows and overwrites the JIT
     * prologue. Configure a bounded safe cache before its first allocation.
     * Check pinned constants so a future reserve change fails explicitly. */
    if (LW_CACHE_MIN <= TCG_MAX_CODE_SIZE + TCG_MAX_SEARCH_SIZE)
      tlib_abort(
          "ARM translation cache is smaller than native code/search reserve");
    tlib_set_translation_cache_configuration(LW_CACHE_MIN, LW_CACHE_MAX);
    if (tlib_init("cortex-a7"))
      strcpy(error, "Cortex-A7 initialization failed");
    else {
      if (tlib_atomic_memory_state_init((uintptr_t)&atomic_state, -1) != 0)
        tlib_abort("native atomic state initialization failed");
      tlib_set_block_finished_hook_present(1);
      tlib_set_block_begin_hook_present(1);
      tlib_set_interrupt_begin_hook_present(1);
    }
    break;
  case 1:
    cancel_reservation(cpu);
    tlib_reset();
    tlib_set_block_finished_hook_present(1);
    tlib_set_block_begin_hook_present(1);
    tlib_set_interrupt_begin_hook_present(1);
    exceptions_without_retirement = 0;
    break;
  case 2:
    *out = tlib_get_register_value(a);
    break;
  case 3:
    tlib_set_register_value(a, b);
    break;
  case 4:
    tlib_register_access_flags_for_range(a, b, 1);
    break;
  case 5:
    tlib_set_irq(2, a != 0);
    tlib_set_irq(16, b != 0);
    break;
  case 6:
  case 12:
    trace_enabled = op == 12;
    trace_valid = 0;
    reject_lpae();
    tlib_execute_ex(a);
    *out = tlib_get_executed_instructions();
    *extra = tlib_is_wfi();
    trace_enabled = 0;
    break;
  case 7:
    tlib_invalidate_translation_cache();
    break;
  case 8:
    tlib_dispose();
    dispose_atomic_state();
    break;
  case 9:
    *out = tlib_get_page_size();
    break;
  case 10:
    /* Internal fatal-path probes, exercised in subprocess unit tests. */
    if (a)
      __assert_fail("guard probe", __FILE__, __LINE__, __func__);
    else
      abort();
    break;
  case 11:
    /* A different physical bus is a different exclusive reservation domain. */
    cancel_reservation(cpu);
    break;
  case 13:
    if (!trace_valid)
      tlib_abort("native retired instruction has no trace encoding");
    *out = trace_pc;
    *extra = trace_opcode;
    break;
  case 14:
    *out = translation_cache_size;
    *extra = translation_cache_allocations;
    break;
  default:
    tlib_abort("unknown bridge operation");
  }
  guard_active = 0;
  return error[0] ? -1 : 0;
}
