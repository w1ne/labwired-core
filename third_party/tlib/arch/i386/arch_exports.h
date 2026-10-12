#pragma once

#include <stdint.h>

void tlib_set_cs_descriptor(uint32_t selector, uint32_t base, uint32_t limit, uint32_t flags);
uint64_t tlib_get_apic_base();
