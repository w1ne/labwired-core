# iCE40UP5K MDP before the timer0 smoke

CPU/memory-only measurement (RV32IM MUL word at 0x10000000, no peripherals
mapped) preserved before the chip gained timer0 from the included Renode
`cpus/fomu.repl` and the smoke started checking it.
