Negative control (2026-11-09): system board_io b1 active_high false
(the repl's invert: true). B1 idles high, so check_b1's idle/press waits
burn their 2M-iteration budgets and the run reaches max_steps before the
b1 line; summary never written (0xA5A5A5A5). FAIL 6/8 (uart_ordered,
memory_value).
