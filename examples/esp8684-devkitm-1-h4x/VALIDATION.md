# Validation scope

The committed fast-boot smoke loads a bare RV32IMAC ELF into the 4 MiB flash
window, captures the UART0 marker, observes GPIO0 edges, and stops at the
configured instruction limit. The UART model uses the C2's 120 MHz core clock,
40 MHz UART source clock, and UART0 matrix source 17. The GPIO and UART
register heads are shared with the C3 models after comparing the official C2
and C3 SVD definitions for the used registers.

This smoke does not execute the mask ROM or bootloader and does not model the
USB-to-UART bridge, IO_MUX pad routing, RGB LED electrical or optical behavior,
Wi-Fi, Bluetooth LE, or any other peripheral. No live-silicon differential was
run.

Vendor availability evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).
