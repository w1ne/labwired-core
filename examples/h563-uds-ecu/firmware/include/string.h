#ifndef LABWIRED_FREESTANDING_STRING_H
#define LABWIRED_FREESTANDING_STRING_H

#include <stddef.h>

void *memcpy(void *dst, const void *src, size_t n);
void *memset(void *dst, int value, size_t n);
int memcmp(const void *lhs, const void *rhs, size_t n);

#endif
