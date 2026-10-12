#include "osdep.h"
#include "cpu.h"

#include "core-imx8m/core-isa.h"
#include "overlay_tool.h"

#define xtensa_modules xtensa_modules_imx8m
#include "core-imx8m/xtensa-modules.c.inc"

XtensaConfig imx8m
    __attribute__((unused)) = { .name = "imx8m", .isa_internal = &xtensa_modules, .clock_freq_khz = 40000, DEFAULT_SECTIONS };
