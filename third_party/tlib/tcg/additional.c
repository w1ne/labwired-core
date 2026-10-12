/*
 * Copyright (c) 2010-2025 Antmicro Ltd <www.antmicro.com>
 * Copyright (c) 2011-2015 Realtime Embedded AB <www.rte.se>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
 * THE SOFTWARE.
 */

#include <stdint.h>
#include <string.h>
#include "tcg.h"

tcg_t *tcg;

void *(*_TCG_malloc)(size_t);

void attach_malloc(void *malloc_callback)
{
    _TCG_malloc = malloc_callback;
}

void *TCG_malloc(size_t size)
{
    return _TCG_malloc(size);
}

void *(*_TCG_realloc)(void *, size_t);

void attach_realloc(void *reall)
{
    _TCG_realloc = reall;
}

void *TCG_realloc(void *ptr, size_t size)
{
    return _TCG_realloc(ptr, size);
}

void (*_TCG_free)(void *ptr);

void attach_free(void *free_callback)
{
    _TCG_free = free_callback;
}

void TCG_free(void *ptr)
{
    _TCG_free(ptr);
}

void TCG_pstrcpy(char *buf, int buf_size, const char *str)
{
    int c;
    char *q = buf;

    if(buf_size <= 0) {
        return;
    }

    for(;;) {
        c = *str++;
        if(c == 0 || q >= buf + buf_size - 1) {
            break;
        }
        *q++ = c;
    }
    *q = '\0';
}

char *TCG_pstrcat(char *buf, int buf_size, const char *s)
{
    int len;
    len = strlen(buf);
    if(len < buf_size) {
        TCG_pstrcpy(buf + len, buf_size - len, s);
    }
    return buf;
}

unsigned int temp_buf_offset;
unsigned int tlb_table_n_0[MMU_MODES_MAX];
unsigned int tlb_table_n_0_addr_read[MMU_MODES_MAX];
unsigned int tlb_table_n_0_addr_write[MMU_MODES_MAX];
unsigned int tlb_table_n_0_addend[MMU_MODES_MAX];
unsigned int tlb_entry_addr_read;
unsigned int tlb_entry_addr_write;
unsigned int tlb_entry_addend;
unsigned int sizeof_CPUTLBEntry;
int TARGET_PAGE_BITS;

void set_TARGET_PAGE_BITS(int val)
{
    TARGET_PAGE_BITS = val;
}

void set_sizeof_CPUTLBEntry(unsigned int sz)
{
    sizeof_CPUTLBEntry = sz;
}

void set_temp_buf_offset(unsigned int offset)
{
    temp_buf_offset = offset;
}

void set_tlb_entry_addr_rwu(unsigned int read, unsigned int write, unsigned int addend)
{
    tlb_entry_addr_read = read;
    tlb_entry_addr_write = write;
    tlb_entry_addend = addend;
}

void set_tlb_table_n_0(int i, unsigned int offset)
{
    tlb_table_n_0[i] = offset;
}

void set_tlb_table_n_0_rwa(int i, unsigned int read, unsigned int write, unsigned int addend)
{
    tlb_table_n_0_addr_read[i] = read;
    tlb_table_n_0_addr_write[i] = write;
    tlb_table_n_0_addend[i] = addend;
}

#ifdef GENERATE_PERF_MAP

#include <unistd.h>
#include "../include/infrastructure.h"

//  not thread safe - it relies on the property of Renode, that it disposes of machines sequentially
static FILE *perf_map_handle;

struct TranslationBlock;
typedef struct tcg_perf_map_symbol {
    void *ptr;
    int size;
    char *label;
    bool reused;
    struct TranslationBlock *tb;

    struct tcg_perf_map_symbol *left, *right;
    int height;
} tcg_perf_map_symbol;

//  self balancing (AVL) tree
static tcg_perf_map_symbol *symbols = NULL;

/* Left/Right Rotation
            |            Right               |
            s           <=====               m
           / \           Left               / \
          l   m         =====>             s   mr
             / \                          / \
            ml  mr                       l   ml
 */

static inline int max(int a, int b)
{
    return a > b ? a : b;
}

static inline int tcg_perf_tree_height(tcg_perf_map_symbol *s)
{
    return s == NULL ? 0 : s->height;
}

static tcg_perf_map_symbol *tcg_perf_tree_left_rotate(tcg_perf_map_symbol *s)
{
    if(s == NULL) {
        return NULL;
    }
    if(unlikely(s->right == NULL)) {
        return s;
    }

    tcg_perf_map_symbol *l = s->left;
    tcg_perf_map_symbol *m = s->right;

    s->right = m->left;
    m->left = s;

    s->height = max(tcg_perf_tree_height(l), tcg_perf_tree_height(s->right)) + 1;
    m->height = max(tcg_perf_tree_height(s), tcg_perf_tree_height(m->right)) + 1;

    return m;
}

static tcg_perf_map_symbol *tcg_perf_tree_right_rotate(tcg_perf_map_symbol *m)
{
    if(m == NULL) {
        return NULL;
    }
    if(unlikely(m->left == NULL)) {
        return m;
    }

    tcg_perf_map_symbol *s = m->left;
    tcg_perf_map_symbol *mr = m->right;

    m->left = s->right;
    s->right = m;

    m->height = max(tcg_perf_tree_height(mr), tcg_perf_tree_height(m->left)) + 1;
    s->height = max(tcg_perf_tree_height(m), tcg_perf_tree_height(s->left)) + 1;

    return s;
}

#define RETURN_EMPTY_PERF_HANDLE            \
    if(unlikely(perf_map_handle == NULL)) { \
        return;                             \
    }

void tcg_perf_init_labeling()
{
    char target[30];

    snprintf(target, sizeof(target), "/tmp/perf-%d.map", getpid());
    perf_map_handle = fopen(target, "a");

    if(unlikely(perf_map_handle == NULL)) {
        tlib_printf(LOG_LEVEL_WARNING, "Cannot generate perf.map.");
    }
}

static void tcg_perf_symbol_tree_recurse_helper(tcg_perf_map_symbol *s, void (*callback)(tcg_perf_map_symbol *))
{
    if(s == NULL) {
        return;
    }

    tcg_perf_symbol_tree_recurse_helper(s->left, callback);
    tcg_perf_symbol_tree_recurse_helper(s->right, callback);
    (*callback)(s);
}

static inline void tcg_perf_flush_symbol(tcg_perf_map_symbol *s)
{
    if(s->label) {
        fprintf(perf_map_handle, "%p %x %stcg_jit_code:%p:%s", s->ptr, s->size, s->reused ? "[REUSED]" : "", s->ptr, s->label);
    } else {
        fprintf(perf_map_handle, "%p %x %stcg_jit_code:%p", s->ptr, s->size, s->reused ? "[REUSED]" : "", s->ptr);
    }

    if(s->tb) {
        char *buffer = TCG_malloc(sizeof(char) * 100);
        tcg_perf_tb_info_to_string(s->tb, buffer, 100);
        fprintf(perf_map_handle, "%s", buffer);
        TCG_free(buffer);
    }

    fprintf(perf_map_handle, "\n");
}

static inline void tcg_perf_free_symbol(tcg_perf_map_symbol *s)
{
    if(s->label) {
        TCG_free(s->label);
    }
    TCG_free(s);
}

void tcg_perf_flush_map()
{
    RETURN_EMPTY_PERF_HANDLE;
    tcg_perf_symbol_tree_recurse_helper(symbols, tcg_perf_flush_symbol);
    tcg_perf_symbol_tree_recurse_helper(symbols, tcg_perf_free_symbol);
    symbols = NULL;
}

void tcg_perf_fini_labeling()
{
    if(likely(perf_map_handle != NULL)) {
        tcg_perf_flush_map();
        fclose(perf_map_handle);
    }
}

static int tcg_perf_tree_get_balance_factor(tcg_perf_map_symbol *node)
{
    if(node == NULL) {
        return 0;
    }
    int leftH = node->left == NULL ? 0 : tcg_perf_tree_height(node->left);
    int rightH = node->right == NULL ? 0 : tcg_perf_tree_height(node->right);
    return rightH - leftH;
}

typedef struct tree_walk_trace {
    tcg_perf_map_symbol **node;
    struct tree_walk_trace *next;
} tree_walk_trace;

static inline tree_walk_trace *trace_step(tree_walk_trace *trace)
{
    tree_walk_trace *next = trace->next;
    TCG_free(trace);
    return next;
}

void tcg_perf_append_symbol(tcg_perf_map_symbol *s)
{
    tcg_perf_map_symbol **next = &symbols;
    tree_walk_trace *head = NULL;

    while(*next != NULL) {
        tree_walk_trace *tmp = TCG_malloc(sizeof(tree_walk_trace));
        tmp->node = next;
        tmp->next = head;
        head = tmp;

        if(s->ptr == (*next)->ptr) {
            s->reused = true;
            s->left = (*next)->left;
            s->right = (*next)->right;
            s->height = (*next)->height;
            s->tb = (*next)->tb;
            tcg_perf_free_symbol(*next);

            //  Dispose of trace
            while(head != NULL) {
                head = trace_step(head);
            }

            break;
        } else if(s->ptr < (*next)->ptr) {
            next = &(*next)->left;
        } else {
            next = &(*next)->right;
        }
    }

    *next = s;
    if(head == NULL) {
        return;
    }

    //  Step one up
    head = trace_step(head);

    //  Rebalance tree and cleanup trace
    while(head != NULL) {
        tcg_perf_map_symbol *current = *(head->node);

        current->height = max(tcg_perf_tree_height(current->left), tcg_perf_tree_height(current->right)) + 1;
        int balance_factor = tcg_perf_tree_get_balance_factor(current);

        if(balance_factor < -1) {  //  Left-heavy
            if(tcg_perf_tree_get_balance_factor(current->left) >= 1) {
                //  Left Right
                current->left = tcg_perf_tree_left_rotate(current->left);
                current = tcg_perf_tree_right_rotate(current);
            } else {
                //  Left Left
                current = tcg_perf_tree_right_rotate(current);
            }
        } else if(balance_factor > 1) {  //  Right-heavy
            if(tcg_perf_tree_get_balance_factor(current->right) <= -1) {
                //  Right Left
                current->right = tcg_perf_tree_right_rotate(current->right);
                current = tcg_perf_tree_left_rotate(current);
            } else {
                //  Right Right
                current = tcg_perf_tree_left_rotate(current);
            }
        }

        *(head->node) = current;
        head = trace_step(head);
    }
}

void tcg_perf_out_symbol_s(void *s, int size, const char *label, struct TranslationBlock *tb)
{
    RETURN_EMPTY_PERF_HANDLE;
    char *_label = NULL;

    if(label) {
        size_t len = strlen(label) + 1;
        _label = TCG_malloc(sizeof(char) * len);
        strncpy(_label, label, len);
    }

    tcg_perf_map_symbol *symbol = TCG_malloc(sizeof(tcg_perf_map_symbol));
    *symbol = (tcg_perf_map_symbol) { .ptr = s,
                                      .size = size,
                                      .label = _label,
                                      .reused = false,
                                      .tb = tb,
                                      .left = NULL,
                                      .right = NULL,
                                      .height = 1 };
    tcg_perf_append_symbol(symbol);
}

void tcg_perf_out_symbol(void *s, int size, struct TranslationBlock *tb)
{
    RETURN_EMPTY_PERF_HANDLE;

    tcg_perf_out_symbol_s(s, size, NULL, tb);
}

void tcg_perf_out_symbol_i(void *s, int size, int label, struct TranslationBlock *tb)
{
    RETURN_EMPTY_PERF_HANDLE;
    char buffer[20];

    snprintf(buffer, sizeof(buffer), "%x", label);
    tcg_perf_out_symbol_s(s, size, buffer, tb);
}

#undef RETURN_EMPTY_PERF_HANDLE

#else
inline void tcg_perf_init_labeling() { }

inline void tcg_perf_fini_labeling() { }
void tcg_perf_flush_map() { }
inline void tcg_perf_out_symbol(void *s, int size, struct TranslationBlock *tb) { }

inline void tcg_perf_out_symbol_s(void *s, int size, const char *label, struct TranslationBlock *tb) { }

inline void tcg_perf_out_symbol_i(void *s, int size, int label, struct TranslationBlock *tb) { }
#endif
