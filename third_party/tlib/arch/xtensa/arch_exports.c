#include "arch_exports.h"
#include "cpu.h"
#include "unwind.h"

void tlib_set_irq_pending_bit(uint32_t irq, uint32_t value)
{
    xtensa_cpu_set_irq_pending_bit(env, irq, value);
}

EXC_VOID_2(tlib_set_irq_pending_bit, uint32_t, irq, uint32_t, value)

void tlib_set_single_step(uint32_t enabled)
{
    cpu->singlestep_enabled = enabled;
}

EXC_VOID_1(tlib_set_single_step, uint32_t, enabled)
