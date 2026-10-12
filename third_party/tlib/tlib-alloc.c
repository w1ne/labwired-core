#if defined(__linux__)
#define _GNU_SOURCE
#endif
#if defined(__linux__) || defined(__APPLE__)
#include <sys/mman.h>
#elif defined(_WIN32)
#include <memoryapi.h>
#include <handleapi.h>
#endif
#if defined(__APPLE__)
#include <mach/mach.h>
#include <mach/mach_vm.h>
#endif

#include <unistd.h>
#include <stdint.h>
#include <errno.h>
#include <string.h>

#include "infrastructure.h"
#include "tlib-alloc.h"
#include "tcg/tcg.h"

uint8_t *tcg_rw_buffer;
uint8_t *tcg_rx_buffer;

uint64_t code_gen_buffer_size;

intptr_t tcg_wx_diff;

#if (defined(__linux__) || defined(__APPLE__)) && (!defined(__aarch64__))
static bool alloc_code_gen_buf_unified(uint64_t size)
{
    //  No write/execute splitting
    int flags = MAP_ANON | MAP_PRIVATE;
    void *rwx = mmap(NULL, size, PROT_READ | PROT_WRITE | PROT_EXEC, flags, -1, 0);
    if(rwx == MAP_FAILED) {
        tlib_printf(LOG_LEVEL_DEBUG, "Failed to mmap rwx buffer, error: %s", strerror(errno));
        return false;
    }
    tcg_rx_buffer = tcg_rw_buffer = rwx;
    tcg_wx_diff = 0;
    return true;
}
#endif
#if (defined(__linux__) || defined(__APPLE__))
void free_code_gen_buf()
{
    //  If not using split buffers the second one will fail, but this causes no issues
    munmap(tcg_rw_buffer, code_gen_buffer_size + TCG_PROLOGUE_SIZE);
    munmap(tcg_rx_buffer, code_gen_buffer_size + TCG_PROLOGUE_SIZE);
}
#endif

#if defined(__linux__) && defined(__aarch64__)
static bool alloc_code_gen_buf_split(uint64_t size)
{
    //  Split writable and executable mapping
    int fd = memfd_create("code_gen_buffer", 0);
    if(fd == -1) {
        tlib_abortf("Failed to create backing file for code_gen_buffer, error: %s", strerror(errno));
    }
    if(ftruncate(fd, size) == -1) {
        tlib_printf(LOG_LEVEL_DEBUG, "Failed to allocate %u bytes for codegen buffer, error: %s", size, strerror(errno));
        //  Cleanup the fd
        close(fd);
        return false;
    }
    //  Backing file creation succeded, mmap buffers
    int flags = MAP_SHARED;
    void *rw = mmap(NULL, size, PROT_READ | PROT_WRITE, flags, fd, 0);
    if(rw == MAP_FAILED) {
        tlib_printf(LOG_LEVEL_DEBUG, "Failed to mmap rw buffer, error: %s", strerror(errno));
        close(fd);
        return false;
    }
    void *rx = mmap(NULL, size, PROT_READ | PROT_EXEC, flags, fd, 0);
    if(rw == MAP_FAILED) {
        tlib_printf(LOG_LEVEL_DEBUG, "Failed to mmap rx buffer, error: %s", strerror(errno));
        close(fd);
        //  unmap region so it does not leak
        munmap(rw, size);
        return false;
    }
    //  Mapping succeded, we can now close the fd safely
    close(fd);
    tcg_rw_buffer = (uint8_t *)rw;
    tcg_rx_buffer = (uint8_t *)rx;
    tcg_wx_diff = tcg_rw_buffer - tcg_rx_buffer;
    return true;
}
#elif defined(__APPLE__) && defined(__aarch64__)
static bool alloc_code_gen_buf_split(uint64_t size)
{
    mach_vm_address_t rw, rx;

    int flags = MAP_ANONYMOUS | MAP_SHARED;
    rw = (mach_vm_address_t)mmap(NULL, size, PROT_READ | PROT_WRITE, flags, -1, 0);
    if(rw == (mach_vm_address_t)MAP_FAILED) {
        tlib_printf(LOG_LEVEL_ERROR, "Failed to mmap rw buffer, error: %s", strerror(errno));
        return false;
    }
    rx = 0;
    vm_prot_t current_prot, max_prot;
    kern_return_t res = mach_vm_remap(mach_task_self(), &rx, size, 0, VM_FLAGS_ANYWHERE, mach_task_self(), rw, false,
                                      &current_prot, &max_prot, VM_INHERIT_NONE);
    if(res != KERN_SUCCESS) {
        tlib_printf(LOG_LEVEL_ERROR, "Failed to mach_vm_remap rx buffer, error: %i", res);
        munmap((void *)rw, size);
        return false;
    }

    if(mprotect((void *)rx, size, PROT_READ | PROT_EXEC) != 0) {
        tlib_printf(LOG_LEVEL_ERROR, "Failed to mprotect rx buffer");
        //  Unmap the memory regions so they don't leak
        munmap((void *)rw, size);
        munmap((void *)rx, size);
        return false;
    }

    tcg_rw_buffer = (uint8_t *)rw;
    tcg_rx_buffer = (uint8_t *)rx;
    tcg_wx_diff = tcg_rw_buffer - tcg_rx_buffer;
    return true;
}
#elif defined(_WIN32)
static bool map_exec(void *addr, long size)
{
    DWORD old_protect;
    return (bool)VirtualProtect(addr, size, PAGE_EXECUTE_READWRITE, &old_protect);
}
static bool alloc_code_gen_buf_unified(uint64_t size)
{
    uint8_t *buf = tlib_malloc(size);
    if(buf == NULL) {
        return false;
    }
    if(!map_exec(buf, size)) {
        tlib_printf(LOG_LEVEL_ERROR, "Failed to VirtualProtect code_gen_buffer");
        return false;
    }
    tcg_rw_buffer = tcg_rx_buffer = buf;
    tcg_wx_diff = 0;
    return true;
}
void free_code_gen_buf()
{
    tlib_free(tcg_rw_buffer);
}
#endif

bool alloc_code_gen_buf(uint64_t size)
{
#if defined(__aarch64__)
    return alloc_code_gen_buf_split(size);
#else
    return alloc_code_gen_buf_unified(size);
#endif
}
