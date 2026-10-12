/* Test-only LD_PRELOAD probe: allocate a new owner at the just-freed JIT
 * address before an erroneous second unmap can race with it. */
#define _GNU_SOURCE
#include <stddef.h>
#include <stdint.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>
static void *replacement;
static int state; /* 0: not observed, 1: replacement owns map, 2: stolen */
int munmap(void *address, size_t length) {
    int result = syscall(SYS_munmap, address, length);
    /* Native buffers are powers of two plus the 1024-byte TCG prologue.
     * Ordinary ELF and stack unmaps use page-rounded lengths. */
    if (result == 0 && length >= 1024 * 1024 && length % 4096 == 1024) {
        if (state == 1 && address == replacement) {
            state = 2;
        } else if (state == 0) {
            replacement = mmap(address, length, PROT_READ | PROT_WRITE,
                               MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
            if (replacement != MAP_FAILED) {
                *(volatile uint8_t *)replacement = 0x42;
                state = 1;
            }
        }
    }
    return result;
}
int lw_check_unmap_replacement(void) {
    return state == 1 && *(volatile uint8_t *)replacement == 0x42 ? 0 : 1;
}
