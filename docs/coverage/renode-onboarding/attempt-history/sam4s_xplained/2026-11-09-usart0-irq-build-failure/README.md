# Failed attempt: SAM4S Xplained USART0 IRQ smoke build

The one-target inventory run stopped at `build_firmware`; no firmware was
executed. The fixture passed integer addresses to pointer-based MMIO helpers.
The pipeline metrics and compiler log are retained here so the failed attempt
is not hidden by the corrected retry.
