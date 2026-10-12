/* public domain */

#pragma once

#define TLIB_NORETURN __attribute__((__noreturn__))

#if defined(_WIN32)
#define TLIB_PACKED __attribute__((gcc_struct, packed))
#else
#define TLIB_PACKED __attribute__((packed))
#endif
