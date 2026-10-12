/* Define target_phys_addr_t if it exists.  */

#pragma once

#ifndef TARGET_PHYS_ADDR_BITS
#define TARGET_PHYS_ADDR_BITS TARGET_LONG_BITS
#endif

/* target_phys_addr_t is the type of a physical address (its size can
   be different from 'target_ulong').

   SPARC architecture has 36-bits wide physical address, that's why
   we use `<=` operators below instead of simple `==`
 */

#if TARGET_PHYS_ADDR_BITS <= 32
typedef uint32_t target_phys_addr_t;
#define TARGET_PHYS_ADDR_MAX UINT32_MAX
#define TARGET_FMT_plx       "%08X"
#elif TARGET_PHYS_ADDR_BITS <= 64
typedef uint64_t target_phys_addr_t;
#define TARGET_PHYS_ADDR_MAX UINT64_MAX
#define TARGET_FMT_plx       "%016" PRIX64
#else
#error "Target physical address width is too big"
#endif
