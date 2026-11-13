# tock_veer_el2_sim before the HTIF console smoke

CPU/memory-only measurement (RV32IMC MUL word at 0x20600000, no peripherals
mapped) preserved before the chip gained Renode's HTIF console write window at
0xD0580000 and the smoke started printing through it.
