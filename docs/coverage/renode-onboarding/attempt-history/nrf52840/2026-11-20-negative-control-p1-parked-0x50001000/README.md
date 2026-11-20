# Negative control: P1 parked at 0x50001000 (pre-fix descriptor)

Same firmware as the passing run (`firmware-nrf52840-renode-smoke` with
`check_gpio_p1`), chip descriptor from `HEAD` before this change (gpio0 4 KB
window, gpio1 at 0x50001000), in a /tmp system copy.

Result: FAIL 5/7. UART: `gpio p1 in=00 drive=0 pull=0 gpiote=0 p0=1 FAIL`,
summary 0x3FFF (bit 14 clear). Firmware P1 accesses at 0x50000804.. land in
gpio0's window and are lost. `gpio_edges gpio1:3` still passed: GPIOTE
resolves gpio1 through ChipMap, so the pad toggled at the parked port while
firmware could not read it at the silicon address.
