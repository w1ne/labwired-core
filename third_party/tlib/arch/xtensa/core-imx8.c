#include "osdep.h"
#include "cpu.h"

#include "core-imx8/core-isa.h"
#include "overlay_tool.h"

#define xtensa_modules xtensa_modules_imx8
#include "core-imx8/xtensa-modules.c.inc"

XtensaConfig imx8
    __attribute__((unused)) = { .name = "imx8", .isa_internal = &xtensa_modules, .clock_freq_khz = 40000, DEFAULT_SECTIONS };
