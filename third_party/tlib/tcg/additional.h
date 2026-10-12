#pragma once

#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>

void *TCG_malloc(size_t size);
void *TCG_realloc(void *ptr, size_t size);
void TCG_free(void *ptr);
void TCG_pstrcpy(char *buf, int buf_size, const char *str);
char *TCG_pstrcat(char *buf, int buf_size, const char *s);

//  The highest value of NB_MMU_MODES used incremented by one.
#define MMU_MODES_MAX 16

extern unsigned int temp_buf_offset;
extern unsigned int tlb_table_n_0_addr_read[MMU_MODES_MAX];
extern unsigned int tlb_table_n_0_addr_write[MMU_MODES_MAX];
extern unsigned int tlb_table_n_0_addend[MMU_MODES_MAX];
extern unsigned int tlb_table_n_0[MMU_MODES_MAX];
extern unsigned int tlb_entry_addr_read;
extern unsigned int tlb_entry_addr_write;
extern unsigned int tlb_entry_addend;
extern unsigned int sizeof_CPUTLBEntry;

/* XXX: make safe guess about sizes */
#define MAX_OP_PER_INSTR 8192

#if HOST_LONG_BITS == 32
#define MAX_OPC_PARAM_PER_ARG 2
#else
#define MAX_OPC_PARAM_PER_ARG 1
#endif
#define MAX_OPC_PARAM_IARGS 5
#define MAX_OPC_PARAM_OARGS 2
#define MAX_OPC_PARAM_ARGS  (MAX_OPC_PARAM_IARGS + MAX_OPC_PARAM_OARGS)

/* A Call op needs up to 4 + 2N parameters on 32-bit archs,
 * and up to 4 + N parameters on 64-bit archs
 * (N = number of input arguments + output arguments).  */
#define MAX_OPC_PARAM (4 + (MAX_OPC_PARAM_PER_ARG * MAX_OPC_PARAM_ARGS))
#define OPC_BUF_SIZE  16384
#define OPC_MAX_SIZE  (OPC_BUF_SIZE - MAX_OP_PER_INSTR)

#if MAX_OP_PER_INSTR >= OPC_BUF_SIZE
#error OPC_BUF_SIZE has to be greater than MAX_OP_PER_INSTR
#endif

/* Maximum size a TCG op can expand to.  This is complicated because a
   single op may require several host instructions and register reloads.
   For now take a wild guess at 192 bytes, which should allow at least
   a couple of fixup instructions per argument.  */
#define TCG_MAX_OP_SIZE 192

/* The maximum size of generated code within a block. */
#define TCG_MAX_CODE_SIZE (TCG_MAX_OP_SIZE * OPC_BUF_SIZE)

/* The maximum size of PC search data within a block. */
#define TCG_MAX_SEARCH_SIZE (TCG_MAX_CODE_SIZE * 0.3)

#define OPPARAM_BUF_SIZE (OPC_BUF_SIZE * MAX_OPC_PARAM)

void tlib_abort(char *msg);

static inline void tcg_abortf(char *fmt, ...)
{
    char result[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(result, 1024, fmt, ap);
    tlib_abort(result);
    va_end(ap);
    __builtin_unreachable();
}
struct TranslationBlock;

void tcg_perf_init_labeling();
void tcg_perf_fini_labeling();
void tcg_perf_flush_map();
void tcg_perf_out_symbol_s(void *s, int size, const char *label, struct TranslationBlock *tb);
void tcg_perf_out_symbol_i(void *s, int size, int label, struct TranslationBlock *tb);
void tcg_perf_out_symbol(void *s, int size, struct TranslationBlock *tb);

void tcg_perf_out_symbol_from_tb(struct TranslationBlock *tb, int host_size, const char *comment);
int tcg_perf_tb_info_to_string(struct TranslationBlock *tb, char *buffer, int maxsize);
