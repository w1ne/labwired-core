The Cortex-M0 stack-frame smoke built and booted successfully, and every stack
assertion reported `frame=1`. This replay still failed the ordered UART clause:
the assertion expected `pc=` immediately after the four saved register values,
but the firmware prints the stacked LR between them. The output and metrics
are preserved here before correcting the expected token sequence.
