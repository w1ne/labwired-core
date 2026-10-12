#pragma once

#include <stdlib.h>

#include "osdep.h"  // For the 'unlikely' macro.

enum log_level { LOG_LEVEL_NOISY = -1, LOG_LEVEL_DEBUG = 0, LOG_LEVEL_INFO = 1, LOG_LEVEL_WARNING = 2, LOG_LEVEL_ERROR = 3 };

void *tlib_mallocz(size_t size);
char *tlib_strdup(const char *str);
void tlib_printf(enum log_level level, char *fmt, ...);
void tlib_abort(char *message);
void tlib_abortf(char *fmt, ...);

#define tlib_assert(x)                                                          \
    {                                                                           \
        if(unlikely(!(x))) {                                                    \
            tlib_abortf("Assert not met in %s:%d: %s", __FILE__, __LINE__, #x); \
            __builtin_unreachable();                                            \
        }                                                                       \
    }

#define tlib_assert_not_reached()                                        \
    {                                                                    \
        tlib_abortf("Should not reach here: %s %d", __FILE__, __LINE__); \
        __builtin_unreachable();                                         \
    }
#define g_assert_not_reached tlib_assert_not_reached

#include "callbacks.h"
