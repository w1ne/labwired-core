#pragma once

#include <stdint.h>

#include "infrastructure.h"

//  Size in bytes of the tcg prologue, put at the end of the code_gen_buffer
#define TCG_PROLOGUE_SIZE 1024

extern uint8_t *tcg_rw_buffer;
extern uint8_t *tcg_rx_buffer;
extern intptr_t tcg_wx_diff;
extern uint64_t code_gen_buffer_size;

static inline bool is_ptr_in_rw_buf(const void *ptr)
{
    return (ptr >= (void *)tcg_rw_buffer) && (ptr < ((void *)tcg_rw_buffer + code_gen_buffer_size + TCG_PROLOGUE_SIZE));
}

static inline bool is_ptr_in_rx_buf(const void *ptr)
{
    return (ptr >= (void *)tcg_rx_buffer) && (ptr < ((void *)tcg_rx_buffer + code_gen_buffer_size + TCG_PROLOGUE_SIZE));
}

static inline void *rw_ptr_to_rx(void *ptr)
{
    if(ptr == NULL) {
        //  null pointers should not be changed
        return ptr;
    }
    tlib_assert(is_ptr_in_rx_buf(ptr - tcg_wx_diff));
    return ptr - tcg_wx_diff;
}

static inline void *rx_ptr_to_rw(const void *ptr)
{
    if(ptr == NULL) {
        //  null pointers should not be changed
        return (void *)ptr;
    }
    tlib_assert(is_ptr_in_rw_buf(ptr + tcg_wx_diff));
    return (void *)(ptr + tcg_wx_diff);
}

bool alloc_code_gen_buf(uint64_t size);
void free_code_gen_buf();
