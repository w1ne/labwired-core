/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdbool.h>
#include <stddef.h>  // NULL
#include <stdint.h>
#include <string.h>  // memset

#include "callbacks.h"
#include "infrastructure.h"

typedef struct {
    void *key;
    void *value;
} TTable_entry;

/*
 * This should behave like standard library comparison functions:
 *  * If `entry` is greater than `value` return 1
 *  * Else If `entry` is less than `value` return -1
 *  * Else return 0
 */
typedef int TTableEntryCompareFn(TTable_entry entry, const void *value);
typedef void TTableEntryRemoveCallback(TTable_entry *entry);
typedef bool ValuesArrayTrySetEntryFn(void **array_entry, void *value);

typedef struct {
    uint32_t count;
    TTable_entry *entries;
    TTableEntryRemoveCallback *entry_remove_callback;
    TTableEntryCompareFn *key_compare_function;
    uint32_t size;
    bool sorted;
} TTable;

//  Only one of these functions will be used for the given TTable depending on what TTable_entry's key is.
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-function"
static int ttable_compare_key_pointer(TTable_entry entry, const void *value)
{
    const void *val1 = entry.key;
    const void *val2 = value;
    return val1 > val2 ? 1 : (val1 < val2 ? -1 : 0);
}

static int ttable_compare_key_string(TTable_entry entry, const void *value)
{
    return strcmp(entry.key, value);
}

static int ttable_compare_key_uint32(TTable_entry entry, const void *value)
{
    uint32_t val1 = *(uint32_t *)entry.key;
    uint32_t val2 = *(uint32_t *)value;
    return val1 > val2 ? 1 : (val1 < val2 ? -1 : 0);
}
#pragma GCC diagnostic pop

static inline void ttable_sort_by_keys(TTable *ttable)
{
    tlib_assert(ttable != NULL);

    //  O(n^2) sort
    for(int i = 0; i < ttable->count - 1; ++i) {
        for(int j = i + 1; j < ttable->count; ++j) {
            if(ttable->key_compare_function(ttable->entries[i], ttable->entries[j].key) > 0) {
                TTable_entry tmp = ttable->entries[i];
                ttable->entries[i] = ttable->entries[j];
                ttable->entries[j] = tmp;
            }
        }
    }

    ttable->sorted = true;
}

static inline TTable *ttable_create(uint32_t entries_max, TTableEntryRemoveCallback *entry_remove_callback,
                                    TTableEntryCompareFn *key_compare_function)
{
    size_t ttable_size = entries_max * sizeof(TTable_entry);
    void *ttable_entries = tlib_mallocz(ttable_size);

    TTable *ttable = tlib_mallocz(sizeof(TTable));
    ttable->count = 0;
    ttable->entries = ttable_entries;
    ttable->entry_remove_callback = entry_remove_callback;
    ttable->key_compare_function = key_compare_function;
    ttable->size = entries_max;
    ttable->sorted = false;
    return ttable;
}

/*
 * Returns entries count.
 */
static inline uint32_t ttable_create_values_array(TTable *ttable, void ***array_pointer,
                                                  ValuesArrayTrySetEntryFn *try_set_array_entry)
{
    tlib_assert(array_pointer != NULL && ttable != NULL);

    if(ttable->count == 0) {
        return 0;
    }

    *array_pointer = tlib_malloc(sizeof(ttable->entries->value) * ttable->count);

    int i, array_index = 0;
    for(i = 0; i < ttable->count; i++) {
        void **array_entry = &(*array_pointer)[array_index];
        bool selected = try_set_array_entry(array_entry, ttable->entries[i].value);
        if(selected) {
            array_index++;
        }
    }
    return array_index;
}

/* Inserting many entries to the sorted TTable will be painfully slow, as it will sort automatically after each insert.
 * It might be a better idea, to change `ttable->sorted` to `false`, insert a batch of new entries
 * and then sort the whole TTable again
 */
static inline void ttable_insert(TTable *ttable, void *key, void *value)
{
    tlib_assert(ttable->count < ttable->size);

    uint32_t first_free_entry_id = ttable->count;
    ttable->entries[first_free_entry_id].key = key;
    ttable->entries[first_free_entry_id].value = value;

    ttable->count++;

    if(ttable->sorted) {
        //  This can be optimized if needed
        //  The current assumption is that there will be not so many insertions after first sort
        ttable_sort_by_keys(ttable);
    }
}

static inline TTable_entry *ttable_lookup_custom(TTable *ttable, TTableEntryCompareFn *entry_compare_function,
                                                 const void *compare_value)
{
    /* The TTable, if sorted, is sorted using the default compare function so the binary
     * search won't work properly if a different compare function is used.
     */
    if(ttable->sorted && entry_compare_function == ttable->key_compare_function) {
        //  Binary search
        /*
         * Even though all valid indices would fit in uint32_t, int64_t is used.
         * Using uint32_t could cause the value to be wrapped around to the maximum
         * (if `right` is 0 and `entry_compare_function` returns a positive value)
         * causing an out of bounds access to `ttable->entries` on the next loop iteration.
         * With a signed integer that operation will result in `right` having a value of -1
         * and the termination of the loop. int64_t was selected, because it can represent
         * all possible values of uint32_t
         */
        int64_t left = 0, right = ttable->count - 1;
        while(left <= right) {
            int64_t pivot = (left + right) / 2;
            TTable_entry *entry = &ttable->entries[pivot];
            if(entry_compare_function(*entry, compare_value) < 0) {
                left = pivot + 1;
            } else if(entry_compare_function(*entry, compare_value) > 0) {
                right = pivot - 1;
            } else {
                return entry;
            }
        }
    } else {
        //  Linear search
        for(uint32_t i = 0; i < ttable->count; i++) {
            TTable_entry *entry = &ttable->entries[i];
            if(entry_compare_function(*entry, compare_value) == 0) {
                return entry;
            }
        }
    }

    //  The search was unsuccesful.
    return NULL;
}

static inline TTable_entry *ttable_lookup(TTable *ttable, void *key)
{
    return ttable_lookup_custom(ttable, ttable->key_compare_function, key);
}

static inline void *ttable_lookup_value_eq(TTable *ttable, void *key)
{
    TTable_entry *entry = ttable_lookup(ttable, key);
    if(entry) {
        return entry->value;
    }
    return NULL;
}

static inline bool ttable_insert_check(TTable *ttable, void *key, void *value)
{
    if(ttable_lookup(ttable, key)) {
        return false;
    }

    ttable_insert(ttable, key, value);
    return true;
}

static inline void ttable_remove(TTable *ttable)
{
    if(ttable->entry_remove_callback) {
        int i;
        for(i = 0; i < ttable->count; i++) {
            ttable->entry_remove_callback(&ttable->entries[i]);
        }
    }

    tlib_free(ttable->entries);
    tlib_free(ttable);
}
