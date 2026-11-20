# core-v-mcu stdout negative control

Same firmware and smoke with the `stdout` (PULP_STDOUT @0x1A10FF80) peripheral
removed from the chip descriptor. The run fails 1/5 with a memory access
violation at 0x1A10FF80, so the passing smoke depends on the mapped window.
