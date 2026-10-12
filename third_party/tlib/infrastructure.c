/*
 *  Basic implementations of common functions.
 *
 *  Copyright (c) Antmicro
 *  Copyright (c) Realtime Embedded
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */
#include <string.h>
#include <stdarg.h>
#include <stdio.h>
#ifndef _WIN32  //  This header is not available on MinGW
#include <execinfo.h>
#endif
#include "callbacks.h"
#include "infrastructure.h"

void *global_retaddr = 0;

#if defined(__linux__) && defined(__x86_64__)

asm(".symver memcpy, memcpy@GLIBC_2.2.5");

void *__wrap_memcpy(void *dest, const void *src, size_t n)
{
    return memcpy(dest, src, n);
}
#endif

void *tlib_mallocz(size_t size)
{
    void *ret = tlib_malloc(size);
    memset(ret, 0, size);
    return ret;
}

char *tlib_strdup(const char *str)
{
    int length = strlen(str);
    char *ret = tlib_malloc(length + 1);
    strcpy(ret, str);
    return ret;
}

void tlib_printf(enum log_level level, char *fmt, ...)
{
    char s[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(s, 1024, fmt, ap);
    tlib_log((int32_t)level, s);
    va_end(ap);
}

void tlib_abortf(char *fmt, ...)
{
    char result[1024];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(result, 1024, fmt, ap);
#if DEBUG && !defined(_WIN32)
#define TRACE_MAX_SIZE 20
    void *array[TRACE_MAX_SIZE];
    char **strings;
    int size, i;
    size = backtrace(array, TRACE_MAX_SIZE);
    strings = backtrace_symbols(array, size);
    if(strings != NULL) {
        tlib_printf(LOG_LEVEL_ERROR, "Stack: [%d frames]", size);
        //  The last frame is meaningless, and the first one is the tlib_abort itself
        for(i = 1; i < size - 1; i++) {
            tlib_printf(LOG_LEVEL_ERROR, "%s\n", strings[i]);
        }
    }
    free(strings);
#endif
    tlib_abort(result);
    va_end(ap);
}
