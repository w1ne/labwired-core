# LiteX Ibex: STM32 timer fallback exposed

The tightened smoke (W1C ev_pending, load readback, update_value/value latch)
run against the UNCHANGED descriptor (`type: litex_timer_csr32`, resolved to
the STM32 `timer` model). Result: uart_ordered assertion failure with UART
`timer=pending_kept`, `load=00000220` (CR1 write mask), `value=00000000`.
The old `timer=zero` line still prints, which is the false pass the previous
smoke reported. Build logs dropped; run log and simulation output kept.
