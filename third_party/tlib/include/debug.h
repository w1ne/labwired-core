#pragma once

#include <stdint.h>
#include "tcg-op.h"

#define MAX_MSG_COUNT 10000
extern char *msgs[MAX_MSG_COUNT];

#ifdef DEBUG_ON
#define LOG_CURRENT_LOCATION()                                                             \
    do {                                                                                   \
        tlib_printf(LOG_LEVEL_INFO, "We are in %s (%s:%d)", __func__, __FILE__, __LINE__); \
    } while(0)
#else
#define LOG_CURRENT_LOCATION()
#endif

void generate_log(uint64_t pc, char *format, ...);
void generate_var_log(TCGv v);
void generate_backtrace_print(void);

void mark_as_locked(struct TranslationBlock *tb, char *filename, int line_number);
void check_locked(struct TranslationBlock *tb);

#if DEBUG
#define LOCK_TB(tb)      mark_as_locked(tb, __FILE__, __LINE__);
#define UNLOCK_TB(tb)    tb->lock_active = 0
#define CHECK_LOCKED(tb) check_locked(tb)
#else
#define LOCK_TB(tb)
#define UNLOCK_TB(tb)
#define CHECK_LOCKED(tb)
#endif
