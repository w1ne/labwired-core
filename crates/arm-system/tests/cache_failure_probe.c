/* Test-only LD_PRELOAD probe. Initial JIT allocation succeeds; fail the
 * larger growth allocation, optionally also failing its old-size fallback. */
#define _GNU_SOURCE
#include <errno.h>
#include <stddef.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>
static unsigned attempts, failures;
void *mmap(void *address, size_t length, int protection, int flags, int fd,
           off_t offset) {
    /* JIT mappings are powers of two plus the 1024-byte prologue. Ordinary
     * ELF mappings are page rounded and are deliberately unaffected. */
    if (length >= 8 * 1024 * 1024 && length % 4096 == 1024 &&
        protection == (PROT_READ | PROT_WRITE | PROT_EXEC) &&
        (flags & MAP_ANONYMOUS)) {
        attempts++;
        const char *mode = getenv("LABWIRED_NATIVE_CACHE_FAILURE");
        if (attempts == 2 || (attempts == 3 && mode && mode[0] == 'd')) {
            failures++;
            errno = ENOMEM;
            return MAP_FAILED;
        }
    }
    return (void *)syscall(SYS_mmap, address, length, protection, flags, fd, offset);
}
unsigned lw_cache_failure_count(void) { return failures; }
